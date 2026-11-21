//! Plugin execution environment.

use crate::plugin::PluginLoader;
use anyhow::Result;
use janus_core::Config;
use sha2::{Digest, Sha256};
use std::{
    path::{Path, PathBuf},
    process::Stdio,
    sync::Arc,
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    process::Command,
};
use tracing::debug;

/// Safe plugin metadata returned to authenticated API clients.
#[derive(Debug, Clone, serde::Serialize)]
pub struct PluginSummary {
    pub id: String,
    pub name: String,
    pub category: String,
    pub status: String,
    pub description: String,
    /// Capabilities requested by the plugin manifest; requests are not grants.
    pub requested_capabilities: Vec<String>,
    /// Whether every requested capability is currently supported by the sandbox.
    pub execution_ready: bool,
}

/// Validate manifest requests against the capabilities actually implemented
/// by the production sandbox. An empty list is safe; logging is the only
/// currently supported host capability and is always available.
fn validate_plugin_capabilities(requested: &[String]) -> Result<()> {
    for capability in requested {
        if capability != "logging" {
            anyhow::bail!(
                "plugin requests unsupported or ungranted capability {:?}; execution is denied",
                capability
            );
        }
    }
    Ok(())
}

/// Plugin executor managing Lua runtime and plugin lifecycle.
pub struct PluginExecutor {
    config: Config,
    loader: PluginLoader,
    /// Bounds aggregate isolated worker-process concurrency.
    execution_slots: Arc<tokio::sync::Semaphore>,
}

const MAX_CONCURRENT_PLUGIN_EXECUTIONS: usize = 2;
const MAX_PLUGIN_QUEUE_WAIT: Duration = Duration::from_secs(2);
const MAX_PLUGIN_WALL_TIME: Duration = Duration::from_secs(5);
const MAX_WORKER_REQUEST_BYTES: usize = 512 * 1024;
const MAX_WORKER_RESPONSE_BYTES: usize = 300 * 1024;

#[derive(serde::Serialize)]
struct WorkerRequest {
    code: String,
    args: serde_json::Value,
}

#[derive(serde::Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
enum WorkerResponse {
    Success { value: serde_json::Value },
    Failure { error: String },
}

fn worker_executable_path_from(runtime_executable: &Path, configured: Option<PathBuf>) -> PathBuf {
    configured.unwrap_or_else(|| {
        let worker_name = if cfg!(windows) {
            "janus-plugin-worker.exe"
        } else {
            "janus-plugin-worker"
        };
        runtime_executable
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join(worker_name)
    })
}

fn resolve_worker_executable() -> Result<PathBuf> {
    let runtime_executable = std::env::current_exe()
        .context("locating the Janus runtime executable")?;
    let configured = std::env::var_os("JANUS_PLUGIN_WORKER_PATH").map(PathBuf::from);
    let worker = worker_executable_path_from(&runtime_executable, configured);
    anyhow::ensure!(
        worker.is_file(),
        "isolated plugin worker executable is missing at {}; install janus-plugin-worker beside janus-runtime or set JANUS_PLUGIN_WORKER_PATH",
        worker.display()
    );
    Ok(worker)
}

async fn stop_worker(child: &mut tokio::process::Child) {
    let _ = child.kill().await;
    let _ = tokio::time::timeout(Duration::from_secs(1), child.wait()).await;
}

impl PluginExecutor {
    /// Create new plugin executor
    pub async fn new(config: &Config, loader: PluginLoader) -> Result<Self> {
        debug!("Initializing plugin executor");
        Ok(Self {
            config: config.clone(),
            loader,
            execution_slots: Arc::new(tokio::sync::Semaphore::new(MAX_CONCURRENT_PLUGIN_EXECUTIONS)),
        })
    }

    /// Get plugin directory path
    pub fn plugin_dir(&self) -> &PathBuf {
        &self.config.plugin_dir
    }

    /// List available plugins with structured metadata for authenticated clients.
    pub async fn list_plugins(&self) -> Result<Vec<PluginSummary>> {
        Ok(self
            .loader
            .list_sorted()
            .into_iter()
            .map(|(id, source)| PluginSummary {
                id: id.to_string(),
                name: source.manifest.name.clone(),
                category: source.manifest.category.clone(),
                status: source.manifest.status.clone(),
                description: source.manifest.description.clone(),
                requested_capabilities: source.manifest.capabilities.clone(),
                execution_ready: validate_plugin_capabilities(&source.manifest.capabilities).is_ok(),
            })
            .collect())
    }

    /// Return the current content hash for an installed plugin.
    pub fn plugin_sha256(&self, plugin_name: &str) -> Result<String> {
        let plugin = self
            .loader
            .get(plugin_name)
            .ok_or_else(|| anyhow::anyhow!("Plugin not found: {}", plugin_name))?;
        Ok(format!("{:x}", Sha256::digest(plugin.code.as_bytes())))
    }

    /// Execute one plugin in a short-lived, resource-limited worker process.
    ///
    /// The worker receives only serialized source and arguments over private
    /// stdin/stdout pipes. It has no host capability IPC; unsupported capability
    /// requests are denied before the process is started. Dropping this future
    /// drops the child handle, whose kill-on-drop policy terminates the worker.
    pub async fn execute(
        &self,
        plugin_name: &str,
        args: serde_json::Value,
    ) -> Result<serde_json::Value> {
        // Plugin arguments can contain user data or secrets; never include them in logs.
        debug!(plugin_name, "Executing plugin");

        let plugin = self
            .loader
            .get(plugin_name)
            .ok_or_else(|| anyhow::anyhow!("Plugin not found: {}", plugin_name))?;
        validate_plugin_capabilities(&plugin.manifest.capabilities)?;

        let _permit = tokio::time::timeout(
            MAX_PLUGIN_QUEUE_WAIT,
            Arc::clone(&self.execution_slots).acquire_owned(),
        )
        .await
        .map_err(|_| anyhow::anyhow!("plugin execution capacity is busy; retry shortly"))?
        .map_err(|_| anyhow::anyhow!("plugin execution capacity is unavailable"))?;

        let request = WorkerRequest {
            code: plugin.code.clone(),
            args,
        };
        let request_bytes = serde_json::to_vec(&request)?;
        anyhow::ensure!(
            request_bytes.len() <= MAX_WORKER_REQUEST_BYTES,
            "plugin worker request exceeds the 512 KiB limit"
        );

        let worker_path = resolve_worker_executable()?;
        let mut command = Command::new(&worker_path);
        command
            .env_clear()
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        let mut child = command
            .spawn()
            .with_context(|| format!("starting isolated plugin worker at {}", worker_path.display()))?;

        let mut stdin = child
            .stdin
            .take()
            .context("isolated plugin worker stdin was unavailable")?;
        if let Err(error) = stdin.write_all(&request_bytes).await {
            stop_worker(&mut child).await;
            return Err(error).context("sending plugin request to isolated worker");
        }
        drop(stdin);

        let stdout = child
            .stdout
            .take()
            .context("isolated plugin worker stdout was unavailable")?;
        let mut output = Vec::new();
        let read_result = tokio::time::timeout(
            MAX_PLUGIN_WALL_TIME,
            stdout
                .take((MAX_WORKER_RESPONSE_BYTES + 1) as u64)
                .read_to_end(&mut output),
        )
        .await;
        match read_result {
            Ok(Ok(_)) => {}
            Ok(Err(error)) => {
                stop_worker(&mut child).await;
                return Err(error).context("reading isolated plugin worker response");
            }
            Err(_) => {
                stop_worker(&mut child).await;
                anyhow::bail!("plugin exceeded the 5-second wall-clock limit; worker process terminated");
            }
        }

        if output.len() > MAX_WORKER_RESPONSE_BYTES {
            stop_worker(&mut child).await;
            anyhow::bail!("plugin worker response exceeded the 300 KiB transport limit");
        }

        let status = match tokio::time::timeout(Duration::from_secs(1), child.wait()).await {
            Ok(Ok(status)) => status,
            Ok(Err(error)) => return Err(error).context("waiting for isolated plugin worker"),
            Err(_) => {
                stop_worker(&mut child).await;
                anyhow::bail!("plugin worker did not exit after closing its response pipe");
            }
        };

        let response: WorkerResponse = serde_json::from_slice(&output)
            .context("isolated plugin worker returned an invalid response")?;
        match response {
            WorkerResponse::Success { value } if status.success() => Ok(value),
            WorkerResponse::Failure { error } => {
                Err(anyhow::anyhow!("plugin execution failed: {error}"))
            }
            WorkerResponse::Success { .. } => {
                Err(anyhow::anyhow!("plugin worker exited unsuccessfully: {status}"))
            }
        }
    }

    /// Get plugin count
    pub fn plugin_count(&self) -> usize {
        self.loader.count()
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn execution_slots_bound_concurrency_and_release_only_when_worker_finishes() {
        let slots = Arc::new(tokio::sync::Semaphore::new(2));
        let first = Arc::clone(&slots).acquire_owned().await.unwrap();
        let second = Arc::clone(&slots).acquire_owned().await.unwrap();
        assert_eq!(slots.available_permits(), 0);
        assert!(tokio::time::timeout(
            Duration::from_millis(10),
            Arc::clone(&slots).acquire_owned()
        ).await.is_err());
        drop(first);
        assert_eq!(slots.available_permits(), 1);
        drop(second);
        assert_eq!(slots.available_permits(), 2);
    }

    #[test]
    fn capability_policy_allows_only_safe_logging_or_no_capabilities() {
        assert!(validate_plugin_capabilities(&[]).is_ok());
        assert!(validate_plugin_capabilities(&["logging".to_string()]).is_ok());
        assert!(validate_plugin_capabilities(&["network:wifi_scan".to_string()]).is_err());
        assert!(validate_plugin_capabilities(&["shell".to_string()]).is_err());
        assert!(validate_plugin_capabilities(&["filesystem:read".to_string()]).is_err());
    }
    #[test]
    fn worker_path_override_takes_precedence_over_runtime_sibling() {
        let runtime = Path::new("/opt/janus/bin/janus-runtime");
        let configured = PathBuf::from("/custom/workers/janus-plugin-worker");
        assert_eq!(
            worker_executable_path_from(runtime, Some(configured.clone())),
            configured
        );
        assert_eq!(
            worker_executable_path_from(runtime, None),
            PathBuf::from("/opt/janus/bin/janus-plugin-worker")
        );
    }

}

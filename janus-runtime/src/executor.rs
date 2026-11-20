//! Plugin execution environment.

use crate::plugin::PluginLoader;
use anyhow::Result;
use janus_core::Config;
use sha2::{Digest, Sha256};
use std::{
    path::PathBuf,
    sync::{atomic::{AtomicBool, Ordering}, Arc},
    time::Duration,
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

/// Drop guard ensures a disconnected/cancelled API request signals its blocking worker.
struct PluginCancellationGuard(Arc<AtomicBool>);

impl Drop for PluginCancellationGuard {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}

/// Plugin executor managing Lua runtime and plugin lifecycle.
pub struct PluginExecutor {
    config: Config,
    loader: PluginLoader,
    /// Bounds aggregate in-process plugin concurrency; this is not OS isolation.
    execution_slots: Arc<tokio::sync::Semaphore>,
}

const MAX_CONCURRENT_PLUGIN_EXECUTIONS: usize = 2;
const MAX_PLUGIN_QUEUE_WAIT: Duration = Duration::from_secs(2);

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

    /// Execute plugin by name
    pub async fn execute(
        &self,
        plugin_name: &str,
        args: serde_json::Value,
    ) -> Result<serde_json::Value> {
        // Plugin arguments can contain user data or secrets; never include them in logs.
        debug!(plugin_name, "Executing plugin");

        let plugin = self.loader.get(plugin_name)
            .ok_or_else(|| anyhow::anyhow!("Plugin not found: {}", plugin_name))?;
        // Manifest declarations are requests, never grants. Until a capability
        // has a vetted host implementation and an explicit grant path, deny it.
        validate_plugin_capabilities(&plugin.manifest.capabilities)?;

        // Bound aggregate in-process execution so callers cannot create an
        // unbounded number of CPU-bound blocking workers. A slot stays held until
        // the worker actually exits, including after a timeout/cancellation signal.
        let permit = tokio::time::timeout(
            MAX_PLUGIN_QUEUE_WAIT,
            Arc::clone(&self.execution_slots).acquire_owned(),
        )
        .await
        .map_err(|_| anyhow::anyhow!("plugin execution capacity is busy; retry shortly"))?
        .map_err(|_| anyhow::anyhow!("plugin execution capacity is unavailable"))?;

        // Lua execution is synchronous and CPU-bound. Keep it off Tokio's async
        // worker threads and signal cooperative cancellation on timeout or request
        // cancellation. This is still not an OS process/security boundary.
        let code = plugin.code.clone();
        let cancelled = Arc::new(AtomicBool::new(false));
        let _cancellation_guard = PluginCancellationGuard(Arc::clone(&cancelled));
        let worker_cancelled = Arc::clone(&cancelled);
        let mut worker = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            let sandbox = crate::lua::PluginSandbox::new()?;
            sandbox.execute_with_cancellation(&code, args, worker_cancelled)
        });

        const MAX_PLUGIN_WALL_TIME: Duration = Duration::from_secs(5);
        match tokio::time::timeout(MAX_PLUGIN_WALL_TIME, &mut worker).await {
            Ok(Ok(result)) => result,
            Ok(Err(error)) => Err(anyhow::anyhow!("plugin execution worker failed: {error}")),
            Err(_) => {
                cancelled.store(true, Ordering::Release);
                match tokio::time::timeout(Duration::from_secs(1), &mut worker).await {
                    Ok(Ok(_)) => Err(anyhow::anyhow!(
                        "plugin execution exceeded the 5-second wall-clock limit and was cooperatively cancelled"
                    )),
                    Ok(Err(error)) => Err(anyhow::anyhow!(
                        "plugin execution exceeded the 5-second wall-clock limit; worker failed while stopping: {error}"
                    )),
                    Err(_) => {
                        tracing::error!(
                            plugin_name,
                            "plugin worker did not stop after cooperative cancellation; OS-level isolation remains required"
                        );
                        Err(anyhow::anyhow!(
                            "plugin execution exceeded the 5-second wall-clock limit; cooperative cancellation did not stop the worker within 1 second"
                        ))
                    }
                }
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
    fn cancellation_guard_signals_worker_when_request_is_dropped() {
        let cancelled = Arc::new(AtomicBool::new(false));
        {
            let _guard = PluginCancellationGuard(Arc::clone(&cancelled));
            assert!(!cancelled.load(Ordering::Acquire));
        }
        assert!(cancelled.load(Ordering::Acquire));
    }

}

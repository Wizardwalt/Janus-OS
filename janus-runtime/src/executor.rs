//! Plugin execution environment.

use crate::plugin::PluginLoader;
use anyhow::Result;
use janus_core::Config;
use sha2::{Digest, Sha256};
use std::path::PathBuf;
use tracing::debug;

/// Safe plugin metadata returned to authenticated API clients.
#[derive(Debug, Clone, serde::Serialize)]
pub struct PluginSummary {
    pub id: String,
    pub name: String,
    pub category: String,
    pub status: String,
    pub description: String,
}

/// Plugin executor managing Lua runtime and plugin lifecycle.
pub struct PluginExecutor {
    config: Config,
    loader: PluginLoader,
}

impl PluginExecutor {
    /// Create new plugin executor
    pub async fn new(config: &Config, loader: PluginLoader) -> Result<Self> {
        debug!("Initializing plugin executor");
        Ok(Self {
            config: config.clone(),
            loader,
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

    /// Execute a plugin inside a sandbox confined to the engagement's approved
    /// evidence paths.
    ///
    /// The caller must already have passed `authorize_production_execution`.
    /// `scope` is the customer-approved read-only filesystem boundary; it is not
    /// optional, so an execution can never silently gain unrestricted access.
    pub async fn execute(
        &self,
        plugin_name: &str,
        args: serde_json::Value,
        scope: crate::evidence::EvidenceScope,
    ) -> Result<serde_json::Value> {
        debug!(plugin_name, ?args, "Executing plugin");

        let plugin = self
            .loader
            .get(plugin_name)
            .ok_or_else(|| anyhow::anyhow!("Plugin not found: {}", plugin_name))?;
        let code = plugin.code.clone();

        // Lua is not Send-safe to hold across an await and must not block the
        // async reactor, so run it on a blocking worker thread.
        let outcome = tokio::task::spawn_blocking(move || -> Result<crate::lua::SandboxOutcome> {
            let sandbox = crate::lua::PluginSandbox::new(scope)?;
            sandbox.execute(&code, args)
        })
        .await
        .map_err(|error| anyhow::anyhow!("plugin execution task failed: {}", error))??;

        Ok(serde_json::json!({
            "module": plugin_name,
            "logs": outcome.logs,
            "result": outcome.value,
        }))
    }

    /// Get plugin count
    pub fn plugin_count(&self) -> usize {
        self.loader.count()
    }
}

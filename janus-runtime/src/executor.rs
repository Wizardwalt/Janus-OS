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
        debug!(plugin_name, ?args, "Executing plugin");

        if let Some(plugin) = self.loader.get(plugin_name) {
            // Manifest declarations are requests, never grants. Until a capability
            // has a vetted host implementation and an explicit grant path, deny it.
            validate_plugin_capabilities(&plugin.manifest.capabilities)?;
            let sandbox = crate::lua::PluginSandbox::new()?;
            let result = sandbox.execute(&plugin.code, args)?;
            Ok(result)
        } else {
            Err(anyhow::anyhow!("Plugin not found: {}", plugin_name))
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

    #[test]
    fn capability_policy_allows_only_safe_logging_or_no_capabilities() {
        assert!(validate_plugin_capabilities(&[]).is_ok());
        assert!(validate_plugin_capabilities(&["logging".to_string()]).is_ok());
        assert!(validate_plugin_capabilities(&["network:wifi_scan".to_string()]).is_err());
        assert!(validate_plugin_capabilities(&["shell".to_string()]).is_err());
        assert!(validate_plugin_capabilities(&["filesystem:read".to_string()]).is_err());
    }
}

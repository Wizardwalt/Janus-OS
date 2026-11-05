//! Plugin discovery, loading, and manifest management.

use anyhow::Result;
use janus_core::{Plugin, PluginMetadata, PluginStatus};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tracing::{debug, error, warn};

/// Plugin manifest (parsed from TOML or JSON metadata)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginManifest {
    pub id: String,
    pub name: String,
    pub version: String,
    pub description: String,
    pub author: String,
    pub priority: u8,
    pub capabilities: Vec<String>,
    pub category: String,
    pub status: String,
    #[serde(default)]
    pub config: HashMap<String, serde_json::Value>,
}

impl PluginManifest {
    /// Convert manifest to plugin metadata
    pub fn to_metadata(self) -> PluginMetadata {
        PluginMetadata {
            id: self.id,
            name: self.name,
            version: self.version,
            description: self.description,
            author: self.author,
            priority: self.priority,
            capabilities: self.capabilities,
            category: self.category,
            status: self.status,
            config: self.config,
        }
    }
}

/// Plugin source code and metadata
#[derive(Debug, Clone)]
pub struct PluginSource {
    pub manifest: PluginManifest,
    pub code: String,
    pub path: PathBuf,
}

/// Plugin loader and registry
pub struct PluginLoader {
    plugin_dir: PathBuf,
    plugins: HashMap<String, PluginSource>,
}

impl PluginLoader {
    /// Create new plugin loader
    pub fn new(plugin_dir: impl AsRef<Path>) -> Self {
        Self {
            plugin_dir: plugin_dir.as_ref().to_path_buf(),
            plugins: HashMap::new(),
        }
    }

    /// Discover and load all Lua plugins below the configured directory.
    pub async fn load_all(&mut self) -> Result<usize> {
        if !self.plugin_dir.exists() {
            warn!("Plugin directory not found: {}", self.plugin_dir.display());
            return Ok(0);
        }

        let paths = collect_lua_paths(&self.plugin_dir)?;
        let mut count = 0;
        for path in paths {
            match self.load_plugin_file(&path).await {
                Ok(()) => count += 1,
                Err(error) => warn!("Failed to load plugin {:?}: {}", path.file_name(), error),
            }
        }

        debug!("Loaded {} plugins", count);
        Ok(count)
    }

    /// Load single plugin file
    async fn load_plugin_file(&mut self, path: &Path) -> Result<()> {
        let code = tokio::fs::read_to_string(path).await?;
        let manifest = extract_manifest_from_lua(&code, path)?;

        let source = PluginSource {
            manifest: manifest.clone(),
            code,
            path: path.to_path_buf(),
        };

        self.plugins.insert(manifest.id.clone(), source);
        debug!("Loaded plugin: {} ({})", manifest.name, manifest.id);
        Ok(())
    }

    /// Get plugin by ID
    pub fn get(&self, id: &str) -> Option<&PluginSource> {
        self.plugins.get(id)
    }

    /// List all plugins sorted by priority, then by identifier so the order is
    /// stable across runs (HashMap iteration order is not).
    pub fn list_sorted(&self) -> Vec<(&str, &PluginSource)> {
        let mut plugins: Vec<_> = self.plugins.iter().map(|(k, v)| (k.as_str(), v)).collect();
        plugins.sort_by(|(left_id, left), (right_id, right)| {
            left.manifest
                .priority
                .cmp(&right.manifest.priority)
                .then_with(|| left_id.cmp(right_id))
        });
        plugins
    }

    /// Get plugin count
    pub fn count(&self) -> usize {
        self.plugins.len()
    }
}

/// Return every Lua file in a directory tree in deterministic order.
fn collect_lua_paths(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut paths = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() {
            paths.extend(collect_lua_paths(&path)?);
        } else if path.extension().and_then(|extension| extension.to_str()) == Some("lua") {
            paths.push(path);
        }
    }
    paths.sort();
    Ok(paths)
}

/// Extract plugin manifest from Lua code comments
fn extract_manifest_from_lua(code: &str, path: &Path) -> Result<PluginManifest> {
    // Look for JSON metadata in Lua comments
    for line in code.lines() {
        if line.trim().starts_with("--[") {
            // Try to extract JSON from block comment
            let start = code.find("--[").unwrap_or(0);
            let json_start = start + 3;
            // Lua metadata blocks end with `}]`; searching for a bare `]`
            // incorrectly stops at JSON arrays such as `capabilities`.
            if let Some(relative_end) = code[json_start..].find("}]") {
                let json_end = json_start + relative_end + 1;
                let json_str = &code[json_start..json_end];
                if let Ok(manifest) = serde_json::from_str::<PluginManifest>(json_str) {
                    return Ok(manifest);
                }
            }
        }
    }

    // Fallback: generate minimal manifest from filename
    let id = path
        .file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or("unknown")
        .to_string();

    Ok(PluginManifest {
        id: id.clone(),
        name: id,
        version: "1.0.0".to_string(),
        description: "Plugin".to_string(),
        author: "Unknown".to_string(),
        priority: 100,
        capabilities: vec![],
        category: "uncategorized".to_string(),
        status: "prototype".to_string(),
        config: HashMap::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_manifest() {
        let code = r#"
--[{
  "id": "test_plugin",
  "name": "Test Plugin",
  "version": "1.0.0",
  "description": "A test plugin",
  "author": "Test",
  "priority": 50,
  "capabilities": ["network:wifi_scan"],
  "category": "test",
  "status": "production"
}]

print("Plugin loaded")
"#;

        let manifest = extract_manifest_from_lua(code, Path::new("test_plugin.lua")).unwrap();
        assert_eq!(manifest.id, "test_plugin");
        assert_eq!(manifest.priority, 50);
    }
}

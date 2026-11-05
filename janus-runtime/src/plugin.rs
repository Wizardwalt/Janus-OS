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

    /// List all plugins sorted by priority
    pub fn list_sorted(&self) -> Vec<(&str, &PluginSource)> {
        let mut plugins: Vec<_> = self.plugins.iter().map(|(k, v)| (k.as_str(), v)).collect();
        plugins.sort_by_key(|(_k, v)| v.manifest.priority);
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

/// Extract the JSON text of a `--[{ ... }]` manifest block.
///
/// A bare `--[{` only comments its own line in Lua, so the remaining JSON lines
/// must each carry a `--` prefix for the file to still compile. This reader
/// accepts both the fully commented form and the older uncommented form, and
/// strips the comment markers before parsing.
fn extract_manifest_json(code: &str) -> Option<String> {
    let mut lines = code.lines();
    while let Some(line) = lines.next() {
        let trimmed = line.trim_start();
        let Some(after_marker) = trimmed.strip_prefix("--[") else {
            continue;
        };
        if !after_marker.starts_with('{') {
            continue;
        }

        // Single-line manifest.
        if let Some(end) = after_marker.find("}]") {
            return Some(after_marker[..end + 1].to_string());
        }

        let mut collected = String::from(after_marker);
        collected.push('\n');
        for line in lines.by_ref() {
            let trimmed = line.trim_start();
            let content = trimmed.strip_prefix("--").unwrap_or(trimmed);
            if let Some(end) = content.find("}]") {
                collected.push_str(&content[..end + 1]);
                return Some(collected);
            }
            collected.push_str(content);
            collected.push('\n');
        }
    }
    None
}

/// Extract plugin manifest from Lua code comments
fn extract_manifest_from_lua(code: &str, path: &Path) -> Result<PluginManifest> {
    if let Some(json) = extract_manifest_json(code) {
        if let Ok(manifest) = serde_json::from_str::<PluginManifest>(&json) {
            return Ok(manifest);
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

    #[test]
    fn parses_a_manifest_whose_every_line_is_commented() {
        // This is the only form that is also valid Lua: `--[{` alone would leave
        // the JSON body uncommented and the module would fail to compile.
        let code = r#"--[{
--  "id": "evidence.hash",
--  "name": "Evidence Hash",
--  "version": "1.0.0",
--  "description": "Read-only hashing",
--  "author": "Janus",
--  "priority": 10,
--  "capabilities": ["storage:file_system"],
--  "category": "evidence_readonly",
--  "status": "production"
--}]

function execute() return 1 end
"#;
        let manifest = extract_manifest_from_lua(code, Path::new("evidence_hash.lua")).unwrap();
        assert_eq!(manifest.id, "evidence.hash");
        assert_eq!(manifest.priority, 10);
        assert_eq!(manifest.category, "evidence_readonly");
        assert_eq!(manifest.capabilities, vec!["storage:file_system".to_string()]);
    }

    #[test]
    fn falls_back_to_a_filename_manifest_when_no_block_exists() {
        let manifest =
            extract_manifest_from_lua("print('hi')", Path::new("some_module.lua")).unwrap();
        assert_eq!(manifest.id, "some_module");
        assert_eq!(manifest.status, "prototype");
    }

    #[test]
    fn shipped_read_only_evidence_manifests_are_parseable() {
        for (file, expected_id) in [
            ("evidence_hash.lua", "evidence.hash"),
            ("evidence_stat.lua", "evidence.stat"),
            ("evidence_list.lua", "evidence.list"),
            ("evidence_read.lua", "evidence.read"),
            ("evidence_manifest.lua", "evidence.manifest"),
        ] {
            let path = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../plugins/evidence_readonly")
                .join(file);
            let code = std::fs::read_to_string(&path)
                .unwrap_or_else(|error| panic!("cannot read {}: {}", path.display(), error));
            let manifest = extract_manifest_from_lua(&code, &path)
                .unwrap_or_else(|error| panic!("cannot parse {}: {}", file, error));
            assert_eq!(manifest.id, expected_id, "wrong id for {}", file);
            assert_eq!(manifest.status, "production");
            assert_eq!(manifest.category, "evidence_readonly");
        }
    }
}

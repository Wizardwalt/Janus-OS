//! Lua loading and isolated plugin execution.
//!
//! Lua values are intentionally kept inside the calling thread. A fresh Lua VM
//! is created for every plugin execution so API request handlers never share a
//! non-thread-safe interpreter instance.

use anyhow::Result;
use mlua::prelude::*;
use mlua::HookTriggers;
use std::{path::Path, sync::atomic::{AtomicU32, Ordering}};
use tracing::{debug, error, warn};

/// Runtime used only while core modules are being loaded during startup.
pub struct LuaEnv {
    lua: Lua,
}

impl LuaEnv {
    pub fn new() -> Result<Self> {
        Ok(Self { lua: Lua::new() })
    }

    /// Load Lua core files. One invalid file is reported but does not prevent
    /// the daemon from starting.
    pub async fn load_core_modules(&self, core_dir: &Path) -> Result<()> {
        if !core_dir.exists() {
            warn!("Core modules directory not found: {}", core_dir.display());
            return Ok(());
        }

        for entry in std::fs::read_dir(core_dir)? {
            let path = entry?.path();
            if path.extension().and_then(|extension| extension.to_str()) != Some("lua") {
                continue;
            }
            let source = tokio::fs::read_to_string(&path).await?;
            let name = path
                .file_stem()
                .and_then(|name| name.to_str())
                .unwrap_or("core_module");
            if let Err(error) = self.lua.load(&source).set_name(name).exec() {
                error!(module = name, %error, "Core Lua module failed to load");
            } else {
                debug!(module = name, "Loaded core Lua module");
            }
        }
        Ok(())
    }
}

/// A short-lived Lua sandbox for one plugin request.
pub struct PluginSandbox;

impl PluginSandbox {
    pub fn new() -> Result<Self> {
        Ok(Self)
    }

    /// Evaluate a plugin in a new Lua VM and expose JSON object arguments as
    /// the global `args` table. No host filesystem, shell, or hardware APIs are
    /// registered here; production handlers must provide vetted capabilities.
    pub fn execute(&self, code: &str, args: serde_json::Value) -> Result<serde_json::Value> {
        // Bound CPU work so an accidental or hostile infinite loop cannot
        // monopolize a Tokio worker thread indefinitely.
        const INSTRUCTION_HOOK_INTERVAL: u32 = 1_000;
        const MAX_INSTRUCTIONS: u32 = 100_000;
        let lua = Lua::new();
        let remaining_instructions = AtomicU32::new(MAX_INSTRUCTIONS);
        lua.set_hook(
            HookTriggers::new().every_nth_instruction(INSTRUCTION_HOOK_INTERVAL),
            move |_, _| {
                let remaining = remaining_instructions
                    .fetch_sub(INSTRUCTION_HOOK_INTERVAL, Ordering::Relaxed);
                if remaining <= INSTRUCTION_HOOK_INTERVAL {
                    Err(LuaError::RuntimeError(
                        "plugin instruction limit exceeded".to_string(),
                    ))
                } else {
                    Ok(())
                }
            },
        );
        let args_table = json_object_to_table(&lua, &args)?;
        lua.globals().set("args", args_table)?;

        // Logging is the first intentionally exposed host capability. Keep the
        // namespace narrow: plugins do not receive shell, filesystem, or device access.
        let janus = lua.create_table()?;
        janus.set(
            "log",
            lua.create_function(|_, message: String| {
                const MAX_LOG_BYTES: usize = 2_048;
                let mut safe_message: String = message
                    .chars()
                    .map(|character| if character.is_control() { ' ' } else { character })
                    .collect();
                if safe_message.len() > MAX_LOG_BYTES {
                    let mut end = MAX_LOG_BYTES;
                    while !safe_message.is_char_boundary(end) {
                        end -= 1;
                    }
                    safe_message.truncate(end);
                }
                tracing::info!(target: "janus_plugin", plugin_log = %safe_message, "plugin log");
                Ok(())
            })?,
        )?;
        lua.globals().set("janus", janus)?;

        let value = lua.load(code).eval::<LuaValue>()?;
        lua_value_to_json(&value)
    }
}

fn json_object_to_table<'lua>(lua: &'lua Lua, value: &serde_json::Value) -> Result<LuaTable<'lua>> {
    let table = lua.create_table()?;
    if let serde_json::Value::Object(values) = value {
        for (key, value) in values {
            table.set(key.as_str(), json_to_lua(lua, value)?)?;
        }
    }
    Ok(table)
}

fn json_to_lua<'lua>(lua: &'lua Lua, value: &serde_json::Value) -> Result<LuaValue<'lua>> {
    Ok(match value {
        serde_json::Value::Null => LuaValue::Nil,
        serde_json::Value::Bool(value) => LuaValue::Boolean(*value),
        serde_json::Value::Number(value) => match (value.as_i64(), value.as_f64()) {
            (Some(value), _) => LuaValue::Integer(value),
            (_, Some(value)) => LuaValue::Number(value),
            _ => return Err(anyhow::anyhow!("invalid JSON number")),
        },
        serde_json::Value::String(value) => LuaValue::String(lua.create_string(value)?),
        serde_json::Value::Array(values) => {
            let table = lua.create_table()?;
            for (index, value) in values.iter().enumerate() {
                table.set(index + 1, json_to_lua(lua, value)?)?;
            }
            LuaValue::Table(table)
        }
        serde_json::Value::Object(values) => {
            let table = lua.create_table()?;
            for (key, value) in values {
                table.set(key.as_str(), json_to_lua(lua, value)?)?;
            }
            LuaValue::Table(table)
        }
    })
}

fn lua_value_to_json(value: &LuaValue<'_>) -> Result<serde_json::Value> {
    lua_value_to_json_at_depth(value, 0)
}

fn lua_value_to_json_at_depth(
    value: &LuaValue<'_>,
    depth: usize,
) -> Result<serde_json::Value> {
    const MAX_TABLE_DEPTH: usize = 64;
    if depth > MAX_TABLE_DEPTH {
        anyhow::bail!("plugin result exceeds maximum table nesting depth");
    }

    Ok(match value {
        LuaValue::Nil => serde_json::Value::Null,
        LuaValue::Boolean(value) => serde_json::Value::Bool(*value),
        LuaValue::Integer(value) => serde_json::json!(*value),
        LuaValue::Number(value) => serde_json::json!(*value),
        LuaValue::String(value) => serde_json::Value::String(value.to_str()?.to_owned()),
        LuaValue::Table(table) => {
            // Lua tables can represent either JSON arrays or objects. Preserve
            // numeric sequence keys instead of trying to coerce every key to text.
            let mut array_entries = Vec::new();
            let mut object = serde_json::Map::new();
            let mut has_array_keys = false;
            let mut has_object_keys = false;

            for pair in table.clone().pairs::<LuaValue, LuaValue>() {
                let (key, item) = pair?;
                match key {
                    LuaValue::Integer(index) if index > 0 => {
                        has_array_keys = true;
                        array_entries.push((index, item));
                    }
                    LuaValue::String(key) => {
                        has_object_keys = true;
                        object.insert(
                            key.to_str()?.to_owned(),
                            lua_value_to_json_at_depth(&item, depth + 1)?,
                        );
                    }
                    _ => anyhow::bail!(
                        "plugin result table keys must be positive integers or strings"
                    ),
                }
            }

            if has_array_keys && has_object_keys {
                anyhow::bail!("plugin result table cannot mix array and object keys");
            }
            if has_array_keys {
                array_entries.sort_by_key(|(index, _)| *index);
                let mut result = Vec::with_capacity(array_entries.len());
                for (offset, (index, item)) in array_entries.into_iter().enumerate() {
                    if index != (offset + 1) as i64 {
                        anyhow::bail!("plugin result array keys must be contiguous from 1");
                    }
                    result.push(lua_value_to_json_at_depth(&item, depth + 1)?);
                }
                serde_json::Value::Array(result)
            } else {
                serde_json::Value::Object(object)
            }
        }
        _ => serde_json::Value::Null,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plugin_sandbox_returns_lua_values() {
        let sandbox = PluginSandbox::new().unwrap();
        let result = sandbox
            .execute("return { status = 'ok', count = args.count }", serde_json::json!({"count": 3}))
            .unwrap();
        assert_eq!(result, serde_json::json!({"status": "ok", "count": 3}));
    }

    #[test]
    fn plugin_sandbox_exposes_safe_logging_without_shell_access() {
        let sandbox = PluginSandbox::new().unwrap();
        let result = sandbox
            .execute(
                "janus.log('plugin started'); return { logger = type(janus.log), shell = type(janus.shell) }",
                serde_json::json!({}),
            )
            .unwrap();
        assert_eq!(
            result,
            serde_json::json!({"logger": "function", "shell": "nil"})
        );
    }

    #[test]
    fn plugin_sandbox_preserves_lua_arrays_and_nested_objects() {
        let sandbox = PluginSandbox::new().unwrap();
        let result = sandbox
            .execute(
                "return { items = { 'pip-boy', 'sensor' }, enabled = true }",
                serde_json::json!({}),
            )
            .unwrap();
        assert_eq!(
            result,
            serde_json::json!({"items": ["pip-boy", "sensor"], "enabled": true})
        );
    }

    #[test]
    fn plugin_sandbox_rejects_sparse_or_mixed_result_tables() {
        let sandbox = PluginSandbox::new().unwrap();
        let sparse = sandbox
            .execute("return { [1] = 'first', [3] = 'third' }", serde_json::json!({}))
            .unwrap_err();
        assert!(sparse.to_string().contains("contiguous"));

        let mixed = sandbox
            .execute("return { [1] = 'first', name = 'mixed' }", serde_json::json!({}))
            .unwrap_err();
        assert!(mixed.to_string().contains("mix array and object"));
    }

    #[test]
    fn plugin_sandbox_stops_unbounded_instruction_loops() {
        let sandbox = PluginSandbox::new().unwrap();
        let error = sandbox.execute("while true do end", serde_json::json!({})).unwrap_err();
        assert!(error.to_string().contains("plugin instruction limit exceeded"));
    }
}

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
    Ok(match value {
        LuaValue::Nil => serde_json::Value::Null,
        LuaValue::Boolean(value) => serde_json::Value::Bool(*value),
        LuaValue::Integer(value) => serde_json::json!(*value),
        LuaValue::Number(value) => serde_json::json!(*value),
        LuaValue::String(value) => serde_json::Value::String(value.to_str()?.to_owned()),
        LuaValue::Table(table) => {
            let mut result = serde_json::Map::new();
            for pair in table.clone().pairs::<String, LuaValue>() {
                let (key, value) = pair?;
                result.insert(key, lua_value_to_json(&value)?);
            }
            serde_json::Value::Object(result)
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
    fn plugin_sandbox_stops_unbounded_instruction_loops() {
        let sandbox = PluginSandbox::new().unwrap();
        let error = sandbox.execute("while true do end", serde_json::json!({})).unwrap_err();
        assert!(error.to_string().contains("plugin instruction limit exceeded"));
    }
}

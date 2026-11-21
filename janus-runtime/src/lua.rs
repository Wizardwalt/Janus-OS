//! Lua loading and isolated plugin execution.
//!
//! Lua values are intentionally kept inside the calling thread. A fresh Lua VM
//! is created for every plugin execution so API request handlers never share a
//! non-thread-safe interpreter instance.

use anyhow::Result;
use mlua::prelude::*;
use mlua::HookTriggers;
use std::{path::Path, sync::{atomic::{AtomicBool, AtomicU32, Ordering}, Arc}};
use tracing::{debug, error, warn};

const MAX_CORE_LUA_MEMORY_BYTES: usize = 32 * 1024 * 1024;
const MAX_CORE_LUA_INSTRUCTIONS: u32 = 500_000;
const LUA_INSTRUCTION_HOOK_INTERVAL: u32 = 1_000;

/// Runtime used only while core modules are being loaded during startup.
pub struct LuaEnv {
    lua: Lua,
    remaining_instructions: Arc<AtomicU32>,
}

impl LuaEnv {
    pub fn new() -> Result<Self> {
        // Lua::new loads mlua's safe standard-library subset (no OS, IO,
        // debug, or native C-module loading). Keep startup modules bounded too.
        let lua = Lua::new();
        lua.set_memory_limit(MAX_CORE_LUA_MEMORY_BYTES)?;
        let remaining_instructions = install_instruction_budget(
            &lua,
            MAX_CORE_LUA_INSTRUCTIONS,
            LUA_INSTRUCTION_HOOK_INTERVAL,
            "core module instruction limit exceeded",
            None,
        )?;
        // These globals can catch instruction-hook errors or create child
        // threads without inheriting the current-thread hook.
        let globals = lua.globals();
        for name in [
            "os", "io", "package", "debug", "require", "dofile", "loadfile",
            "load", "loadstring", "coroutine", "collectgarbage", "pcall", "xpcall",
        ] {
            globals.set(name, LuaValue::Nil)?;
        }
        drop(globals);
        Ok(Self { lua, remaining_instructions })
    }

    fn execute_core_source(&self, name: &str, source: &str) -> Result<()> {
        self.remaining_instructions.store(MAX_CORE_LUA_INSTRUCTIONS, Ordering::Relaxed);
        self.lua.load(source).set_name(name).exec()?;
        Ok(())
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
            if let Err(error) = self.execute_core_source(name, &source) {
                error!(module = name, %error, "Core Lua module failed to load");
            } else {
                debug!(module = name, "Loaded core Lua module");
            }
        }
        Ok(())
    }
}

/// Install a non-wrapping instruction budget. The caller must remove pcall/xpcall
/// from untrusted Lua globals so scripts cannot catch the hook error and continue.
fn install_instruction_budget(
    lua: &Lua,
    max_instructions: u32,
    interval: u32,
    error_message: &'static str,
    cancelled: Option<Arc<AtomicBool>>,
) -> Result<Arc<AtomicU32>> {
    let remaining = Arc::new(AtomicU32::new(max_instructions));
    let hook_remaining = Arc::clone(&remaining);
    lua.set_hook(
        HookTriggers::new().every_nth_instruction(interval),
        move |_, _| {
            if cancelled.as_ref().is_some_and(|flag| flag.load(Ordering::Acquire)) {
                return Err(LuaError::RuntimeError("plugin execution cancelled".to_string()));
            }
            let mut current = hook_remaining.load(Ordering::Relaxed);
            loop {
                if current < interval {
                    return Err(LuaError::RuntimeError(error_message.to_string()));
                }
                match hook_remaining.compare_exchange_weak(
                    current,
                    current - interval,
                    Ordering::Relaxed,
                    Ordering::Relaxed,
                ) {
                    Ok(_) => return Ok(()),
                    Err(actual) => current = actual,
                }
            }
        },
    );
    Ok(remaining)
}

/// Maximum Lua VM heap per plugin execution. This is a VM allocator limit,
/// not an operating-system process or total-runtime memory limit.
const MAX_PLUGIN_LUA_MEMORY_BYTES: usize = 16 * 1024 * 1024;

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
        self.execute_with_cancellation(code, args, Arc::new(AtomicBool::new(false)))
    }

    /// Execute with a cooperative cancellation flag checked by the Lua instruction hook.
    /// This releases a timed-out worker at the next hook; it is not an OS process boundary.
    pub fn execute_with_cancellation(
        &self,
        code: &str,
        args: serde_json::Value,
        cancelled: Arc<AtomicBool>,
    ) -> Result<serde_json::Value> {
        // Bound CPU work so an accidental or hostile infinite loop cannot
        // monopolize a Tokio worker thread indefinitely.
        const INSTRUCTION_HOOK_INTERVAL: u32 = 1_000;
        const MAX_INSTRUCTIONS: u32 = 100_000;
        const MAX_INPUT_BYTES: usize = 256 * 1024;
        const MAX_OUTPUT_BYTES: usize = 256 * 1024;

        let input_bytes = serde_json::to_vec(&args)?;
        anyhow::ensure!(
            input_bytes.len() <= MAX_INPUT_BYTES,
            "plugin input exceeds the 256 KiB limit"
        );
        let lua = Lua::new();
        // mlua 0.9's Lua 5.4 allocator enforces this cap on VM allocations.
        // Keep it separate from JSON byte limits and the instruction budget.
        lua.set_memory_limit(MAX_PLUGIN_LUA_MEMORY_BYTES)?;

        // Explicitly remove ambient loaders and process/debug libraries even if
        // the selected Lua build exposes them as safe standard globals.
        let globals = lua.globals();
        for name in [
            "os",
            "io",
            "package",
            "debug",
            "require",
            "dofile",
            "loadfile",
            "load",
            "loadstring",
            "coroutine",
            "collectgarbage",
            "pcall",
            "xpcall",
            "print",
        ] {
            globals.set(name, LuaValue::Nil)?;
        }

        install_instruction_budget(
            &lua,
            MAX_INSTRUCTIONS,
            INSTRUCTION_HOOK_INTERVAL,
            "plugin instruction limit exceeded",
            Some(cancelled),
        )?;
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
        let result = lua_value_to_json(&value)?;
        let output_bytes = serde_json::to_vec(&result)?;
        anyhow::ensure!(
            output_bytes.len() <= MAX_OUTPUT_BYTES,
            "plugin output exceeds the 256 KiB limit"
        );
        Ok(result)
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
    fn core_lua_excludes_process_filesystem_and_hook_catchers() {
        let env = LuaEnv::new().unwrap();
        env.execute_core_source(
            "safe-core",
            "assert(os == nil, 'os remains available'); assert(io == nil, 'io remains available'); assert(package == nil, 'package remains available'); assert(debug == nil, 'debug remains available'); assert(coroutine == nil, 'coroutine remains available'); assert(pcall == nil, 'pcall remains available'); assert(xpcall == nil, 'xpcall remains available')",
        )
        .unwrap();
    }

    #[test]
    fn core_lua_instruction_budget_stops_loops_and_resets_between_modules() {
        let env = LuaEnv::new().unwrap();
        let error = env.execute_core_source("loop", "while true do end").unwrap_err();
        assert!(error.to_string().contains("core module instruction limit exceeded"));
        env.execute_core_source("next-module", "return true").unwrap();
    }

    #[test]
    fn core_lua_enforces_a_hard_heap_limit() {
        let env = LuaEnv::new().unwrap();
        let error = env
            .execute_core_source("heap", "return string.rep('x', 64 * 1024 * 1024)")
            .unwrap_err();
        assert!(error.to_string().to_lowercase().contains("memory"));
    }

    #[test]
    fn plugin_sandbox_rejects_oversized_json_input() {
        let sandbox = PluginSandbox::new().unwrap();
        let args = serde_json::json!({ "payload": "x".repeat(256 * 1024) });
        let error = sandbox.execute("return true", args).unwrap_err();
        assert!(error.to_string().contains("input exceeds the 256 KiB limit"));
    }

    #[test]
    fn plugin_sandbox_rejects_oversized_json_output() {
        let sandbox = PluginSandbox::new().unwrap();
        let error = sandbox
            .execute("return { payload = string.rep('x', 256 * 1024) }", serde_json::json!({}))
            .unwrap_err();
        assert!(error.to_string().contains("output exceeds the 256 KiB limit"));
    }

    #[test]
    fn plugin_sandbox_enforces_a_hard_lua_heap_limit() {
        let sandbox = PluginSandbox::new().unwrap();
        let error = sandbox
            .execute("return string.rep('x', 32 * 1024 * 1024)", serde_json::json!({}))
            .unwrap_err();
        assert!(error.to_string().to_lowercase().contains("memory"));
    }

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
    fn plugin_sandbox_rejects_excessively_nested_result_tables() {
        let sandbox = PluginSandbox::new().unwrap();
        let error = sandbox
            .execute(
                "local root = {}; local node = root; for i = 1, 66 do local child = {}; node.child = child; node = child end; return root",
                serde_json::json!({}),
            )
            .unwrap_err();
        assert!(error.to_string().contains("maximum table nesting depth"));
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
    fn plugin_sandbox_hides_process_filesystem_and_dynamic_loader_globals() {
        let sandbox = PluginSandbox::new().unwrap();
        let result = sandbox
            .execute(
                "return { os = type(os), io = type(io), package = type(package), debug = type(debug), require = type(require), dofile = type(dofile), loadfile = type(loadfile), load = type(load), loadstring = type(loadstring), coroutine = type(coroutine), collectgarbage = type(collectgarbage), pcall = type(pcall), xpcall = type(xpcall) }",
                serde_json::json!({}),
            )
            .unwrap();
        assert_eq!(
            result,
            serde_json::json!({
                "os": "nil",
                "io": "nil",
                "package": "nil",
                "debug": "nil",
                "require": "nil",
                "dofile": "nil",
                "loadfile": "nil",
                "load": "nil",
                "loadstring": "nil",
                "coroutine": "nil",
                "collectgarbage": "nil",
                "pcall": "nil",
                "xpcall": "nil"
            })
        );
    }

    #[test]
    fn plugin_sandbox_uses_a_fresh_vm_for_each_execution() {
        let sandbox = PluginSandbox::new().unwrap();
        sandbox
            .execute("private_value = 'must not leak'; return true", serde_json::json!({}))
            .unwrap();

        let result = sandbox
            .execute("return { private_value = type(private_value) }", serde_json::json!({}))
            .unwrap();

        assert_eq!(result, serde_json::json!({"private_value": "nil"}));
    }

    #[test]
    fn plugin_sandbox_stops_unbounded_instruction_loops() {
        let sandbox = PluginSandbox::new().unwrap();
        let error = sandbox.execute("while true do end", serde_json::json!({})).unwrap_err();
        assert!(error.to_string().contains("plugin instruction limit exceeded"));
    }

    #[test]
    fn plugin_sandbox_observes_cooperative_cancellation() {
        let sandbox = PluginSandbox::new().unwrap();
        let cancelled = Arc::new(AtomicBool::new(true));
        let error = sandbox.execute_with_cancellation(
            "while true do end",
            serde_json::json!({}),
            cancelled,
        ).unwrap_err();
        assert!(error.to_string().contains("plugin execution cancelled"));
    }
    #[test]
    fn plugin_sandbox_does_not_inherit_startup_core_globals() {
        let core = LuaEnv::new().unwrap();
        core.execute_core_source(
            "trusted-core",
            "janus_private = 'must remain private to startup'",
        )
        .unwrap();

        let plugin = PluginSandbox::new().unwrap();
        let result = plugin
            .execute("return type(janus_private)", serde_json::json!({}))
            .unwrap();

        assert_eq!(result, serde_json::json!("nil"));
    }

}


#[cfg(test)]
mod process_worker_surface_tests {
    use super::*;

    #[test]
    fn plugin_sandbox_exposes_only_the_explicit_logging_host_function() {
        let sandbox = PluginSandbox::new().unwrap();
        let value = sandbox
            .execute(
                "return { print_type = type(print), os_type = type(os), io_type = type(io), package_type = type(package), log_type = type(janus.log) }",
                serde_json::json!({}),
            )
            .unwrap();
        assert_eq!(value["print_type"], "nil");
        assert_eq!(value["os_type"], "nil");
        assert_eq!(value["io_type"], "nil");
        assert_eq!(value["package_type"], "nil");
        assert_eq!(value["log_type"], "function");
    }
}

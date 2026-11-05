//! Lua loading and isolated plugin execution.
//!
//! Lua values are intentionally kept inside the calling thread. A fresh Lua VM
//! is created for every plugin execution so API request handlers never share a
//! non-thread-safe interpreter instance.

use anyhow::Result;
use mlua::prelude::*;
use std::path::Path;
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

/// Output produced by one sandboxed module run.
#[derive(Debug, Clone, Default)]
pub struct SandboxOutcome {
    /// Lines the module emitted through `print` / `janus.log`.
    pub logs: Vec<String>,
    /// The module's return value, converted to JSON.
    pub value: serde_json::Value,
}

/// A short-lived Lua sandbox for one plugin request.
///
/// Only vetted, read-only capabilities are exposed. There is deliberately no
/// shell, ADB, network, or write access, and no auto-stubbing of unknown
/// globals: calling an unimplemented function is an error, never a fake success.
pub struct PluginSandbox {
    scope: crate::evidence::EvidenceScope,
}

impl PluginSandbox {
    /// Create a sandbox restricted to the customer's approved evidence paths.
    pub fn new(scope: crate::evidence::EvidenceScope) -> Result<Self> {
        Ok(Self { scope })
    }

    /// Evaluate a plugin in a new Lua VM.
    ///
    /// `args` is exposed as the global `args` table. The `janus` table exposes
    /// only real read-only evidence collection plus logging.
    pub fn execute(&self, code: &str, args: serde_json::Value) -> Result<SandboxOutcome> {
        let lua = new_sandbox_lua()?;
        let logs = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));

        let args_table = json_object_to_table(&lua, &args)?;
        lua.globals().set("args", args_table)?;

        self.register_api(&lua, &logs)?;

        // Modules are written either as a script that self-calls `execute()` or
        // as a definition of `execute()`. Support both deterministically.
        let chunk = lua.load(code);
        let value = chunk.eval::<LuaValue>()?;
        let value = if matches!(value, LuaValue::Nil) {
            if let Ok(function) = lua.globals().get::<_, LuaFunction>("execute") {
                function.call::<_, LuaValue>(())?
            } else {
                value
            }
        } else {
            value
        };

        let logs = logs
            .lock()
            .map_err(|_| anyhow::anyhow!("sandbox log buffer poisoned"))?
            .clone();
        Ok(SandboxOutcome {
            logs,
            value: lua_value_to_json(&value)?,
        })
    }

    /// Register the real, read-only capability surface.
    fn register_api(
        &self,
        lua: &Lua,
        logs: &std::sync::Arc<std::sync::Mutex<Vec<String>>>,
    ) -> Result<()> {
        fn push_log(
            logs: &std::sync::Arc<std::sync::Mutex<Vec<String>>>,
            line: String,
        ) -> LuaResult<()> {
            logs.lock()
                .map_err(|_| LuaError::RuntimeError("log buffer poisoned".into()))?
                .push(line);
            Ok(())
        }

        // print(...) -> captured, not written to the host's stdout.
        {
            let sink = logs.clone();
            lua.globals().set(
                "print",
                lua.create_function(move |_, values: LuaMultiValue| {
                    let parts: Vec<String> = values.iter().map(lua_display).collect();
                    push_log(&sink, parts.join("\t"))
                })?,
            )?;
        }

        let janus = lua.create_table()?;

        {
            let sink = logs.clone();
            janus.set(
                "log",
                lua.create_function(move |_, values: LuaMultiValue| {
                    let parts: Vec<String> = values.iter().map(lua_display).collect();
                    push_log(&sink, parts.join(" "))
                })?,
            )?;
        }

        // Read-only evidence collection, confined to the approved scope.
        let evidence = lua.create_table()?;

        {
            let scope = self.scope.clone();
            evidence.set(
                "hash",
                lua.create_function(move |lua_ctx, path: String| {
                    let resolved = scope
                        .resolve(&path)
                        .map_err(|error| LuaError::RuntimeError(error.to_string()))?;
                    let (sha256, size_bytes) = crate::evidence::hash_file(&resolved)
                        .map_err(|error| LuaError::RuntimeError(error.to_string()))?;
                    let table = lua_ctx.create_table()?;
                    table.set("path", resolved.display().to_string())?;
                    table.set("sha256", sha256)?;
                    table.set("size_bytes", size_bytes)?;
                    Ok(table)
                })?,
            )?;
        }

        {
            let scope = self.scope.clone();
            evidence.set(
                "stat",
                lua.create_function(move |lua_ctx, path: String| {
                    let resolved = scope
                        .resolve(&path)
                        .map_err(|error| LuaError::RuntimeError(error.to_string()))?;
                    let stat = crate::evidence::stat_path(&resolved)
                        .map_err(|error| LuaError::RuntimeError(error.to_string()))?;
                    let table = lua_ctx.create_table()?;
                    table.set("path", stat.path)?;
                    table.set("size_bytes", stat.size_bytes)?;
                    table.set("is_file", stat.is_file)?;
                    table.set("is_dir", stat.is_dir)?;
                    table.set("readonly", stat.readonly)?;
                    if let Some(modified) = stat.modified_unix {
                        table.set("modified_unix", modified)?;
                    }
                    Ok(table)
                })?,
            )?;
        }

        {
            let scope = self.scope.clone();
            evidence.set(
                "list",
                lua.create_function(move |lua_ctx, path: String| {
                    let resolved = scope
                        .resolve(&path)
                        .map_err(|error| LuaError::RuntimeError(error.to_string()))?;
                    let entries = crate::evidence::list_dir(&resolved)
                        .map_err(|error| LuaError::RuntimeError(error.to_string()))?;
                    let table = lua_ctx.create_table()?;
                    for (index, entry) in entries.iter().enumerate() {
                        let row = lua_ctx.create_table()?;
                        row.set("name", entry.name.clone())?;
                        row.set("is_file", entry.is_file)?;
                        row.set("is_dir", entry.is_dir)?;
                        row.set("is_symlink", entry.is_symlink)?;
                        table.set(index + 1, row)?;
                    }
                    Ok(table)
                })?,
            )?;
        }

        {
            let scope = self.scope.clone();
            evidence.set(
                "read",
                lua.create_function(
                    move |lua_ctx, (path, offset, length): (String, Option<u64>, Option<u64>)| {
                        let resolved = scope
                            .resolve(&path)
                            .map_err(|error| LuaError::RuntimeError(error.to_string()))?;
                        let (hex, bytes_read) = crate::evidence::read_slice(
                            &resolved,
                            offset.unwrap_or(0),
                            length.unwrap_or(4096),
                        )
                        .map_err(|error| LuaError::RuntimeError(error.to_string()))?;
                        let table = lua_ctx.create_table()?;
                        table.set("path", resolved.display().to_string())?;
                        table.set("offset", offset.unwrap_or(0))?;
                        table.set("bytes_read", bytes_read)?;
                        table.set("hex", hex)?;
                        Ok(table)
                    },
                )?,
            )?;
        }

        janus.set("evidence", evidence)?;

        // Explicitly unavailable operations. These raise a Lua error rather than
        // returning simulated success, so a module cannot claim an operation ran.
        for name in ["shell", "adb", "exec_system_cmd", "pull", "write"] {
            janus.set(
                name,
                lua.create_function(move |_, _: LuaMultiValue| -> LuaResult<()> {
                    Err(LuaError::RuntimeError(format!(
                        "janus.{} is not available: no certified implementation is installed for this operation",
                        name
                    )))
                })?,
            )?;
        }

        lua.globals().set("janus", janus)?;
        Ok(())
    }
}

/// Build a Lua state with no ambient host access.
///
/// `Lua::new()` uses `StdLib::ALL_SAFE`, which still includes `io`, `os`, and
/// `package`. Those would let a module open any file, run `os.execute`, or delete
/// evidence, completely bypassing the approved engagement scope. Only the pure
/// computation libraries are loaded, and the remaining filesystem/process
/// globals are removed explicitly.
fn new_sandbox_lua() -> Result<Lua> {
    let libs = LuaStdLib::COROUTINE
        | LuaStdLib::TABLE
        | LuaStdLib::STRING
        | LuaStdLib::UTF8
        | LuaStdLib::MATH;
    let lua = Lua::new_with(libs, LuaOptions::default())?;

    // Defence in depth: even if a library is loaded transitively, these globals
    // must not be reachable from module code.
    let globals = lua.globals();
    for name in [
        "io", "os", "package", "require", "module", "dofile", "loadfile", "debug",
    ] {
        globals.set(name, LuaValue::Nil)?;
    }

    Ok(lua)
}

/// Render a Lua value for log output without exposing host internals.
fn lua_display(value: &LuaValue<'_>) -> String {
    match value {
        LuaValue::Nil => "nil".to_string(),
        LuaValue::Boolean(value) => value.to_string(),
        LuaValue::Integer(value) => value.to_string(),
        LuaValue::Number(value) => value.to_string(),
        LuaValue::String(value) => value.to_string_lossy().to_string(),
        LuaValue::Table(_) => "[table]".to_string(),
        _ => "[value]".to_string(),
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
    lua_value_to_json_depth(value, 0)
}

/// Maximum nesting depth converted to JSON. Evidence tables are shallow; this
/// bound stops a self-referential table from overflowing the stack.
const MAX_JSON_DEPTH: usize = 32;

fn lua_value_to_json_depth(value: &LuaValue<'_>, depth: usize) -> Result<serde_json::Value> {
    if depth > MAX_JSON_DEPTH {
        return Err(anyhow::anyhow!(
            "module result exceeds the maximum nesting depth of {}",
            MAX_JSON_DEPTH
        ));
    }
    Ok(match value {
        LuaValue::Nil => serde_json::Value::Null,
        LuaValue::Boolean(value) => serde_json::Value::Bool(*value),
        LuaValue::Integer(value) => serde_json::json!(*value),
        LuaValue::Number(value) => serde_json::json!(*value),
        LuaValue::String(value) => serde_json::Value::String(value.to_str()?.to_owned()),
        LuaValue::Table(table) => {
            // Handle keys by their actual Lua type. Relying on mlua to coerce
            // integer keys into strings is not guaranteed, and `evidence.list`
            // returns a 1-based sequence that should become a JSON array.
            let mut object = serde_json::Map::new();
            let mut sequence: Vec<(i64, serde_json::Value)> = Vec::new();
            let mut only_sequential_keys = true;

            for pair in table.clone().pairs::<LuaValue, LuaValue>() {
                let (key, value) = pair?;
                let converted = lua_value_to_json_depth(&value, depth + 1)?;
                match key {
                    LuaValue::Integer(index) => sequence.push((index, converted)),
                    LuaValue::String(text) => {
                        only_sequential_keys = false;
                        object.insert(text.to_str()?.to_owned(), converted);
                    }
                    LuaValue::Number(index) => {
                        only_sequential_keys = false;
                        object.insert(index.to_string(), converted);
                    }
                    _ => {
                        return Err(anyhow::anyhow!(
                            "module result contains a table key that cannot be represented in JSON"
                        ));
                    }
                }
            }

            if only_sequential_keys && !sequence.is_empty() {
                sequence.sort_by_key(|(index, _)| *index);
                let is_array = sequence
                    .iter()
                    .enumerate()
                    .all(|(position, (index, _))| *index == (position as i64) + 1);
                if is_array {
                    serde_json::Value::Array(sequence.into_iter().map(|(_, v)| v).collect())
                } else {
                    for (index, value) in sequence {
                        object.insert(index.to_string(), value);
                    }
                    serde_json::Value::Object(object)
                }
            } else {
                for (index, value) in sequence {
                    object.insert(index.to_string(), value);
                }
                serde_json::Value::Object(object)
            }
        }
        _ => serde_json::Value::Null,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn temp_dir(label: &str) -> std::path::PathBuf {
        let unique = format!(
            "janus-lua-test-{}-{}-{}",
            label,
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let dir = std::env::temp_dir().join(unique);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn scope_for(dir: &std::path::Path) -> crate::evidence::EvidenceScope {
        crate::evidence::EvidenceScope::new(&[dir.display().to_string()]).unwrap()
    }

    #[test]
    fn captures_print_and_janus_log_output() {
        let dir = temp_dir("logs");
        let sandbox = PluginSandbox::new(scope_for(&dir)).unwrap();
        let outcome = sandbox
            .execute(
                "print('one', 2) janus.log('three') return { status = 'success' }",
                serde_json::json!({}),
            )
            .unwrap();
        assert_eq!(outcome.logs, vec!["one\t2".to_string(), "three".to_string()]);
        assert_eq!(outcome.value["status"], "success");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn exposes_args_as_a_global_table() {
        let dir = temp_dir("args");
        let sandbox = PluginSandbox::new(scope_for(&dir)).unwrap();
        let outcome = sandbox
            .execute("return { echoed = args.path }", serde_json::json!({"path": "/tmp/x"}))
            .unwrap();
        assert_eq!(outcome.value["echoed"], "/tmp/x");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn hashes_a_real_file_through_the_lua_api() {
        let dir = temp_dir("hash");
        let file = dir.join("evidence.bin");
        std::fs::File::create(&file).unwrap().write_all(b"abc").unwrap();
        let sandbox = PluginSandbox::new(scope_for(&dir)).unwrap();
        let outcome = sandbox
            .execute(
                "local r = janus.evidence.hash(args.path) return { sha = r.sha256, n = r.size_bytes }",
                serde_json::json!({"path": file.display().to_string()}),
            )
            .unwrap();
        assert_eq!(
            outcome.value["sha"],
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(outcome.value["n"], 3);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn refuses_to_hash_a_file_outside_the_approved_scope() {
        let approved = temp_dir("approved");
        let outside = temp_dir("outside");
        let secret = outside.join("secret.bin");
        std::fs::File::create(&secret).unwrap().write_all(b"no").unwrap();
        let sandbox = PluginSandbox::new(scope_for(&approved)).unwrap();
        let result = sandbox.execute(
            "return janus.evidence.hash(args.path)",
            serde_json::json!({"path": secret.display().to_string()}),
        );
        assert!(result.is_err(), "out-of-scope read must fail");
        std::fs::remove_dir_all(&approved).unwrap();
        std::fs::remove_dir_all(&outside).unwrap();
    }

    #[test]
    fn list_returns_a_json_array_of_entries() {
        let dir = temp_dir("list");
        std::fs::File::create(dir.join("a.bin")).unwrap().write_all(b"a").unwrap();
        std::fs::File::create(dir.join("b.bin")).unwrap().write_all(b"b").unwrap();
        let sandbox = PluginSandbox::new(scope_for(&dir)).unwrap();
        let outcome = sandbox
            .execute(
                "return janus.evidence.list(args.path)",
                serde_json::json!({"path": dir.display().to_string()}),
            )
            .unwrap();
        let entries = outcome.value.as_array().expect("entries must be a JSON array");
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0]["name"], "a.bin");
        assert_eq!(entries[1]["name"], "b.bin");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn unavailable_operations_raise_instead_of_reporting_success() {
        let dir = temp_dir("unavailable");
        let sandbox = PluginSandbox::new(scope_for(&dir)).unwrap();
        for call in [
            "return janus.shell('id')",
            "return janus.adb('devices')",
            "return janus.exec_system_cmd('id')",
        ] {
            let result = sandbox.execute(call, serde_json::json!({}));
            assert!(result.is_err(), "{} must not succeed", call);
            let message = result.unwrap_err().to_string();
            assert!(
                message.contains("not available"),
                "expected an honest unavailable error, got: {}",
                message
            );
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn unknown_globals_are_not_auto_stubbed() {
        let dir = temp_dir("nostub");
        let sandbox = PluginSandbox::new(scope_for(&dir)).unwrap();
        let result = sandbox.execute("return unleash_god_tier_power('target')", serde_json::json!({}));
        assert!(result.is_err(), "unknown globals must fail, not fake success");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn calls_a_defined_execute_function_when_the_script_returns_nothing() {
        let dir = temp_dir("execute");
        let sandbox = PluginSandbox::new(scope_for(&dir)).unwrap();
        let outcome = sandbox
            .execute(
                "function execute() janus.log('ran') return { status = 'success' } end",
                serde_json::json!({}),
            )
            .unwrap();
        assert_eq!(outcome.value["status"], "success");
        assert_eq!(outcome.logs, vec!["ran".to_string()]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn sandbox_blocks_filesystem_and_process_escapes() {
        let dir = temp_dir("escape");
        let sandbox = PluginSandbox::new(scope_for(&dir)).unwrap();
        // Each of these would bypass the approved evidence scope if the default
        // `Lua::new()` library set were used.
        for attack in [
            "local f = io.open('/etc/passwd', 'r') return f:read('a')",
            "return os.execute('id')",
            "return os.remove('/tmp/janus-should-not-be-deleted')",
            "return require('os')",
            "return dofile('/etc/passwd')",
            "return loadfile('/etc/passwd')",
            "return debug.getinfo(1)",
            "io.open('/tmp/janus-should-not-be-created', 'w'):write('x')",
        ] {
            let result = sandbox.execute(attack, serde_json::json!({}));
            assert!(result.is_err(), "sandbox escape must fail: {}", attack);
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn sandbox_keeps_pure_computation_libraries() {
        let dir = temp_dir("pure");
        let sandbox = PluginSandbox::new(scope_for(&dir)).unwrap();
        let outcome = sandbox
            .execute(
                "return { s = string.format('%s-%d', string.upper('ab'), math.floor(3.7)) }",
                serde_json::json!({}),
            )
            .unwrap();
        assert_eq!(outcome.value["s"], "AB-3");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn shipped_read_only_evidence_modules_run_for_real() {
        let dir = temp_dir("shipped");
        let file = dir.join("evidence.bin");
        std::fs::File::create(&file).unwrap().write_all(b"abc").unwrap();
        let modules = Path::new(env!("CARGO_MANIFEST_DIR")).join("../plugins/evidence_readonly");
        let code = std::fs::read_to_string(modules.join("evidence_hash.lua")).unwrap();
        let sandbox = PluginSandbox::new(scope_for(&dir)).unwrap();
        let outcome = sandbox
            .execute(&code, serde_json::json!({"path": file.display().to_string()}))
            .unwrap();
        assert_eq!(outcome.value["status"], "success");
        assert_eq!(outcome.value["read_only"], true);
        assert_eq!(
            outcome.value["sha256"],
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert!(!outcome.logs.is_empty(), "module must emit real log output");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}

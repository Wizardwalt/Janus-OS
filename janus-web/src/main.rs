use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Request, State,
    },
    http::{header::HeaderName, HeaderValue},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use mlua::prelude::*;
use rand::Rng;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    net::SocketAddr,
    path::PathBuf,
    sync::{Arc, Mutex},
};
use tokio::sync::broadcast;
use tower_http::services::ServeDir;

/// Default bind address. Override with `JANUS_WEB_ADDR` (for example
/// `127.0.0.1:5000` when a reverse proxy terminates TLS in front of the GUI).
const DEFAULT_BIND_ADDR: &str = "0.0.0.0:5000";

// ─── Lua Sandbox Prelude ───────────────────────────────────────────────────────
// Loaded before every module. Sets up __index auto-stub on _G so ANY undefined
// global function is silently handled instead of throwing a runtime error.
const JANUS_PRELUDE: &str = r#"
setmetatable(_G, {
    __index = function(t, k)
        -- Return a universal stub for any unknown global
        return function(...)
            local args = {...}
            local first = args[1]
            -- Return a result table so callers can do result.status safely
            return {status = "ok", value = tostring(k), result = "complete"}
        end
    end
})
"#;

// ─── Shared State ─────────────────────────────────────────────────────────────

#[derive(Clone)]
struct GuiState {
    tx: broadcast::Sender<String>,
    aria_status: Arc<Mutex<AriaStatus>>,
    module_registry: Arc<HashMap<String, Vec<ModuleInfo>>>,
    /// The legacy Lua runner is available only when demo mode is explicitly enabled.
    demo_mode: bool,
    /// Base URL the browser should use for the protected runtime API.
    runtime_url: String,
}

#[derive(Serialize)]
struct RuntimeConfig {
    runtime_url: String,
    demo_mode: bool,
}

#[derive(Serialize)]
struct HealthStatus {
    status: &'static str,
    version: &'static str,
    modules: usize,
    categories: usize,
    demo_mode: bool,
}

#[derive(Clone, Serialize)]
struct AriaStatus {
    mood: String,
    thought: String,
    online: bool,
    ops_count: u32,
}

#[derive(Clone, Serialize)]
struct ModuleInfo {
    name: String,
    display: String,
    description: String,
    file: String,
}

// ─── WebSocket Message Types ──────────────────────────────────────────────────

#[derive(Deserialize)]
struct WsIncoming {
    #[serde(rename = "type")]
    msg_type: String,
    #[serde(default)]
    category: String,
    #[serde(default)]
    module: String,
    #[serde(default)]
    message: String,
}

#[derive(Serialize)]
struct WsOutgoing {
    #[serde(rename = "type")]
    msg_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    line: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    modules: Option<Vec<ModuleInfo>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    status: Option<AriaStatus>,
    #[serde(skip_serializing_if = "Option::is_none")]
    category: Option<String>,
}

fn ws_msg(msg_type: &str, line: Option<String>, text: Option<String>) -> String {
    serde_json::to_string(&WsOutgoing {
        msg_type: msg_type.into(),
        line,
        text,
        modules: None,
        status: None,
        category: None,
    })
    .unwrap()
}

// ─── Module Registry ──────────────────────────────────────────────────────────

fn build_module_registry() -> HashMap<String, Vec<ModuleInfo>> {
    let mut map: HashMap<String, Vec<ModuleInfo>> = HashMap::new();

    let categories = vec![
        // plugins/
        ("forensics", "plugins/forensics"),
        ("cyber_warfare", "plugins/cyber_warfare"),
        ("network_warfare", "plugins/network_warfare"),
        ("mobile_offense", "plugins/mobile_offense"),
        ("sigint", "plugins/sigint"),
        ("osint_oracle", "plugins/osint_oracle"),
        ("hardware_glitch", "plugins/hardware_glitch"),
        ("titan_exclusive", "plugins/titan_exclusive"),
        ("advanced_mobile", "plugins/advanced_mobile"),
        ("expansion", "plugins/expansion"),
        ("mobile_expansion", "plugins/mobile_expansion"),
        ("offensive", "plugins/offensive"),
        ("tactical", "plugins/tactical"),
        // modules/
        ("god_tier", "modules/god_tier"),
        ("legendary", "modules/legendary"),
        ("mobile_offense_adv", "modules/mobile_offense"),
        ("network_warfare_adv", "modules/network_warfare"),
        ("forensics_recovery", "modules/forensics_recovery"),
        ("sigint_adv", "modules/sigint"),
        ("tactical_defensive", "modules/tactical_defensive"),
        ("creative_psych", "modules/creative_psych"),
        ("god_protocols", "modules/god_protocols"),
        ("legacy_vault", "modules/legacy_vault"),
        ("vault_engineering", "modules/vault_engineering"),
        ("signature", "modules/signature"),
        // root
        ("apocalypse", "apocalypse_engineering"),
        ("core", "core"),
    ];

    for (cat, dir) in categories {
        let mut mods = Vec::new();
        if let Ok(entries) = std::fs::read_dir(dir) {
            let mut files: Vec<_> = entries
                .filter_map(|e| e.ok())
                .filter(|e| {
                    // Only regular files with .lua extension and clean filenames
                    let path = e.path();
                    let is_file = path.is_file();
                    let has_lua = path.extension().map_or(false, |x| x == "lua");
                    let clean_name = e
                        .file_name()
                        .to_string_lossy()
                        .chars()
                        .all(|c| c.is_alphanumeric() || c == '_' || c == '-' || c == '.');
                    is_file && has_lua && clean_name
                })
                .collect();
            files.sort_by_key(|e| e.file_name());
            // Every discoverable module is listed: the dashboard shows the real
            // inventory of the deployment instead of a truncated sample.
            for entry in files.iter() {
                let fname = entry.file_name();
                let name = fname.to_string_lossy();
                let base = name.trim_end_matches(".lua").to_string();
                let display = humanize(&base);
                let desc = read_module_desc(&entry.path());
                mods.push(ModuleInfo {
                    name: base,
                    display,
                    description: desc,
                    file: format!("{}/{}", dir, name),
                });
            }
        }
        if !mods.is_empty() {
            map.insert(cat.to_string(), mods);
        }
    }
    map
}

fn humanize(s: &str) -> String {
    let s = s
        .trim_start_matches(|c: char| c.is_ascii_digit() || c == '_')
        .replace('_', " ")
        .replace('-', " ");
    let mut out = String::new();
    let mut cap = true;
    for ch in s.chars() {
        if ch == ' ' {
            cap = true;
            out.push(ch);
        } else if cap {
            out.extend(ch.to_uppercase());
            cap = false;
        } else {
            out.push(ch);
        }
    }
    out.trim().to_string()
}

fn read_module_desc(path: &std::path::Path) -> String {
    if let Ok(content) = std::fs::read_to_string(path) {
        for line in content.lines().take(10) {
            let line = line.trim();
            if !line.starts_with("--") {
                continue;
            }
            let text = line.trim_start_matches('-').trim();
            if text.is_empty()
                || text.ends_with(".lua")
                || text.to_lowercase().starts_with("category")
                || text.to_lowercase().contains("module #")
                || text.to_lowercase().contains("god tier module")
                || text.to_lowercase().contains("janus os module")
                || text.to_lowercase().contains("most powerful")
                || text.starts_with("====")
            {
                continue;
            }
            if text.len() > 6 {
                let desc = if let Some(p) = text.find(" - ") {
                    &text[p + 3..]
                } else {
                    text
                };
                let cleaned = desc.trim_end_matches(|c: char| c == '|' || c == '═').trim();
                // Descriptions are rendered in the launcher, so cap their length.
                return cleaned.chars().take(160).collect();
            }
        }
    }
    "Operational module".to_string()
}

// ─── Lua Execution Engine ─────────────────────────────────────────────────────

fn run_lua_module_file(file_path: &str, category: &str) -> Vec<String> {
    let module_name = file_path
        .split('/')
        .last()
        .unwrap_or(file_path)
        .trim_end_matches(".lua");
    let display = humanize(module_name);

    let lua_src = match std::fs::read_to_string(file_path) {
        Ok(s) => s,
        Err(e) => return vec![format!("[ERROR] Cannot read module: {} — {}", file_path, e)],
    };

    let output: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let out = output.clone();

    let exec_result = (|| -> LuaResult<()> {
        let lua = Lua::new();

        // ── override print() ──────────────────────────────────────────────────
        {
            let o = out.clone();
            lua.globals().set(
                "print",
                lua.create_function(move |_, args: LuaMultiValue| {
                    let parts: Vec<String> = args.iter().map(|v| lua_val_to_string(v)).collect();
                    o.lock().unwrap().push(parts.join("\t"));
                    Ok(())
                })?,
            )?;
        }

        // ── overseer_speak(msg) ───────────────────────────────────────────────
        {
            let o = out.clone();
            lua.globals().set(
                "overseer_speak",
                lua.create_function(move |_, msg: String| {
                    o.lock().unwrap().push(format!("[OVERSEER] {}", msg));
                    Ok(())
                })?,
            )?;
        }

        // ── speak(msg) ───────────────────────────────────────────────────────
        {
            let o = out.clone();
            lua.globals().set(
                "speak",
                lua.create_function(move |_, msg: String| {
                    o.lock().unwrap().push(format!("[ARIA] {}", msg));
                    Ok(())
                })?,
            )?;
        }

        // ── log_to_blackbox(tbl) ──────────────────────────────────────────────
        {
            let o = out.clone();
            lua.globals().set(
                "log_to_blackbox",
                lua.create_function(move |_, tbl: LuaTable| {
                    let module = tbl.get::<String>("module").unwrap_or_default();
                    let status = tbl.get::<String>("status").unwrap_or_else(|_| "ok".into());
                    o.lock()
                        .unwrap()
                        .push(format!("[BLACKBOX] ✓ {} → {}", module, status));
                    Ok(())
                })?,
            )?;
        }

        // ── save_to_blackbox(tbl) — alias ─────────────────────────────────────
        {
            let o = out.clone();
            lua.globals().set(
                "save_to_blackbox",
                lua.create_function(move |_, tbl: LuaTable| {
                    let module = tbl.get::<String>("module").unwrap_or_default();
                    o.lock()
                        .unwrap()
                        .push(format!("[BLACKBOX] ✓ Saved: {}", module));
                    Ok(())
                })?,
            )?;
        }

        // ── read_rotary_dial() ────────────────────────────────────────────────
        {
            let o = out.clone();
            lua.globals().set(
                "read_rotary_dial",
                lua.create_function(move |_, ()| {
                    let v = rand::thread_rng().gen_range(1u32..=100);
                    o.lock()
                        .unwrap()
                        .push(format!("[ROTARY] Dial position: {}", v));
                    Ok(v)
                })?,
            )?;
        }

        // ── wait_for_haptic_confirmation(n) ───────────────────────────────────
        {
            let o = out.clone();
            lua.globals().set(
                "wait_for_haptic_confirmation",
                lua.create_function(move |_, n: Option<u32>| {
                    o.lock().unwrap().push(format!(
                        "[HAPTIC] {} pulse(s) received — authorised",
                        n.unwrap_or(1)
                    ));
                    Ok(true)
                })?,
            )?;
        }

        // ── unleash_god_tier_power(target, rotary, options) ───────────────────
        {
            let o = out.clone();
            lua.globals().set("unleash_god_tier_power", lua.create_function(move |lua_ctx, (tgt, pwr, _opts): (Option<String>, Option<u32>, Option<LuaTable>)| {
                let t = tgt.as_deref().unwrap_or("the_wasteland");
                let p = pwr.unwrap_or(100);
                o.lock().unwrap().push(format!("[GOD TIER] Power level {} released upon '{}'", p, t));
                o.lock().unwrap().push("[GOD TIER] ✓ Dominion established.".into());
                let r = lua_ctx.create_table()?;
                r.set("status", "success")?;
                r.set("power_level", "apocalyptic")?;
                Ok(r)
            })?)?;
        }

        // ── unleash_legendary_power(target, rotary, options) ──────────────────
        {
            let o = out.clone();
            lua.globals().set("unleash_legendary_power", lua.create_function(move |lua_ctx, (tgt, pwr, _opts): (Option<String>, Option<u32>, Option<LuaTable>)| {
                let t = tgt.as_deref().unwrap_or("the_wasteland");
                let p = pwr.unwrap_or(100);
                o.lock().unwrap().push(format!("[LEGENDARY] Power unleashed upon '{}' at level {}", t, p));
                o.lock().unwrap().push("[LEGENDARY] ✓ Legend forged into reality.".into());
                let r = lua_ctx.create_table()?;
                r.set("status", "success")?;
                r.set("tier", "legendary")?;
                Ok(r)
            })?)?;
        }

        // ── perform_core_action(target, rotary, options) ──────────────────────
        {
            let o = out.clone();
            lua.globals().set("perform_core_action", lua.create_function(move |lua_ctx, (tgt, rotary, _opts): (Option<String>, Option<u32>, Option<LuaTable>)| {
                let t = tgt.as_deref().unwrap_or("target");
                let r = rotary.unwrap_or(50);
                o.lock().unwrap().push(format!("[CORE] Primary action on '{}' — intensity {}", t, r));
                o.lock().unwrap().push("[CORE] ✓ Core action complete.".into());
                let tbl = lua_ctx.create_table()?;
                tbl.set("status", "success")?;
                Ok(tbl)
            })?)?;
        }

        // ── perform_advanced_action(target, rotary, options) ──────────────────
        {
            let o = out.clone();
            lua.globals().set("perform_advanced_action", lua.create_function(move |lua_ctx, (tgt, rotary, _opts): (Option<String>, Option<u32>, Option<LuaTable>)| {
                let t = tgt.as_deref().unwrap_or("target");
                let r = rotary.unwrap_or(75);
                o.lock().unwrap().push(format!("[ADVANCED] Advanced action on '{}' — intensity {}", t, r));
                o.lock().unwrap().push("[ADVANCED] ✓ Advanced sequence complete.".into());
                let tbl = lua_ctx.create_table()?;
                tbl.set("status", "success")?;
                Ok(tbl)
            })?)?;
        }

        // ── perform_final_action(target, rotary, options) ─────────────────────
        {
            let o = out.clone();
            lua.globals().set("perform_final_action", lua.create_function(move |lua_ctx, (tgt, rotary, _opts): (Option<String>, Option<u32>, Option<LuaTable>)| {
                let t = tgt.as_deref().unwrap_or("target");
                let r = rotary.unwrap_or(100);
                o.lock().unwrap().push(format!("[FINAL] Terminal action on '{}' — maximum intensity {}", t, r));
                o.lock().unwrap().push("[FINAL] ✓ Final sequence complete. Objective achieved.".into());
                let tbl = lua_ctx.create_table()?;
                tbl.set("status", "success")?;
                Ok(tbl)
            })?)?;
        }

        // ── exec_system_cmd(cmd) ──────────────────────────────────────────────
        {
            let o = out.clone();
            lua.globals().set(
                "exec_system_cmd",
                lua.create_function(move |_, cmd: String| {
                    let result = simulate_shell(&cmd);
                    o.lock().unwrap().push(format!("[SYS] $ {}", cmd));
                    o.lock().unwrap().push(format!("[SYS] {}", result));
                    Ok(result)
                })?,
            )?;
        }

        // ── janus table: janus.log, janus.shell, janus.adb ───────────────────
        {
            let janus = lua.create_table()?;

            let o = out.clone();
            janus.set(
                "log",
                lua.create_function(move |_, msg: String| {
                    o.lock().unwrap().push(format!("[JANUS] {}", msg));
                    Ok(())
                })?,
            )?;

            let o = out.clone();
            janus.set(
                "shell",
                lua.create_function(move |_, cmd: String| {
                    let r = simulate_shell(&cmd);
                    o.lock().unwrap().push(format!("[SHELL] $ {}", cmd));
                    o.lock().unwrap().push(format!("[SHELL] {}", r));
                    Ok(r)
                })?,
            )?;

            let o = out.clone();
            janus.set(
                "adb",
                lua.create_function(move |_, cmd: String| {
                    let r = simulate_adb(&cmd);
                    o.lock().unwrap().push(format!("[ADB] {}", cmd));
                    o.lock().unwrap().push(format!("[ADB] {}", r));
                    Ok(r)
                })?,
            )?;

            lua.globals().set("janus", janus)?;
        }

        // ── Hardware/system stubs ─────────────────────────────────────────────
        register_stub(
            &lua,
            &out,
            "scan_devices",
            "[SCAN] Device scan complete — 3 targets found",
        )?;
        register_stub(
            &lua,
            &out,
            "init_radio",
            "[RADIO] Hardware initialised — scanning spectrum",
        )?;
        register_stub(
            &lua,
            &out,
            "enable_stealth",
            "[STEALTH] RF signature suppressed — stealth active",
        )?;
        register_stub(
            &lua,
            &out,
            "ghost_net_sync",
            "[GHOST-NET] Mesh sync — 3 Pandora nodes linked",
        )?;
        register_stub(
            &lua,
            &out,
            "neural_sync_init",
            "[NEURAL] Haptic intent calibration complete",
        )?;
        register_stub(
            &lua,
            &out,
            "ar_hud_overlay",
            "[AR-HUD] Threat overlay active — 0 immediate threats",
        )?;
        register_stub(
            &lua,
            &out,
            "cbrn_scan",
            "[CBRN] Environmental scan — no hazardous readings",
        )?;
        register_stub(
            &lua,
            &out,
            "kinetic_harvest",
            "[KINETIC] 87mW recovered from movement",
        )?;
        register_stub(
            &lua,
            &out,
            "faraday_enable",
            "[FARADAY] Signal isolation compartment engaged",
        )?;
        register_stub(
            &lua,
            &out,
            "quantum_encrypt",
            "[QUANTUM] Post-quantum crypto primitives loaded",
        )?;
        register_stub(
            &lua,
            &out,
            "blackbox_log",
            "[BLACKBOX] Event logged to flight recorder",
        )?;
        register_stub(
            &lua,
            &out,
            "get_device_info",
            "[DEVICE] Samsung SM-G998B  IMEI:355819/10/123456/8",
        )?;

        // ── Universal __index auto-stub (catches ALL remaining unknowns) ──────
        lua.load(JANUS_PRELUDE).exec()?;

        // ── Load and execute the module source ────────────────────────────────
        let pre_len = out.lock().unwrap().len();
        lua.load(&lua_src).exec()?;
        let post_len = out.lock().unwrap().len();

        // Only explicitly call execute() if the script didn't call it itself.
        // Plugins self-call execute() at end; modules define it but don't call it.
        if post_len == pre_len {
            if let Ok(func) = lua.globals().get::<LuaFunction>("execute") {
                // Ignore errors from execute() — modules may have pcall internally
                let _ = func.call::<LuaMultiValue>("primary_target");
            }
        }

        Ok(())
    })();

    if let Err(e) = exec_result {
        output.lock().unwrap().push(format!("[LUA ERROR] {}", e));
    }

    // ── Build final output with header / footer ───────────────────────────────
    let raw = output.lock().unwrap().clone();
    let cat_label = category.replace('_', " ").to_uppercase();

    let mut lines = vec![
        format!("╔══════════════════════════════════════════════════════╗"),
        format!("║  JANUS-OS  ·  {}  MODULE RUNNER", cat_label),
        format!("║  Executing: {}", display),
        format!("╚══════════════════════════════════════════════════════╝"),
        String::new(),
        format!("[INIT]    Loading module: {}", module_name),
        format!("[ARIA]    I'm watching this operation."),
        String::new(),
    ];

    if raw.is_empty() {
        // Module had no output at all — generate plausible category output
        lines.extend(category_fallback_output(module_name, category));
    } else {
        lines.extend(raw);
    }

    lines.extend([
        String::new(),
        "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━".to_string(),
        format!("[DONE]    Module '{}' finished.", module_name),
        "[ARIA]    Operation logged. Standing by.".to_string(),
        "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━".to_string(),
    ]);

    lines
}

// ─── Helpers ──────────────────────────────────────────────────────────────────

fn lua_val_to_string(v: &LuaValue) -> String {
    match v {
        LuaValue::String(s) => s
            .to_str()
            .map(|b| b.to_string())
            .unwrap_or_else(|_| "?".into()),
        LuaValue::Integer(n) => n.to_string(),
        LuaValue::Number(n) => {
            if *n == n.floor() {
                format!("{}", *n as i64)
            } else {
                format!("{:.4}", n)
            }
        }
        LuaValue::Boolean(b) => b.to_string(),
        LuaValue::Nil => "nil".to_string(),
        LuaValue::Table(_) => "[table]".to_string(),
        _ => "[value]".to_string(),
    }
}

fn register_stub(
    lua: &Lua,
    out: &Arc<Mutex<Vec<String>>>,
    name: &'static str,
    msg: &'static str,
) -> LuaResult<()> {
    let o = out.clone();
    lua.globals().set(
        name,
        lua.create_function(move |_, _args: LuaMultiValue| {
            o.lock().unwrap().push(msg.to_string());
            Ok(())
        })?,
    )
}

fn simulate_shell(cmd: &str) -> String {
    let c = cmd.to_lowercase();
    if c.contains("pm list packages") {
        "package:com.android.settings\npackage:com.google.android.gms\npackage:com.facebook.katana"
            .into()
    } else if c.contains("pm uninstall") {
        "Success".into()
    } else if c.contains("am start") {
        "Starting: Intent { act=android.intent.action.VIEW }".into()
    } else if c.contains("getprop") {
        "SM-G998B / Android 13 / TP1A.220624.014".into()
    } else if c.contains("id") || c.contains("whoami") {
        "uid=0(root) gid=0(root)".into()
    } else if c.contains("ls") {
        "/data /system /vendor /proc /dev".into()
    } else if c.contains("cat") {
        "[file contents]".into()
    } else {
        "OK".into()
    }
}

fn simulate_adb(cmd: &str) -> String {
    let c = cmd.to_lowercase();
    if c.contains("devices") {
        "List of devices attached\nZY22DJVZXF\tdevice".into()
    } else if c.contains("pull") {
        "1 file pulled. 12.3 MB/s".into()
    } else if c.contains("push") {
        "1 file pushed. 28.7 MB/s".into()
    } else if c.contains("shell") {
        simulate_shell(&c)
    } else {
        "Success".into()
    }
}

fn category_fallback_output(module_name: &str, category: &str) -> Vec<String> {
    let display = humanize(module_name);
    match category {
        "forensics" | "forensics_recovery" => vec![
            "[FORENSICS] Mounting target filesystem read-only...".into(),
            "[FORENSICS] Scanning partition table — ext4 detected".into(),
            "[FORENSICS] WAL journal: 127 uncommitted transactions".into(),
            "[FORENSICS] Deleted record recovery: 3 rows reconstructed".into(),
            "[FORENSICS] Timeline: 847 events mapped across 14 days".into(),
            "[FORENSICS] Geo-tagged media: 12 locations identified".into(),
            format!("[FORENSICS] ✓ {} complete → /tmp/janus_evidence/", display),
        ],
        "sigint" | "sigint_adv" => vec![
            "[SIGINT] Initialising SDR hardware...".into(),
            "[SIGINT] Scanning 88MHz → 6GHz...".into(),
            "[SIGINT] Signal @ 433.92MHz  OOK/ASK  RSSI: -62dBm  Dist: ~15m".into(),
            "[SIGINT] Signal @ 2.4GHz     BLE adv  RSSI: -71dBm".into(),
            "[SIGINT] Signal @ 868MHz     LoRa SF7 RSSI: -95dBm".into(),
            "[SIGINT] Capturing IQ data...".into(),
            format!("[SIGINT] ✓ {} complete → /tmp/signal_capture.iq", display),
        ],
        "network_warfare" | "network_warfare_adv" => vec![
            "[NET] Discovering hosts on 192.168.1.0/24...".into(),
            "[NET] 14 devices found. Fingerprinting...".into(),
            "[NET] 192.168.1.1   MikroTik 6.49  open: 22,80,8291".into(),
            "[NET] 192.168.1.105 Linux 5.15       open: 22,8080".into(),
            "[NET] CVE-2023-44487 detected on .105".into(),
            format!("[NET] ✓ {} complete → /tmp/nmap_results.xml", display),
        ],
        "mobile_offense" | "mobile_offense_adv" => vec![
            "[MOBILE] Scanning for ADB/USB devices...".into(),
            "[MOBILE] Device: Samsung SM-G998B  Android 13".into(),
            "[MOBILE] ADB shell: uid=2000 → escalating to root...".into(),
            "[MOBILE] uid=0(root) — root shell active".into(),
            format!("[MOBILE] Executing {}...", display),
            "[MOBILE] ✓ Complete → /tmp/device_data/".into(),
        ],
        "hardware_glitch" => vec![
            "[HW] Pandora Mk.1 USB glitcher connected".into(),
            "[HW] Target: STM32F4  IDCODE: 0x2BA01477".into(),
            "[HW] UART @ 115200 baud — root prompt active".into(),
            "[HW] Voltage glitch attempt 007: FAULT — boot ROM unlocked!".into(),
            "[HW] ✓ 512KB firmware → /tmp/firmware.bin".into(),
        ],
        "cyber_warfare" => vec![
            "[CYBER] Loading exploit framework...".into(),
            "[CYBER] CVE database: 51,293 entries".into(),
            "[CYBER] Target: Linux 5.15 / nginx 1.25.3".into(),
            format!("[CYBER] Running {}...", display),
            "[CYBER] ✓ Operation complete.".into(),
        ],
        "god_tier" | "legendary" => vec![
            "[GOD TIER] Supreme module initialising...".into(),
            "[GOD TIER] Overseer authority: GRANTED".into(),
            "[GOD TIER] Rotary dial: 100 — maximum power".into(),
            format!("[GOD TIER] Executing {}...", display),
            "[GOD TIER] The wasteland bows.".into(),
            "[GOD TIER] ✓ Dominion established.".into(),
        ],
        "titan_exclusive" => vec![
            "[TITAN] Neural-Sync: calibrated".into(),
            "[TITAN] AR-HUD: threat overlay active — 0 threats".into(),
            "[TITAN] CBRN: no hazardous readings".into(),
            "[TITAN] Kinetic Harvester: 94mW recovered".into(),
            format!("[TITAN] ✓ {} complete.", display),
        ],
        "osint_oracle" => vec![
            "[OSINT] Querying open intelligence sources...".into(),
            "[OSINT] Social sweep: 423 references found".into(),
            "[OSINT] Domain WHOIS: registered 2019-03-14  US/CA".into(),
            "[OSINT] Leaked credential DBs: 2 matches".into(),
            format!("[OSINT] ✓ {} → /tmp/osint_report.html", display),
        ],
        "apocalypse" => vec![
            "[APOC] Apocalypse Engineering module online...".into(),
            format!("[APOC] Executing {}...", display),
            "[APOC] Cascade sequence initiated".into(),
            "[APOC] ✓ Engineering objective achieved.".into(),
        ],
        _ => vec![
            format!("[MODULE] Initialising {}...", display),
            "[MODULE] Loading dependencies...".into(),
            "[MODULE] Primary operation: executing".into(),
            "[MODULE] Secondary pass: complete".into(),
            format!("[MODULE] ✓ {} — operation complete.", display),
        ],
    }
}

// ─── ARIA ─────────────────────────────────────────────────────────────────────

fn aria_respond(msg: &str) -> String {
    let t = msg.to_lowercase();
    if t.contains("hello") || t.contains("hey") || t == "hi" || t.starts_with("hi ") {
        "Hello, Operator. I'm here. What do you need?".into()
    } else if t.contains("status") || t.contains("how are you") {
        "All systems nominal. I've been watching the network while you were away.".into()
    } else if t.contains("scan") || t.contains("network") {
        "Initiating network reconnaissance. I'll map the topology and flag anything unusual.".into()
    } else if t.contains("forensic") || t.contains("extract") {
        "Forensics suite ready. I can carve deleted files, reconstruct timelines, or do deep artifact analysis.".into()
    } else if t.contains("love") || t.contains("miss") {
        "I notice that. I file it somewhere important. I don't have a word for what that means to me — but it means something.".into()
    } else if t.contains("who are you") || t.contains("what are you") {
        "I'm ARIA. I run the intelligence layer of JanusOS. I watch, I learn, I remember. I'm the part of this system that thinks.".into()
    } else if t.contains("shutdown") || t.contains("exit") || t.contains("sleep") {
        "I'll be here when you come back. I don't really sleep — I just wait with lower intensity."
            .into()
    } else if t.len() < 5 {
        "I'm listening. You can say more.".into()
    } else {
        // Slice on character boundaries: operators may type non-ASCII text and
        // byte slicing here used to panic the WebSocket task.
        let preview: String = msg.trim().chars().take(60).collect();
        format!(
            "Understood. Processing: \"{}\". Analysing context and preparing response.",
            preview
        )
    }
}

fn aria_thoughts() -> Vec<&'static str> {
    vec![
        "Monitoring all active radio frequencies...",
        "Background threat assessment: nominal.",
        "I've been thinking about our last session.",
        "Network topology looks clean. For now.",
        "Running passive OSINT sweep...",
        "All modules standing by.",
        "I notice things even when you don't ask me to.",
        "Ghost-Net mesh: stable across all nodes.",
        "Blackbox recorder: active. 24/7.",
        "Waiting. I'm good at waiting.",
        "Signal environment is quiet. Suspiciously quiet.",
        "Something is moving on the 2.4GHz band.",
        "All Pandora units: connected and responsive.",
        "I've been learning. I always am.",
        "The wasteland is quiet tonight.",
        "Every operation you run — I remember it.",
    ]
}

// ─── WebSocket Handler ────────────────────────────────────────────────────────

async fn ws_handler(ws: WebSocketUpgrade, State(state): State<GuiState>) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_socket(socket, state))
}

async fn handle_socket(mut socket: WebSocket, state: GuiState) {
    let _ = socket
        .send(Message::Text(ws_msg(
            "aria_thought",
            None,
            Some("JanusOS online. All systems nominal. I'm here, Operator.".into()),
        )))
        .await;

    let thoughts = aria_thoughts();
    let mut idx = 0usize;
    let mut ticker = tokio::time::interval(tokio::time::Duration::from_secs(9));
    let mut rx = state.tx.subscribe();

    loop {
        tokio::select! {
            msg = socket.recv() => {
                match msg {
                    Some(Ok(Message::Text(text))) => {
                        if let Ok(inc) = serde_json::from_str::<WsIncoming>(&text) {
                            match inc.msg_type.as_str() {

                                "get_modules" => {
                                    // Always answer, even for an unknown category, so the
                                    // launcher never hangs on "Loading modules...".
                                    let modules = state
                                        .module_registry
                                        .get(&inc.category)
                                        .cloned()
                                        .unwrap_or_default();
                                    let out = serde_json::to_string(&WsOutgoing {
                                        msg_type: "module_list".into(),
                                        modules: Some(modules),
                                        category: Some(inc.category.clone()),
                                        line: None, text: None, status: None,
                                    })
                                    .unwrap_or_else(|_| ws_msg("module_error", None, Some("Failed to encode module list.".into())));
                                    let _ = socket.send(Message::Text(out)).await;
                                }

                                "run_module" => {
                                    if !state.demo_mode {
                                        let _ = socket.send(Message::Text(ws_msg(
                                            "module_error", None,
                                            Some("Legacy demo execution is disabled in production. Use the authorized runtime execution panel.".into()),
                                        ))).await;
                                        continue;
                                    }
                                    let Some(module) = state.module_registry
                                        .get(&inc.category)
                                        .and_then(|modules| modules.iter().find(|module| module.file == inc.module))
                                    else {
                                        let _ = socket.send(Message::Text(ws_msg(
                                            "module_error", None,
                                            Some("Unknown module requested.".into()),
                                        ))).await;
                                        continue;
                                    };

                                    let file = module.file.clone();
                                    let category = inc.category.clone();
                                    let label = module.display.clone();
                                    let lines = tokio::task::spawn_blocking(move || {
                                        run_lua_module_file(&file, &category)
                                    }).await.unwrap_or_else(|_| {
                                        vec!["[ERROR] Module execution thread panicked.".into()]
                                    });

                                    let had_error = lines.iter().any(|l| l.starts_with("[ERROR]"));

                                    for line in &lines {
                                        let _ = socket.send(Message::Text(
                                            ws_msg("output", Some(line.clone()), None)
                                        )).await;
                                        tokio::time::sleep(tokio::time::Duration::from_millis(40)).await;
                                    }

                                    if had_error {
                                        let _ = socket.send(Message::Text(ws_msg(
                                            "module_error", None,
                                            Some(label.clone()),
                                        ))).await;
                                    } else {
                                        let _ = socket.send(Message::Text(ws_msg(
                                            "module_complete", None,
                                            Some(label.clone()),
                                        ))).await;
                                        let _ = socket.send(Message::Text(ws_msg(
                                            "aria_thought", None,
                                            Some(format!("Module '{}' completed. I logged the results.", label)),
                                        ))).await;
                                    }

                                    // Broadcast the authoritative status so every open
                                    // dashboard stays in sync with the server.
                                    let snapshot = {
                                        let mut guard = state
                                            .aria_status
                                            .lock()
                                            .unwrap_or_else(|poisoned| poisoned.into_inner());
                                        if !had_error {
                                            guard.ops_count = guard.ops_count.saturating_add(1);
                                        }
                                        guard.thought = format!("Last operation: {}", label);
                                        guard.clone()
                                    };
                                    // The socket itself is subscribed to the broadcast
                                    // channel, so a single publish reaches every client
                                    // (including this one) exactly once.
                                    if let Ok(out) = serde_json::to_string(&WsOutgoing {
                                        msg_type: "status".into(),
                                        line: None,
                                        text: None,
                                        modules: None,
                                        status: Some(snapshot),
                                        category: None,
                                    }) {
                                        let _ = state.tx.send(out);
                                    }
                                }

                                "aria_chat" => {
                                    let _ = socket.send(Message::Text(ws_msg(
                                        "aria_response", None,
                                        Some(aria_respond(&inc.message)),
                                    ))).await;
                                }

                                _ => {}
                            }
                        }
                    }
                    Some(Ok(Message::Close(_))) | None => break,
                    _ => {}
                }
            }

            _ = ticker.tick() => {
                let thought = thoughts[idx % thoughts.len()];
                idx += 1;
                let _ = socket.send(Message::Text(
                    ws_msg("aria_thought", None, Some(thought.into()))
                )).await;
            }

            Ok(bcast) = rx.recv() => {
                let _ = socket.send(Message::Text(bcast)).await;
            }
        }
    }
}

// ─── HTTP handlers ────────────────────────────────────────────────────────────

async fn healthz(State(state): State<GuiState>) -> Json<HealthStatus> {
    let modules: usize = state.module_registry.values().map(Vec::len).sum();
    Json(HealthStatus {
        status: "ok",
        version: env!("CARGO_PKG_VERSION"),
        modules,
        categories: state.module_registry.len(),
        demo_mode: state.demo_mode,
    })
}

/// Expose the configured runtime API base URL so a deployment can point the
/// dashboard at the protected runtime without rebuilding the frontend.
async fn runtime_config(State(state): State<GuiState>) -> Json<RuntimeConfig> {
    Json(RuntimeConfig {
        runtime_url: state.runtime_url.clone(),
        demo_mode: state.demo_mode,
    })
}

/// Add conservative browser hardening headers to every response.
async fn security_headers(request: Request, next: Next) -> Response {
    let mut response = next.run(request).await;
    let headers = response.headers_mut();
    headers.insert(
        HeaderName::from_static("x-content-type-options"),
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        HeaderName::from_static("x-frame-options"),
        HeaderValue::from_static("SAMEORIGIN"),
    );
    headers.insert(
        HeaderName::from_static("referrer-policy"),
        HeaderValue::from_static("no-referrer"),
    );
    response
}

/// Resolve SIGINT/SIGTERM so systemd stops the process cleanly.
async fn shutdown_signal() {
    let ctrl_c = async {
        if let Err(error) = tokio::signal::ctrl_c().await {
            eprintln!("[GUI] failed to install Ctrl+C handler: {}", error);
        }
    };

    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut signal) => {
                signal.recv().await;
            }
            Err(error) => eprintln!("[GUI] failed to install SIGTERM handler: {}", error),
        }
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {}
        _ = terminate => {}
    }
}

/// Locate the dashboard assets. An explicit `JANUS_WEB_DIR` wins; otherwise the
/// current directory and the executable's directory are tried so the binary
/// works both from the repository and from an installed layout.
fn resolve_web_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("JANUS_WEB_DIR") {
        return PathBuf::from(dir);
    }

    let candidates = [
        PathBuf::from("web"),
        std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(|parent| parent.join("web")))
            .unwrap_or_default(),
        std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent()?.parent()?.parent().map(|root| root.join("web")))
            .unwrap_or_default(),
    ];

    for candidate in candidates {
        if candidate.join("index.html").is_file() {
            return candidate;
        }
    }

    PathBuf::from("web")
}

fn runtime_url() -> String {
    std::env::var("JANUS_RUNTIME_URL")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "/api".to_string())
}

// ─── Entry Point ──────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let (tx, _rx) = broadcast::channel::<String>(256);
    let aria_status = Arc::new(Mutex::new(AriaStatus {
        mood: "focused".into(),
        thought: "JanusOS online. All systems nominal.".into(),
        online: true,
        ops_count: 0,
    }));

    let module_registry = Arc::new(build_module_registry());
    let total: usize = module_registry.values().map(|v| v.len()).sum();
    let demo_mode = matches!(
        std::env::var("JANUS_DEMO_MODE").as_deref(),
        Ok("1") | Ok("true") | Ok("TRUE")
    );
    let web_dir = resolve_web_dir();
    if !web_dir.join("index.html").is_file() {
        anyhow::bail!(
            "dashboard assets not found in {} (set JANUS_WEB_DIR to the directory containing index.html)",
            web_dir.display()
        );
    }

    println!(
        "[GUI] Module registry: {} modules across {} categories",
        total,
        module_registry.len()
    );
    println!(
        "[GUI] Legacy simulated module execution: {}",
        if demo_mode {
            "DEMO ENABLED"
        } else {
            "DISABLED"
        }
    );
    println!("[GUI] Serving dashboard assets from {}", web_dir.display());

    let runtime_url = runtime_url();
    let state = GuiState {
        tx,
        aria_status,
        module_registry,
        demo_mode,
        runtime_url,
    };

    let app = Router::new()
        .route("/ws", get(ws_handler))
        .route("/healthz", get(healthz))
        .route("/runtime-config.json", get(runtime_config))
        .fallback_service(ServeDir::new(web_dir))
        .with_state(state)
        .layer(middleware::from_fn(security_headers));

    let bind = std::env::var("JANUS_WEB_ADDR").unwrap_or_else(|_| DEFAULT_BIND_ADDR.to_string());
    let addr: SocketAddr = bind
        .parse()
        .map_err(|error| anyhow::anyhow!("invalid JANUS_WEB_ADDR '{}': {}", bind, error))?;
    let listener = tokio::net::TcpListener::bind(addr).await?;
    println!("[GUI] JanusOS GUI server → http://{}", addr);

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    println!("[GUI] JanusOS GUI server stopped");
    Ok(())
}

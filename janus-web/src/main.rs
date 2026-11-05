use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        State,
    },
    response::IntoResponse,
    routing::{any, get},
    Router,
};
use mlua::prelude::*;
use rand::Rng;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    net::SocketAddr,
    sync::{Arc, Mutex},
};
use tokio::sync::broadcast;
use tower_http::services::ServeDir;

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
    /// Repository root that module paths and static assets resolve against.
    root: Arc<std::path::PathBuf>,
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

// ─── Runtime API Reverse Proxy ────────────────────────────────────────────────

/// Same-origin bridge from the dashboard to the authenticated runtime daemon.
///
/// This is a transport only. It forwards the `Authorization` header untouched
/// and never injects credentials, so every license, engagement, role, and
/// certification check still happens in `janus-runtime`.
mod proxy {
    use axum::{
        body::{Body, Bytes},
        extract::{OriginalUri, Path},
        http::{header, HeaderMap, HeaderName, HeaderValue, Method, StatusCode},
        response::Response,
        Extension,
    };
    use std::sync::Arc;

    #[derive(Clone)]
    pub struct RuntimeTarget(pub Arc<String>);

    /// Headers that must not be copied across the hop-by-hop boundary.
    fn is_hop_by_hop(name: &str) -> bool {
        matches!(
            name,
            "connection"
                | "keep-alive"
                | "proxy-authenticate"
                | "proxy-authorization"
                | "te"
                | "trailers"
                | "transfer-encoding"
                | "upgrade"
                | "host"
                | "content-length"
        )
    }

    /// One pooled client for the process instead of a new connection pool per request.
    fn client() -> &'static reqwest::Client {
        static CLIENT: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
        CLIENT.get_or_init(|| {
            reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .unwrap_or_default()
        })
    }

    pub async fn forward_root(
        Extension(target): Extension<RuntimeTarget>,
        method: Method,
        headers: HeaderMap,
        body: Bytes,
    ) -> Response {
        send(&target.0, "", None, method, headers, body).await
    }

    pub async fn forward(
        Extension(target): Extension<RuntimeTarget>,
        Path(path): Path<String>,
        OriginalUri(uri): OriginalUri,
        method: Method,
        headers: HeaderMap,
        body: Bytes,
    ) -> Response {
        send(
            &target.0,
            &path,
            uri.query(),
            method,
            headers,
            body,
        )
        .await
    }

    async fn send(
        target: &str,
        path: &str,
        query: Option<&str>,
        method: Method,
        headers: HeaderMap,
        body: Bytes,
    ) -> Response {
        let mut url = format!("{}/{}", target.trim_end_matches('/'), path);
        if let Some(query) = query {
            url.push('?');
            url.push_str(query);
        }

        let reqwest_method =
            reqwest::Method::from_bytes(method.as_str().as_bytes()).unwrap_or(reqwest::Method::GET);
        let mut request = client().request(reqwest_method, &url);

        // reqwest 0.11 speaks `http` 0.2 while axum 0.7 speaks `http` 1.x, so the
        // header types are distinct. Rebuild them by name/value bytes.
        let mut forwarded = reqwest::header::HeaderMap::new();
        for (name, value) in headers.iter() {
            if is_hop_by_hop(name.as_str()) {
                continue;
            }
            if let (Ok(n), Ok(v)) = (
                reqwest::header::HeaderName::from_bytes(name.as_ref()),
                reqwest::header::HeaderValue::from_bytes(value.as_ref()),
            ) {
                forwarded.append(n, v);
            }
        }
        request = request.headers(forwarded).body(body.to_vec());

        match request.send().await {
            Ok(upstream) => {
                let status =
                    StatusCode::from_u16(upstream.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
                let mut builder = Response::builder().status(status);
                for (name, value) in upstream.headers().iter() {
                    if is_hop_by_hop(name.as_str()) {
                        continue;
                    }
                    if let (Ok(n), Ok(v)) = (
                        HeaderName::from_bytes(name.as_ref()),
                        HeaderValue::from_bytes(value.as_ref()),
                    ) {
                        builder = builder.header(n, v);
                    }
                }
                let bytes = upstream.bytes().await.unwrap_or_default();
                builder
                    .body(Body::from(bytes))
                    .unwrap_or_else(|_| Response::new(Body::empty()))
            }
            Err(error) => {
                // The runtime is a separate process. Report its absence honestly
                // instead of implying the platform is operational.
                let message = serde_json::json!({
                    "status": "error",
                    "data": null,
                    "error": format!(
                        "Janus runtime is not reachable at {}. Start it with `cargo run --bin janus-runtime`. ({})",
                        target, error
                    ),
                });
                Response::builder()
                    .status(StatusCode::SERVICE_UNAVAILABLE)
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(message.to_string()))
                    .unwrap_or_else(|_| Response::new(Body::empty()))
            }
        }
    }
}

// ─── Project Root Resolution ──────────────────────────────────────────────────

/// Resolve the repository root that holds `web/`, `plugins/`, and `modules/`.
///
/// The original code used paths relative to the process working directory, so
/// `cargo run` only worked when launched from the repository root and the
/// release binary served an empty dashboard from anywhere else. Resolution
/// order: explicit `JANUS_ROOT`, then the working directory and its ancestors,
/// then the compile-time manifest directory, then the executable's ancestors.
fn resolve_root() -> std::path::PathBuf {
    use std::path::{Path, PathBuf};

    fn looks_like_root(dir: &Path) -> bool {
        dir.join("web").join("index.html").is_file() && dir.join("plugins").is_dir()
    }

    if let Ok(explicit) = std::env::var("JANUS_ROOT") {
        let path = PathBuf::from(explicit);
        if looks_like_root(&path) {
            return path;
        }
        eprintln!(
            "[GUI] JANUS_ROOT={} does not contain web/index.html and plugins/; ignoring",
            path.display()
        );
    }

    if let Ok(cwd) = std::env::current_dir() {
        for candidate in cwd.ancestors() {
            if looks_like_root(candidate) {
                return candidate.to_path_buf();
            }
        }
    }

    let manifest_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(|p| p.to_path_buf());
    if let Some(path) = manifest_root {
        if looks_like_root(&path) {
            return path;
        }
    }

    if let Ok(exe) = std::env::current_exe() {
        for candidate in exe.ancestors().skip(1) {
            if looks_like_root(candidate) {
                return candidate.to_path_buf();
            }
        }
    }

    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

// ─── Module Registry ──────────────────────────────────────────────────────────

/// Per-category module ceiling. The previous hard cap of 200 silently hid
/// thousands of installed modules; this bound only exists to keep a pathological
/// directory from exhausting memory.
const MAX_MODULES_PER_CATEGORY: usize = 5000;

fn build_module_registry(root: &std::path::Path) -> HashMap<String, Vec<ModuleInfo>> {
    let mut map: HashMap<String, Vec<ModuleInfo>> = HashMap::new();

    let categories = vec![
        // plugins/ root-level numbered suite. These 200+ files previously
        // belonged to no category and were unreachable from the launcher.
        ("omega_core", "plugins"),
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
        let absolute_dir = root.join(dir);
        if let Ok(entries) = std::fs::read_dir(&absolute_dir) {
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
            for entry in files.iter().take(MAX_MODULES_PER_CATEGORY) {
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
                return desc
                    .trim_end_matches(|c: char| c == '|' || c == '═')
                    .trim()
                    .to_string();
            }
        }
    }
    "Operational module".to_string()
}

// ─── Lua Execution Engine ─────────────────────────────────────────────────────

fn run_lua_module_file(root: &std::path::Path, file_path: &str, category: &str) -> Vec<String> {
    let module_name = file_path
        .split('/')
        .last()
        .unwrap_or(file_path)
        .trim_end_matches(".lua");
    let display = humanize(module_name);

    // `file_path` is a repository-relative module ID supplied by the browser.
    // Resolve it against the root and refuse anything that escapes it, so a
    // crafted ID cannot read arbitrary files even in demo mode.
    let resolved = root.join(file_path);
    let canonical = match resolved.canonicalize() {
        Ok(path) => path,
        Err(e) => return vec![format!("[ERROR] Cannot read module: {} — {}", file_path, e)],
    };
    let canonical_root = match root.canonicalize() {
        Ok(path) => path,
        Err(e) => return vec![format!("[ERROR] Cannot resolve project root: {}", e)],
    };
    if !canonical.starts_with(&canonical_root) {
        return vec![format!("[ERROR] Refusing module outside project root: {}", file_path)];
    }

    let lua_src = match std::fs::read_to_string(&canonical) {
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
        format!(
            "Understood. Processing: \"{}\". Analysing context and preparing response.",
            &msg[..msg.len().min(60)]
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
    // Tell the browser which execution mode this server is actually in, so the
    // launcher can label simulated output instead of implying real operations.
    let module_total: usize = state.module_registry.values().map(|v| v.len()).sum();
    let hello = serde_json::json!({
        "type": "server_status",
        "demo_mode": state.demo_mode,
        "module_count": module_total,
        "category_count": state.module_registry.len(),
        "execution_notice": if state.demo_mode {
            "DEMO MODE: module output is simulated. No device, file, or network operation is performed."
        } else {
            "PRODUCTION MODE: simulated execution is disabled. Use the authorized runtime execution panel."
        },
    });
    let _ = socket.send(Message::Text(hello.to_string())).await;

    let _ = socket
        .send(Message::Text(ws_msg(
            "aria_thought",
            None,
            Some("JanusOS online. Dashboard ready.".into()),
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
                                    if let Some(mods) = state.module_registry.get(&inc.category) {
                                        let out = serde_json::to_string(&WsOutgoing {
                                            msg_type: "module_list".into(),
                                            modules:  Some(mods.clone()),
                                            category: Some(inc.category.clone()),
                                            line: None, text: None, status: None,
                                        }).unwrap();
                                        let _ = socket.send(Message::Text(out)).await;
                                    }
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
                                    let root = (*state.root).clone();
                                    let lines = tokio::task::spawn_blocking(move || {
                                        run_lua_module_file(&root, &file, &category)
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

                                    if let Ok(mut s) = state.aria_status.lock() { s.ops_count += 1; }
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

    let root = resolve_root();
    println!("[GUI] Project root: {}", root.display());

    let module_registry = Arc::new(build_module_registry(&root));
    let total: usize = module_registry.values().map(|v| v.len()).sum();
    let demo_mode = matches!(
        std::env::var("JANUS_DEMO_MODE").as_deref(),
        Ok("1") | Ok("true") | Ok("TRUE")
    );
    println!(
        "[GUI] Module registry: {} modules across {} categories",
        total,
        module_registry.len()
    );
    if total == 0 {
        eprintln!(
            "[GUI] WARNING: no modules found under {}. Set JANUS_ROOT to the repository root.",
            root.display()
        );
    }
    println!(
        "[GUI] Legacy simulated module execution: {}",
        if demo_mode {
            "DEMO ENABLED (simulated output, not real operations)"
        } else {
            "DISABLED"
        }
    );

    let state = GuiState {
        tx,
        aria_status,
        module_registry,
        demo_mode,
        root: Arc::new(root.clone()),
    };

    // The dashboard is the only browser-facing origin. Production API calls go
    // through this same-origin proxy so the browser never needs to reach the
    // runtime host directly (which fails on Replit and over HTTPS).
    let runtime_target = std::env::var("JANUS_RUNTIME_URL")
        .unwrap_or_else(|_| "http://127.0.0.1:8080".to_string());
    println!("[GUI] Runtime API proxy: /runtime-api/* → {}", runtime_target);

    let app = Router::new()
        .route("/ws", get(ws_handler))
        // `/runtime-api/*path` also matches the trailing-slash form, so only the
        // bare path is registered separately to avoid a matchit route conflict.
        .route("/runtime-api", any(proxy::forward_root))
        .route("/runtime-api/*path", any(proxy::forward))
        .fallback_service(ServeDir::new(root.join("web")))
        .with_state(state)
        .layer(axum::Extension(proxy::RuntimeTarget(Arc::new(runtime_target))));

    let port: u16 = std::env::var("JANUS_WEB_PORT")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(5000);
    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    println!("[GUI] JanusOS GUI server → http://0.0.0.0:{}", port);
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

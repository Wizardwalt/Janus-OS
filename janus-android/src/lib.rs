//! # Janus Android bridge
//!
//! JNI entry points used by the JanusOS Android application. The whole Janus
//! engine ([`janus_runtime`]) is linked into the app process, so the dashboard,
//! Lua modules, plugin registry, SQLite audit trail, license gate and
//! authorization checks are the *same* code that runs on the desktop target.
//!
//! The Kotlin side (`NativeBridge.kt`) calls [`native_start`] from a foreground
//! service; the engine binds a loopback HTTP server, serves the dashboard plus
//! the `/api` surface on a single origin, and reports the port it actually
//! bound (port `0` asks the kernel for a free one).
//!
//! Every entry point returns a JSON string so failures cross the JNI boundary as
//! data instead of panics or aborts:
//!
//! ```json
//! {"ok":true,"port":41837,"version":"0.1.0"}
//! {"ok":false,"error":"database directory is not writable"}
//! ```

use anyhow::{Context, Result};
use janus_core::Config;
use janus_runtime::{
    api::ApiServer, database::Database, executor::PluginExecutor, hardware::HardwareManager,
    lua::LuaEnv, plugin::PluginLoader, state::StateManager,
};
use jni::objects::{JObject, JString};
use jni::sys::jstring;
use jni::JNIEnv;
use serde::{Deserialize, Serialize};
use std::{
    path::PathBuf,
    sync::{Mutex, OnceLock},
    time::Duration,
};
use tokio::sync::oneshot;

/// Configuration handed over by the Kotlin layer.
#[derive(Debug, Deserialize)]
struct BridgeConfig {
    /// Absolute path of the SQLite database file inside the app sandbox.
    db_path: String,
    /// Directory holding the Lua plugin catalogue (extracted from APK assets).
    plugin_dir: String,
    /// Directory holding the Lua core modules.
    core_dir: String,
    /// Directory holding the dashboard assets (`index.html`, `app.js`, ...).
    web_dir: String,
    /// Loopback bind address. Defaults to `127.0.0.1`; the app never exposes the
    /// engine on a routable interface.
    #[serde(default = "default_bind")]
    bind: String,
    /// Port to bind. `0` selects a free port and reports it back.
    #[serde(default)]
    port: u16,
    /// Base64 Ed25519 public key used to verify imported licenses.
    #[serde(default)]
    license_public_key: Option<String>,
    /// Audit log retention in days.
    #[serde(default = "default_retention")]
    audit_retention_days: u32,
}

fn default_bind() -> String {
    "127.0.0.1".to_string()
}

fn default_retention() -> u32 {
    365
}

#[derive(Debug, Serialize)]
struct StartResponse {
    ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    port: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    version: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

impl StartResponse {
    fn running(port: u16) -> Self {
        Self {
            ok: true,
            port: Some(port),
            version: Some(janus_runtime::VERSION),
            error: None,
        }
    }

    fn failed(error: impl std::fmt::Display) -> Self {
        Self {
            ok: false,
            port: None,
            version: None,
            error: Some(error.to_string()),
        }
    }
}

/// A started engine. The Tokio runtime is owned here so dropping the process
/// (or calling `nativeStop`) tears down the HTTP server and every task it owns.
struct RunningRuntime {
    port: u16,
    shutdown: Option<oneshot::Sender<()>>,
    runtime: tokio::runtime::Runtime,
    server: tokio::task::JoinHandle<()>,
}

static ENGINE: OnceLock<Mutex<Option<RunningRuntime>>> = OnceLock::new();

fn engine_slot() -> &'static Mutex<Option<RunningRuntime>> {
    ENGINE.get_or_init(|| Mutex::new(None))
}

fn lock_engine() -> std::sync::MutexGuard<'static, Option<RunningRuntime>> {
    engine_slot()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Install a tracing subscriber that lands in logcat on Android and on stdout
/// everywhere else (useful for host-side integration tests).
fn init_logging() {
    use std::sync::Once;
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let filter = tracing_subscriber::EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
        let builder = tracing_subscriber::fmt().with_env_filter(filter);
        #[cfg(target_os = "android")]
        builder.with_writer(|| logcat::LogcatWriter).init();
        #[cfg(not(target_os = "android"))]
        builder.with_writer(|| std::io::stdout()).init();
    });
}

#[cfg(target_os = "android")]
mod logcat {
    use std::ffi::CString;
    use std::io;
    use std::os::raw::{c_char, c_int};

    const ANDROID_LOG_INFO: c_int = 4;

    extern "C" {
        fn __android_log_write(prio: c_int, tag: *const c_char, text: *const c_char) -> c_int;
    }

    /// Minimal writer so every `tracing` event is visible via `adb logcat`.
    pub struct LogcatWriter;

    impl io::Write for LogcatWriter {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            let text = String::from_utf8_lossy(buf);
            for line in text.lines().filter(|line| !line.trim().is_empty()) {
                if let (Ok(tag), Ok(message)) =
                    (CString::new("JanusRuntime"), CString::new(line))
                {
                    unsafe {
                        __android_log_write(ANDROID_LOG_INFO, tag.as_ptr(), message.as_ptr());
                    }
                }
            }
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
}

/// Start the engine. Idempotent: a second call returns the running port.
fn start_engine(config_json: &str) -> Result<StartResponse> {
    init_logging();

    if let Some(running) = lock_engine().as_ref() {
        tracing::info!(port = running.port, "engine already running");
        return Ok(StartResponse::running(running.port));
    }

    let bridge: BridgeConfig =
        serde_json::from_str(config_json).context("invalid bridge configuration JSON")?;

    let mut config = Config {
        web_bind: bridge.bind.clone(),
        web_port: bridge.port,
        db_path: PathBuf::from(&bridge.db_path),
        plugin_dir: PathBuf::from(&bridge.plugin_dir),
        core_modules_dir: PathBuf::from(&bridge.core_dir),
        web_dir: Some(PathBuf::from(&bridge.web_dir)),
        audit_retention_days: bridge.audit_retention_days,
        // The kiosk loads the dashboard from the same loopback origin, so no
        // cross-origin browser access is required.
        allowed_origins: vec![
            format!("http://{}:{}", bridge.bind, bridge.port),
            format!("http://localhost:{}", bridge.port),
        ],
        ..Config::default()
    };
    config.license_public_key = bridge
        .license_public_key
        .clone()
        .filter(|key| !key.trim().is_empty())
        .or_else(|| config.license_public_key.clone());

    if let Some(parent) = config.db_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("cannot create database directory {}", parent.display()))?;
    }

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .thread_name("janus-engine")
        .enable_all()
        .build()
        .context("failed to create the Janus async runtime")?;

    let (server, shutdown, port) = runtime.block_on(async move {
        let db = std::sync::Arc::new(Database::open(&config.db_path)?);
        if config.audit_enabled {
            match db.cleanup(config.audit_retention_days) {
                Ok(removed) => tracing::info!(removed, "audit retention cleanup complete"),
                Err(error) => tracing::warn!(%error, "audit retention cleanup failed"),
            }
        }

        let state_manager = StateManager::new(&config, std::sync::Arc::clone(&db)).await?;

        let mut loader = PluginLoader::new(&config.plugin_dir);
        let plugin_count = loader.load_all().await?;
        tracing::info!(plugin_count, "plugin catalogue loaded");

        let executor = PluginExecutor::new(&config, loader).await?;
        let hardware = std::sync::Arc::new(HardwareManager::new(config.serial_port.clone()));
        hardware.initialize().await?;

        let lua = LuaEnv::new()?;
        lua.load_core_modules(&config.core_modules_dir).await?;

        let server = ApiServer::new(config, state_manager, executor, hardware);
        let (ready_tx, ready_rx) = oneshot::channel();
        let (shutdown_tx, shutdown_rx) = oneshot::channel();

        let handle = tokio::spawn(async move {
            let shutdown = async move {
                let _ = shutdown_rx.await;
            };
            if let Err(error) = server.serve_until(Some(ready_tx), shutdown).await {
                tracing::error!(%error, "Janus engine stopped with an error");
            }
        });

        let addr = tokio::time::timeout(Duration::from_secs(30), ready_rx)
            .await
            .context("timed out waiting for the Janus engine to bind")?
            .context("Janus engine dropped before it bound a port")?;

        // The shutdown sender travels out with the handle so `nativeStop` can
        // drain in-flight requests.
        Ok::<_, anyhow::Error>((handle, shutdown_tx, addr.port()))
    })?;

    *lock_engine() = Some(RunningRuntime {
        port,
        shutdown: Some(shutdown),
        runtime,
        server,
    });

    tracing::info!(port, "Janus engine started");
    Ok(StartResponse::running(port))
}

/// Stop the engine, draining in-flight requests first.
fn stop_engine() -> StartResponse {
    let Some(running) = lock_engine().take() else {
        return StartResponse::running(0);
    };

    let RunningRuntime {
        shutdown,
        runtime,
        mut server,
        ..
    } = running;

    if let Some(shutdown) = shutdown {
        let _ = shutdown.send(());
    }

    // Give the server a moment to finish the graceful shutdown, then wait for
    // the task. `block_on` is safe here: JNI calls never happen inside an async
    // context.
    runtime.block_on(async {
        let _ = tokio::time::timeout(Duration::from_secs(10), &mut server).await;
    });
    runtime.shutdown_timeout(Duration::from_secs(5));

    tracing::info!("Janus engine stopped");
    StartResponse::running(0)
}

fn engine_status() -> StartResponse {
    match lock_engine().as_ref() {
        Some(running) => StartResponse::running(running.port),
        None => StartResponse {
            ok: true,
            port: None,
            version: Some(janus_runtime::VERSION),
            error: None,
        },
    }
}

// ─── JNI plumbing ────────────────────────────────────────────────────────────

fn to_jstring(env: &mut JNIEnv<'_>, value: &str) -> jstring {
    match env.new_string(value) {
        Ok(string) => string.into_raw(),
        Err(error) => {
            tracing::error!(%error, "failed to allocate a JNI string");
            std::ptr::null_mut()
        }
    }
}

fn json_response(response: &StartResponse) -> String {
    serde_json::to_string(response)
        .unwrap_or_else(|error| format!("{{\"ok\":false,\"error\":\"{error}\"}}"))
}

/// Run `body`, converting panics into a JSON error instead of unwinding into
/// the JVM.
fn guarded(env: &mut JNIEnv<'_>, body: impl FnOnce() -> StartResponse) -> jstring {
    let response = std::panic::catch_unwind(std::panic::AssertUnwindSafe(body))
        .unwrap_or_else(|payload| {
            let message = payload
                .downcast_ref::<&str>()
                .map(|value| value.to_string())
                .or_else(|| payload.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "unknown panic".to_string());
            tracing::error!(message, "panic in the Janus engine bridge");
            StartResponse::failed(format!("internal error: {message}"))
        });
    let payload = json_response(&response);
    to_jstring(env, &payload)
}

fn read_jstring(env: &mut JNIEnv<'_>, value: &JString<'_>) -> Result<String> {
    env.get_string(value)
        .map(|string| string.into())
        .context("configuration string was not valid UTF-16")
}

/// `NativeBridge.nativeStart(configJson)` — start the engine and return JSON.
#[no_mangle]
pub extern "system" fn Java_com_janus_omega_core_NativeBridge_nativeStart(
    mut env: JNIEnv<'_>,
    _this: JObject<'_>,
    config_json: JString<'_>,
) -> jstring {
    let config = read_jstring(&mut env, &config_json);
    guarded(&mut env, move || match config {
        Ok(json) => match start_engine(&json) {
            Ok(response) => response,
            Err(error) => {
                tracing::error!(%error, "failed to start the Janus engine");
                StartResponse::failed(error)
            }
        },
        Err(error) => StartResponse::failed(error),
    })
}

/// `NativeBridge.nativeStop()` — stop the engine and return JSON.
#[no_mangle]
pub extern "system" fn Java_com_janus_omega_core_NativeBridge_nativeStop(
    mut env: JNIEnv<'_>,
    _this: JObject<'_>,
) -> jstring {
    guarded(&mut env, stop_engine)
}

/// `NativeBridge.nativeStatus()` — report the running port, if any.
#[no_mangle]
pub extern "system" fn Java_com_janus_omega_core_NativeBridge_nativeStatus(
    mut env: JNIEnv<'_>,
    _this: JObject<'_>,
) -> jstring {
    guarded(&mut env, engine_status)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_configuration_reports_an_error_instead_of_panicking() {
        let result = std::panic::catch_unwind(|| start_engine("{not json}"));
        assert!(result.is_ok(), "start_engine must not panic");
        let error = result.unwrap().unwrap_err().to_string();
        assert!(error.contains("invalid bridge configuration"));
    }

    #[test]
    fn responses_serialize_without_null_noise() {
        let running = json_response(&StartResponse::running(41234));
        assert_eq!(
            running,
            r#"{"ok":true,"port":41234,"version":"0.1.0"}"#
        );

        let stopped = json_response(&engine_status());
        assert!(stopped.starts_with(r#"{"ok":true"#));
    }
}

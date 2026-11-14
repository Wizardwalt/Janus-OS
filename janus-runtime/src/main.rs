//! # Janus Runtime
//!
//! Core runtime daemon providing:
//! - Plugin execution environment (Lua-based)
//! - HTTP/WebSocket API server
//! - State management and persistence (SQLite)
//! - Audit logging
//! - Hardware abstraction layer

use anyhow::Result;
use clap::Parser;
use janus_core::Config;
use std::path::PathBuf;
use std::sync::Arc;
use tracing::info;

mod api;
mod backup;
mod database;
pub mod device_enrollment;
mod executor;
mod hardware;
mod lua;
mod plugin;
mod state;

/// Janus Runtime CLI arguments
#[derive(Parser, Debug)]
#[command(name = "janus-runtime")]
#[command(about = "Janus OS runtime daemon", long_about = None)]
struct Args {
    /// Configuration file path
    #[arg(short, long, value_name = "FILE")]
    config: Option<PathBuf>,

    /// Restore an encrypted Janus database backup and exit without starting the runtime.
    #[arg(long, value_name = "FILE")]
    restore_backup: Option<PathBuf>,

    /// New database path for --restore-backup (must not already exist).
    #[arg(long, value_name = "FILE")]
    restore_db_path: Option<PathBuf>,

    /// Plugin directory
    #[arg(short = 'p', long, value_name = "DIR")]
    plugin_dir: Option<PathBuf>,

    /// Database path
    #[arg(short = 'd', long, value_name = "FILE")]
    db_path: Option<PathBuf>,

    /// Web API bind address
    #[arg(short = 'b', long, value_name = "ADDR")]
    bind: Option<String>,

    /// Web API port
    #[arg(short = 'P', long, value_name = "PORT")]
    port: Option<u16>,

    /// Serial device path to open during startup (for example /dev/ttyUSB0)
    #[arg(long, value_name = "PATH")]
    serial_port: Option<String>,

    /// Disable all hardware integration
    #[arg(long, default_value_t = false)]
    disable_hardware: bool,

    /// Log level
    #[arg(long, value_name = "LEVEL", default_value = "info")]
    log_level: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    // Initialize logging
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .or_else(|_| tracing_subscriber::EnvFilter::try_new(&args.log_level))?;

    tracing_subscriber::fmt().with_env_filter(filter).init();

    info!("Janus Runtime v{} starting", janus_core::VERSION);

    if let Some(backup_path) = args.restore_backup.as_deref() {
        let destination = args.restore_db_path.as_deref()
            .ok_or_else(|| anyhow::anyhow!("--restore-db-path is required with --restore-backup"))?;
        let encoded = std::env::var("JANUS_BACKUP_KEY")
            .map_err(|_| anyhow::anyhow!("JANUS_BACKUP_KEY must contain the Base64 recovery key"))?;
        use base64::Engine;
        let key_bytes = base64::engine::general_purpose::STANDARD.decode(encoded.trim())?;
        let key: [u8; 32] = key_bytes.try_into()
            .map_err(|_| anyhow::anyhow!("JANUS_BACKUP_KEY must decode to exactly 32 bytes"))?;
        backup::restore_encrypted_backup(backup_path, destination, &key)?;
        info!("Recovery restore completed. Review hardware, re-enroll devices, and validate the host before normal startup.");
        return Ok(());
    } else if args.restore_db_path.is_some() {
        return Err(anyhow::anyhow!("--restore-db-path can only be used with --restore-backup"));
    }

    // Load configuration, optionally from the file supplied on the command line.
    let mut config = match args.config.as_deref() {
        Some(path) => Config::load_from_path(path)?,
        None => Config::load()?,
    };

    // Override with CLI arguments
    if let Some(plugin_dir) = args.plugin_dir {
        config.plugin_dir = plugin_dir;
    }
    if let Some(db_path) = args.db_path {
        config.db_path = db_path;
    }
    if let Some(bind) = args.bind {
        config.web_bind = bind;
    }
    if let Some(port) = args.port {
        config.web_port = port;
    }
    if let Some(serial_port) = args.serial_port {
        config.serial_port = Some(serial_port);
    }
    if args.disable_hardware {
        config.hardware_enabled = false;
    }

    config.validate()?;

    info!("Configuration loaded: {:?}", config);

    // Open database
    let db = database::Database::open(&config.db_path)?;
    info!("Database opened: {}", config.db_path.display());

    // Start encrypted automatic snapshots when an external recovery key is configured.
    backup::start_automatic_backups(config.db_path.clone())?;

    // Initialize state manager
    let state_mgr = state::StateManager::new(&config, Arc::new(db)).await?;
    info!("State manager initialized");

    // Load plugins
    let mut plugin_loader = plugin::PluginLoader::new(&config.plugin_dir);
    let plugin_count = plugin_loader.load_all().await?;
    info!("Loaded {} plugins", plugin_count);

    // Initialize plugin executor
    let executor = executor::PluginExecutor::new(&config, plugin_loader).await?;
    info!("Plugin executor initialized");

    // Initialize hardware manager
    let hw_manager = hardware::HardwareManager::new(config.serial_port.clone(), config.hardware_enabled);
    hw_manager.initialize().await?;
    info!("Hardware manager initialized");

    // Initialize Lua environment
    let lua_env = lua::LuaEnv::new()?;
    lua_env.load_core_modules(&config.core_modules_dir).await?;
    info!("Lua environment initialized with core modules");

    // Start API server
    let api_server = api::ApiServer::new(config.clone(), state_mgr, executor, Arc::new(hw_manager));
    api_server.start().await?;

    Ok(())
}

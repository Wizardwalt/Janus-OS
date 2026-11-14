//! # Janus Runtime
//!
//! Core runtime daemon providing:
//! - Plugin execution environment (Lua-based)
//! - HTTP/WebSocket API server
//! - State management and persistence (SQLite)
//! - Audit logging
//! - Hardware abstraction layer

use anyhow::Result;
use clap::{Parser, Subcommand};
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

    /// Run a one-shot administrative command instead of starting the runtime.
    #[command(subcommand)]
    command: Option<RuntimeCommand>,
}

#[derive(Subcommand, Debug)]
enum RuntimeCommand {
    /// Decrypt, integrity-check, and restore a backup to a new database path.
    /// Requires JANUS_BACKUP_KEY. The destination must not already exist.
    RestoreBackup {
        /// Existing .janusbk encrypted backup file.
        #[arg(long, value_name = "FILE")]
        backup: PathBuf,
        /// New database path to create after verification.
        #[arg(long, value_name = "FILE")]
        destination: PathBuf,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .or_else(|_| tracing_subscriber::EnvFilter::try_new(&args.log_level))?;
    tracing_subscriber::fmt().with_env_filter(filter).init();

    if let Some(RuntimeCommand::RestoreBackup { backup, destination }) = args.command {
        backup::restore_from_environment(&backup, &destination)?;
        info!("Recovery completed. Review the restored database before starting Janus.");
        return Ok(());
    }

    info!("Janus Runtime v{} starting", janus_core::VERSION);

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

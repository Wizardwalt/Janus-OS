//! # Janus Runtime library
//!
//! The Janus engine as a reusable library. It is shared by:
//!
//! * `janus-runtime` (the desktop/server daemon binary),
//! * `janus-android` (the JNI library embedded in the Android application),
//! * the integration tests in this crate.
//!
//! Modules:
//!
//! * [`api`] — protected HTTP API plus optional same-origin dashboard hosting.
//! * [`database`] — SQLite persistence, audit log, sessions, licenses.
//! * [`state`] — state manager, authentication and the authorization gate.
//! * [`executor`] — plugin registry and sandboxed execution.
//! * [`lua`] — isolated Lua 5.4 environments.
//! * [`plugin`] — Lua plugin discovery and manifest parsing.
//! * [`hardware`] — hardware adapter reporting.

pub mod api;
pub mod database;
pub mod executor;
pub mod hardware;
pub mod lua;
pub mod plugin;
pub mod state;

pub use database::Database;
pub use executor::PluginExecutor;
pub use hardware::HardwareManager;
pub use plugin::PluginLoader;
pub use state::StateManager;

/// Engine version, taken from the crate manifest.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

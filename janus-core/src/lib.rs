//! # Janus-Core
//!
//! Shared library providing foundational types, configuration structures,
//! plugin metadata, capability registry, audit trail types, and execution mode state.
//!
//! This crate is the single source of truth for all cross-crate data structures
//! and enables strong type safety across the distributed Janus ecosystem.

pub mod aria;
pub mod audit;
pub mod authorization;
pub mod capabilities;
pub mod config;
pub mod engagement;
pub mod error;
pub mod identity;
pub mod license;
pub mod modes;
pub mod module_registry;
pub mod plugin;
pub mod state;

pub use audit::{AuditEntry, AuditLevel, AuditLog};
pub use authorization::{authorize_execution, ExecutionAuthorization};
pub use capabilities::{Capability, CapabilityRegistry, CapabilitySet};
pub use config::{Config, ConfigError};
pub use engagement::{Engagement, EngagementScope};
pub use error::{JanusError, Result};
pub use identity::{hash_password, verify_password, Organization, UserAccount, UserRole};
pub use license::{LicenseClaims, LicensedFeature, SignedLicense};
pub use modes::{ExecutionMode, SystemMode};
pub use module_registry::{CertificationStatus, ModuleCertification};
pub use plugin::{Plugin, PluginMetadata, PluginStatus};
pub use state::{StateKey, SystemState};

/// Library version matching Cargo.toml
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

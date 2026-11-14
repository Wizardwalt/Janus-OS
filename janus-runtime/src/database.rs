//! SQLite database layer for state persistence and audit logging.

use anyhow::Result;
use chrono::Utc;
use janus_core::{
    AuditEntry, AuditLevel, AuditLog, CertificationStatus, Engagement, EngagementScope,
    LicensedFeature, ModuleCertification, Organization, SignedLicense, StateKey, UserAccount,
    UserRole,
};
use rusqlite::{params, Connection, OptionalExtension};
use std::{path::Path, sync::Mutex};
use tracing::debug;

/// User record including the password hash. This type must never be serialized
/// into an API response or written to an audit event.
#[derive(Debug, Clone)]
pub struct StoredUserAccount {
    pub account: UserAccount,
    pub password_hash: String,
}

/// Database manager for persistence
pub struct Database {
    conn: Mutex<Connection>,
}

impl Database {
    /// Open or create database
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let conn = Connection::open(path)?;
        let db = Self {
            conn: Mutex::new(conn),
        };
        db.init_schema()?;
        db.migrate_legacy_addon_setting()?;
        Ok(db)
    }

    /// Initialize database schema
    fn init_schema(&self) -> Result<()> {
        self.conn
            .lock()
            .map_err(|_| anyhow::anyhow!("database lock poisoned"))?
            .execute_batch(
                r#"
            CREATE TABLE IF NOT EXISTS state (
                id INTEGER PRIMARY KEY,
                key TEXT UNIQUE NOT NULL,
                value TEXT NOT NULL,
                updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
            );

            CREATE TABLE IF NOT EXISTS audit_log (
                id TEXT PRIMARY KEY,
                timestamp TIMESTAMP NOT NULL,
                level TEXT NOT NULL,
                actor TEXT NOT NULL,
                action TEXT NOT NULL,
                resource TEXT NOT NULL,
                result TEXT NOT NULL,
                error TEXT,
                metadata TEXT
            );

            CREATE INDEX IF NOT EXISTS idx_audit_timestamp ON audit_log(timestamp);
            CREATE INDEX IF NOT EXISTS idx_audit_level ON audit_log(level);
            CREATE INDEX IF NOT EXISTS idx_state_key ON state(key);

            CREATE TABLE IF NOT EXISTS organizations (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                active INTEGER NOT NULL DEFAULT 1,
                created_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS organization_state (
                organization_id TEXT NOT NULL,
                key TEXT NOT NULL,
                value TEXT NOT NULL,
                updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
                PRIMARY KEY (organization_id, key),
                FOREIGN KEY (organization_id) REFERENCES organizations(id)
            );
            CREATE INDEX IF NOT EXISTS idx_organization_state_key
                ON organization_state(organization_id, key);

            CREATE TABLE IF NOT EXISTS user_accounts (
                id TEXT PRIMARY KEY,
                organization_id TEXT NOT NULL,
                email TEXT NOT NULL UNIQUE,
                password_hash TEXT NOT NULL,
                role TEXT NOT NULL,
                active INTEGER NOT NULL DEFAULT 1,
                failed_login_count INTEGER NOT NULL DEFAULT 0,
                locked_until TEXT,
                created_at TEXT NOT NULL,
                FOREIGN KEY (organization_id) REFERENCES organizations(id)
            );
            CREATE INDEX IF NOT EXISTS idx_user_accounts_organization
                ON user_accounts(organization_id);

            CREATE TABLE IF NOT EXISTS engagements (
                id TEXT PRIMARY KEY,
                organization_id TEXT NOT NULL,
                authorization_reference TEXT NOT NULL,
                starts_at TEXT NOT NULL,
                ends_at TEXT NOT NULL,
                active INTEGER NOT NULL DEFAULT 1,
                created_at TEXT NOT NULL,
                FOREIGN KEY (organization_id) REFERENCES organizations(id)
            );

            CREATE TABLE IF NOT EXISTS engagement_assets (
                engagement_id TEXT NOT NULL,
                asset TEXT NOT NULL,
                PRIMARY KEY (engagement_id, asset),
                FOREIGN KEY (engagement_id) REFERENCES engagements(id)
            );

            CREATE TABLE IF NOT EXISTS engagement_evidence_paths (
                engagement_id TEXT NOT NULL,
                evidence_path TEXT NOT NULL,
                PRIMARY KEY (engagement_id, evidence_path),
                FOREIGN KEY (engagement_id) REFERENCES engagements(id)
            );

            CREATE TABLE IF NOT EXISTS engagement_features (
                engagement_id TEXT NOT NULL,
                feature TEXT NOT NULL,
                PRIMARY KEY (engagement_id, feature),
                FOREIGN KEY (engagement_id) REFERENCES engagements(id)
            );

            CREATE TABLE IF NOT EXISTS organization_licenses (
                license_id TEXT PRIMARY KEY,
                organization_id TEXT NOT NULL,
                signed_document TEXT NOT NULL,
                active INTEGER NOT NULL DEFAULT 1,
                imported_at TEXT NOT NULL,
                FOREIGN KEY (organization_id) REFERENCES organizations(id)
            );
            CREATE INDEX IF NOT EXISTS idx_organization_licenses_org
                ON organization_licenses(organization_id);

            CREATE TABLE IF NOT EXISTS sessions (
                token_hash TEXT PRIMARY KEY,
                user_id TEXT NOT NULL,
                expires_at TEXT NOT NULL,
                created_at TEXT NOT NULL,
                FOREIGN KEY (user_id) REFERENCES user_accounts(id)
            );
            CREATE INDEX IF NOT EXISTS idx_sessions_user ON sessions(user_id);

            CREATE TABLE IF NOT EXISTS owner_devices (
                device_id TEXT PRIMARY KEY,
                organization_id TEXT NOT NULL,
                owner_user_id TEXT NOT NULL,
                device_type TEXT NOT NULL CHECK (device_type IN ('monolith', 'pipboy')),
                policies TEXT NOT NULL,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                FOREIGN KEY (organization_id) REFERENCES organizations(id),
                FOREIGN KEY (owner_user_id) REFERENCES user_accounts(id)
            );
            CREATE INDEX IF NOT EXISTS idx_owner_devices_owner
                ON owner_devices(owner_user_id);

            CREATE TABLE IF NOT EXISTS device_enrollment_challenges (
                challenge_id TEXT PRIMARY KEY,
                nonce TEXT NOT NULL,
                device_id TEXT NOT NULL,
                owner_user_id TEXT NOT NULL,
                device_type TEXT NOT NULL CHECK (device_type IN ('monolith', 'pipboy')),
                issued_at INTEGER NOT NULL,
                expires_at INTEGER NOT NULL,
                consumed_at TEXT,
                FOREIGN KEY (device_id) REFERENCES owner_devices(device_id),
                FOREIGN KEY (owner_user_id) REFERENCES user_accounts(id)
            );
            CREATE INDEX IF NOT EXISTS idx_device_enrollment_expiry
                ON device_enrollment_challenges(expires_at);

            CREATE TABLE IF NOT EXISTS owner_device_keys (
                device_id TEXT PRIMARY KEY,
                owner_user_id TEXT NOT NULL,
                public_key TEXT NOT NULL,
                key_fingerprint TEXT NOT NULL,
                enrolled_at TEXT NOT NULL,
                FOREIGN KEY (device_id) REFERENCES owner_devices(device_id),
                FOREIGN KEY (owner_user_id) REFERENCES user_accounts(id)
            );

            CREATE TABLE IF NOT EXISTS operation_approvals (
                id TEXT PRIMARY KEY,
                engagement_id TEXT NOT NULL,
                requested_by TEXT NOT NULL,
                reviewed_by TEXT,
                module_id TEXT NOT NULL,
                target_asset TEXT NOT NULL,
                status TEXT NOT NULL,
                expires_at TEXT NOT NULL,
                created_at TEXT NOT NULL,
                FOREIGN KEY (engagement_id) REFERENCES engagements(id)
            );
            CREATE INDEX IF NOT EXISTS idx_operation_approvals_engagement
                ON operation_approvals(engagement_id);

            CREATE TABLE IF NOT EXISTS module_certifications (
                module_id TEXT PRIMARY KEY,
                module_sha256 TEXT NOT NULL,
                status TEXT NOT NULL,
                required_feature TEXT NOT NULL,
                reviewed_by TEXT,
                reviewed_at TEXT,
                notes TEXT,
                updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
            );
            CREATE INDEX IF NOT EXISTS idx_module_certification_status
                ON module_certifications(status);
        "#,
            )?;
        debug!("Database schema initialized");
        Ok(())
    }

    /// Copy the one legacy organization-wide add-on setting into each existing
    /// organization. Other legacy global state is deliberately not exposed.
    fn migrate_legacy_addon_setting(&self) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("database lock poisoned"))?;
        let organization_ids = {
            let mut statement = conn.prepare("SELECT id FROM organizations")?;
            let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
            rows.collect::<rusqlite::Result<Vec<_>>>()?
        };
        for organization_id in organization_ids {
            copy_legacy_addon_setting(&conn, &organization_id)?;
        }
        Ok(())
    }

    /// Create the first organization and administrator exactly once.
    ///
    /// The existence check and inserts share a transaction, so concurrent setup
    /// requests cannot create two initial administrators.
    pub fn bootstrap_first_admin(
        &self,
        organization: &Organization,
        account: &UserAccount,
        password_hash: &str,
    ) -> Result<()> {
        let mut conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("database lock poisoned"))?;
        let transaction = conn.transaction()?;
        let organization_count: i64 =
            transaction.query_row("SELECT COUNT(*) FROM organizations", [], |row| row.get(0))?;
        if organization_count != 0 {
            return Err(anyhow::anyhow!("initial setup has already been completed"));
        }
        transaction.execute(
            "INSERT INTO organizations (id, name, active, created_at) VALUES (?1, ?2, ?3, ?4)",
            params![
                organization.id,
                organization.name,
                organization.active,
                organization.created_at.to_rfc3339()
            ],
        )?;
        transaction.execute(
            "INSERT INTO user_accounts (id, organization_id, email, password_hash, role, active, failed_login_count, locked_until, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![account.id, account.organization_id, account.email, password_hash, account.role.as_str(), account.active, account.failed_login_count, account.locked_until.map(|value| value.to_rfc3339()), account.created_at.to_rfc3339()],
        )?;
        copy_legacy_addon_setting(&transaction, &organization.id)?;
        transaction.commit()?;
        Ok(())
    }

    /// Persist an organization.
    pub fn create_organization(&self, organization: &Organization) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("database lock poisoned"))?;
        conn.execute(
            "INSERT INTO organizations (id, name, active, created_at) VALUES (?1, ?2, ?3, ?4)",
            params![
                organization.id,
                organization.name,
                organization.active,
                organization.created_at.to_rfc3339()
            ],
        )?;
        copy_legacy_addon_setting(&conn, &organization.id)?;
        Ok(())
    }

    /// Persist a Janus-managed user account and its Argon2id password hash.
    pub fn create_user(&self, account: &UserAccount, password_hash: &str) -> Result<()> {
        self.conn.lock().map_err(|_| anyhow::anyhow!("database lock poisoned"))?.execute(
            "INSERT INTO user_accounts (id, organization_id, email, password_hash, role, active, failed_login_count, locked_until, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![account.id, account.organization_id, account.email, password_hash, account.role.as_str(), account.active, account.failed_login_count, account.locked_until.map(|value| value.to_rfc3339()), account.created_at.to_rfc3339()],
        )?;
        Ok(())
    }

    /// Retrieve an account for authentication. Callers must not expose the
    /// returned password hash outside the authentication service.
    pub fn find_user_by_email(&self, email: &str) -> Result<Option<StoredUserAccount>> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("database lock poisoned"))?;
        conn.query_row(
            "SELECT id, organization_id, email, password_hash, role, active, failed_login_count, locked_until, created_at FROM user_accounts WHERE email = ?1",
            params![email],
            |row| {
                let role_value: String = row.get(4)?;
                let role = UserRole::from_str(&role_value).ok_or(rusqlite::Error::InvalidQuery)?;
                let locked_until: Option<String> = row.get(7)?;
                let created_at: String = row.get(8)?;
                Ok(StoredUserAccount {
                    account: UserAccount {
                        id: row.get(0)?, organization_id: row.get(1)?, email: row.get(2)?, role,
                        active: row.get(5)?, failed_login_count: row.get(6)?,
                        locked_until: locked_until.and_then(|value| chrono::DateTime::parse_from_rfc3339(&value).ok()).map(|value| value.with_timezone(&Utc)),
                        created_at: chrono::DateTime::parse_from_rfc3339(&created_at).map_err(|_| rusqlite::Error::InvalidQuery)?.with_timezone(&Utc),
                    },
                    password_hash: row.get(3)?,
                })
            },
        ).optional().map_err(Into::into)
    }

    /// Query audit records created by accounts in one organization.
    pub fn query_audit_for_organization(
        &self,
        organization_id: &str,
        limit: usize,
    ) -> Result<Vec<AuditEntry>> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("database lock poisoned"))?;
        let mut statement = conn.prepare(
            "SELECT a.id, a.timestamp, a.level, a.actor, a.action, a.resource, a.result, a.error, a.metadata FROM audit_log a JOIN user_accounts u ON u.id = a.actor WHERE u.organization_id = ?1 ORDER BY a.timestamp DESC LIMIT ?2",
        )?;
        let rows = statement.query_map(params![organization_id, limit as i64], |row| {
            let timestamp: String = row.get(1)?;
            let level: String = row.get(2)?;
            Ok(AuditEntry {
                id: row.get(0)?,
                timestamp: chrono::DateTime::parse_from_rfc3339(&timestamp)
                    .map_err(|_| rusqlite::Error::InvalidQuery)?
                    .with_timezone(&Utc),
                level: match level.as_str() {
                    "WARN" => AuditLevel::Warn,
                    "ERROR" => AuditLevel::Error,
                    "CRITICAL" => AuditLevel::Critical,
                    _ => AuditLevel::Info,
                },
                actor: row.get(3)?,
                action: row.get(4)?,
                resource: row.get(5)?,
                result: row.get(6)?,
                error: row.get(7)?,
                metadata: row
                    .get::<_, Option<String>>(8)?
                    .and_then(|value| serde_json::from_str(&value).ok())
                    .unwrap_or_else(|| serde_json::json!({})),
            })
        })?;
        Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
    }

    /// List safe account metadata for one organization.
    pub fn list_users(&self, organization_id: &str) -> Result<Vec<UserAccount>> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("database lock poisoned"))?;
        let mut statement = conn.prepare("SELECT id, organization_id, email, role, active, failed_login_count, locked_until, created_at FROM user_accounts WHERE organization_id = ?1 ORDER BY email")?;
        let rows = statement.query_map(params![organization_id], |row| {
            let role: String = row.get(3)?;
            let locked_until: Option<String> = row.get(6)?;
            let created_at: String = row.get(7)?;
            Ok(UserAccount {
                id: row.get(0)?,
                organization_id: row.get(1)?,
                email: row.get(2)?,
                role: UserRole::from_str(&role).ok_or(rusqlite::Error::InvalidQuery)?,
                active: row.get(4)?,
                failed_login_count: row.get(5)?,
                locked_until: locked_until
                    .and_then(|value| chrono::DateTime::parse_from_rfc3339(&value).ok())
                    .map(|value| value.with_timezone(&Utc)),
                created_at: chrono::DateTime::parse_from_rfc3339(&created_at)
                    .map_err(|_| rusqlite::Error::InvalidQuery)?
                    .with_timezone(&Utc),
            })
        })?;
        Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
    }

    /// Update account activation only when it belongs to the stated organization.
    pub fn set_user_active(
        &self,
        organization_id: &str,
        user_id: &str,
        active: bool,
    ) -> Result<bool> {
        let changed = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("database lock poisoned"))?
            .execute(
                "UPDATE user_accounts SET active = ?1 WHERE id = ?2 AND organization_id = ?3",
                params![active, user_id, organization_id],
            )?;
        Ok(changed == 1)
    }

    /// Replace a password hash for an account in the stated organization.
    pub fn update_user_password(
        &self,
        organization_id: &str,
        user_id: &str,
        password_hash: &str,
    ) -> Result<bool> {
        let changed = self.conn.lock().map_err(|_| anyhow::anyhow!("database lock poisoned"))?.execute("UPDATE user_accounts SET password_hash = ?1, failed_login_count = 0, locked_until = NULL WHERE id = ?2 AND organization_id = ?3", params![password_hash, user_id, organization_id])?;
        Ok(changed == 1)
    }

    /// Revoke all sessions owned by an account.
    pub fn delete_user_sessions(&self, user_id: &str) -> Result<usize> {
        Ok(self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("database lock poisoned"))?
            .execute("DELETE FROM sessions WHERE user_id = ?1", params![user_id])?)
    }

    /// Clear failed-login state after a successful authentication.
    pub fn reset_login_failures(&self, user_id: &str) -> Result<()> {
        self.conn.lock().map_err(|_| anyhow::anyhow!("database lock poisoned"))?.execute(
            "UPDATE user_accounts SET failed_login_count = 0, locked_until = NULL WHERE id = ?1",
            params![user_id],
        )?;
        Ok(())
    }

    /// Record a failed authentication. Five failures produce a fifteen-minute lockout.
    pub fn record_login_failure(&self, user_id: &str, now: chrono::DateTime<Utc>) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("database lock poisoned"))?;
        let attempts: i64 = conn.query_row(
            "SELECT failed_login_count FROM user_accounts WHERE id = ?1",
            params![user_id],
            |row| row.get(0),
        )?;
        let next_attempt = attempts + 1;
        let locked_until = if next_attempt >= 5 {
            Some((now + chrono::Duration::minutes(15)).to_rfc3339())
        } else {
            None
        };
        conn.execute(
            "UPDATE user_accounts SET failed_login_count = ?1, locked_until = ?2 WHERE id = ?3",
            params![next_attempt, locked_until, user_id],
        )?;
        Ok(())
    }

    /// Persist a hashed opaque session token.
    pub fn create_session(
        &self,
        token_hash: &str,
        user_id: &str,
        expires_at: chrono::DateTime<Utc>,
    ) -> Result<()> {
        self.conn.lock().map_err(|_| anyhow::anyhow!("database lock poisoned"))?.execute(
            "INSERT INTO sessions (token_hash, user_id, expires_at, created_at) VALUES (?1, ?2, ?3, ?4)",
            params![token_hash, user_id, expires_at.to_rfc3339(), Utc::now().to_rfc3339()],
        )?;
        Ok(())
    }

    /// Resolve an unexpired session hash to its active user account.
    pub fn find_session_user(
        &self,
        token_hash: &str,
        now: chrono::DateTime<Utc>,
    ) -> Result<Option<UserAccount>> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("database lock poisoned"))?;
        conn.query_row(
            "SELECT u.id, u.organization_id, u.email, u.role, u.active, u.failed_login_count, u.locked_until, u.created_at FROM sessions s JOIN user_accounts u ON u.id = s.user_id WHERE s.token_hash = ?1 AND s.expires_at > ?2",
            params![token_hash, now.to_rfc3339()],
            |row| {
                let role_value: String = row.get(3)?;
                let role = UserRole::from_str(&role_value).ok_or(rusqlite::Error::InvalidQuery)?;
                let locked_until: Option<String> = row.get(6)?;
                let created_at: String = row.get(7)?;
                Ok(UserAccount {
                    id: row.get(0)?, organization_id: row.get(1)?, email: row.get(2)?, role,
                    active: row.get(4)?, failed_login_count: row.get(5)?,
                    locked_until: locked_until.and_then(|value| chrono::DateTime::parse_from_rfc3339(&value).ok()).map(|value| value.with_timezone(&Utc)),
                    created_at: chrono::DateTime::parse_from_rfc3339(&created_at).map_err(|_| rusqlite::Error::InvalidQuery)?.with_timezone(&Utc),
                })
            },
        ).optional().map_err(Into::into)
    }

    /// Remove expired session records as part of normal authentication activity.
    pub fn cleanup_expired_sessions(&self, now: chrono::DateTime<Utc>) -> Result<usize> {
        let rows = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("database lock poisoned"))?
            .execute(
                "DELETE FROM sessions WHERE expires_at <= ?1",
                params![now.to_rfc3339()],
            )?;
        Ok(rows)
    }

    /// Revoke an opaque session by its token hash.
    pub fn delete_session(&self, token_hash: &str) -> Result<()> {
        self.conn
            .lock()
            .map_err(|_| anyhow::anyhow!("database lock poisoned"))?
            .execute(
                "DELETE FROM sessions WHERE token_hash = ?1",
                params![token_hash],
            )?;
        Ok(())
    }

    /// Register a physical Janus device to the authenticated purchaser account.
    /// The device ID is globally unique; an existing registration is never reassigned.
    pub fn register_owner_device(
        &self,
        device_id: &str,
        organization_id: &str,
        owner_user_id: &str,
        device_type: &str,
        policies: &serde_json::Value,
    ) -> Result<()> {
        let policies_json = serde_json::to_string(policies)?;
        let now = Utc::now().to_rfc3339();
        self.conn.lock().map_err(|_| anyhow::anyhow!("database lock poisoned"))?.execute(
            "INSERT INTO owner_devices (device_id, organization_id, owner_user_id, device_type, policies, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)",
            params![device_id, organization_id, owner_user_id, device_type, policies_json, now],
        )?;
        Ok(())
    }

    /// Persist a short-lived enrollment challenge for an already owner-registered device.
    pub fn create_device_enrollment_challenge(
        &self,
        challenge_id: &str,
        nonce: &str,
        device_id: &str,
        owner_user_id: &str,
        device_type: &str,
        issued_at: i64,
        expires_at: i64,
    ) -> Result<()> {
        self.conn.lock().map_err(|_| anyhow::anyhow!("database lock poisoned"))?.execute(
            "INSERT INTO device_enrollment_challenges (challenge_id, nonce, device_id, owner_user_id, device_type, issued_at, expires_at, consumed_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, NULL)",
            params![challenge_id, nonce, device_id, owner_user_id, device_type, issued_at, expires_at],
        )?;
        Ok(())
    }

    /// Load challenge claims. Completion still has to atomically consume the row.
    pub fn get_device_enrollment_challenge(
        &self,
        challenge_id: &str,
        owner_user_id: &str,
        device_id: &str,
    ) -> Result<Option<(String, String, String, i64, i64)>> {
        let conn = self.conn.lock().map_err(|_| anyhow::anyhow!("database lock poisoned"))?;
        let row = conn.query_row(
            "SELECT nonce, device_type, device_id, issued_at, expires_at FROM device_enrollment_challenges WHERE challenge_id = ?1 AND owner_user_id = ?2 AND device_id = ?3 AND consumed_at IS NULL",
            params![challenge_id, owner_user_id, device_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
        ).optional()?;
        Ok(row)
    }

    /// Consume a challenge exactly once and persist its public key in one transaction.
    pub fn consume_device_enrollment_challenge(
        &self,
        challenge_id: &str,
        owner_user_id: &str,
        device_id: &str,
        public_key: &str,
        fingerprint: &str,
        now_unix: i64,
    ) -> Result<bool> {
        let mut conn = self.conn.lock().map_err(|_| anyhow::anyhow!("database lock poisoned"))?;
        let tx = conn.transaction()?;
        let changed = tx.execute(
            "UPDATE device_enrollment_challenges SET consumed_at = ?1 WHERE challenge_id = ?2 AND owner_user_id = ?3 AND device_id = ?4 AND consumed_at IS NULL AND expires_at > ?5",
            params![Utc::now().to_rfc3339(), challenge_id, owner_user_id, device_id, now_unix],
        )?;
        if changed != 1 {
            return Ok(false);
        }
        // Do not silently replace an enrolled key. Key rotation needs its own
        // explicit owner-authorized ceremony.
        tx.execute(
            "INSERT INTO owner_device_keys (device_id, owner_user_id, public_key, key_fingerprint, enrolled_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![device_id, owner_user_id, public_key, fingerprint, Utc::now().to_rfc3339()],
        )?;
        tx.commit()?;
        Ok(true)
    }

    /// Read an owned device only for its registered owner.
    pub fn get_owned_device(
        &self,
        device_id: &str,
        owner_user_id: &str,
    ) -> Result<Option<(String, String, serde_json::Value)>> {
        let conn = self.conn.lock().map_err(|_| anyhow::anyhow!("database lock poisoned"))?;
        let row = conn.query_row(
            "SELECT organization_id, device_type, policies FROM owner_devices WHERE device_id = ?1 AND owner_user_id = ?2",
            params![device_id, owner_user_id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?)),
        ).optional()?;
        row.map(|(organization_id, device_type, json)| {
            Ok((organization_id, device_type, serde_json::from_str(&json)?))
        }).transpose()
    }

    /// List devices owned by one account without exposing another account's devices.
    pub fn list_owned_devices(
        &self,
        owner_user_id: &str,
    ) -> Result<Vec<(String, String, serde_json::Value)>> {
        let conn = self.conn.lock().map_err(|_| anyhow::anyhow!("database lock poisoned"))?;
        let mut statement = conn.prepare(
            "SELECT device_id, device_type, policies FROM owner_devices WHERE owner_user_id = ?1 ORDER BY created_at",
        )?;
        let rows = statement.query_map(params![owner_user_id], |row| {
            let json: String = row.get(2)?;
            let policies = serde_json::from_str(&json).map_err(|_| rusqlite::Error::InvalidQuery)?;
            Ok((row.get(0)?, row.get(1)?, policies))
        })?;
        Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
    }

    /// Replace a registered device's configurable policy only when called by its owner.
    pub fn update_owned_device_policy(
        &self,
        device_id: &str,
        owner_user_id: &str,
        policies: &serde_json::Value,
    ) -> Result<bool> {
        let json = serde_json::to_string(policies)?;
        let changed = self.conn.lock().map_err(|_| anyhow::anyhow!("database lock poisoned"))?.execute(
            "UPDATE owner_devices SET policies = ?1, updated_at = ?2 WHERE device_id = ?3 AND owner_user_id = ?4",
            params![json, Utc::now().to_rfc3339(), device_id, owner_user_id],
        )?;
        Ok(changed == 1)
    }

    /// Create or update a reviewer-approved module certification record.
    pub fn upsert_module_certification(&self, certification: &ModuleCertification) -> Result<()> {
        self.conn.lock().map_err(|_| anyhow::anyhow!("database lock poisoned"))?.execute(
            "INSERT INTO module_certifications (module_id, module_sha256, status, required_feature, reviewed_by, reviewed_at, notes) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7) ON CONFLICT(module_id) DO UPDATE SET module_sha256 = excluded.module_sha256, status = excluded.status, required_feature = excluded.required_feature, reviewed_by = excluded.reviewed_by, reviewed_at = excluded.reviewed_at, notes = excluded.notes, updated_at = CURRENT_TIMESTAMP",
            params![certification.module_id, certification.module_sha256, certification.status.as_str(), certification.required_feature.as_str(), certification.reviewed_by, certification.reviewed_at.map(|value| value.to_rfc3339()), certification.notes],
        )?;
        Ok(())
    }

    /// Verify and store a signed license document for an organization.
    pub fn store_license(
        &self,
        license: &SignedLicense,
        license_public_key_base64: &str,
    ) -> Result<()> {
        license.verify(license_public_key_base64, Utc::now())?;
        let document = serde_json::to_string(license)?;
        self.conn.lock().map_err(|_| anyhow::anyhow!("database lock poisoned"))?.execute(
            "INSERT OR REPLACE INTO organization_licenses (license_id, organization_id, signed_document, active, imported_at) VALUES (?1, ?2, ?3, 1, ?4)",
            params![license.claims.license_id, license.claims.organization_id, document, Utc::now().to_rfc3339()],
        )?;
        Ok(())
    }

    /// Return the current active license for an organization, if present.
    pub fn find_active_license(&self, organization_id: &str) -> Result<Option<SignedLicense>> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("database lock poisoned"))?;
        let document: Option<String> = conn.query_row(
            "SELECT signed_document FROM organization_licenses WHERE organization_id = ?1 AND active = 1 ORDER BY imported_at DESC LIMIT 1",
            params![organization_id],
            |row| row.get(0),
        ).optional()?;
        document
            .map(|value| serde_json::from_str(&value).map_err(Into::into))
            .transpose()
    }

    /// Persist an engagement and its explicit approved scope in one transaction.
    pub fn create_engagement(&self, engagement: &Engagement) -> Result<()> {
        let mut conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("database lock poisoned"))?;
        let transaction = conn.transaction()?;
        transaction.execute(
            "INSERT INTO engagements (id, organization_id, authorization_reference, starts_at, ends_at, active, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![engagement.id, engagement.organization_id, engagement.authorization_reference, engagement.starts_at.to_rfc3339(), engagement.ends_at.to_rfc3339(), engagement.active, Utc::now().to_rfc3339()],
        )?;
        for asset in &engagement.scope.approved_assets {
            transaction.execute(
                "INSERT INTO engagement_assets (engagement_id, asset) VALUES (?1, ?2)",
                params![engagement.id, asset],
            )?;
        }
        for path in &engagement.scope.approved_evidence_paths {
            transaction.execute("INSERT INTO engagement_evidence_paths (engagement_id, evidence_path) VALUES (?1, ?2)", params![engagement.id, path])?;
        }
        for feature in &engagement.scope.approved_features {
            transaction.execute(
                "INSERT INTO engagement_features (engagement_id, feature) VALUES (?1, ?2)",
                params![engagement.id, feature.as_str()],
            )?;
        }
        transaction.commit()?;
        Ok(())
    }

    /// Update engagement activity only when it belongs to the stated organization.
    pub fn set_engagement_active(
        &self,
        organization_id: &str,
        engagement_id: &str,
        active: bool,
    ) -> Result<bool> {
        let changed = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("database lock poisoned"))?
            .execute(
                "UPDATE engagements SET active = ?1 WHERE id = ?2 AND organization_id = ?3",
                params![active, engagement_id, organization_id],
            )?;
        Ok(changed == 1)
    }

    /// List all engagements belonging to one organization.
    pub fn list_engagements(&self, organization_id: &str) -> Result<Vec<Engagement>> {
        let ids = {
            let conn = self
                .conn
                .lock()
                .map_err(|_| anyhow::anyhow!("database lock poisoned"))?;
            let mut statement = conn.prepare(
                "SELECT id FROM engagements WHERE organization_id = ?1 ORDER BY starts_at DESC",
            )?;
            let rows =
                statement.query_map(params![organization_id], |row| row.get::<_, String>(0))?;
            let ids = rows.collect::<std::result::Result<Vec<String>, _>>()?;
            ids
        };
        ids.into_iter()
            .map(|id| {
                self.find_engagement(&id)?
                    .ok_or_else(|| anyhow::anyhow!("engagement disappeared during lookup"))
            })
            .collect()
    }

    /// Load an engagement and its explicit asset/feature scope.
    pub fn find_engagement(&self, id: &str) -> Result<Option<Engagement>> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("database lock poisoned"))?;
        let record = conn.query_row(
            "SELECT id, organization_id, authorization_reference, starts_at, ends_at, active FROM engagements WHERE id = ?1",
            params![id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?, row.get::<_, String>(3)?, row.get::<_, String>(4)?, row.get::<_, bool>(5)?)),
        ).optional()?;
        let Some((id, organization_id, authorization_reference, starts_at, ends_at, active)) =
            record
        else {
            return Ok(None);
        };
        let approved_assets = {
            let mut statement =
                conn.prepare("SELECT asset FROM engagement_assets WHERE engagement_id = ?1")?;
            let rows = statement.query_map(params![&id], |row| row.get(0))?;
            rows.collect::<std::result::Result<Vec<String>, _>>()?
        };
        let approved_evidence_paths = {
            let mut statement = conn.prepare(
                "SELECT evidence_path FROM engagement_evidence_paths WHERE engagement_id = ?1",
            )?;
            let rows = statement.query_map(params![&id], |row| row.get(0))?;
            rows.collect::<std::result::Result<Vec<String>, _>>()?
        };
        let feature_values = {
            let mut statement =
                conn.prepare("SELECT feature FROM engagement_features WHERE engagement_id = ?1")?;
            let rows = statement.query_map(params![&id], |row| row.get::<_, String>(0))?;
            rows.collect::<std::result::Result<Vec<String>, _>>()?
        };
        let approved_features = feature_values
            .into_iter()
            .filter_map(|value| LicensedFeature::from_str(&value))
            .collect();
        Ok(Some(Engagement {
            id,
            organization_id,
            authorization_reference,
            starts_at: chrono::DateTime::parse_from_rfc3339(&starts_at)?.with_timezone(&Utc),
            ends_at: chrono::DateTime::parse_from_rfc3339(&ends_at)?.with_timezone(&Utc),
            active,
            scope: EngagementScope {
                approved_assets,
                approved_evidence_paths,
                approved_features,
            },
        }))
    }

    /// Load a module's reviewed production certification record.
    pub fn find_module_certification(
        &self,
        module_id: &str,
    ) -> Result<Option<ModuleCertification>> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("database lock poisoned"))?;
        let record = conn.query_row(
            "SELECT module_id, module_sha256, status, required_feature, reviewed_by, reviewed_at, notes FROM module_certifications WHERE module_id = ?1",
            params![module_id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?, row.get::<_, String>(3)?, row.get::<_, Option<String>>(4)?, row.get::<_, Option<String>>(5)?, row.get::<_, Option<String>>(6)?)),
        ).optional()?;
        let Some((
            module_id,
            module_sha256,
            status,
            required_feature,
            reviewed_by,
            reviewed_at,
            notes,
        )) = record
        else {
            return Ok(None);
        };
        Ok(Some(ModuleCertification {
            module_id,
            module_sha256,
            status: CertificationStatus::from_str(&status)
                .ok_or_else(|| anyhow::anyhow!("invalid certification status"))?,
            required_feature: LicensedFeature::from_str(&required_feature)
                .ok_or_else(|| anyhow::anyhow!("invalid licensed feature"))?,
            reviewed_by,
            reviewed_at: reviewed_at
                .map(|value| {
                    chrono::DateTime::parse_from_rfc3339(&value)
                        .map(|value| value.with_timezone(&Utc))
                })
                .transpose()?,
            notes,
        }))
    }

    /// Retrieve a state value scoped to one organization.
    pub fn get_organization_state(
        &self,
        organization_id: &str,
        key: &StateKey,
    ) -> Result<Option<serde_json::Value>> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("database lock poisoned"))?;
        let value = conn
            .query_row(
                "SELECT value FROM organization_state WHERE organization_id = ?1 AND key = ?2",
                params![organization_id, key.fqn()],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        value
            .map(|value| serde_json::from_str(&value).map_err(anyhow::Error::from))
            .transpose()
    }

    /// Persist a state value for exactly one organization.
    pub fn set_organization_state(
        &self,
        organization_id: &str,
        key: &StateKey,
        value: &serde_json::Value,
    ) -> Result<()> {
        let json = serde_json::to_string(value)?;
        self.conn
            .lock()
            .map_err(|_| anyhow::anyhow!("database lock poisoned"))?
            .execute(
                "INSERT INTO organization_state (organization_id, key, value) VALUES (?1, ?2, ?3) ON CONFLICT(organization_id, key) DO UPDATE SET value = excluded.value, updated_at = CURRENT_TIMESTAMP",
                params![organization_id, key.fqn(), json],
            )?;
        debug!(organization_id, key = %key, "Organization state persisted");
        Ok(())
    }

    /// Remove a state value belonging to exactly one organization.
    pub fn remove_organization_state(&self, organization_id: &str, key: &StateKey) -> Result<()> {
        self.conn
            .lock()
            .map_err(|_| anyhow::anyhow!("database lock poisoned"))?
            .execute(
                "DELETE FROM organization_state WHERE organization_id = ?1 AND key = ?2",
                params![organization_id, key.fqn()],
            )?;
        Ok(())
    }
}

/// Audit log implementation
impl AuditLog for Database {
    fn record(&self, entry: AuditEntry) -> janus_core::Result<()> {
        let json = serde_json::to_string(&entry.metadata).unwrap_or_else(|_| "{}".to_string());

        self.conn
            .lock()
            .map_err(|_| janus_core::JanusError::Audit("database lock poisoned".to_string()))?
            .execute(
                r#"
                INSERT INTO audit_log 
                (id, timestamp, level, actor, action, resource, result, error, metadata)
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
            "#,
                params![
                    entry.id,
                    entry.timestamp.to_rfc3339(),
                    entry.level.to_string(),
                    entry.actor,
                    entry.action,
                    entry.resource,
                    entry.result,
                    entry.error,
                    json
                ],
            )
            .map_err(|e| janus_core::JanusError::Audit(e.to_string()))?;

        Ok(())
    }

    fn query(&self, limit: usize) -> janus_core::Result<Vec<AuditEntry>> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| janus_core::JanusError::Audit("database lock poisoned".to_string()))?;
        let mut stmt = conn
            .prepare(
                "SELECT id, timestamp, level, actor, action, resource, result, error, metadata 
                 FROM audit_log ORDER BY timestamp DESC LIMIT ?1",
            )
            .map_err(|e| janus_core::JanusError::Audit(e.to_string()))?;

        let entries = stmt
            .query_map(params![limit as i32], |row| {
                Ok(AuditEntry {
                    id: row.get(0)?,
                    timestamp: chrono::DateTime::parse_from_rfc3339(&row.get::<_, String>(1)?)
                        .map(|dt| dt.with_timezone(&Utc))
                        .unwrap_or_else(|_| Utc::now()),
                    level: match row.get::<_, String>(2)?.as_str() {
                        "WARN" => AuditLevel::Warn,
                        "ERROR" => AuditLevel::Error,
                        "CRITICAL" => AuditLevel::Critical,
                        _ => AuditLevel::Info,
                    },
                    actor: row.get(3)?,
                    action: row.get(4)?,
                    resource: row.get(5)?,
                    result: row.get(6)?,
                    error: row.get(7)?,
                    metadata: row
                        .get::<_, String>(8)
                        .ok()
                        .and_then(|s| serde_json::from_str(&s).ok())
                        .unwrap_or(serde_json::json!({})),
                })
            })
            .map_err(|e| janus_core::JanusError::Audit(e.to_string()))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| janus_core::JanusError::Audit(e.to_string()))?;

        Ok(entries)
    }

    fn cleanup(&self, days_old: u32) -> janus_core::Result<usize> {
        let cutoff = Utc::now() - chrono::Duration::days(days_old as i64);
        let rows = self
            .conn
            .lock()
            .map_err(|_| janus_core::JanusError::Audit("database lock poisoned".to_string()))?
            .execute(
                "DELETE FROM audit_log WHERE timestamp < ?1",
                params![cutoff.to_rfc3339()],
            )
            .map_err(|e| janus_core::JanusError::Audit(e.to_string()))?;

        Ok(rows)
    }
}

/// Preserve the legacy organization-wide add-on switch without copying any
/// other global state into the tenant-scoped store.
fn copy_legacy_addon_setting(conn: &Connection, organization_id: &str) -> Result<()> {
    let key = "system:addons_enabled";
    let value: Option<String> = conn
        .query_row("SELECT value FROM state WHERE key = ?1", params![key], |row| row.get(0))
        .optional()?;
    if let Some(value) = value {
        if matches!(serde_json::from_str::<serde_json::Value>(&value), Ok(serde_json::Value::Bool(_))) {
            conn.execute(
                "INSERT OR IGNORE INTO organization_state (organization_id, key, value) VALUES (?1, ?2, ?3)",
                params![organization_id, key, value],
            )?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod organization_state_tests {
    use super::*;

    #[test]
    fn organization_state_is_isolated_and_legacy_addon_setting_is_migrated() {
        let path = std::env::temp_dir().join(format!("janus-state-test-{}.db", uuid::Uuid::new_v4()));
        {
            let legacy = Connection::open(&path).unwrap();
            legacy.execute_batch(
                "CREATE TABLE state (id INTEGER PRIMARY KEY, key TEXT UNIQUE NOT NULL, value TEXT NOT NULL, updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP);
                 CREATE TABLE organizations (id TEXT PRIMARY KEY, name TEXT NOT NULL, active INTEGER NOT NULL DEFAULT 1, created_at TEXT NOT NULL);
                 INSERT INTO organizations (id, name, active, created_at) VALUES ('org_legacy', 'Legacy', 1, '2026-01-01T00:00:00Z');
                 INSERT INTO state (key, value) VALUES ('system:addons_enabled', 'false');"
            ).unwrap();
        }

        let db = Database::open(&path).unwrap();
        let key = StateKey::new("system", "addons_enabled");
        assert_eq!(
            db.get_organization_state("org_legacy", &key).unwrap(),
            Some(serde_json::Value::Bool(false))
        );

        let second = Organization {
            id: "org_second".into(),
            name: "Second".into(),
            active: true,
            created_at: Utc::now(),
        };
        db.create_organization(&second).unwrap();
        assert_eq!(
            db.get_organization_state("org_second", &key).unwrap(),
            Some(serde_json::Value::Bool(false))
        );

        db.set_organization_state("org_second", &key, &serde_json::Value::Bool(true)).unwrap();
        assert_eq!(
            db.get_organization_state("org_legacy", &key).unwrap(),
            Some(serde_json::Value::Bool(false))
        );
        assert_eq!(
            db.get_organization_state("org_second", &key).unwrap(),
            Some(serde_json::Value::Bool(true))
        );

        db.remove_organization_state("org_second", &key).unwrap();
        assert_eq!(db.get_organization_state("org_second", &key).unwrap(), None);
        drop(db);
        std::fs::remove_file(path).unwrap();
    }
}

#[cfg(test)]
mod owner_device_persistence_tests {
    use super::*;

    #[test]
    fn owner_device_records_are_isolated_and_cannot_be_reassigned() {
        let path = std::env::temp_dir().join(format!("janus-owner-device-test-{}.db", uuid::Uuid::new_v4()));
        let db = Database::open(&path).unwrap();
        let now = Utc::now();
        let org = Organization {
            id: "org_owner_test".into(),
            name: "Owner Test".into(),
            active: true,
            created_at: now,
        };
        db.create_organization(&org).unwrap();

        let owner = UserAccount {
            id: "usr_owner_test".into(),
            organization_id: org.id.clone(),
            email: "owner@example.test".into(),
            role: UserRole::OrganizationAdmin,
            active: true,
            failed_login_count: 0,
            locked_until: None,
            created_at: now,
        };
        let other = UserAccount {
            id: "usr_other_test".into(),
            organization_id: org.id.clone(),
            email: "other@example.test".into(),
            role: UserRole::Operator,
            active: true,
            failed_login_count: 0,
            locked_until: None,
            created_at: now,
        };
        db.create_user(&owner, "test-hash").unwrap();
        db.create_user(&other, "test-hash").unwrap();

        let policies = serde_json::json!({"strict_mode": true});
        db.register_owner_device("MONO-TEST-1234", &org.id, &owner.id, "monolith", &policies).unwrap();

        assert_eq!(
            db.get_owned_device("MONO-TEST-1234", &owner.id).unwrap(),
            Some((org.id.clone(), "monolith".into(), policies.clone()))
        );
        assert!(db.get_owned_device("MONO-TEST-1234", &other.id).unwrap().is_none());
        assert_eq!(db.list_owned_devices(&owner.id).unwrap().len(), 1);
        assert!(db.list_owned_devices(&other.id).unwrap().is_empty());
        assert!(!db.update_owned_device_policy(
            "MONO-TEST-1234", &other.id, &serde_json::json!({"strict_mode": false})
        ).unwrap());
        assert!(db.register_owner_device(
            "MONO-TEST-1234", &org.id, &other.id, "pipboy", &policies
        ).is_err());

        drop(db);
        std::fs::remove_file(path).unwrap();
    }
}

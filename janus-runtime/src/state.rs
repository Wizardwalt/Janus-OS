//! State management and persistence.

use crate::database::Database;
use anyhow::Result;
use janus_core::{
    hash_password, AuditEntry, AuditLog, Config, Organization, StateKey, UserAccount,
    UserRole,
};
use sha2::{Digest, Sha256};
use std::sync::Arc;
use tracing::debug;

/// Authenticated session returned only after password verification.
pub struct LoginSession {
    pub token: String,
    pub expires_at: chrono::DateTime<chrono::Utc>,
    pub account: janus_core::UserAccount,
}

/// State manager with persistence layer.
pub struct StateManager {
    config: Config,
    db: Arc<Database>,
}

impl StateManager {
    /// Create new state manager
    pub async fn new(config: &Config, db: Arc<Database>) -> Result<Self> {
        debug!(
            "Initializing state manager with DB: {}",
            config.db_path.display()
        );
        Ok(Self {
            config: config.clone(),
            db,
        })
    }

    /// Complete the one-time initial organization administrator setup.
    pub async fn bootstrap_first_admin(
        &self,
        organization_name: String,
        email: String,
        password: String,
    ) -> Result<(Organization, UserAccount)> {
        let organization_name = organization_name.trim();
        let email = email.trim().to_ascii_lowercase();
        if organization_name.is_empty() || email.is_empty() || !email.contains('@') {
            return Err(anyhow::anyhow!(
                "organization name and a valid email are required"
            ));
        }

        let now = chrono::Utc::now();
        let organization = Organization {
            id: format!("org_{}", uuid::Uuid::new_v4()),
            name: organization_name.to_string(),
            active: true,
            created_at: now,
        };
        let account = UserAccount {
            id: format!("usr_{}", uuid::Uuid::new_v4()),
            organization_id: organization.id.clone(),
            email,
            role: UserRole::OrganizationAdmin,
            active: true,
            failed_login_count: 0,
            locked_until: None,
            created_at: now,
        };
        let password_hash = hash_password(&password)?;
        self.db
            .bootstrap_first_admin(&organization, &account, &password_hash)?;
        self.db.record(
            AuditEntry::new("bootstrap", "INITIAL_ADMIN_CREATED", &account.id)
                .success()
                .with_metadata(
                    serde_json::json!({"organization_id": organization.id, "email": account.email}),
                ),
        )?;
        Ok((organization, account))
    }

    /// Authenticate an active account and return an opaque, expiring session token.
    pub async fn login(&self, email: String, password: String) -> Result<LoginSession> {
        let email = email.trim().to_ascii_lowercase();
        let now = chrono::Utc::now();
        self.db.cleanup_expired_sessions(now)?;
        let stored = self
            .db
            .find_user_by_email(&email)?
            .ok_or_else(|| anyhow::anyhow!("invalid email or password"))?;

        if !stored.account.active {
            return Err(anyhow::anyhow!("account is inactive"));
        }
        if stored.account.locked_until.is_some_and(|until| until > now) {
            return Err(anyhow::anyhow!("account is temporarily locked"));
        }

        if !janus_core::verify_password(&password, &stored.password_hash)? {
            self.db.record_login_failure(&stored.account.id, now)?;
            self.db.record(
                AuditEntry::new(&stored.account.id, "LOGIN_FAILURE", "authentication")
                    .failed("invalid password"),
            )?;
            return Err(anyhow::anyhow!("invalid email or password"));
        }

        self.db.reset_login_failures(&stored.account.id)?;
        let token = format!("janus_{}", uuid::Uuid::new_v4());
        let token_hash = format!("{:x}", Sha256::digest(token.as_bytes()));
        let expires_at = now + chrono::Duration::hours(12);
        self.db
            .create_session(&token_hash, &stored.account.id, expires_at)?;
        self.db.record(
            AuditEntry::new(&stored.account.id, "LOGIN_SUCCESS", "authentication").success(),
        )?;

        Ok(LoginSession {
            token,
            expires_at,
            account: stored.account,
        })
    }

    /// Activate or deactivate an engagement in the administrator's organization.
    pub async fn set_engagement_active(
        &self,
        account: &janus_core::UserAccount,
        engagement_id: &str,
        active: bool,
    ) -> Result<()> {
        if !account.role.may_administer_organization() {
            return Err(anyhow::anyhow!("role is not allowed to manage engagements"));
        }
        if !self
            .db
            .set_engagement_active(&account.organization_id, engagement_id, active)?
        {
            return Err(anyhow::anyhow!(
                "engagement was not found in this organization"
            ));
        }
        self.db.record(
            AuditEntry::new(
                &account.id,
                if active {
                    "ENGAGEMENT_ENABLED"
                } else {
                    "ENGAGEMENT_DISABLED"
                },
                engagement_id,
            )
            .success(),
        )?;
        Ok(())
    }

    /// Return engagements belonging only to the authenticated organization.
    pub async fn engagements(
        &self,
        account: &janus_core::UserAccount,
    ) -> Result<Vec<janus_core::Engagement>> {
        self.db.list_engagements(&account.organization_id)
    }

    /// Return audit events that belong to the requesting organization.
    pub async fn audit_logs(
        &self,
        account: &janus_core::UserAccount,
        limit: usize,
    ) -> Result<Vec<AuditEntry>> {
        if !account.role.may_view_audit_logs() {
            return Err(anyhow::anyhow!("role is not allowed to view audit logs"));
        }
        self.db
            .query_audit_for_organization(&account.organization_id, limit.min(500))
    }

    /// Create a user inside the administrator's organization.
    pub async fn create_user(
        &self,
        account: &janus_core::UserAccount,
        email: String,
        password: String,
        role: janus_core::UserRole,
    ) -> Result<janus_core::UserAccount> {
        if !account.role.may_administer_organization() {
            return Err(anyhow::anyhow!("role is not allowed to create users"));
        }
        if role == janus_core::UserRole::PlatformBreakGlassAdmin {
            return Err(anyhow::anyhow!(
                "break-glass administrators cannot be created through organization administration"
            ));
        }
        let email = email.trim().to_ascii_lowercase();
        if !email.contains('@') {
            return Err(anyhow::anyhow!("a valid email is required"));
        }
        if self.db.find_user_by_email(&email)?.is_some() {
            return Err(anyhow::anyhow!("an account already exists for this email"));
        }
        let created = janus_core::UserAccount {
            id: format!("usr_{}", uuid::Uuid::new_v4()),
            organization_id: account.organization_id.clone(),
            email,
            role,
            active: true,
            failed_login_count: 0,
            locked_until: None,
            created_at: chrono::Utc::now(),
        };
        let password_hash = janus_core::hash_password(&password)?;
        self.db.create_user(&created, &password_hash)?;
        self.db.record(
            AuditEntry::new(&account.id, "USER_CREATED", &created.id)
                .success()
                .with_metadata(
                    serde_json::json!({"role": created.role.as_str(), "email": created.email}),
                ),
        )?;
        Ok(created)
    }

    /// Return safe user metadata for the administrator's organization.
    pub async fn users(
        &self,
        account: &janus_core::UserAccount,
    ) -> Result<Vec<janus_core::UserAccount>> {
        if !account.role.may_administer_organization() {
            return Err(anyhow::anyhow!("role is not allowed to list users"));
        }
        self.db.list_users(&account.organization_id)
    }

    /// Enable or disable an organization account and revoke sessions when disabling.
    pub async fn set_user_active(
        &self,
        account: &janus_core::UserAccount,
        user_id: &str,
        active: bool,
    ) -> Result<()> {
        if !account.role.may_administer_organization() {
            return Err(anyhow::anyhow!("role is not allowed to manage users"));
        }
        if account.id == user_id && !active {
            return Err(anyhow::anyhow!(
                "administrators cannot disable their own active account"
            ));
        }
        if !self
            .db
            .set_user_active(&account.organization_id, user_id, active)?
        {
            return Err(anyhow::anyhow!("user was not found in this organization"));
        }
        if !active {
            self.db.delete_user_sessions(user_id)?;
        }
        self.db.record(
            AuditEntry::new(
                &account.id,
                if active {
                    "USER_ENABLED"
                } else {
                    "USER_DISABLED"
                },
                user_id,
            )
            .success(),
        )?;
        Ok(())
    }

    /// Reset an organization account password and revoke every existing session.
    pub async fn reset_user_password(
        &self,
        account: &janus_core::UserAccount,
        user_id: &str,
        password: String,
    ) -> Result<()> {
        if !account.role.may_administer_organization() {
            return Err(anyhow::anyhow!("role is not allowed to reset passwords"));
        }
        let hash = janus_core::hash_password(&password)?;
        if !self
            .db
            .update_user_password(&account.organization_id, user_id, &hash)?
        {
            return Err(anyhow::anyhow!("user was not found in this organization"));
        }
        self.db.delete_user_sessions(user_id)?;
        self.db
            .record(AuditEntry::new(&account.id, "PASSWORD_RESET", user_id).success())?;
        Ok(())
    }

    /// Record a reviewer decision for a module's exact content hash.
    pub async fn certify_module(
        &self,
        account: &janus_core::UserAccount,
        module_id: String,
        module_sha256: String,
        status: janus_core::CertificationStatus,
        required_feature: janus_core::LicensedFeature,
        notes: Option<String>,
    ) -> Result<janus_core::ModuleCertification> {
        if !account.role.may_certify_modules() {
            return Err(anyhow::anyhow!("role is not allowed to certify modules"));
        }
        if module_id.trim().is_empty()
            || module_sha256.len() != 64
            || !module_sha256.chars().all(|value| value.is_ascii_hexdigit())
        {
            return Err(anyhow::anyhow!("a module ID and SHA-256 hash are required"));
        }
        let certification = janus_core::ModuleCertification {
            module_id: module_id.trim().to_string(),
            module_sha256: module_sha256.to_ascii_lowercase(),
            status,
            required_feature,
            reviewed_by: Some(account.id.clone()),
            reviewed_at: Some(chrono::Utc::now()),
            notes,
        };
        self.db.upsert_module_certification(&certification)?;
        self.db.record(
            AuditEntry::new(&account.id, "MODULE_CERTIFIED", &certification.module_id)
                .success()
                .with_metadata(serde_json::json!({"status": certification.status.as_str(), "sha256": certification.module_sha256})),
        )?;
        Ok(certification)
    }

    /// Create a customer-authorized engagement for the administrator's organization.
    pub async fn create_engagement(
        &self,
        account: &janus_core::UserAccount,
        authorization_reference: String,
        starts_at: chrono::DateTime<chrono::Utc>,
        ends_at: chrono::DateTime<chrono::Utc>,
        scope: janus_core::EngagementScope,
        active: bool,
    ) -> Result<janus_core::Engagement> {
        if !account.role.may_administer_organization() {
            return Err(anyhow::anyhow!("role is not allowed to create engagements"));
        }
        if authorization_reference.trim().is_empty() || ends_at <= starts_at {
            return Err(anyhow::anyhow!(
                "valid authorization reference and date range are required"
            ));
        }
        if scope.approved_assets.is_empty() && scope.approved_evidence_paths.is_empty() {
            return Err(anyhow::anyhow!(
                "an engagement requires at least one approved asset or evidence path"
            ));
        }
        if scope.approved_features.is_empty() {
            return Err(anyhow::anyhow!(
                "an engagement requires at least one approved feature"
            ));
        }
        let engagement = janus_core::Engagement {
            id: format!("eng_{}", uuid::Uuid::new_v4()),
            organization_id: account.organization_id.clone(),
            authorization_reference: authorization_reference.trim().to_string(),
            starts_at,
            ends_at,
            scope,
            active,
        };
        self.db.create_engagement(&engagement)?;
        self.db.record(
            AuditEntry::new(&account.id, "ENGAGEMENT_CREATED", &engagement.id)
                .success()
                .with_metadata(serde_json::json!({"authorization_reference": engagement.authorization_reference})),
        )?;
        Ok(engagement)
    }

    /// Return the active entitlement claims without exposing the signed document.
    pub async fn license_claims(
        &self,
        account: &janus_core::UserAccount,
    ) -> Result<janus_core::LicenseClaims> {
        if !account.role.may_administer_organization() {
            return Err(anyhow::anyhow!(
                "role is not allowed to view license status"
            ));
        }
        self.db
            .find_active_license(&account.organization_id)?
            .map(|license| license.claims)
            .ok_or_else(|| anyhow::anyhow!("organization has no active license"))
    }

    /// Import a verified license for the authenticated organization.
    pub async fn import_license(
        &self,
        account: &janus_core::UserAccount,
        license: janus_core::SignedLicense,
    ) -> Result<()> {
        if !account.role.may_administer_organization() {
            return Err(anyhow::anyhow!("role is not allowed to import licenses"));
        }
        if license.claims.organization_id != account.organization_id {
            return Err(anyhow::anyhow!(
                "license belongs to a different organization"
            ));
        }
        let public_key = self
            .config
            .license_public_key
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("JANUS_LICENSE_PUBLIC_KEY is not configured"))?;
        self.db.store_license(&license, public_key)?;
        self.db.record(
            AuditEntry::new(&account.id, "LICENSE_IMPORTED", &license.claims.license_id)
                .success()
                .with_metadata(serde_json::json!({"organization_id": account.organization_id})),
        )?;
        Ok(())
    }

    /// Record an authorization or execution decision against a customer asset.
    pub async fn audit_execution_event(
        &self,
        account: &janus_core::UserAccount,
        action: &str,
        plugin_id: &str,
        target_asset: &str,
        result: std::result::Result<(), &str>,
    ) -> Result<()> {
        let entry = AuditEntry::new(&account.id, action, plugin_id)
            .with_metadata(serde_json::json!({"organization_id": account.organization_id, "target_asset": target_asset}));
        let entry = match result {
            Ok(()) => entry.success(),
            Err(error) => entry.failed(error),
        };
        self.db.record(entry)?;
        Ok(())
    }

    /// Apply the complete license, engagement, target, and module certification gate.
    pub async fn authorize_production_execution(
        &self,
        account: &janus_core::UserAccount,
        engagement_id: &str,
        target_asset: &str,
        module_id: &str,
        module_sha256: &str,
    ) -> Result<()> {
        let public_key = self
            .config
            .license_public_key
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("JANUS_LICENSE_PUBLIC_KEY is not configured"))?;
        let license = self
            .db
            .find_active_license(&account.organization_id)?
            .ok_or_else(|| anyhow::anyhow!("organization has no active license"))?;
        let engagement = self
            .db
            .find_engagement(engagement_id)?
            .ok_or_else(|| anyhow::anyhow!("engagement was not found"))?;
        let certification = self
            .db
            .find_module_certification(module_id)?
            .ok_or_else(|| anyhow::anyhow!("module has no certification record"))?;
        janus_core::authorize_execution(janus_core::ExecutionAuthorization {
            license: &license,
            license_public_key_base64: public_key,
            engagement: &engagement,
            certification: &certification,
            module_sha256,
            target_asset,
            now: chrono::Utc::now(),
        })?;
        Ok(())
    }

    /// Resolve an opaque session token to an active account.
    pub async fn authenticate_session(&self, token: &str) -> Result<janus_core::UserAccount> {
        let token_hash = format!("{:x}", Sha256::digest(token.as_bytes()));
        let account = self
            .db
            .find_session_user(&token_hash, chrono::Utc::now())?
            .ok_or_else(|| anyhow::anyhow!("session is invalid or expired"))?;
        if !account.active {
            return Err(anyhow::anyhow!("account is inactive"));
        }
        Ok(account)
    }

    /// Revoke an opaque session token and record the authenticated logout.
    pub async fn logout(&self, account: &janus_core::UserAccount, token: &str) -> Result<()> {
        let token_hash = format!("{:x}", Sha256::digest(token.as_bytes()));
        self.db.delete_session(&token_hash)?;
        self.db
            .record(AuditEntry::new(&account.id, "LOGOUT", "authentication").success())?;
        Ok(())
    }

    /// Read a state value only within the authenticated account's organization.
    pub async fn get(
        &self,
        account: &UserAccount,
        key: &StateKey,
    ) -> Result<Option<serde_json::Value>> {
        self.db.get_organization_state(&account.organization_id, key)
    }

    /// Update a state value within the authenticated organization.
    pub async fn update_state(
        &self,
        account: &UserAccount,
        key: &StateKey,
        value: serde_json::Value,
    ) -> Result<()> {
        if !account.role.may_modify_state() {
            anyhow::bail!("role is not allowed to modify state");
        }
        self.db
            .set_organization_state(&account.organization_id, key, &value)?;
        self.db.record(
            AuditEntry::new(&account.id, "ORGANIZATION_STATE_UPDATED", &key.fqn())
                .success()
                .with_metadata(serde_json::json!({
                    "organization_id": account.organization_id,
                    "namespace": key.namespace,
                    "key": key.key
                })),
        )?;
        Ok(())
    }
}

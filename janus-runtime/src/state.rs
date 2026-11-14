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

const OWNER_POLICY_DEFAULTS: &[(&str, bool)] = &[
    ("require_approval_for_sensitive", true),
    ("remote_access_enabled", false),
    ("cloud_provider_enabled", false),
    ("microphone_enabled", false),
    ("hardware_modules_enabled", false),
    ("strict_mode", true),
];

fn validate_owner_device_identity(device_id: &str, device_type: &str) -> Result<()> {
    let device_id = device_id.trim();
    if !(8..=128).contains(&device_id.len())
        || !device_id.chars().all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
    {
        anyhow::bail!("device_id must be 8-128 ASCII letters, numbers, hyphens, or underscores");
    }
    if !matches!(device_type, "monolith" | "pipboy") {
        anyhow::bail!("device_type must be 'monolith' or 'pipboy'");
    }
    Ok(())
}

fn merge_owner_policy(
    current: Option<serde_json::Value>,
    requested: &serde_json::Value,
) -> Result<serde_json::Value> {
    let object = requested.as_object()
        .ok_or_else(|| anyhow::anyhow!("policies must be a JSON object"))?;
    let allowed = OWNER_POLICY_DEFAULTS.iter().map(|(key, _)| *key).collect::<Vec<_>>();
    if object.is_empty()
        || object.keys().any(|key| !allowed.contains(&key.as_str()))
        || object.values().any(|value| !value.is_boolean())
    {
        anyhow::bail!("policy updates must contain only allowlisted boolean settings");
    }
    let mut policies = current.unwrap_or_else(|| {
        serde_json::Value::Object(OWNER_POLICY_DEFAULTS.iter()
            .map(|(key, value)| ((*key).to_string(), serde_json::Value::Bool(*value)))
            .collect())
    });
    let stored = policies.as_object_mut()
        .ok_or_else(|| anyhow::anyhow!("stored device policy is invalid"))?;
    for (key, value) in object {
        stored.insert(key.clone(), value.clone());
    }
    Ok(policies)
}

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

    /// Register a Monolith or Pip-Boy to the authenticated purchaser account.
    /// Password re-entry makes the initial ownership claim an intentional action.
    pub async fn register_owned_device(
        &self,
        account: &janus_core::UserAccount,
        password: &str,
        device_id: &str,
        device_type: &str,
    ) -> Result<serde_json::Value> {
        if !account.role.may_administer_organization() {
            anyhow::bail!("only an organization administrator can initialize device ownership");
        }
        let device_id = device_id.trim();
        validate_owner_device_identity(device_id, device_type)?;
        let stored = self.db.find_user_by_email(&account.email)?
            .ok_or_else(|| anyhow::anyhow!("owner account was not found"))?;
        if !janus_core::verify_password(password, &stored.password_hash)? {
            self.db.record(
                AuditEntry::new(&account.id, "DEVICE_OWNER_AUTH_FAILURE", device_id)
                    .failed("password re-authentication failed"),
            )?;
            anyhow::bail!("owner password verification failed");
        }
        let policies = merge_owner_policy(None, &serde_json::json!({
            "require_approval_for_sensitive": true,
            "remote_access_enabled": false,
            "cloud_provider_enabled": false,
            "microphone_enabled": false,
            "hardware_modules_enabled": false,
            "strict_mode": true
        }))?;
        self.db.register_owner_device(
            device_id, &account.organization_id, &account.id, device_type, &policies,
        )?;
        self.db.record(
            AuditEntry::new(&account.id, "DEVICE_OWNER_ENROLLED", device_id)
                .success()
                .with_metadata(serde_json::json!({"device_type": device_type})),
        )?;
        Ok(serde_json::json!({
            "device_id": device_id,
            "device_type": device_type,
            "policies": policies
        }))
    }

    /// Issue a five-minute, owner- and device-bound proof-of-possession challenge.
    pub async fn create_device_enrollment_challenge(
        &self,
        account: &janus_core::UserAccount,
        device_id: &str,
    ) -> Result<serde_json::Value> {
        if !account.role.may_administer_organization() {
            anyhow::bail!("only an organization administrator can enroll a device key");
        }
        let (organization_id, device_type, _) = self.db.get_owned_device(device_id, &account.id)?
            .ok_or_else(|| anyhow::anyhow!("device is not registered to this owner"))?;
        if organization_id != account.organization_id {
            anyhow::bail!("device ownership organization mismatch");
        }
        let issued_at = chrono::Utc::now().timestamp();
        let expires_at = issued_at + 300;
        let challenge_id = uuid::Uuid::new_v4().to_string();
        let nonce = uuid::Uuid::new_v4().to_string();
        let claims = crate::device_enrollment::EnrollmentClaims {
            challenge_id: challenge_id.clone(),
            nonce: nonce.clone(),
            device_id: device_id.to_string(),
            device_type: device_type.clone(),
            owner_user_id: account.id.clone(),
            issued_at_unix: issued_at,
            expires_at_unix: expires_at,
            protocol_version: 1,
        };
        // Validate the exact claims before persisting or returning them.
        claims.signing_bytes()?;
        self.db.create_device_enrollment_challenge(
            &challenge_id, &nonce, device_id, &account.id, &device_type, issued_at, expires_at,
        )?;
        self.db.record(
            AuditEntry::new(&account.id, "DEVICE_ENROLLMENT_CHALLENGE_ISSUED", device_id)
                .success()
                .with_metadata(serde_json::json!({"device_type": device_type, "expires_at_unix": expires_at})),
        )?;
        Ok(serde_json::json!({
            "challenge_id": claims.challenge_id,
            "nonce": claims.nonce,
            "device_id": claims.device_id,
            "device_type": claims.device_type,
            "owner_user_id": claims.owner_user_id,
            "issued_at_unix": claims.issued_at_unix,
            "expires_at_unix": claims.expires_at_unix,
            "protocol_version": claims.protocol_version
        }))
    }

    /// Verify the device signature, then atomically consume the challenge and save
    /// the public key. The private key never leaves the physical device.
    pub async fn complete_device_enrollment(
        &self,
        account: &janus_core::UserAccount,
        device_id: &str,
        challenge_id: &str,
        public_key: &str,
        signature: &str,
    ) -> Result<serde_json::Value> {
        if !account.role.may_administer_organization() {
            anyhow::bail!("only an organization administrator can complete device enrollment");
        }
        let (organization_id, device_type, _) = self.db.get_owned_device(device_id, &account.id)?
            .ok_or_else(|| anyhow::anyhow!("device is not registered to this owner"))?;
        if organization_id != account.organization_id {
            anyhow::bail!("device ownership organization mismatch");
        }
        let (nonce, stored_type, stored_device, issued_at, expires_at) =
            self.db.get_device_enrollment_challenge(challenge_id, &account.id, device_id)?
                .ok_or_else(|| anyhow::anyhow!("challenge is missing, consumed, or not owned by this account"))?;
        if stored_type != device_type || stored_device != device_id {
            anyhow::bail!("enrollment challenge device binding mismatch");
        }
        let claims = crate::device_enrollment::EnrollmentClaims {
            challenge_id: challenge_id.to_string(),
            nonce,
            device_id: device_id.to_string(),
            device_type,
            owner_user_id: account.id.clone(),
            issued_at_unix: issued_at,
            expires_at_unix: expires_at,
            protocol_version: 1,
        };
        crate::device_enrollment::verify_device_proof(
            &claims, public_key, signature, chrono::Utc::now().timestamp(),
        )?;
        let fingerprint = format!("{:x}", Sha256::digest(public_key.as_bytes()));
        if !self.db.consume_device_enrollment_challenge(
            challenge_id, &account.id, device_id, public_key, &fingerprint,
            chrono::Utc::now().timestamp(),
        )? {
            anyhow::bail!("enrollment challenge expired or has already been consumed");
        }
        self.db.record(
            AuditEntry::new(&account.id, "DEVICE_PUBLIC_KEY_ENROLLED", device_id)
                .success()
                .with_metadata(serde_json::json!({"key_fingerprint": fingerprint})),
        )?;
        Ok(serde_json::json!({"device_id": device_id, "key_fingerprint": fingerprint, "enrolled": true}))
    }

    /// Add an opaque encrypted sync event on behalf of an enrolled owner's device.
    /// This endpoint is owner-session authenticated; device-signature authentication
    /// for unattended device-to-device transport is a separate follow-up.
    pub async fn append_device_sync_event(
        &self,
        account: &janus_core::UserAccount,
        source_device_id: &str,
        event_id: &str,
        sequence: i64,
        event_type: &str,
        payload_ciphertext: &str,
    ) -> Result<serde_json::Value> {
        if !account.role.may_administer_organization() {
            anyhow::bail!("only the device owner can write sync events");
        }
        if event_id.is_empty() || event_id.len() > 128 || !event_id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
            anyhow::bail!("event_id must be 1-128 ASCII letters, numbers, hyphens, or underscores");
        }
        if sequence <= 0 || event_type.is_empty() || event_type.len() > 64 || payload_ciphertext.is_empty() || payload_ciphertext.len() > 1_048_576 {
            anyhow::bail!("invalid sync event sequence, type, or payload size");
        }
        let (organization_id, _, _) = self.db.get_owned_device(source_device_id, &account.id)?
            .ok_or_else(|| anyhow::anyhow!("source device is not registered to this owner"))?;
        if organization_id != account.organization_id {
            anyhow::bail!("sync source device organization mismatch");
        }
        let payload_sha256 = format!("{:x}", Sha256::digest(payload_ciphertext.as_bytes()));
        let inserted = self.db.append_device_sync_event(
            event_id, &account.id, source_device_id, sequence, event_type, payload_ciphertext, &payload_sha256,
        )?;
        Ok(serde_json::json!({"event_id": event_id, "sequence": sequence, "payload_sha256": payload_sha256, "inserted": inserted}))
    }

    /// Fetch pending journal entries from one of this owner's devices for another.
    pub async fn pending_device_sync_events(
        &self,
        account: &janus_core::UserAccount,
        target_device_id: &str,
        source_device_id: &str,
        limit: i64,
    ) -> Result<Vec<serde_json::Value>> {
        if !account.role.may_administer_organization() {
            anyhow::bail!("only the device owner can read sync events");
        }
        for device_id in [target_device_id, source_device_id] {
            let (organization_id, _, _) = self.db.get_owned_device(device_id, &account.id)?
                .ok_or_else(|| anyhow::anyhow!("sync device is not registered to this owner"))?;
            if organization_id != account.organization_id {
                anyhow::bail!("sync device organization mismatch");
            }
        }
        if !(1..=500).contains(&limit) {
            anyhow::bail!("sync event batch limit must be between 1 and 500");
        }
        self.db.pending_device_sync_events(&account.id, target_device_id, source_device_id, limit)
    }

    /// Acknowledge events only after the target device has durably applied them.
    pub async fn acknowledge_device_sync_events(
        &self,
        account: &janus_core::UserAccount,
        target_device_id: &str,
        source_device_id: &str,
        last_sequence: i64,
    ) -> Result<serde_json::Value> {
        if !account.role.may_administer_organization() {
            anyhow::bail!("only the device owner can acknowledge sync events");
        }
        for device_id in [target_device_id, source_device_id] {
            let (organization_id, _, _) = self.db.get_owned_device(device_id, &account.id)?
                .ok_or_else(|| anyhow::anyhow!("sync device is not registered to this owner"))?;
            if organization_id != account.organization_id {
                anyhow::bail!("sync device organization mismatch");
            }
        }
        let cursor = self.db.acknowledge_device_sync_events(&account.id, target_device_id, source_device_id, last_sequence)?;
        Ok(serde_json::json!({"target_device_id": target_device_id, "source_device_id": source_device_id, "last_sequence": cursor}))
    }

    /// List only devices registered to the authenticated account.
    pub async fn owned_devices(
        &self,
        account: &janus_core::UserAccount,
    ) -> Result<Vec<serde_json::Value>> {
        self.db.list_owned_devices(&account.id).map(|devices| {
            devices.into_iter().map(|(device_id, device_type, policies)| {
                serde_json::json!({
                    "device_id": device_id,
                    "device_type": device_type,
                    "policies": policies
                })
            }).collect()
        })
    }

    /// Update a device's allowlisted configurable security switches after fresh
    /// owner password verification. Core integrity boundaries are not configurable.
    pub async fn update_owned_device_policy(
        &self,
        account: &janus_core::UserAccount,
        device_id: &str,
        password: &str,
        requested: serde_json::Value,
    ) -> Result<serde_json::Value> {
        if !account.role.may_administer_organization() {
            anyhow::bail!("only an organization administrator can change device security policies");
        }
        validate_owner_device_identity(device_id, "monolith")
            .or_else(|_| validate_owner_device_identity(device_id, "pipboy"))?;
        let changed_settings = requested
            .as_object()
            .ok_or_else(|| anyhow::anyhow!("policies must be a JSON object"))?
            .keys()
            .cloned()
            .collect::<Vec<_>>();
        let stored = self.db.find_user_by_email(&account.email)?
            .ok_or_else(|| anyhow::anyhow!("owner account was not found"))?;
        if stored.account.id != account.id
            || !janus_core::verify_password(password, &stored.password_hash)?
        {
            self.db.record(
                AuditEntry::new(&account.id, "DEVICE_POLICY_AUTH_FAILURE", device_id)
                    .failed("password re-authentication failed"),
            )?;
            anyhow::bail!("owner password verification failed");
        }
        let (organization_id, device_type, current_policies) =
            self.db.get_owned_device(device_id, &account.id)?
                .ok_or_else(|| anyhow::anyhow!("device is not registered to this owner"))?;
        if organization_id != account.organization_id {
            anyhow::bail!("device ownership organization mismatch");
        }
        let policies = merge_owner_policy(Some(current_policies), &requested)?;
        if !self.db.update_owned_device_policy(device_id, &account.id, &policies)? {
            anyhow::bail!("device policy update did not affect an owned device");
        }
        self.db.record(
            AuditEntry::new(&account.id, "DEVICE_SECURITY_POLICY_UPDATED", device_id)
                .success()
                .with_metadata(serde_json::json!({
                    "device_type": device_type,
                    "changed_settings": changed_settings,
                    "policies": policies
                })),
        )?;
        Ok(policies)
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

#[cfg(test)]
mod owner_policy_tests {
    use super::*;

    #[test]
    fn owner_device_identity_requires_valid_id_and_known_device_type() {
        assert!(validate_owner_device_identity("MONO-123456", "monolith").is_ok());
        assert!(validate_owner_device_identity("PIPBOY_123", "pipboy").is_ok());
        assert!(validate_owner_device_identity("short", "monolith").is_err());
        assert!(validate_owner_device_identity("../outside-123", "pipboy").is_err());
        assert!(validate_owner_device_identity("VALID-123456", "unknown").is_err());
    }

    #[test]
    fn owner_policy_rejects_unknown_keys_and_non_boolean_values() {
        assert!(merge_owner_policy(None, &serde_json::json!({"shell_access": true})).is_err());
        assert!(merge_owner_policy(None, &serde_json::json!({"remote_access_enabled": "yes"})).is_err());
        assert!(merge_owner_policy(None, &serde_json::json!({})).is_err());
        assert!(merge_owner_policy(None, &serde_json::json!([])).is_err());
    }

    #[test]
    fn owner_policy_preserves_existing_values_when_one_setting_changes() {
        let existing = serde_json::json!({
            "require_approval_for_sensitive": true,
            "remote_access_enabled": false,
            "cloud_provider_enabled": true,
            "microphone_enabled": false,
            "hardware_modules_enabled": false,
            "strict_mode": true
        });
        let updated = merge_owner_policy(
            Some(existing),
            &serde_json::json!({"remote_access_enabled": true}),
        ).unwrap();
        assert_eq!(updated["remote_access_enabled"], true);
        assert_eq!(updated["cloud_provider_enabled"], true);
        assert_eq!(updated["strict_mode"], true);
    }
}


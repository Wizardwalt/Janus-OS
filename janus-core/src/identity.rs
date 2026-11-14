//! Organization and operator identity primitives.

use argon2::{
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Argon2,
};
use chrono::{DateTime, Utc};
use rand_core::OsRng;
use serde::{Deserialize, Serialize};

/// Privileges granted to an authenticated Janus operator.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UserRole {
    PlatformBreakGlassAdmin,
    OrganizationAdmin,
    Operator,
    Reviewer,
    CustomerReadOnly,
}

impl UserRole {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::PlatformBreakGlassAdmin => "platform_break_glass_admin",
            Self::OrganizationAdmin => "organization_admin",
            Self::Operator => "operator",
            Self::Reviewer => "reviewer",
            Self::CustomerReadOnly => "customer_read_only",
        }
    }

    /// Whether this role may record production module certification decisions.
    pub fn may_certify_modules(self) -> bool {
        matches!(self, Self::PlatformBreakGlassAdmin)
    }

    /// Whether this role may modify protected organization/runtime state.
    pub fn may_modify_state(self) -> bool {
        matches!(self, Self::OrganizationAdmin)
    }

    /// Whether this role may view organization audit events.
    pub fn may_view_audit_logs(self) -> bool {
        matches!(self, Self::OrganizationAdmin | Self::Reviewer)
    }

    /// Whether this role may manage organization licenses and engagements.
    pub fn may_administer_organization(self) -> bool {
        matches!(self, Self::OrganizationAdmin)
    }

    /// Whether this role may submit an execution request. This does not bypass
    /// engagement, license, target, or module-certification checks.
    pub fn may_request_execution(self) -> bool {
        matches!(self, Self::OrganizationAdmin | Self::Operator)
    }

    pub fn from_str(value: &str) -> Option<Self> {
        match value {
            "platform_break_glass_admin" => Some(Self::PlatformBreakGlassAdmin),
            "organization_admin" => Some(Self::OrganizationAdmin),
            "operator" => Some(Self::Operator),
            "reviewer" => Some(Self::Reviewer),
            "customer_read_only" => Some(Self::CustomerReadOnly),
            _ => None,
        }
    }
}

/// A customer or platform organization.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Organization {
    pub id: String,
    pub name: String,
    pub active: bool,
    pub created_at: DateTime<Utc>,
}

/// Janus-managed account metadata. Password hashes are intentionally not
/// serialized or exposed through public API response types.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserAccount {
    pub id: String,
    pub organization_id: String,
    pub email: String,
    pub role: UserRole,
    pub active: bool,
    pub failed_login_count: u32,
    pub locked_until: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

/// Hash a Janus-managed password with Argon2id and a cryptographically random salt.
/// Passwords are never persisted or logged in plaintext.
pub fn hash_password(password: &str) -> crate::Result<String> {
    if password.len() < 12 {
        return Err(crate::JanusError::Security(
            "password must contain at least 12 characters".to_string(),
        ));
    }
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|hash| hash.to_string())
        .map_err(|error| crate::JanusError::Security(error.to_string()))
}

/// Verify a password against an Argon2id password hash.
pub fn verify_password(password: &str, password_hash: &str) -> crate::Result<bool> {
    let parsed_hash = PasswordHash::new(password_hash)
        .map_err(|_| crate::JanusError::Security("stored password hash is invalid".to_string()))?;
    Ok(Argon2::default()
        .verify_password(password.as_bytes(), &parsed_hash)
        .is_ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn password_hashing_accepts_only_the_matching_password() {
        let hash = hash_password("correct-horse-battery-staple").unwrap();
        assert_ne!(hash, "correct-horse-battery-staple");
        assert!(verify_password("correct-horse-battery-staple", &hash).unwrap());
        assert!(!verify_password("incorrect-password", &hash).unwrap());
    }

    #[test]
    fn password_policy_rejects_short_passwords() {
        assert!(hash_password("too-short").is_err());
    }
}

#[cfg(test)]
mod role_tests {
    use super::UserRole;

    #[test]
    fn role_permissions_keep_execution_and_administration_separate() {
        assert!(UserRole::OrganizationAdmin.may_request_execution());
        assert!(UserRole::Operator.may_request_execution());
        assert!(!UserRole::Reviewer.may_request_execution());
        assert!(!UserRole::CustomerReadOnly.may_request_execution());

        assert!(UserRole::OrganizationAdmin.may_administer_organization());
        assert!(!UserRole::Operator.may_administer_organization());
        assert!(UserRole::PlatformBreakGlassAdmin.may_certify_modules());
        assert!(!UserRole::Reviewer.may_certify_modules());
        assert!(!UserRole::OrganizationAdmin.may_certify_modules());
        assert!(UserRole::OrganizationAdmin.may_view_audit_logs());
        assert!(UserRole::Reviewer.may_view_audit_logs());
        assert!(!UserRole::Operator.may_view_audit_logs());
    }
}

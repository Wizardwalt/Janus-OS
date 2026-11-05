//! End-to-end tests for the production authorization and execution path.
//!
//! These exercise the real chain against a real temporary SQLite database and a
//! real file on disk: bootstrap -> signed license -> reviewer certification ->
//! customer engagement -> authorization decision -> sandboxed execution.
//!
//! Licenses are minted with a throwaway Ed25519 key generated per test run. No
//! private key is committed, and the public key is injected through configuration
//! exactly as a real deployment would.

use crate::{database::Database, executor::PluginExecutor, plugin::PluginLoader, state::StateManager};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use ed25519_dalek::{Signer, SigningKey};
use janus_core::{
    CertificationStatus, Config, EngagementScope, LicenseClaims, LicensedFeature, SignedLicense,
    UserRole,
};
use std::sync::Arc;

/// A throwaway signing key. Generated fresh for every test run.
fn test_signing_key() -> SigningKey {
    let mut seed = [0_u8; 32];
    // Mix time and pid so runs do not share a key. This is test-only entropy;
    // production licenses are signed by the licensing service.
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let pid = std::process::id() as u128;
    let mixed = nanos ^ (pid << 64) ^ 0x5a4a_4e55_5354_4b45_59u128;
    seed.copy_from_slice(&mixed.to_le_bytes()[..16]);
    seed[16..].copy_from_slice(&(mixed >> 32).to_le_bytes()[..16]);
    SigningKey::from_bytes(&seed)
}

fn temp_dir(label: &str) -> std::path::PathBuf {
    let unique = format!(
        "janus-e2e-{}-{}-{}",
        label,
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let dir = std::env::temp_dir().join(unique);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn signed_license(
    key: &SigningKey,
    organization_id: &str,
    features: Vec<LicensedFeature>,
    expired: bool,
) -> SignedLicense {
    let now = chrono::Utc::now();
    let claims = LicenseClaims {
        license_id: format!("lic_{}", uuid::Uuid::new_v4()),
        organization_id: organization_id.to_string(),
        plan: "professional".to_string(),
        issued_at: now - chrono::Duration::hours(2),
        expires_at: if expired {
            now - chrono::Duration::hours(1)
        } else {
            now + chrono::Duration::hours(2)
        },
        max_operators: 5,
        enabled_features: features,
    };
    let signature = key.sign(&serde_json::to_vec(&claims).unwrap());
    SignedLicense {
        claims,
        signature: STANDARD.encode(signature.to_bytes()),
    }
}

/// Everything a test needs, wired exactly as the daemon wires it.
struct Fixture {
    dir: std::path::PathBuf,
    db: Arc<Database>,
    state: StateManager,
    executor: PluginExecutor,
    admin: janus_core::UserAccount,
    reviewer: janus_core::UserAccount,
    evidence_dir: std::path::PathBuf,
    evidence_file: std::path::PathBuf,
    module_sha: String,
}

impl Fixture {
    async fn new(label: &str) -> Self {
        let dir = temp_dir(label);
        let evidence_dir = dir.join("evidence");
        std::fs::create_dir_all(&evidence_dir).unwrap();
        let evidence_file = evidence_dir.join("case-1.bin");
        std::fs::write(&evidence_file, b"abc").unwrap();

        // Load the real shipped module library.
        let plugin_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../plugins");
        let mut loader = PluginLoader::new(&plugin_dir);
        loader.load_all().await.unwrap();

        let key = test_signing_key();
        let config = Config {
            db_path: dir.join("janus.db"),
            plugin_dir: plugin_dir.clone(),
            license_public_key: Some(STANDARD.encode(key.verifying_key().to_bytes())),
            hardware_enabled: false,
            ..Config::default()
        };

        let db = Arc::new(Database::open(&config.db_path).unwrap());
        let state = StateManager::new(&config, db.clone()).await.unwrap();
        let executor = PluginExecutor::new(&config, loader).await.unwrap();

        let (_org, admin) = state
            .bootstrap_first_admin(
                "Test Organization".to_string(),
                "admin@example.test".to_string(),
                "correct-horse-battery-staple".to_string(),
            )
            .await
            .unwrap();

        let reviewer = state
            .create_user(
                &admin,
                "reviewer@example.test".to_string(),
                "reviewer-passphrase-long".to_string(),
                UserRole::Reviewer,
            )
            .await
            .unwrap();

        // Import a valid forensics license.
        let license = signed_license(
            &key,
            &admin.organization_id,
            vec![LicensedFeature::Forensics],
            false,
        );
        state.import_license(&admin, license).await.unwrap();

        // Certify the real shipped module against its real content hash.
        let module_sha = executor.plugin_sha256("evidence.hash").unwrap();
        state
            .certify_module(
                &reviewer,
                "evidence.hash".to_string(),
                module_sha.clone(),
                CertificationStatus::ProductionApproved,
                LicensedFeature::Forensics,
                Some("read-only evidence hashing".to_string()),
            )
            .await
            .unwrap();

        Self {
            dir,
            db,
            state,
            executor,
            admin,
            reviewer,
            evidence_dir,
            evidence_file,
            module_sha,
        }
    }

    async fn create_engagement(&self, evidence_paths: Vec<String>) -> janus_core::Engagement {
        let now = chrono::Utc::now();
        self.state
            .create_engagement(
                &self.admin,
                "AUTH-E2E-1".to_string(),
                now - chrono::Duration::hours(1),
                now + chrono::Duration::hours(1),
                EngagementScope {
                    approved_assets: vec![],
                    approved_evidence_paths: evidence_paths,
                    approved_features: vec![LicensedFeature::Forensics],
                },
                true,
            )
            .await
            .unwrap()
    }

    fn cleanup(self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

#[tokio::test]
async fn authorized_execution_hashes_a_real_approved_file() {
    let fx = Fixture::new("happy").await;
    let engagement = fx
        .create_engagement(vec![fx.evidence_dir.display().to_string()])
        .await;

    let scope = fx
        .state
        .authorize_production_execution(
            &fx.admin,
            &engagement.id,
            &fx.evidence_dir.display().to_string(),
            "evidence.hash",
            &fx.module_sha,
        )
        .await
        .expect("a fully authorized request must pass the gate");

    let evidence_scope =
        crate::evidence::EvidenceScope::new(&scope.approved_evidence_paths).unwrap();
    let result = fx
        .executor
        .execute(
            "evidence.hash",
            serde_json::json!({ "path": fx.evidence_file.display().to_string() }),
            evidence_scope,
        )
        .await
        .expect("execution must succeed");

    assert_eq!(result["result"]["status"], "success");
    assert_eq!(result["result"]["read_only"], true);
    assert_eq!(
        result["result"]["sha256"],
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
        "must be the real SHA-256 of the real file"
    );
    assert!(
        result["logs"].as_array().map(|l| !l.is_empty()).unwrap_or(false),
        "module must emit real log output"
    );
    fx.cleanup();
}

#[tokio::test]
async fn execution_is_refused_when_the_module_content_changed() {
    let fx = Fixture::new("tamper").await;
    let engagement = fx
        .create_engagement(vec![fx.evidence_dir.display().to_string()])
        .await;

    let result = fx
        .state
        .authorize_production_execution(
            &fx.admin,
            &engagement.id,
            &fx.evidence_dir.display().to_string(),
            "evidence.hash",
            &"0".repeat(64),
        )
        .await;
    assert!(result.is_err(), "a changed module must not execute");
    fx.cleanup();
}

#[tokio::test]
async fn execution_is_refused_for_a_target_outside_the_engagement_scope() {
    let fx = Fixture::new("scope").await;
    let engagement = fx
        .create_engagement(vec![fx.evidence_dir.display().to_string()])
        .await;

    let result = fx
        .state
        .authorize_production_execution(
            &fx.admin,
            &engagement.id,
            "/etc",
            "evidence.hash",
            &fx.module_sha,
        )
        .await;
    assert!(result.is_err(), "an unapproved target must be refused");
    fx.cleanup();
}

#[tokio::test]
async fn execution_is_refused_without_a_certification_record() {
    let fx = Fixture::new("uncertified").await;
    let engagement = fx
        .create_engagement(vec![fx.evidence_dir.display().to_string()])
        .await;

    // evidence.stat was never certified in this fixture.
    let sha = fx.executor.plugin_sha256("evidence.stat").unwrap();
    let result = fx
        .state
        .authorize_production_execution(
            &fx.admin,
            &engagement.id,
            &fx.evidence_dir.display().to_string(),
            "evidence.stat",
            &sha,
        )
        .await;
    assert!(result.is_err(), "an uncertified module must not execute");
    fx.cleanup();
}

#[tokio::test]
async fn a_pending_review_certification_does_not_permit_execution() {
    let fx = Fixture::new("pending").await;
    let engagement = fx
        .create_engagement(vec![fx.evidence_dir.display().to_string()])
        .await;
    let sha = fx.executor.plugin_sha256("evidence.list").unwrap();
    fx.state
        .certify_module(
            &fx.reviewer,
            "evidence.list".to_string(),
            sha.clone(),
            CertificationStatus::PendingReview,
            LicensedFeature::Forensics,
            None,
        )
        .await
        .unwrap();

    let result = fx
        .state
        .authorize_production_execution(
            &fx.admin,
            &engagement.id,
            &fx.evidence_dir.display().to_string(),
            "evidence.list",
            &sha,
        )
        .await;
    assert!(result.is_err(), "pending_review is not production approval");
    fx.cleanup();
}

#[tokio::test]
async fn an_operator_cannot_certify_modules() {
    let fx = Fixture::new("role").await;
    let operator = fx
        .state
        .create_user(
            &fx.admin,
            "operator@example.test".to_string(),
            "operator-passphrase-long".to_string(),
            UserRole::Operator,
        )
        .await
        .unwrap();
    let result = fx
        .state
        .certify_module(
            &operator,
            "evidence.hash".to_string(),
            fx.module_sha.clone(),
            CertificationStatus::ProductionApproved,
            LicensedFeature::Forensics,
            None,
        )
        .await;
    assert!(result.is_err(), "only a reviewer may certify");
    fx.cleanup();
}

#[tokio::test]
async fn a_disabled_engagement_authorizes_nothing() {
    let fx = Fixture::new("disabled").await;
    let engagement = fx
        .create_engagement(vec![fx.evidence_dir.display().to_string()])
        .await;
    fx.state
        .set_engagement_active(&fx.admin, &engagement.id, false)
        .await
        .unwrap();

    let result = fx
        .state
        .authorize_production_execution(
            &fx.admin,
            &engagement.id,
            &fx.evidence_dir.display().to_string(),
            "evidence.hash",
            &fx.module_sha,
        )
        .await;
    assert!(result.is_err(), "a disabled engagement must not authorize");
    fx.cleanup();
}

#[tokio::test]
async fn the_sandbox_refuses_a_file_outside_the_authorized_scope() {
    let fx = Fixture::new("escape").await;
    let engagement = fx
        .create_engagement(vec![fx.evidence_dir.display().to_string()])
        .await;
    let scope = fx
        .state
        .authorize_production_execution(
            &fx.admin,
            &engagement.id,
            &fx.evidence_dir.display().to_string(),
            "evidence.hash",
            &fx.module_sha,
        )
        .await
        .unwrap();

    // Authorization passed for the approved directory, but the module is asked
    // to hash a file outside it. The sandbox must still refuse.
    let outside = fx.dir.join("not-approved.bin");
    std::fs::write(&outside, b"secret").unwrap();
    let evidence_scope =
        crate::evidence::EvidenceScope::new(&scope.approved_evidence_paths).unwrap();
    let result = fx
        .executor
        .execute(
            "evidence.hash",
            serde_json::json!({ "path": outside.display().to_string() }),
            evidence_scope,
        )
        .await;
    assert!(
        result.is_err(),
        "the sandbox must enforce scope independently of the target check"
    );
    fx.cleanup();
}

#[tokio::test]
async fn denied_and_successful_executions_are_both_audited() {
    let fx = Fixture::new("audit").await;
    let engagement = fx
        .create_engagement(vec![fx.evidence_dir.display().to_string()])
        .await;

    // A denied attempt.
    let denied = fx
        .state
        .authorize_production_execution(
            &fx.admin,
            &engagement.id,
            "/etc",
            "evidence.hash",
            &fx.module_sha,
        )
        .await;
    assert!(denied.is_err());
    fx.state
        .audit_execution_event(
            &fx.admin,
            "EXECUTION_DENIED",
            "evidence.hash",
            "/etc",
            Err("out of scope"),
        )
        .await
        .unwrap();

    // An authorized attempt.
    fx.state
        .authorize_production_execution(
            &fx.admin,
            &engagement.id,
            &fx.evidence_dir.display().to_string(),
            "evidence.hash",
            &fx.module_sha,
        )
        .await
        .unwrap();
    fx.state
        .audit_execution_event(
            &fx.admin,
            "EXECUTION_AUTHORIZED",
            "evidence.hash",
            &fx.evidence_dir.display().to_string(),
            Ok(()),
        )
        .await
        .unwrap();

    let logs = fx.state.audit_logs(&fx.admin, 200).await.unwrap();
    let actions: Vec<&str> = logs.iter().map(|entry| entry.action.as_str()).collect();
    for expected in [
        "INITIAL_ADMIN_CREATED",
        "LICENSE_IMPORTED",
        "MODULE_CERTIFIED",
        "ENGAGEMENT_CREATED",
        "EXECUTION_DENIED",
        "EXECUTION_AUTHORIZED",
    ] {
        assert!(
            actions.contains(&expected),
            "audit trail must contain {} (found {:?})",
            expected,
            actions
        );
    }
    fx.cleanup();
}

#[tokio::test]
async fn an_expired_license_blocks_execution() {
    let dir = temp_dir("expired");
    let evidence_dir = dir.join("evidence");
    std::fs::create_dir_all(&evidence_dir).unwrap();
    std::fs::write(evidence_dir.join("case.bin"), b"abc").unwrap();

    let plugin_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../plugins");
    let mut loader = PluginLoader::new(&plugin_dir);
    loader.load_all().await.unwrap();

    let key = test_signing_key();
    let config = Config {
        db_path: dir.join("janus.db"),
        plugin_dir: plugin_dir.clone(),
        license_public_key: Some(STANDARD.encode(key.verifying_key().to_bytes())),
        ..Config::default()
    };
    let db = Arc::new(Database::open(&config.db_path).unwrap());
    let state = StateManager::new(&config, db).await.unwrap();
    let executor = PluginExecutor::new(&config, loader).await.unwrap();
    let (_org, admin) = state
        .bootstrap_first_admin(
            "Expired Org".to_string(),
            "expired@example.test".to_string(),
            "correct-horse-battery-staple".to_string(),
        )
        .await
        .unwrap();
    let reviewer = state
        .create_user(
            &admin,
            "rev@example.test".to_string(),
            "reviewer-passphrase-long".to_string(),
            UserRole::Reviewer,
        )
        .await
        .unwrap();

    // store_license rejects an already-expired document at import time, which is
    // itself the correct behaviour: an expired license never becomes active.
    let expired = signed_license(
        &key,
        &admin.organization_id,
        vec![LicensedFeature::Forensics],
        true,
    );
    let import = state.import_license(&admin, expired).await;
    assert!(import.is_err(), "an expired license must not import");

    let sha = executor.plugin_sha256("evidence.hash").unwrap();
    state
        .certify_module(
            &reviewer,
            "evidence.hash".to_string(),
            sha.clone(),
            CertificationStatus::ProductionApproved,
            LicensedFeature::Forensics,
            None,
        )
        .await
        .unwrap();
    let now = chrono::Utc::now();
    let engagement = state
        .create_engagement(
            &admin,
            "AUTH-EXPIRED".to_string(),
            now - chrono::Duration::hours(1),
            now + chrono::Duration::hours(1),
            EngagementScope {
                approved_assets: vec![],
                approved_evidence_paths: vec![evidence_dir.display().to_string()],
                approved_features: vec![LicensedFeature::Forensics],
            },
            true,
        )
        .await
        .unwrap();

    let result = state
        .authorize_production_execution(
            &admin,
            &engagement.id,
            &evidence_dir.display().to_string(),
            "evidence.hash",
            &sha,
        )
        .await;
    assert!(result.is_err(), "no valid license means no execution");
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn a_license_for_another_organization_cannot_be_imported() {
    let fx = Fixture::new("crossorg").await;
    let key = test_signing_key();
    let foreign = signed_license(&key, "org_someone_else", vec![LicensedFeature::Forensics], false);
    let result = fx.state.import_license(&fx.admin, foreign).await;
    assert!(result.is_err(), "a foreign license must be refused");
    fx.cleanup();
}

#[tokio::test]
async fn all_shipped_read_only_modules_execute_and_stay_read_only() {
    let fx = Fixture::new("family").await;
    let before = crate::evidence::hash_file(&fx.evidence_file).unwrap().0;
    let scope = crate::evidence::EvidenceScope::new(&[fx.evidence_dir.display().to_string()]).unwrap();

    for (module, args, operation) in [
        ("evidence.hash", serde_json::json!({"path": fx.evidence_file.display().to_string()}), "sha256"),
        ("evidence.stat", serde_json::json!({"path": fx.evidence_file.display().to_string()}), "stat"),
        ("evidence.list", serde_json::json!({"path": fx.evidence_dir.display().to_string()}), "list"),
        ("evidence.read", serde_json::json!({"path": fx.evidence_file.display().to_string()}), "read"),
        ("evidence.manifest", serde_json::json!({"path": fx.evidence_dir.display().to_string()}), "manifest"),
    ] {
        let result = fx
            .executor
            .execute(module, args, scope.clone())
            .await
            .unwrap_or_else(|error| panic!("{} must execute: {}", module, error));
        assert_eq!(result["result"]["status"], "success", "{} status", module);
        assert_eq!(result["result"]["operation"], operation, "{} operation", module);
        assert_eq!(result["result"]["read_only"], true, "{} read_only", module);
    }

    let after = crate::evidence::hash_file(&fx.evidence_file).unwrap().0;
    assert_eq!(before, after, "no module may modify evidence");
    let entries = std::fs::read_dir(&fx.evidence_dir).unwrap().count();
    assert_eq!(entries, 1, "no module may create files in the evidence root");
    fx.cleanup();
}

#[tokio::test]
async fn an_engagement_from_another_organization_is_not_visible_to_the_gate() {
    let fx = Fixture::new("tenant").await;
    // An engagement belonging to a different organization, inserted directly so
    // the test does not depend on a second bootstrap being allowed.
    let now = chrono::Utc::now();
    let foreign = janus_core::Engagement {
        id: "eng_foreign".to_string(),
        organization_id: "org_someone_else".to_string(),
        authorization_reference: "AUTH-FOREIGN".to_string(),
        starts_at: now - chrono::Duration::hours(1),
        ends_at: now + chrono::Duration::hours(1),
        active: true,
        scope: EngagementScope {
            approved_assets: vec![],
            approved_evidence_paths: vec![fx.evidence_dir.display().to_string()],
            approved_features: vec![LicensedFeature::Forensics],
        },
    };
    fx.db.create_engagement(&foreign).unwrap();

    let result = fx
        .state
        .authorize_production_execution(
            &fx.admin,
            "eng_foreign",
            &fx.evidence_dir.display().to_string(),
            "evidence.hash",
            &fx.module_sha,
        )
        .await;
    assert!(
        result.is_err(),
        "another organization's engagement must never authorize execution"
    );
    fx.cleanup();
}

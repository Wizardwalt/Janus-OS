//! Read-only import contract for user-approved OmniVault conversation snapshots.
//!
//! This module consumes a normalized snapshot supplied by an authorized host.
//! It has no API for changing, deleting, or writing to OmniVault.

use super::{AriaIngestionStore, ArtifactManifest, TextArtifactInput};
use anyhow::{anyhow, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashSet;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OmniVaultMessage {
    pub role: String,
    pub content: String,
    pub timestamp: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OmniVaultConversationSnapshot {
    pub id: String,
    pub title: String,
    pub provider: String,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
    pub messages: Vec<OmniVaultMessage>,
    /// Normalized source object exposed by OmniVault's conversation reader.
    /// This is not a replacement for the original provider archive.
    pub source_json: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OmniVaultReadApproval {
    pub owner_id: String,
    pub approved_conversation_ids: HashSet<String>,
    pub approved_at: DateTime<Utc>,
}

/// Import one snapshot only when the host supplies an explicit owner-scoped
/// approval. The source JSON snapshot is preserved unchanged in Aria's store.
pub fn import_approved_snapshot(
    store: &mut AriaIngestionStore,
    approval: &OmniVaultReadApproval,
    snapshot: &OmniVaultConversationSnapshot,
    imported_at: DateTime<Utc>,
    max_chars: usize,
    overlap_chars: usize,
) -> Result<ArtifactManifest> {
    if approval.owner_id.trim().is_empty() {
        return Err(anyhow!("OmniVault approval requires an authenticated owner ID"));
    }
    if !approval.approved_conversation_ids.contains(&snapshot.id) {
        return Err(anyhow!("conversation was not included in the user's OmniVault approval"));
    }
    if approval.approved_at > imported_at {
        return Err(anyhow!("OmniVault approval timestamp cannot be later than import time"));
    }
    if snapshot.id.trim().is_empty() || snapshot.title.trim().is_empty() {
        return Err(anyhow!("OmniVault conversation ID and title must not be empty"));
    }
    if snapshot.messages.is_empty() || snapshot.messages.iter().all(|m|m.content.trim().is_empty()) {
        return Err(anyhow!("OmniVault snapshot contains no readable messages"));
    }
    let parsed: serde_json::Value = serde_json::from_str(&snapshot.source_json)
        .map_err(|error|anyhow!("OmniVault source snapshot is not valid JSON: {error}"))?;
    if !parsed.is_object() {
        return Err(anyhow!("OmniVault source snapshot must be a JSON object"));
    }
    let source_id = parsed.get("id").and_then(serde_json::Value::as_str)
        .or_else(|| parsed.get("evaluation").and_then(|value|value.get("id"))
            .and_then(serde_json::Value::as_str));
    if source_id != Some(snapshot.id.as_str()) {
        return Err(anyhow!("OmniVault source snapshot ID does not match the approved conversation"));
    }
    let digest = format!("{:x}",Sha256::digest(snapshot.source_json.as_bytes()));
    let artifact_id = format!(
        "omnivault:{}:{}:{}",
        approval.owner_id, snapshot.id, &digest[..16]
    );
    let input = TextArtifactInput {
        artifact_id,
        title: format!("{} ({})",snapshot.title,snapshot.provider),
        media_type: "application/json; charset=utf-8".into(),
        source_type: "omnivault_readonly_normalized_snapshot".into(),
        handling: "private".into(),
        project_ids: vec![format!("omnivault-owner:{}",approval.owner_id)],
        imported_at,
        expires_at: None,
    };
    store.import_text(&input,snapshot.source_json.as_bytes(),max_chars,overlap_chars)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::aria::IngestionAccessScope;

    fn snapshot()->OmniVaultConversationSnapshot {
        OmniVaultConversationSnapshot {
            id:"conv-123".into(),title:"Electronics repair".into(),provider:"LM Arena".into(),
            created_at:Some("2026-11-01T10:00:00Z".into()),
            updated_at:Some("2026-11-01T10:05:00Z".into()),
            messages:vec![OmniVaultMessage {role:"user".into(),
                content:"How do I test a diode?".into(),timestamp:None}],
            source_json:r#"{"id":"conv-123","messages":[{"role":"user","content":"How do I test a diode?"}]}"#.into(),
        }
    }

    #[test]
    fn refuses_snapshots_without_explicit_conversation_approval() {
        let mut store=AriaIngestionStore::in_memory().unwrap();
        let approval=OmniVaultReadApproval {
            owner_id:"owner-1".into(),approved_conversation_ids:HashSet::new(),approved_at:Utc::now()
        };
        assert!(import_approved_snapshot(&mut store,&approval,&snapshot(),Utc::now(),100,10).is_err());
    }

    #[test]
    fn imports_only_approved_snapshot_as_private_read_only_copy() {
        let mut store=AriaIngestionStore::in_memory().unwrap();
        let item=snapshot();
        let approval=OmniVaultReadApproval {
            owner_id:"owner-1".into(),
            approved_conversation_ids:vec!["conv-123".to_owned()].into_iter().collect(),
            approved_at:Utc::now(),
        };
        let manifest=import_approved_snapshot(&mut store,&approval,&item,Utc::now(),100,10).unwrap();
        assert_eq!(manifest.handling,"private");
        assert_eq!(manifest.source_type,"omnivault_readonly_normalized_snapshot");
        let denied=IngestionAccessScope::public_internal();
        assert!(store.original_bytes(&manifest.artifact_id,&denied).unwrap().is_none());
        let allowed=IngestionAccessScope::host_authorized(
            &["private"],&["omnivault-owner:owner-1"],false
        );
        assert_eq!(
            store.original_bytes(&manifest.artifact_id,&allowed).unwrap().unwrap(),
            item.source_json.as_bytes()
        );
    }
    #[test]
    fn accepts_lm_arena_evaluation_wrapped_source_json() {
        let mut store=AriaIngestionStore::in_memory().unwrap();
        let mut item=snapshot();
        item.source_json=r#"{"evaluation":{"id":"conv-123","messages":[{"role":"user","content":"How do I test a diode?"}]}}"#.into();
        let approval=OmniVaultReadApproval {
            owner_id:"owner-1".into(),
            approved_conversation_ids:vec!["conv-123".to_owned()].into_iter().collect(),
            approved_at:Utc::now(),
        };
        assert!(import_approved_snapshot(&mut store,&approval,&item,Utc::now(),100,10).is_ok());
    }


}

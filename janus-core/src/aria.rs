//! Aria's provider-independent Multicolored Hat knowledge contracts.
//!
//! Knowledge classification is not authorization. A record can describe a
//! capability without granting permission to execute it.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

pub mod store;
pub use store::KnowledgeStore;

/// Stable domain identifiers shared with docs/aria/MULTICOLORED_HAT_REGISTRY.json.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Hat {
    Red,
    Orange,
    Yellow,
    Green,
    Blue,
    Purple,
    Black,
    White,
    Brown,
    Rainbow,
}

/// Epistemic state of a knowledge item; classification never implies truth.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EpistemicStatus {
    Verified,
    Reviewed,
    Unverified,
    Disputed,
    Superseded,
}

/// Broad content shape stored independently of any model provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KnowledgeContentType {
    Fact,
    Concept,
    Procedure,
    Reference,
    Observation,
    Question,
    Correction,
    Summary,
}

impl Default for KnowledgeContentType {
    fn default() -> Self {
        Self::Reference
    }
}

/// Source class for provenance. Model output is explicitly not equivalent to
/// an authoritative publication or direct measurement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KnowledgeSourceType {
    User,
    File,
    OfficialDocumentation,
    Standard,
    ResearchPublication,
    WebPage,
    Measurement,
    ModelGenerated,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KnowledgeSource {
    pub source_type: KnowledgeSourceType,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub publisher: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uri: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
    pub retrieved_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quoted_locator: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
}

/// Portable knowledge item. Original files remain in their host's
/// preservation layer; this record stores references and derived knowledge.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KnowledgeRecord {
    pub id: String,
    pub schema_version: String,
    pub title: String,
    pub content: String,
    #[serde(default)]
    pub content_type: KnowledgeContentType,
    pub hat_ids: Vec<Hat>,
    #[serde(default)]
    pub tags: Vec<String>,
    pub epistemic_status: EpistemicStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence_method: Option<String>,
    #[serde(default)]
    pub sources: Vec<KnowledgeSource>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    #[serde(default = "default_version")]
    pub version: u32,
    #[serde(default)]
    pub supersedes: Vec<String>,
    #[serde(default = "default_handling")]
    pub handling: String,
    #[serde(default)]
    pub project_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review_notes: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<DateTime<Utc>>,
}

fn default_version() -> u32 { 1 }
fn default_handling() -> String { "internal".to_owned() }

impl KnowledgeRecord {
    /// Validate core invariants shared by every Aria host.
    pub fn validate(&self) -> Result<(), String> {
        if self.id.trim().is_empty() { return Err("id must not be empty".into()); }
        if self.title.trim().is_empty() { return Err("title must not be empty".into()); }
        if self.content.trim().is_empty() { return Err("content must not be empty".into()); }
        if self.schema_version != "1.0.0" {
            return Err(format!("unsupported schema_version: {}", self.schema_version));
        }
        if self.hat_ids.is_empty() { return Err("at least one hat_id is required".into()); }
        let unique_hats: HashSet<_> = self.hat_ids.iter().collect();
        if unique_hats.len() != self.hat_ids.len() {
            return Err("hat_ids must not contain duplicates".into());
        }
        if self.version == 0 { return Err("version must be at least 1".into()); }
        if self.updated_at < self.created_at {
            return Err("updated_at must not precede created_at".into());
        }
        if let Some(confidence) = self.confidence {
            if !confidence.is_finite() || !(0.0..=1.0).contains(&confidence) {
                return Err("confidence must be a finite number between 0 and 1".into());
            }
            if self.confidence_method.as_deref().unwrap_or("").trim().is_empty() {
                return Err("confidence_method is required when confidence is supplied".into());
            }
        }
        if self.epistemic_status == EpistemicStatus::Verified && self.sources.is_empty() {
            return Err("verified records require at least one source".into());
        }
        if self.epistemic_status == EpistemicStatus::Disputed
            && self.review_notes.as_deref().unwrap_or("").trim().is_empty()
        {
            return Err("disputed records require review_notes".into());
        }
        for source in &self.sources {
            if let Some(digest) = &source.sha256 {
                if digest.len() != 64 || !digest.chars().all(|c| c.is_ascii_hexdigit()) {
                    return Err("source sha256 must contain exactly 64 hexadecimal characters".into());
                }
            }
        }
        const HANDLING: &[&str] = &["public", "internal", "private", "sensitive"];
        if !HANDLING.contains(&self.handling.as_str()) {
            return Err(format!("unsupported handling classification: {}", self.handling));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(status: EpistemicStatus) -> KnowledgeRecord {
        let now = Utc::now();
        KnowledgeRecord {
            id: "knowledge:test:1".into(),
            schema_version: "1.0.0".into(),
            title: "A sourced example".into(),
            content: "A knowledge item for validation.".into(),
            content_type: KnowledgeContentType::Fact,
            hat_ids: vec![Hat::White, Hat::Green],
            tags: vec!["test".into()],
            epistemic_status: status,
            confidence: None,
            confidence_method: None,
            sources: vec![],
            created_at: now,
            updated_at: now,
            version: 1,
            supersedes: vec![],
            handling: "internal".into(),
            project_ids: vec!["janus".into()],
            review_notes: None,
            expires_at: None,
        }
    }

    #[test]
    fn accepts_cross_domain_unverified_record_without_fabricated_source() {
        assert!(record(EpistemicStatus::Unverified).validate().is_ok());
    }

    #[test]
    fn verified_record_requires_provenance() {
        let error = record(EpistemicStatus::Verified).validate().unwrap_err();
        assert!(error.contains("require at least one source"));
    }

    #[test]
    fn disputed_record_requires_review_notes() {
        let error = record(EpistemicStatus::Disputed).validate().unwrap_err();
        assert!(error.contains("review_notes"));
    }

    #[test]
    fn rejects_duplicate_hat_classification() {
        let mut item = record(EpistemicStatus::Unverified);
        item.hat_ids = vec![Hat::Red, Hat::Red];
        assert!(item.validate().unwrap_err().contains("duplicates"));
    }

    #[test]
    fn rejects_invalid_confidence_and_digest() {
        let mut item = record(EpistemicStatus::Unverified);
        item.confidence = Some(f32::NAN);
        item.confidence_method = Some("test".into());
        assert!(item.validate().unwrap_err().contains("finite"));
    }
}

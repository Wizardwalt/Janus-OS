//! Human-reviewed learning proposals for Aria.
//!
//! A proposal is never knowledge merely because a model produced it. The
//! queue preserves the proposed record and source locator until a human
//! reviewer accepts or rejects it. Acceptance produces a Reviewed record;
//! verification remains a separate evidence decision.

use super::{EpistemicStatus, Hat, KnowledgeContentType, KnowledgeRecord, KnowledgeSource, KnowledgeSourceType, SourcePassage};
use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SourceLocator {
    pub artifact_id: String,
    pub sha256: String,
    pub start_byte: usize,
    pub end_byte: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LearningProposal {
    pub proposal_id: String,
    pub record: KnowledgeRecord,
    pub source: SourceLocator,
    pub proposed_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReviewDecision {
    Accepted,
    Rejected,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReviewReceipt {
    pub proposal_id: String,
    pub decision: ReviewDecision,
    pub reviewer_id: String,
    pub reviewed_at: DateTime<Utc>,
    pub notes: String,
}

impl LearningProposal {
    /// Build an unverified proposal directly from a retrieved source passage.
    /// The passage's byte range and digest are carried into provenance unchanged.
    pub fn from_passage(
        proposal_id: impl Into<String>,
        title: impl Into<String>,
        passage: &SourcePassage,
        source_type: KnowledgeSourceType,
        hat_ids: Vec<Hat>,
        tags: Vec<String>,
        handling: impl Into<String>,
        project_ids: Vec<String>,
        proposed_at: DateTime<Utc>,
    ) -> Self {
        let proposal_id = proposal_id.into();
        let title = title.into();
        let locator = SourceLocator {
            artifact_id: passage.artifact_id.clone(),
            sha256: passage.sha256.clone(),
            start_byte: passage.start_byte,
            end_byte: passage.end_byte,
        };
        let record = KnowledgeRecord {
            id: format!("aria:proposal:{proposal_id}"),
            schema_version: "1.0.0".into(),
            title,
            content: passage.text.clone(),
            content_type: KnowledgeContentType::Reference,
            hat_ids,
            tags,
            epistemic_status: EpistemicStatus::Unverified,
            confidence: None,
            confidence_method: None,
            sources: vec![KnowledgeSource {
                source_type,
                title: Some(passage.title.clone()),
                publisher: None,
                uri: None,
                artifact_id: Some(passage.artifact_id.clone()),
                sha256: Some(passage.sha256.clone()),
                retrieved_at: proposed_at,
                quoted_locator: Some(format!("bytes {}..{}",passage.start_byte,passage.end_byte)),
                notes: Some(format!("source chunk ordinal {}",passage.ordinal)),
            }],
            created_at: proposed_at,
            updated_at: proposed_at,
            version: 1,
            supersedes: Vec::new(),
            handling: handling.into(),
            project_ids,
            review_notes: None,
            expires_at: None,
        };
        Self { proposal_id, record, source: locator, proposed_at }
    }
}

pub struct LearningQueue {
    connection: Connection,
}

impl LearningQueue {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        Self::from_connection(Connection::open(path)?)
    }

    pub fn in_memory() -> Result<Self> {
        Self::from_connection(Connection::open_in_memory()?)
    }

    fn from_connection(connection: Connection) -> Result<Self> {
        connection.busy_timeout(std::time::Duration::from_secs(5))?;
        connection.execute_batch(
            "CREATE TABLE IF NOT EXISTS aria_learning_proposals (
                proposal_id TEXT PRIMARY KEY NOT NULL,
                proposal_json TEXT NOT NULL,
                proposed_at TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS aria_learning_reviews (
                proposal_id TEXT PRIMARY KEY NOT NULL REFERENCES aria_learning_proposals(proposal_id),
                decision TEXT NOT NULL CHECK(decision IN ('accepted','rejected')),
                reviewer_id TEXT NOT NULL,
                reviewed_at TEXT NOT NULL,
                notes TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS aria_learning_outbox (
                proposal_id TEXT PRIMARY KEY NOT NULL REFERENCES aria_learning_proposals(proposal_id),
                record_json TEXT NOT NULL,
                persisted_at TEXT
            );
            CREATE INDEX IF NOT EXISTS idx_aria_learning_proposals_time
                ON aria_learning_proposals(proposed_at DESC);"
        ).context("initializing Aria learning review queue")?;
        Ok(Self { connection })
    }

    /// Queue a source-linked proposal. Verified claims cannot enter via this
    /// path; a reviewer must first accept it as Reviewed.
    pub fn propose(&mut self, proposal: &LearningProposal) -> Result<()> {
        if proposal.proposal_id.trim().is_empty() || proposal.proposal_id.len() > 160 {
            return Err(anyhow!("proposal_id must be 1..=160 bytes"));
        }
        if proposal.source.artifact_id.trim().is_empty() {
            return Err(anyhow!("source artifact_id must not be empty"));
        }
        if proposal.source.sha256.len() != 64
            || !proposal.source.sha256.chars().all(|c| c.is_ascii_hexdigit())
        {
            return Err(anyhow!("source SHA-256 must contain 64 hexadecimal characters"));
        }
        if proposal.source.start_byte >= proposal.source.end_byte {
            return Err(anyhow!("source locator must have a non-empty byte range"));
        }
        if proposal.record.epistemic_status == EpistemicStatus::Verified {
            return Err(anyhow!("learning proposals cannot be pre-marked verified"));
        }
        proposal.record.validate().map_err(|e| anyhow!("invalid proposed record: {e}"))?;
        let linked = proposal.record.sources.iter().any(|source| {
            source.artifact_id.as_deref() == Some(proposal.source.artifact_id.as_str())
                && source.sha256.as_deref().is_some_and(|v| v.eq_ignore_ascii_case(&proposal.source.sha256))
        });
        if !linked {
            return Err(anyhow!("proposed knowledge record must cite the exact source artifact and SHA-256"));
        }
        let json = serde_json::to_string(proposal)?;
        self.connection.execute(
            "INSERT INTO aria_learning_proposals(proposal_id,proposal_json,proposed_at)
             VALUES (?1,?2,?3)",
            params![proposal.proposal_id,json,proposal.proposed_at.to_rfc3339()],
        ).with_context(|| format!("storing learning proposal {}", proposal.proposal_id))?;
        Ok(())
    }

    pub fn pending(&self, limit: usize) -> Result<Vec<LearningProposal>> {
        if limit == 0 { return Ok(Vec::new()); }
        let mut statement = self.connection.prepare(
            "SELECT p.proposal_json FROM aria_learning_proposals p
             LEFT JOIN aria_learning_reviews r ON r.proposal_id=p.proposal_id
             WHERE r.proposal_id IS NULL ORDER BY p.proposed_at ASC,p.proposal_id ASC LIMIT ?1"
        )?;
        let rows = statement.query_map([i64::try_from(limit)?], |row| row.get::<_,String>(0))?;
        rows.map(|row| Ok(serde_json::from_str(&row?)?)).collect()
    }

    /// Record one immutable human decision. The returned record is Reviewed,
    /// never Verified; callers persist it in KnowledgeStore after authorization.
    pub fn review(&mut self, receipt: &ReviewReceipt) -> Result<Option<KnowledgeRecord>> {
        if receipt.reviewer_id.trim().is_empty() {
            return Err(anyhow!("reviewer_id must not be empty"));
        }
        let decision = match receipt.decision {
            ReviewDecision::Accepted => "accepted",
            ReviewDecision::Rejected => "rejected",
        };
        let tx = self.connection.transaction()?;
        let json: Option<String> = tx.query_row(
            "SELECT proposal_json FROM aria_learning_proposals WHERE proposal_id=?1",
            [&receipt.proposal_id], |row| row.get(0)
        ).optional()?;
        let Some(json) = json else { return Err(anyhow!("learning proposal not found")); };
        let proposal: LearningProposal = serde_json::from_str(&json)?;
        tx.execute(
            "INSERT INTO aria_learning_reviews(proposal_id,decision,reviewer_id,reviewed_at,notes)
             VALUES (?1,?2,?3,?4,?5)",
            params![receipt.proposal_id,decision,receipt.reviewer_id,
                receipt.reviewed_at.to_rfc3339(),receipt.notes],
        ).context("recording immutable learning review")?;
        if receipt.decision == ReviewDecision::Rejected {
            tx.commit()?;
            return Ok(None);
        }
        let mut record = proposal.record;
        record.epistemic_status = EpistemicStatus::Reviewed;
        record.review_notes = Some(format!("Accepted by {}: {}",receipt.reviewer_id,receipt.notes));
        record.updated_at = receipt.reviewed_at.max(record.created_at);
        let record_json = serde_json::to_string(&record)?;
        tx.execute(
            "INSERT INTO aria_learning_outbox(proposal_id,record_json,persisted_at)
             VALUES (?1,?2,NULL)",
            params![receipt.proposal_id,record_json],
        ).context("queuing accepted knowledge for durable persistence")?;
        tx.commit()?;
        Ok(Some(record))
    }

    /// Persist accepted records from the durable outbox into KnowledgeStore.
    ///
    /// The stores may use separate SQLite databases, so this is a recoverable
    /// at-least-once hand-off rather than a cross-database transaction. If the
    /// process crashes after knowledge insertion but before marking the outbox
    /// row complete, an identical existing record is recognized on retry.
    pub fn persist_pending(&mut self, knowledge: &mut super::KnowledgeStore, limit: usize) -> Result<usize> {
        if limit == 0 { return Ok(0); }
        let rows: Vec<(String,String)> = {
            let mut statement = self.connection.prepare(
                "SELECT proposal_id,record_json FROM aria_learning_outbox
                 WHERE persisted_at IS NULL ORDER BY proposal_id LIMIT ?1"
            )?;
            let mapped = statement.query_map([i64::try_from(limit)?], |row| {
                Ok((row.get::<_,String>(0)?,row.get::<_,String>(1)?))
            })?;
            mapped.collect::<std::result::Result<Vec<_>,_>>()?
        };
        let mut completed=0usize;
        for (proposal_id,json) in rows {
            let record: KnowledgeRecord = serde_json::from_str(&json)
                .context("decoding accepted learning outbox record")?;
            let handling=[record.handling.as_str()];
            let projects:Vec<&str>=record.project_ids.iter().map(String::as_str).collect();
            let scope=super::KnowledgeAccessScope::host_authorized(&handling,&projects,false);
            match knowledge.get_with_scope(&record.id,true,&scope)? {
                Some(existing) if existing == record => {},
                Some(_) => return Err(anyhow!("knowledge record {} exists with different content",record.id)),
                None => knowledge.insert(&record)
                    .with_context(||format!("persisting accepted learning proposal {proposal_id}"))?,
            }
            self.connection.execute(
                "UPDATE aria_learning_outbox SET persisted_at=?1
                 WHERE proposal_id=?2 AND persisted_at IS NULL",
                params![Utc::now().to_rfc3339(),proposal_id],
            ).context("marking accepted learning record persisted")?;
            completed+=1;
        }
        Ok(completed)
    }

    pub fn pending_persistence(&self, limit: usize) -> Result<usize> {
        let count:i64=self.connection.query_row(
            "SELECT COUNT(*) FROM aria_learning_outbox WHERE persisted_at IS NULL",
            [],|row|row.get(0)
        )?;
        Ok(usize::try_from(count)?.min(limit))
    }

    pub fn receipt(&self, proposal_id: &str) -> Result<Option<ReviewReceipt>> {
        let row: Option<(String,String,String,String)> = self.connection.query_row(
            "SELECT decision,reviewer_id,reviewed_at,notes FROM aria_learning_reviews
             WHERE proposal_id=?1", [proposal_id],
            |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))
        ).optional()?;
        row.map(|(decision,reviewer_id,reviewed_at,notes)| {
            let decision = match decision.as_str() {
                "accepted" => ReviewDecision::Accepted,
                "rejected" => ReviewDecision::Rejected,
                _ => return Err(anyhow!("invalid stored review decision")),
            };
            Ok(ReviewReceipt {
                proposal_id:proposal_id.to_owned(),decision,reviewer_id,
                reviewed_at:DateTime::parse_from_rfc3339(&reviewed_at)?.with_timezone(&Utc),
                notes,
            })
        }).transpose()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::aria::{Hat,KnowledgeContentType,KnowledgeSource,KnowledgeSourceType};
    use chrono::Utc;

    fn proposal(id:&str)->LearningProposal {
        let now=Utc::now();
        let source=SourceLocator {artifact_id:"manual-1".into(),sha256:"a".repeat(64),
            start_byte:0,end_byte:24};
        let record=KnowledgeRecord {
            id:format!("knowledge:{id}"),schema_version:"1.0.0".into(),
            title:"Ohm's law".into(),content:"Voltage equals current times resistance.".into(),
            content_type:KnowledgeContentType::Concept,hat_ids:vec![Hat::Red,Hat::White],
            tags:vec!["electronics".into()],epistemic_status:EpistemicStatus::Unverified,
            confidence:None,confidence_method:None,
            sources:vec![KnowledgeSource {source_type:KnowledgeSourceType::File,
                title:Some("Manual".into()),publisher:None,uri:None,
                artifact_id:Some("manual-1".into()),sha256:Some("a".repeat(64)),
                retrieved_at:now,quoted_locator:Some("bytes 0..24".into()),notes:None}],
            created_at:now,updated_at:now,version:1,supersedes:vec![],
            handling:"internal".into(),project_ids:vec!["janus".into()],
            review_notes:None,expires_at:None,
        };
        LearningProposal {proposal_id:id.into(),record,source,proposed_at:now}
    }

    #[test]
    fn proposal_requires_exact_source_link_and_cannot_claim_verified() {
        let mut queue=LearningQueue::in_memory().unwrap();
        let mut item=proposal("p1");
        item.record.epistemic_status=EpistemicStatus::Verified;
        assert!(queue.propose(&item).is_err());
        item.record.epistemic_status=EpistemicStatus::Unverified;
        item.source.sha256="b".repeat(64);
        assert!(queue.propose(&item).is_err());
    }

    #[test]
    fn accepted_proposal_becomes_reviewed_and_has_immutable_receipt() {
        let mut queue=LearningQueue::in_memory().unwrap();
        let item=proposal("p1");
        queue.propose(&item).unwrap();
        assert_eq!(queue.pending(10).unwrap().len(),1);
        let receipt=ReviewReceipt {proposal_id:"p1".into(),decision:ReviewDecision::Accepted,
            reviewer_id:"owner-1".into(),reviewed_at:Utc::now(),notes:"Checked against manual.".into()};
        let record=queue.review(&receipt).unwrap().unwrap();
        assert_eq!(record.epistemic_status,EpistemicStatus::Reviewed);
        assert!(record.review_notes.unwrap().contains("owner-1"));
        assert_eq!(queue.pending(10).unwrap().len(),0);
        assert_eq!(queue.receipt("p1").unwrap(),Some(receipt.clone()));
        assert!(queue.review(&ReviewReceipt {proposal_id:"p1".into(),..receipt}).is_err());
    }

    #[test]
    fn rejected_proposal_never_returns_knowledge_record() {
        let mut queue=LearningQueue::in_memory().unwrap();
        queue.propose(&proposal("p2")).unwrap();
        let receipt=ReviewReceipt {proposal_id:"p2".into(),decision:ReviewDecision::Rejected,
            reviewer_id:"owner-1".into(),reviewed_at:Utc::now(),notes:"Contradicts source.".into()};
        assert!(queue.review(&receipt).unwrap().is_none());
        assert_eq!(queue.receipt("p2").unwrap().unwrap().decision,ReviewDecision::Rejected);
    }
    #[test]
    fn retrieved_passage_can_become_a_source_linked_learning_proposal() {
        let now=Utc::now();
        let passage=SourcePassage {
            artifact_id:"manual-1".into(),title:"Electronics Manual".into(),
            sha256:"c".repeat(64),start_byte:12,end_byte:48,ordinal:2,
            text:"Voltage equals current times resistance.".into(),
        };
        let proposal=LearningProposal::from_passage(
            "from-search","Ohm's law",&passage,KnowledgeSourceType::File,
            vec![Hat::Red,Hat::White],vec!["electronics".into()],
            "internal",vec!["janus".into()],now,
        );
        assert_eq!(proposal.record.epistemic_status,EpistemicStatus::Unverified);
        assert_eq!(proposal.source.start_byte,12);
        assert_eq!(proposal.record.sources[0].artifact_id.as_deref(),Some("manual-1"));
        assert_eq!(proposal.record.sources[0].sha256.as_deref(),Some("c".repeat(64).as_str()));
        let mut queue=LearningQueue::in_memory().unwrap();
        queue.propose(&proposal).unwrap();
        assert_eq!(queue.pending(5).unwrap().len(),1);
    }


}

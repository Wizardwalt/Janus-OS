//! Unified search across Aria knowledge, preserved source chunks, and
//! explicitly owner-scoped personal memories.
//!
//! All three stores retain their own access boundary. Personal memory is never
//! queried without an owner ID supplied by an authenticated host.

use super::{
    AriaIngestionStore, IngestionAccessScope, KnowledgeAccessScope, KnowledgeRecord,
    KnowledgeStore, MemoryStore, EncryptedMemoryStore, PersonalMemory, SourcePassage,
};
use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Read-only search contract shared by plaintext test stores and encrypted
/// production memory stores.
pub trait PersonalMemorySearch {
    fn search_memories(&self, owner_id:&str, query:&str, at:DateTime<Utc>, limit:usize)
        -> Result<Vec<PersonalMemory>>;
}

impl PersonalMemorySearch for MemoryStore {
    fn search_memories(&self,owner_id:&str,query:&str,at:DateTime<Utc>,limit:usize)
        -> Result<Vec<PersonalMemory>> {
        self.search(owner_id,query,at,limit)
    }
}

impl PersonalMemorySearch for EncryptedMemoryStore {
    fn search_memories(&self,owner_id:&str,query:&str,at:DateTime<Utc>,limit:usize)
        -> Result<Vec<PersonalMemory>> {
        self.search(owner_id,query,at,limit)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "data", rename_all = "snake_case")]
pub enum UnifiedSearchHit {
    Knowledge(KnowledgeRecord),
    Source(SourcePassage),
    Personal(PersonalMemory),
}

fn hit_text(hit: &UnifiedSearchHit) -> String {
    match hit {
        UnifiedSearchHit::Knowledge(record) => format!("{} {}",record.title,record.content),
        UnifiedSearchHit::Source(passage) => format!("{} {}",passage.title,passage.text),
        UnifiedSearchHit::Personal(memory) => memory.content.clone(),
    }
}

fn score(text: &str, query: &str) -> usize {
    let needle=query.trim().to_lowercase();
    if needle.is_empty() { return 0; }
    text.to_lowercase().match_indices(&needle).count()
}

/// Stable identity used to break relevance ties across all three stores.
fn hit_identity(hit: &UnifiedSearchHit) -> String {
    match hit {
        UnifiedSearchHit::Knowledge(record) => format!("knowledge:{}", record.id),
        UnifiedSearchHit::Source(passage) => {
            format!("source:{}:{:020}", passage.artifact_id, passage.ordinal)
        }
        UnifiedSearchHit::Personal(memory) => format!("personal:{}", memory.memory_id),
    }
}

/// Search all three stores and return a deterministic relevance-sorted result.
/// The knowledge and source scopes must be constructed by the authenticated
/// host. The owner ID must likewise come from authenticated session context.
pub fn search_all(
    knowledge: &KnowledgeStore,
    artifacts: &AriaIngestionStore,
    personal: Option<&dyn PersonalMemorySearch>,
    query: &str,
    owner_id: &str,
    knowledge_scope: &KnowledgeAccessScope,
    artifact_scope: &IngestionAccessScope,
    at: DateTime<Utc>,
    limit: usize,
) -> Result<Vec<UnifiedSearchHit>> {
    if limit==0 || query.trim().is_empty() { return Ok(Vec::new()); }
    let mut ranked:Vec<(usize,UnifiedSearchHit)>=Vec::new();
    for record in knowledge.search_with_scope(query,None,limit,knowledge_scope)? {
        let hit=UnifiedSearchHit::Knowledge(record);
        ranked.push((score(&hit_text(&hit),query),hit));
    }
    for passage in artifacts.search_chunks(query,artifact_scope,limit)? {
        let hit=UnifiedSearchHit::Source(passage);
        ranked.push((score(&hit_text(&hit),query),hit));
    }
    if let Some(personal)=personal {
        for memory in personal.search_memories(owner_id,query,at,limit)? {
            let hit=UnifiedSearchHit::Personal(memory);
            ranked.push((score(&hit_text(&hit),query),hit));
        }
    }
    ranked.sort_by(|a, b| {
        b.0.cmp(&a.0)
            .then_with(|| hit_identity(&a.1).cmp(&hit_identity(&b.1)))
    });
    Ok(ranked.into_iter().take(limit).map(|(_,hit)|hit).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::aria::{
        EncryptedMemoryStore, EpistemicStatus, Hat, KnowledgeContentType, PersonalMemory,
        TextArtifactInput,
    };

    #[test]
    fn unified_search_excludes_expired_memories_and_preserves_source_references() {
        let knowledge = KnowledgeStore::in_memory().unwrap();
        let artifacts = AriaIngestionStore::in_memory().unwrap();
        let mut personal = MemoryStore::in_memory().unwrap();
        let now = Utc::now();

        personal.remember(&PersonalMemory {
            memory_id: "m-active".into(),
            owner_id: "alice".into(),
            content: "User prefers offline model operation.".into(),
            source_refs: vec!["conversation:provider-preferences".into()],
            created_at: now,
            updated_at: now,
            expires_at: Some(now + chrono::Duration::hours(1)),
            version: 1,
        }).unwrap();
        personal.remember(&PersonalMemory {
            memory_id: "m-expired".into(),
            owner_id: "alice".into(),
            content: "Expired offline model preference.".into(),
            source_refs: vec!["conversation:old-preference".into()],
            created_at: now - chrono::Duration::days(2),
            updated_at: now - chrono::Duration::days(2),
            expires_at: Some(now - chrono::Duration::seconds(1)),
            version: 1,
        }).unwrap();

        let hits = search_all(
            &knowledge,
            &artifacts,
            Some(&personal),
            "offline model",
            "alice",
            &KnowledgeAccessScope::public_internal(),
            &IngestionAccessScope::public_internal(),
            now,
            10,
        ).unwrap();

        let memories: Vec<_> = hits.iter().filter_map(|hit| match hit {
            UnifiedSearchHit::Personal(memory) => Some(memory),
            _ => None,
        }).collect();
        assert_eq!(memories.len(), 1);
        assert_eq!(memories[0].memory_id, "m-active");
        assert_eq!(memories[0].source_refs, vec!["conversation:provider-preferences"]);
    }

    #[test]
    fn unified_search_orders_equal_score_personal_hits_deterministically() {
        let knowledge = KnowledgeStore::in_memory().unwrap();
        let artifacts = AriaIngestionStore::in_memory().unwrap();
        let mut personal = MemoryStore::in_memory().unwrap();
        let now = Utc::now();

        for memory_id in ["memory-b", "memory-a"] {
            personal.remember(&PersonalMemory {
                memory_id: memory_id.into(),
                owner_id: "alice".into(),
                content: "diode reference".into(),
                source_refs: vec![format!("conversation:{memory_id}")],
                created_at: now,
                updated_at: now,
                expires_at: None,
                version: 1,
            }).unwrap();
        }

        let hits = search_all(
            &knowledge,
            &artifacts,
            Some(&personal),
            "diode",
            "alice",
            &KnowledgeAccessScope::public_internal(),
            &IngestionAccessScope::public_internal(),
            now,
            10,
        ).unwrap();
        let ids: Vec<_> = hits.iter().filter_map(|hit| match hit {
            UnifiedSearchHit::Personal(memory) => Some(memory.memory_id.as_str()),
            _ => None,
        }).collect();

        assert_eq!(ids, vec!["memory-a", "memory-b"]);
        let refs: Vec<_> = hits.iter().filter_map(|hit| match hit {
            UnifiedSearchHit::Personal(memory) => memory.source_refs.first().map(String::as_str),
            _ => None,
        }).collect();
        assert_eq!(refs, vec!["conversation:memory-a", "conversation:memory-b"]);
    }

    #[test]
    fn unified_search_combines_sources_and_keeps_personal_owner_boundary() {
        let mut knowledge=KnowledgeStore::in_memory().unwrap();
        let mut artifacts=AriaIngestionStore::in_memory().unwrap();
        let mut personal=MemoryStore::in_memory().unwrap();
        let now=Utc::now();

        knowledge.insert(&KnowledgeRecord {
            id:"k-diode".into(),schema_version:"1.0.0".into(),
            title:"Diode reference".into(),content:"A diode conducts current.".into(),
            content_type:KnowledgeContentType::Concept,hat_ids:vec![Hat::Red,Hat::White],
            tags:vec!["electronics".into()],epistemic_status:EpistemicStatus::Unverified,
            confidence:None,confidence_method:None,sources:vec![],created_at:now,
            updated_at:now,version:1,supersedes:vec![],handling:"internal".into(),
            project_ids:vec![],review_notes:None,expires_at:None,
        }).unwrap();

        artifacts.import_text(&TextArtifactInput {
            artifact_id:"manual-diode".into(),title:"Diode manual".into(),
            media_type:"text/plain".into(),source_type:"file".into(),
            handling:"internal".into(),project_ids:vec![],imported_at:now,expires_at:None,
        },b"Diode forward voltage test procedure.",100,0).unwrap();

        personal.remember(&PersonalMemory {
            memory_id:"m-diode".into(),owner_id:"alice".into(),
            content:"User is learning diode testing.".into(),
            source_refs:vec!["conversation:diode".into()],
            created_at:now,updated_at:now,expires_at:None,version:1,
        }).unwrap();
        personal.remember(&PersonalMemory {
            memory_id:"m-private".into(),owner_id:"bob".into(),
            content:"Private diode detail for another owner.".into(),
            source_refs:vec![],created_at:now,updated_at:now,expires_at:None,version:1,
        }).unwrap();

        let hits=search_all(
            &knowledge,&artifacts,Some(&personal),"diode","alice",
            &KnowledgeAccessScope::public_internal(),
            &IngestionAccessScope::public_internal(),now,10,
        ).unwrap();
        assert!(hits.iter().any(|h|matches!(h,UnifiedSearchHit::Knowledge(_))));
        assert!(hits.iter().any(|h|matches!(h,UnifiedSearchHit::Source(_))));
        assert!(hits.iter().any(|h|matches!(h,UnifiedSearchHit::Personal(m) if m.owner_id=="alice")));
        assert!(!hits.iter().any(|h|matches!(h,UnifiedSearchHit::Personal(m) if m.owner_id=="bob")));
    }

    #[test]
    fn unified_search_uses_encrypted_owner_scoped_memory_and_preserves_evidence_refs() {
        let knowledge = KnowledgeStore::in_memory().unwrap();
        let artifacts = AriaIngestionStore::in_memory().unwrap();
        let key = [11u8; 32];
        let mut personal = EncryptedMemoryStore::in_memory(&key).unwrap();
        let now = Utc::now();

        personal.remember(&PersonalMemory {
            memory_id: "alice-bmx".into(),
            owner_id: "alice".into(),
            content: "BMX brake setup and cable adjustment.".into(),
            source_refs: vec!["omnivault:owner-1:conversation-42".into()],
            created_at: now,
            updated_at: now,
            expires_at: None,
            version: 1,
        }).unwrap();
        personal.remember(&PersonalMemory {
            memory_id: "bob-bmx".into(),
            owner_id: "bob".into(),
            content: "BMX brake setup from another owner's archive.".into(),
            source_refs: vec!["omnivault:owner-2:conversation-99".into()],
            created_at: now,
            updated_at: now,
            expires_at: None,
            version: 1,
        }).unwrap();

        let hits = search_all(
            &knowledge,
            &artifacts,
            Some(&personal),
            "BMX brake",
            "alice",
            &KnowledgeAccessScope::public_internal(),
            &IngestionAccessScope::public_internal(),
            now,
            10,
        ).unwrap();
        let memories: Vec<_> = hits.iter().filter_map(|hit| match hit {
            UnifiedSearchHit::Personal(memory) => Some(memory),
            _ => None,
        }).collect();
        assert_eq!(memories.len(), 1);
        assert_eq!(memories[0].owner_id, "alice");
        assert_eq!(
            memories[0].source_refs,
            vec!["omnivault:owner-1:conversation-42"]
        );

        // Evidence identifiers are searchable without exposing another owner's memory.
        let evidence_hits = search_all(
            &knowledge,
            &artifacts,
            Some(&personal),
            "omnivault:owner-1:conversation-42",
            "alice",
            &KnowledgeAccessScope::public_internal(),
            &IngestionAccessScope::public_internal(),
            now,
            10,
        ).unwrap();
        assert!(evidence_hits.iter().any(|hit| matches!(
            hit,
            UnifiedSearchHit::Personal(memory)
                if memory.memory_id == "alice-bmx"
                    && memory.source_refs.contains(&"omnivault:owner-1:conversation-42".to_owned())
        )));
        assert!(!evidence_hits.iter().any(|hit| matches!(
            hit,
            UnifiedSearchHit::Personal(memory) if memory.owner_id == "bob"
        )));
    }

}

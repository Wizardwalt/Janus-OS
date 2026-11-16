//! Unified search across Aria knowledge, preserved source chunks, and
//! explicitly owner-scoped personal memories.
//!
//! All three stores retain their own access boundary. Personal memory is never
//! queried without an owner ID supplied by an authenticated host.

use super::{
    AriaIngestionStore, IngestionAccessScope, KnowledgeAccessScope, KnowledgeRecord,
    KnowledgeStore, MemoryStore, PersonalMemory, SourcePassage,
};
use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

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

/// Search all three stores and return a deterministic relevance-sorted result.
/// The knowledge and source scopes must be constructed by the authenticated
/// host. The owner ID must likewise come from authenticated session context.
pub fn search_all(
    knowledge: &KnowledgeStore,
    artifacts: &AriaIngestionStore,
    personal: &MemoryStore,
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
    for memory in personal.search(owner_id,query,at,limit)? {
        let hit=UnifiedSearchHit::Personal(memory);
        ranked.push((score(&hit_text(&hit),query),hit));
    }
    ranked.sort_by(|a,b|b.0.cmp(&a.0));
    Ok(ranked.into_iter().take(limit).map(|(_,hit)|hit).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::aria::{
        EpistemicStatus, Hat, KnowledgeContentType, TextArtifactInput,
    };

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
            &knowledge,&artifacts,&personal,"diode","alice",
            &KnowledgeAccessScope::public_internal(),
            &IngestionAccessScope::public_internal(),now,10,
        ).unwrap();
        assert!(hits.iter().any(|h|matches!(h,UnifiedSearchHit::Knowledge(_))));
        assert!(hits.iter().any(|h|matches!(h,UnifiedSearchHit::Source(_))));
        assert!(hits.iter().any(|h|matches!(h,UnifiedSearchHit::Personal(m) if m.owner_id=="alice")));
        assert!(!hits.iter().any(|h|matches!(h,UnifiedSearchHit::Personal(m) if m.owner_id=="bob")));
    }
}

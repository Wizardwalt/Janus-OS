//! Explicit, owner-scoped personal memory storage for Aria.
//!
//! The host must authenticate the caller and pass the verified owner ID. This
//! store enforces owner isolation but is not an authentication mechanism.
//! Forgetting removes the memory content and retains only a minimal audit event.

use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PersonalMemory {
    pub memory_id: String,
    pub owner_id: String,
    pub content: String,
    pub source_refs: Vec<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub expires_at: Option<DateTime<Utc>>,
    pub version: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MemoryAuditEvent {
    pub memory_id: String,
    pub owner_id: String,
    pub event_type: String,
    pub occurred_at: DateTime<Utc>,
    pub prior_content_sha256: Option<String>,
}

pub struct MemoryStore {
    connection: Connection,
}

impl MemoryStore {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        Self::from_connection(Connection::open(path)?)
    }

    pub fn in_memory() -> Result<Self> {
        Self::from_connection(Connection::open_in_memory()?)
    }

    fn from_connection(connection: Connection) -> Result<Self> {
        connection.busy_timeout(std::time::Duration::from_secs(5))?;
        connection.execute_batch(
            "CREATE TABLE IF NOT EXISTS aria_personal_memories (
                memory_id TEXT PRIMARY KEY NOT NULL,
                owner_id TEXT NOT NULL,
                content TEXT NOT NULL,
                source_refs_json TEXT NOT NULL,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                expires_at TEXT,
                version INTEGER NOT NULL CHECK(version > 0)
            );
            CREATE INDEX IF NOT EXISTS idx_aria_memories_owner_updated
                ON aria_personal_memories(owner_id,updated_at DESC);
            CREATE TABLE IF NOT EXISTS aria_memory_audit (
                event_id INTEGER PRIMARY KEY AUTOINCREMENT,
                memory_id TEXT NOT NULL,
                owner_id TEXT NOT NULL,
                event_type TEXT NOT NULL CHECK(event_type IN ('remembered','corrected','forgotten')),
                occurred_at TEXT NOT NULL,
                prior_content_sha256 TEXT
            );
            CREATE INDEX IF NOT EXISTS idx_aria_memory_audit_owner
                ON aria_memory_audit(owner_id,occurred_at DESC);"
        ).context("initializing Aria personal memory schema")?;
        Ok(Self { connection })
    }

    /// Explicitly save a memory. The host should call this only after a user
    /// requested remembering or an approved memory proposal was accepted.
    pub fn remember(&mut self, memory: &PersonalMemory) -> Result<()> {
        validate(memory)?;
        if memory.version != 1 { return Err(anyhow!("new memories must start at version 1")); }
        let refs = serde_json::to_string(&memory.source_refs)?;
        let tx = self.connection.transaction()?;
        tx.execute(
            "INSERT INTO aria_personal_memories
             (memory_id,owner_id,content,source_refs_json,created_at,updated_at,expires_at,version)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
            params![memory.memory_id,memory.owner_id,memory.content,refs,
                memory.created_at.to_rfc3339(),memory.updated_at.to_rfc3339(),
                memory.expires_at.map(|v|v.to_rfc3339()),memory.version],
        ).with_context(|| format!("saving personal memory {}",memory.memory_id))?;
        audit(&tx,&memory.memory_id,&memory.owner_id,"remembered",memory.updated_at,None)?;
        tx.commit()?;
        Ok(())
    }

    /// Correct a memory belonging to the authenticated owner. The old content
    /// is replaced; only its digest remains in the audit event.
    pub fn correct(
        &mut self, owner_id: &str, memory_id: &str, content: &str,
        source_refs: &[String], at: DateTime<Utc>,
    ) -> Result<PersonalMemory> {
        if owner_id.trim().is_empty() || content.trim().is_empty() {
            return Err(anyhow!("owner_id and corrected content must not be empty"));
        }
        let tx = self.connection.transaction()?;
        let row: Option<(String,String,String,String,Option<String>,i64)> = tx.query_row(
            "SELECT content,source_refs_json,created_at,updated_at,expires_at,version
             FROM aria_personal_memories WHERE memory_id=?1 AND owner_id=?2",
            params![memory_id,owner_id],
            |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?))
        ).optional()?;
        let Some((old_content,_,created,_,expiry,version))=row else {
            return Err(anyhow!("memory not found for this owner"));
        };
        let expires_at=parse_time(expiry)?;
        if expires_at.is_some_and(|v|v<=at) { return Err(anyhow!("expired memories cannot be corrected")); }
        let next_version=u32::try_from(version)?.checked_add(1).ok_or_else(||anyhow!("memory version overflow"))?;
        let refs=serde_json::to_string(source_refs)?;
        tx.execute(
            "UPDATE aria_personal_memories SET content=?1,source_refs_json=?2,updated_at=?3,version=?4
             WHERE memory_id=?5 AND owner_id=?6",
            params![content,refs,at.to_rfc3339(),next_version,memory_id,owner_id],
        )?;
        let digest=format!("{:x}",Sha256::digest(old_content.as_bytes()));
        audit(&tx,memory_id,owner_id,"corrected",at,Some(&digest))?;
        let result=PersonalMemory {
            memory_id:memory_id.to_owned(),owner_id:owner_id.to_owned(),content:content.to_owned(),
            source_refs:source_refs.to_vec(),created_at:DateTime::parse_from_rfc3339(&created)?.with_timezone(&Utc),
            updated_at:at,expires_at,version:next_version,
        };
        tx.commit()?;
        Ok(result)
    }

    /// Forget removes the content and source references. The audit log retains
    /// only owner, opaque memory ID, event time, and event type.
    pub fn forget(&mut self, owner_id: &str, memory_id: &str, at: DateTime<Utc>) -> Result<bool> {
        let tx=self.connection.transaction()?;
        let exists:bool=tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM aria_personal_memories WHERE memory_id=?1 AND owner_id=?2)",
            params![memory_id,owner_id],|r|r.get(0)
        )?;
        if !exists { return Ok(false); }
        tx.execute("DELETE FROM aria_personal_memories WHERE memory_id=?1 AND owner_id=?2",
            params![memory_id,owner_id])?;
        // Remove historical content digests too: low-entropy private content
        // can sometimes be guessed from a hash. Keep only event metadata.
        tx.execute(
            "UPDATE aria_memory_audit SET prior_content_sha256=NULL
             WHERE memory_id=?1 AND owner_id=?2",
            params![memory_id,owner_id],
        )?;
        audit(&tx,memory_id,owner_id,"forgotten",at,None)?;
        tx.commit()?;
        Ok(true)
    }

    pub fn get(&self, owner_id: &str, memory_id: &str, at: DateTime<Utc>) -> Result<Option<PersonalMemory>> {
        let row:Option<(String,String,String,String,Option<String>,i64)>=self.connection.query_row(
            "SELECT content,source_refs_json,created_at,updated_at,expires_at,version
             FROM aria_personal_memories WHERE memory_id=?1 AND owner_id=?2",
            params![memory_id,owner_id],
            |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?))
        ).optional()?;
        row_to_memory(memory_id,owner_id,row,at)
    }

    pub fn list(&self, owner_id: &str, at: DateTime<Utc>, limit: usize) -> Result<Vec<PersonalMemory>> {
        if limit==0 { return Ok(Vec::new()); }
        let mut stmt=self.connection.prepare(
            "SELECT memory_id,content,source_refs_json,created_at,updated_at,expires_at,version
             FROM aria_personal_memories WHERE owner_id=?1
             ORDER BY updated_at DESC,memory_id ASC LIMIT ?2"
        )?;
        let rows=stmt.query_map(params![owner_id,i64::try_from(limit)?],|r|Ok((
            r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,
            r.get::<_,String>(3)?,r.get::<_,String>(4)?,r.get::<_,Option<String>>(5)?,r.get::<_,i64>(6)?
        )))?;
        let mut result=Vec::new();
        for row in rows {
            let (id,content,refs,created,updated,expiry,version)=row?;
            if let Some(item)=row_to_memory(&id,owner_id,Some((content,refs,created,updated,expiry,version)),at)? {
                result.push(item);
            }
        }
        Ok(result)
    }


    /// Search only the authenticated owner's active memories.
    pub fn search(&self, owner_id: &str, query: &str, at: DateTime<Utc>, limit: usize) -> Result<Vec<PersonalMemory>> {
        if limit == 0 { return Ok(Vec::new()); }
        let needle=query.trim();
        let mut statement=self.connection.prepare(
            "SELECT memory_id,content,source_refs_json,created_at,updated_at,expires_at,version
             FROM aria_personal_memories
             WHERE owner_id=?1 AND (expires_at IS NULL OR expires_at>?2)
               AND (?3='' OR instr(lower(content),lower(?3))>0
                    OR instr(lower(source_refs_json),lower(?3))>0)
             ORDER BY updated_at DESC,memory_id ASC LIMIT ?4"
        )?;
        let rows=statement.query_map(
            params![owner_id,at.to_rfc3339(),needle,i64::try_from(limit)?],
            |r|Ok((
                r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,
                r.get::<_,String>(3)?,r.get::<_,String>(4)?,r.get::<_,Option<String>>(5)?,
                r.get::<_,i64>(6)?
            ))
        )?;
        let mut result=Vec::new();
        for row in rows {
            let (id,content,refs,created,updated,expiry,version)=row?;
            if let Some(memory)=row_to_memory(
                &id,owner_id,Some((content,refs,created,updated,expiry,version)),at
            )? { result.push(memory); }
        }
        Ok(result)
    }

    pub fn audit_events(&self, owner_id: &str, memory_id: &str) -> Result<Vec<MemoryAuditEvent>> {
        let mut stmt=self.connection.prepare(
            "SELECT event_type,occurred_at,prior_content_sha256 FROM aria_memory_audit
             WHERE owner_id=?1 AND memory_id=?2 ORDER BY event_id ASC"
        )?;
        let rows=stmt.query_map(params![owner_id,memory_id],|r|Ok((
            r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,Option<String>>(2)?
        )))?;
        rows.map(|row|{
            let (event_type,occurred_at,prior_content_sha256)=row?;
            Ok(MemoryAuditEvent {memory_id:memory_id.to_owned(),owner_id:owner_id.to_owned(),
                event_type,occurred_at:DateTime::parse_from_rfc3339(&occurred_at)?.with_timezone(&Utc),
                prior_content_sha256})
        }).collect()
    }
}

fn audit(tx:&rusqlite::Transaction<'_>,id:&str,owner:&str,event:&str,at:DateTime<Utc>,digest:Option<&str>)->Result<()> {
    tx.execute(
        "INSERT INTO aria_memory_audit(memory_id,owner_id,event_type,occurred_at,prior_content_sha256)
         VALUES (?1,?2,?3,?4,?5)",params![id,owner,event,at.to_rfc3339(),digest]
    )?;
    Ok(())
}

pub(super) fn validate(memory:&PersonalMemory)->Result<()> {
    if memory.memory_id.trim().is_empty() || memory.memory_id.len()>160 {return Err(anyhow!("memory_id must be 1..=160 bytes"));}
    if memory.owner_id.trim().is_empty() || memory.owner_id.len()>160 {return Err(anyhow!("owner_id must be 1..=160 bytes"));}
    if memory.content.trim().is_empty() {return Err(anyhow!("memory content must not be empty"));}
    if memory.updated_at<memory.created_at {return Err(anyhow!("updated_at must not precede created_at"));}
    if memory.expires_at.is_some_and(|v|v<=memory.created_at) {return Err(anyhow!("expiry must follow creation"));}
    if memory.source_refs.iter().any(|v|v.trim().is_empty()) {return Err(anyhow!("source references must not be empty"));}
    Ok(())
}

fn parse_time(value:Option<String>)->Result<Option<DateTime<Utc>>> {
    value.map(|v|DateTime::parse_from_rfc3339(&v).map(|d|d.with_timezone(&Utc))
        .context("parsing memory expiry")).transpose()
}

fn row_to_memory(
    id:&str,owner:&str,row:Option<(String,String,String,String,Option<String>,i64)>,at:DateTime<Utc>
)->Result<Option<PersonalMemory>> {
    let Some((content,refs,created,updated,expiry,version))=row else{return Ok(None)};
    let expires_at=parse_time(expiry)?;
    if expires_at.is_some_and(|v|v<=at) {return Ok(None);}
    Ok(Some(PersonalMemory {
        memory_id:id.to_owned(),owner_id:owner.to_owned(),content,
        source_refs:serde_json::from_str(&refs)?,
        created_at:DateTime::parse_from_rfc3339(&created)?.with_timezone(&Utc),
        updated_at:DateTime::parse_from_rfc3339(&updated)?.with_timezone(&Utc),
        expires_at,version:u32::try_from(version)?,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    fn memory(id:&str,owner:&str)->PersonalMemory {
        let now=Utc::now();
        PersonalMemory {memory_id:id.into(),owner_id:owner.into(),
            content:"User prefers concise explanations.".into(),source_refs:vec!["conversation:event-1".into()],
            created_at:now,updated_at:now,expires_at:None,version:1}
    }

    #[test]
    fn memories_are_isolated_by_owner() {
        let mut store=MemoryStore::in_memory().unwrap();
        store.remember(&memory("m1","alice")).unwrap();
        assert!(store.get("alice","m1",Utc::now()).unwrap().is_some());
        assert!(store.get("bob","m1",Utc::now()).unwrap().is_none());
        assert!(store.list("bob",Utc::now(),10).unwrap().is_empty());
    }

    #[test]
    fn correction_is_versioned_and_audited() {
        let mut store=MemoryStore::in_memory().unwrap();
        store.remember(&memory("m1","alice")).unwrap();
        let corrected=store.correct("alice","m1","User prefers detailed explanations.",
            &["conversation:event-2".into()],Utc::now()).unwrap();
        assert_eq!(corrected.version,2);
        assert!(corrected.content.contains("detailed"));
        let events=store.audit_events("alice","m1").unwrap();
        assert_eq!(events.iter().map(|e|e.event_type.as_str()).collect::<Vec<_>>(),
            vec!["remembered","corrected"]);
        assert!(events[1].prior_content_sha256.as_ref().is_some_and(|v|v.len()==64));
    }

    #[test]
    fn forgetting_removes_content_but_keeps_minimal_audit() {
        let mut store=MemoryStore::in_memory().unwrap();
        store.remember(&memory("m1","alice")).unwrap();
        store.correct("alice","m1","Corrected private detail",&[],Utc::now()).unwrap();
        assert!(store.forget("alice","m1",Utc::now()).unwrap());
        assert!(store.get("alice","m1",Utc::now()).unwrap().is_none());
        let events=store.audit_events("alice","m1").unwrap();
        assert_eq!(events.last().unwrap().event_type,"forgotten");
        assert!(events.iter().all(|e|e.prior_content_sha256.is_none()));
    }

    #[test]
    fn expired_memories_are_hidden_and_cannot_be_corrected() {
        let mut store=MemoryStore::in_memory().unwrap();
        let mut item=memory("m1","alice");
        item.expires_at=Some(item.created_at+Duration::seconds(1));
        store.remember(&item).unwrap();
        let later=item.created_at+Duration::seconds(2);
        assert!(store.get("alice","m1",later).unwrap().is_none());
        assert!(store.correct("alice","m1","new",&[],later).is_err());
    }
    #[test]
    fn search_finds_matching_memory_beyond_recent_unmatched_items() {
        let mut store=MemoryStore::in_memory().unwrap();
        for i in 0..8 {
            let mut item=memory(&format!("m{i}"),"alice");
            item.content=format!("Unrelated note number {i}");
            item.updated_at=item.created_at+chrono::Duration::seconds(i);
            store.remember(&item).unwrap();
        }
        let mut target=memory("target","alice");
        target.content="Important diode testing preference".into();
        target.updated_at=target.created_at+chrono::Duration::seconds(20);
        store.remember(&target).unwrap();
        let found=store.search("alice","diode",Utc::now(),1).unwrap();
        assert_eq!(found.len(),1);
        assert_eq!(found[0].memory_id,"target");
    }


}

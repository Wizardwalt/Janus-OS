//! Encrypted, owner-scoped personal memory storage.
//!
//! Content and source references are serialized together and protected with
//! XChaCha20-Poly1305. The 256-bit key is supplied by the host; this module
//! never derives it from an owner ID or stores it beside the database. Key
//! storage, access control, rotation, and backup protection remain host duties.

use super::memory::validate;
use super::{MemoryAuditEvent, PersonalMemory};
use anyhow::{anyhow, Context, Result};
use chacha20poly1305::{
    aead::{Aead, KeyInit},
    XChaCha20Poly1305, XNonce,
};
use chrono::{DateTime, Utc};
use rand_core::{OsRng, RngCore};
use rusqlite::{params, Connection, OptionalExtension};
use sha2::{Digest, Sha256};
use std::path::Path;

pub struct EncryptedMemoryStore {
    connection: Connection,
    key: [u8; 32],
}

impl EncryptedMemoryStore {
    /// Open an encrypted memory database using a key held by the host.
    pub fn open(path: impl AsRef<Path>, key: &[u8; 32]) -> Result<Self> {
        Self::from_connection(Connection::open(path)?, key)
    }

    /// Create an encrypted in-memory store, primarily for tests.
    pub fn in_memory(key: &[u8; 32]) -> Result<Self> {
        Self::from_connection(Connection::open_in_memory()?, key)
    }

    fn from_connection(connection: Connection, key: &[u8; 32]) -> Result<Self> {
        connection.busy_timeout(std::time::Duration::from_secs(5))?;
        connection.execute_batch(
            "CREATE TABLE IF NOT EXISTS aria_personal_memories_encrypted (
                memory_id TEXT PRIMARY KEY NOT NULL,
                owner_id TEXT NOT NULL,
                nonce BLOB NOT NULL,
                ciphertext BLOB NOT NULL,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                expires_at TEXT,
                version INTEGER NOT NULL CHECK(version > 0)
            );
            CREATE INDEX IF NOT EXISTS idx_aria_encrypted_memories_owner_updated
                ON aria_personal_memories_encrypted(owner_id,updated_at DESC);
            CREATE TABLE IF NOT EXISTS aria_encrypted_memory_audit (
                event_id INTEGER PRIMARY KEY AUTOINCREMENT,
                memory_id TEXT NOT NULL,
                owner_id TEXT NOT NULL,
                event_type TEXT NOT NULL CHECK(event_type IN ('remembered','corrected','forgotten')),
                occurred_at TEXT NOT NULL,
                prior_content_sha256 TEXT
            );
            CREATE INDEX IF NOT EXISTS idx_aria_encrypted_memory_audit_owner
                ON aria_encrypted_memory_audit(owner_id,occurred_at DESC);"
        ).context("initializing encrypted Aria memory schema")?;
        Ok(Self { connection, key:*key })
    }

    fn encrypt(&self, memory:&PersonalMemory)->Result<(Vec<u8>,Vec<u8>)> {
        let cipher=XChaCha20Poly1305::new_from_slice(&self.key)
            .map_err(|_|anyhow!("invalid encrypted memory key"))?;
        let mut nonce=[0u8;24];
        OsRng.fill_bytes(&mut nonce);
        let plaintext=serde_json::to_vec(memory).context("serializing personal memory")?;
        let ciphertext=cipher.encrypt(XNonce::from_slice(&nonce),plaintext.as_ref())
            .map_err(|_|anyhow!("encrypting personal memory failed"))?;
        Ok((nonce.to_vec(),ciphertext))
    }

    fn decrypt(&self, id:&str, owner:&str, nonce:&[u8], ciphertext:&[u8])->Result<PersonalMemory> {
        if nonce.len()!=24 { return Err(anyhow!("stored encrypted memory nonce has invalid length")); }
        let cipher=XChaCha20Poly1305::new_from_slice(&self.key)
            .map_err(|_|anyhow!("invalid encrypted memory key"))?;
        let plaintext=cipher.decrypt(XNonce::from_slice(nonce),ciphertext)
            .map_err(|_|anyhow!("encrypted memory authentication failed"))?;
        let memory:PersonalMemory=serde_json::from_slice(&plaintext)
            .context("decoding encrypted personal memory")?;
        if memory.memory_id!=id || memory.owner_id!=owner {
            return Err(anyhow!("encrypted memory identity does not match its database index"));
        }
        Ok(memory)
    }

    fn audit(
        tx:&rusqlite::Transaction<'_>,id:&str,owner:&str,event:&str,
        at:DateTime<Utc>,digest:Option<&str>
    )->Result<()> {
        tx.execute(
            "INSERT INTO aria_encrypted_memory_audit
             (memory_id,owner_id,event_type,occurred_at,prior_content_sha256)
             VALUES (?1,?2,?3,?4,?5)",
            params![id,owner,event,at.to_rfc3339(),digest],
        )?;
        Ok(())
    }

    pub fn remember(&mut self,memory:&PersonalMemory)->Result<()> {
        validate(memory)?;
        if memory.version!=1 { return Err(anyhow!("new memories must start at version 1")); }
        let (nonce,ciphertext)=self.encrypt(memory)?;
        let tx=self.connection.transaction()?;
        tx.execute(
            "INSERT INTO aria_personal_memories_encrypted
             (memory_id,owner_id,nonce,ciphertext,created_at,updated_at,expires_at,version)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
            params![memory.memory_id,memory.owner_id,nonce,ciphertext,
                memory.created_at.to_rfc3339(),memory.updated_at.to_rfc3339(),
                memory.expires_at.map(|v|v.to_rfc3339()),memory.version],
        )?;
        Self::audit(&tx,&memory.memory_id,&memory.owner_id,"remembered",memory.updated_at,None)?;
        tx.commit()?;
        Ok(())
    }

    pub fn get(&self,owner_id:&str,memory_id:&str,at:DateTime<Utc>)->Result<Option<PersonalMemory>> {
        let row:Option<(Vec<u8>,Vec<u8>)>=self.connection.query_row(
            "SELECT nonce,ciphertext FROM aria_personal_memories_encrypted
             WHERE memory_id=?1 AND owner_id=?2",
            params![memory_id,owner_id],|r|Ok((r.get(0)?,r.get(1)?))
        ).optional()?;
        let Some((nonce,ciphertext))=row else{return Ok(None)};
        let memory=self.decrypt(memory_id,owner_id,&nonce,&ciphertext)?;
        if memory.expires_at.is_some_and(|expiry|expiry<=at) {return Ok(None);}
        Ok(Some(memory))
    }

    pub fn list(&self,owner_id:&str,at:DateTime<Utc>,limit:usize)->Result<Vec<PersonalMemory>> {
        if limit==0 {return Ok(Vec::new());}
        let mut statement=self.connection.prepare(
            "SELECT memory_id,nonce,ciphertext FROM aria_personal_memories_encrypted
             WHERE owner_id=?1 ORDER BY updated_at DESC,memory_id ASC"
        )?;
        let rows=statement.query_map([owner_id],|r|Ok((
            r.get::<_,String>(0)?,r.get::<_,Vec<u8>>(1)?,r.get::<_,Vec<u8>>(2)?
        )))?;
        let mut result=Vec::new();
        for row in rows {
            let (id,nonce,ciphertext)=row?;
            let memory=self.decrypt(&id,owner_id,&nonce,&ciphertext)?;
            if memory.expires_at.is_some_and(|expiry|expiry<=at) {continue;}
            result.push(memory);
            if result.len()>=limit {break;}
        }
        Ok(result)
    }

    pub fn search(&self,owner_id:&str,query:&str,at:DateTime<Utc>,limit:usize)->Result<Vec<PersonalMemory>> {
        if limit==0 {return Ok(Vec::new());}
        let needle=query.to_lowercase();
        let mut statement=self.connection.prepare(
            "SELECT memory_id,nonce,ciphertext FROM aria_personal_memories_encrypted
             WHERE owner_id=?1 ORDER BY updated_at DESC,memory_id ASC"
        )?;
        let rows=statement.query_map([owner_id],|r|Ok((
            r.get::<_,String>(0)?,r.get::<_,Vec<u8>>(1)?,r.get::<_,Vec<u8>>(2)?
        )))?;
        let mut result=Vec::new();
        for row in rows {
            let (id,nonce,ciphertext)=row?;
            let memory=self.decrypt(&id,owner_id,&nonce,&ciphertext)?;
            if memory.expires_at.is_some_and(|expiry|expiry<=at) {continue;}
            let matched=needle.trim().is_empty()
                || memory.content.to_lowercase().contains(&needle)
                || memory.source_refs.iter().any(|source|source.to_lowercase().contains(&needle));
            if matched {
                result.push(memory);
                if result.len()>=limit {break;}
            }
        }
        Ok(result)
    }

    pub fn correct(
        &mut self,owner_id:&str,memory_id:&str,content:&str,
        source_refs:&[String],at:DateTime<Utc>
    )->Result<PersonalMemory> {
        if owner_id.trim().is_empty() || content.trim().is_empty() {
            return Err(anyhow!("owner_id and corrected content must not be empty"));
        }
        let mut memory=self.get(owner_id,memory_id,at)?
            .ok_or_else(||anyhow!("memory not found or expired for this owner"))?;
        let old_digest=format!("{:x}",Sha256::digest(memory.content.as_bytes()));
        memory.content=content.to_owned();
        memory.source_refs=source_refs.to_vec();
        memory.updated_at=at;
        memory.version=memory.version.checked_add(1)
            .ok_or_else(||anyhow!("memory version overflow"))?;
        validate(&memory)?;
        let (nonce,ciphertext)=self.encrypt(&memory)?;
        let tx=self.connection.transaction()?;
        let changed=tx.execute(
            "UPDATE aria_personal_memories_encrypted
             SET nonce=?1,ciphertext=?2,updated_at=?3,version=?4
             WHERE memory_id=?5 AND owner_id=?6 AND version=?7",
            params![nonce,ciphertext,at.to_rfc3339(),memory.version,
                memory_id,owner_id,memory.version-1],
        )?;
        if changed!=1 {return Err(anyhow!("memory changed concurrently; reload before correcting"));}
        Self::audit(&tx,memory_id,owner_id,"corrected",at,Some(&old_digest))?;
        tx.commit()?;
        Ok(memory)
    }

    pub fn forget(&mut self,owner_id:&str,memory_id:&str,at:DateTime<Utc>)->Result<bool> {
        let tx=self.connection.transaction()?;
        let changed=tx.execute(
            "DELETE FROM aria_personal_memories_encrypted WHERE memory_id=?1 AND owner_id=?2",
            params![memory_id,owner_id],
        )?;
        if changed==0 {return Ok(false);}
        tx.execute(
            "UPDATE aria_encrypted_memory_audit SET prior_content_sha256=NULL
             WHERE memory_id=?1 AND owner_id=?2",params![memory_id,owner_id]
        )?;
        Self::audit(&tx,memory_id,owner_id,"forgotten",at,None)?;
        tx.commit()?;
        Ok(true)
    }

    pub fn audit_events(&self,owner_id:&str,memory_id:&str)->Result<Vec<MemoryAuditEvent>> {
        let mut statement=self.connection.prepare(
            "SELECT event_type,occurred_at,prior_content_sha256
             FROM aria_encrypted_memory_audit WHERE owner_id=?1 AND memory_id=?2
             ORDER BY event_id ASC"
        )?;
        let rows=statement.query_map(params![owner_id,memory_id],|r|Ok((
            r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,Option<String>>(2)?
        )))?;
        rows.map(|row|{
            let (event_type,occurred_at,prior_content_sha256)=row?;
            Ok(MemoryAuditEvent {
                memory_id:memory_id.to_owned(),owner_id:owner_id.to_owned(),event_type,
                occurred_at:DateTime::parse_from_rfc3339(&occurred_at)?.with_timezone(&Utc),
                prior_content_sha256,
            })
        }).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    fn sample(id:&str,owner:&str)->PersonalMemory {
        let now=Utc::now();
        PersonalMemory {
            memory_id:id.into(),owner_id:owner.into(),
            content:"Private memory about bicycle repair.".into(),
            source_refs:vec!["conversation:private".into()],
            created_at:now,updated_at:now,expires_at:None,version:1,
        }
    }

    #[test]
    fn encrypted_memory_roundtrips_and_is_owner_scoped() {
        let key=[7u8;32];
        let mut store=EncryptedMemoryStore::in_memory(&key).unwrap();
        let item=sample("secret-1","alice");
        store.remember(&item).unwrap();
        assert_eq!(store.get("alice","secret-1",Utc::now()).unwrap(),Some(item));
        assert!(store.get("bob","secret-1",Utc::now()).unwrap().is_none());
        let stored:Vec<u8>=store.connection.query_row(
            "SELECT ciphertext FROM aria_personal_memories_encrypted WHERE memory_id='secret-1'",
            [],|row|row.get(0)
        ).unwrap();
        assert!(!stored.windows(b"Private memory".len()).any(|w|w==b"Private memory"));
    }

    #[test]
    fn encrypted_memory_rejects_wrong_key_and_supports_correction_and_forget() {
        let key=[9u8;32];
        let mut store=EncryptedMemoryStore::in_memory(&key).unwrap();
        store.remember(&sample("secret-2","alice")).unwrap();
        let (nonce,ciphertext):(Vec<u8>,Vec<u8>)=store.connection.query_row(
            "SELECT nonce,ciphertext FROM aria_personal_memories_encrypted WHERE memory_id='secret-2'",
            [],|row|Ok((row.get(0)?,row.get(1)?))
        ).unwrap();
        let wrong=EncryptedMemoryStore {
            connection:Connection::open_in_memory().unwrap(),key:[3u8;32]
        };
        assert!(wrong.decrypt("secret-2","alice",&nonce,&ciphertext).is_err());
        let corrected=store.correct("alice","secret-2","Updated private detail",&[],Utc::now()).unwrap();
        assert_eq!(corrected.version,2);
        assert_eq!(store.get("alice","secret-2",Utc::now()).unwrap().unwrap().content,"Updated private detail");
        assert!(store.forget("alice","secret-2",Utc::now()).unwrap());
        assert!(store.get("alice","secret-2",Utc::now()).unwrap().is_none());
        assert!(store.audit_events("alice","secret-2").unwrap().iter().all(|e|e.prior_content_sha256.is_none()));
    }

    #[test]
    fn encrypted_memory_expiry_is_enforced() {
        let key=[1u8;32];
        let mut store=EncryptedMemoryStore::in_memory(&key).unwrap();
        let mut item=sample("secret-3","alice");
        item.expires_at=Some(item.created_at+Duration::seconds(1));
        store.remember(&item).unwrap();
        assert!(store.get("alice","secret-3",item.created_at+Duration::seconds(2)).unwrap().is_none());
    }
    #[test]
    fn encrypted_memory_search_is_owner_scoped() {
        let key=[4u8;32];
        let mut store=EncryptedMemoryStore::in_memory(&key).unwrap();
        let mut alice=sample("alice-memory","alice");
        alice.content="Alice is repairing a BMX bicycle.".into();
        store.remember(&alice).unwrap();
        let mut bob=sample("bob-memory","bob");
        bob.content="Bob is repairing a bicycle.".into();
        store.remember(&bob).unwrap();
        let hits=store.search("alice","BMX",Utc::now(),10).unwrap();
        assert_eq!(hits.len(),1);
        assert_eq!(hits[0].owner_id,"alice");
        assert!(store.search("bob","BMX",Utc::now(),10).unwrap().is_empty());
    }


}

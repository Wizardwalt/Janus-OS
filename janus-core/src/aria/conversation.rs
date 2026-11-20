//! Durable, owner-scoped Aria conversation event storage.
//!
//! This store persists conversation history separately from long-term memory.
//! The host must authenticate the caller and supply the verified owner ID.

use anyhow::{anyhow, Context, Result};
use chacha20poly1305::{
    aead::{Aead, KeyInit, Payload},
    XChaCha20Poly1305, XNonce,
};
use chrono::{DateTime, Utc};
use rand_core::{OsRng, RngCore};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::Path;

const MAX_OWNER_ID_BYTES: usize = 160;
const MAX_CONVERSATION_ID_BYTES: usize = 160;
const MAX_EVENT_ID_BYTES: usize = 160;
const MAX_CONTENT_BYTES: usize = 64 * 1024;
const MAX_PROVIDER_FIELD_BYTES: usize = 128;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConversationEvent {
    pub event_id: String,
    pub role: String,
    pub content: String,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ConversationSummary {
    pub conversation_id: String,
    pub event_count: usize,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// SQLite-backed history. Owner IDs are supplied by the authenticated host;
/// callers must never accept an owner ID directly from an untrusted request.
pub struct ConversationStore {
    connection: Connection,
    key: [u8; 32],
}

impl ConversationStore {
    pub fn open(path: impl AsRef<Path>, key: &[u8; 32]) -> Result<Self> {
        let connection = Connection::open(path).context("opening Aria conversation database")?;
        Self::from_connection(connection, key)
    }

    pub fn in_memory(key: &[u8; 32]) -> Result<Self> {
        Self::from_connection(Connection::open_in_memory()?, key)
    }

    fn from_connection(mut connection: Connection, key: &[u8; 32]) -> Result<Self> {
        connection.pragma_update(None, "foreign_keys", "ON")?;
        connection.pragma_update(None, "secure_delete", "ON")?;
        connection.busy_timeout(std::time::Duration::from_secs(5))?;
        connection.execute_batch(
            "CREATE TABLE IF NOT EXISTS aria_conversations (
                owner_id TEXT NOT NULL,
                conversation_id TEXT NOT NULL,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                PRIMARY KEY(owner_id, conversation_id)
            );
            CREATE TABLE IF NOT EXISTS aria_conversation_events (
                event_id TEXT NOT NULL,
                owner_id TEXT NOT NULL,
                conversation_id TEXT NOT NULL,
                nonce BLOB NOT NULL,
                ciphertext BLOB NOT NULL,
                created_at TEXT NOT NULL,
                FOREIGN KEY(owner_id, conversation_id)
                    REFERENCES aria_conversations(owner_id, conversation_id)
                    ON DELETE CASCADE,
                PRIMARY KEY(owner_id, event_id)
            );
            CREATE INDEX IF NOT EXISTS idx_aria_conversation_events_owner_time
                ON aria_conversation_events(owner_id, conversation_id, created_at, event_id);
            CREATE INDEX IF NOT EXISTS idx_aria_conversations_owner_updated
                ON aria_conversations(owner_id, updated_at DESC);",
        )?;
        let conversation_key = derive_conversation_key(key);
        migrate_encrypted_event_schema(&mut connection, &conversation_key)?;
        Ok(Self { connection, key: conversation_key })
    }

    /// Append one event atomically, creating the conversation on first use.
    pub fn append_event(
        &mut self,
        owner_id: &str,
        conversation_id: &str,
        event: &ConversationEvent,
    ) -> Result<()> {
        validate_identifier(owner_id, MAX_OWNER_ID_BYTES, "owner_id")?;
        validate_identifier(conversation_id, MAX_CONVERSATION_ID_BYTES, "conversation_id")?;
        validate_identifier(&event.event_id, MAX_EVENT_ID_BYTES, "event_id")?;
        if !matches!(event.role.as_str(), "system" | "user" | "assistant" | "tool") {
            anyhow::bail!("role must be system, user, assistant, or tool");
        }
        if event.content.trim().is_empty() || event.content.len() > MAX_CONTENT_BYTES {
            anyhow::bail!("content must contain 1..=65536 bytes");
        }
        for (name, value) in [("provider", &event.provider), ("model", &event.model)] {
            if value.as_ref().is_some_and(|value| value.len() > MAX_PROVIDER_FIELD_BYTES) {
                anyhow::bail!("{name} must not exceed 128 bytes");
            }
        }

        let timestamp = event.created_at.to_rfc3339();
        let (nonce, ciphertext) = encrypt_event(
            &self.key, owner_id, conversation_id, &event.event_id, &timestamp, event,
        )?;
        let tx = self.connection.transaction()?;
        tx.execute(
            "INSERT INTO aria_conversations(owner_id, conversation_id, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?3)
             ON CONFLICT(owner_id, conversation_id) DO UPDATE SET
                 updated_at = MAX(aria_conversations.updated_at, excluded.updated_at)",
            params![owner_id, conversation_id, timestamp],
        )?;
        tx.execute(
            "INSERT INTO aria_conversation_events
             (event_id, owner_id, conversation_id, nonce, ciphertext, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![event.event_id, owner_id, conversation_id, nonce, ciphertext, timestamp],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// List conversations belonging to exactly this owner, newest activity first.
    pub fn list_conversations(&self, owner_id: &str, limit: usize) -> Result<Vec<ConversationSummary>> {
        validate_identifier(owner_id, MAX_OWNER_ID_BYTES, "owner_id")?;
        let limit = limit.clamp(1, 100);
        let mut statement = self.connection.prepare(
            "SELECT c.conversation_id, COUNT(e.event_id), c.created_at, c.updated_at
             FROM aria_conversations c
             LEFT JOIN aria_conversation_events e
               ON e.owner_id = c.owner_id AND e.conversation_id = c.conversation_id
             WHERE c.owner_id = ?1
             GROUP BY c.owner_id, c.conversation_id
             ORDER BY c.updated_at DESC, c.conversation_id ASC
             LIMIT ?2",
        )?;
        let rows = statement.query_map(params![owner_id, limit as i64], |row| {
            let created: String = row.get(2)?;
            let updated: String = row.get(3)?;
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?, created, updated))
        })?;
        rows.map(|row| {
            let (conversation_id, event_count, created, updated) = row?;
            Ok(ConversationSummary {
                conversation_id,
                event_count: event_count as usize,
                created_at: parse_timestamp(&created)?,
                updated_at: parse_timestamp(&updated)?,
            })
        }).collect()
    }

    /// Read events only when both the owner and conversation match.
    pub fn list_events(
        &self,
        owner_id: &str,
        conversation_id: &str,
        limit: usize,
    ) -> Result<Vec<ConversationEvent>> {
        validate_identifier(owner_id, MAX_OWNER_ID_BYTES, "owner_id")?;
        validate_identifier(conversation_id, MAX_CONVERSATION_ID_BYTES, "conversation_id")?;
        let limit = limit.clamp(1, 1_000);
        let mut statement = self.connection.prepare(
            "SELECT event_id, nonce, ciphertext, created_at
             FROM aria_conversation_events
             WHERE owner_id = ?1 AND conversation_id = ?2
             ORDER BY created_at ASC, event_id ASC LIMIT ?3",
        )?;
        let rows = statement.query_map(params![owner_id, conversation_id, limit as i64], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Vec<u8>>(1)?,
                row.get::<_, Vec<u8>>(2)?,
                row.get::<_, String>(3)?,
            ))
        })?;
        let encrypted = rows.collect::<rusqlite::Result<Vec<_>>>()?;
        encrypted.into_iter().map(|(event_id, nonce, ciphertext, created_at)| {
            decrypt_event(
                &self.key, owner_id, conversation_id, &event_id, &created_at, &nonce, &ciphertext,
            )
        }).collect()
    }

    /// Fetch one event only within the authenticated owner's conversation.
    pub fn get_event(
        &self,
        owner_id: &str,
        conversation_id: &str,
        event_id: &str,
    ) -> Result<Option<ConversationEvent>> {
        validate_identifier(owner_id, MAX_OWNER_ID_BYTES, "owner_id")?;
        validate_identifier(conversation_id, MAX_CONVERSATION_ID_BYTES, "conversation_id")?;
        validate_identifier(event_id, MAX_EVENT_ID_BYTES, "event_id")?;
        let encrypted = self.connection.query_row(
            "SELECT nonce, ciphertext, created_at
             FROM aria_conversation_events
             WHERE owner_id = ?1 AND conversation_id = ?2 AND event_id = ?3",
            params![owner_id, conversation_id, event_id],
            |row| Ok((
                row.get::<_, Vec<u8>>(0)?,
                row.get::<_, Vec<u8>>(1)?,
                row.get::<_, String>(2)?,
            )),
        ).optional()?;
        encrypted.map(|(nonce, ciphertext, created_at)| {
            decrypt_event(
                &self.key, owner_id, conversation_id, event_id, &created_at, &nonce, &ciphertext,
            )
        }).transpose()
    }

    /// Read the next chronological event page after an owner-scoped event cursor.
    /// The cursor event must belong to the same owner and conversation.
    pub fn list_events_after(
        &self,
        owner_id: &str,
        conversation_id: &str,
        after_event_id: Option<&str>,
        limit: usize,
    ) -> Result<Vec<ConversationEvent>> {
        validate_identifier(owner_id, MAX_OWNER_ID_BYTES, "owner_id")?;
        validate_identifier(conversation_id, MAX_CONVERSATION_ID_BYTES, "conversation_id")?;
        if let Some(cursor) = after_event_id {
            validate_identifier(cursor, MAX_EVENT_ID_BYTES, "after_event_id")?;
        }
        let limit = limit.clamp(1, 1_000);
        let mut statement = self.connection.prepare(
            "SELECT event_id, nonce, ciphertext, created_at
             FROM aria_conversation_events
             WHERE owner_id = ?1 AND conversation_id = ?2
               AND (?3 IS NULL OR (created_at, event_id) > (
                    SELECT created_at, event_id FROM aria_conversation_events
                    WHERE owner_id = ?1 AND conversation_id = ?2 AND event_id = ?3
               ))
             ORDER BY created_at ASC, event_id ASC LIMIT ?4",
        )?;
        let rows = statement.query_map(
            params![owner_id, conversation_id, after_event_id, limit as i64],
            |row| Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Vec<u8>>(1)?,
                row.get::<_, Vec<u8>>(2)?,
                row.get::<_, String>(3)?,
            )),
        )?;
        let encrypted = rows.collect::<rusqlite::Result<Vec<_>>>()?;
        if after_event_id.is_some() && encrypted.is_empty() {
            let cursor_exists: bool = self.connection.query_row(
                "SELECT EXISTS(SELECT 1 FROM aria_conversation_events
                 WHERE owner_id = ?1 AND conversation_id = ?2 AND event_id = ?3)",
                params![owner_id, conversation_id, after_event_id],
                |row| row.get(0),
            )?;
            if !cursor_exists {
                anyhow::bail!("conversation cursor was not found for this owner and conversation");
            }
        }
        encrypted.into_iter().map(|(event_id, nonce, ciphertext, created_at)| {
            decrypt_event(
                &self.key, owner_id, conversation_id, &event_id, &created_at, &nonce, &ciphertext,
            )
        }).collect()
    }

    /// Return the newest events for provider context, in chronological order.
    pub fn list_recent_events(
        &self,
        owner_id: &str,
        conversation_id: &str,
        limit: usize,
    ) -> Result<Vec<ConversationEvent>> {
        validate_identifier(owner_id, MAX_OWNER_ID_BYTES, "owner_id")?;
        validate_identifier(conversation_id, MAX_CONVERSATION_ID_BYTES, "conversation_id")?;
        let limit = limit.clamp(1, 100);
        let mut statement = self.connection.prepare(
            "SELECT event_id, nonce, ciphertext, created_at
             FROM aria_conversation_events
             WHERE owner_id = ?1 AND conversation_id = ?2
             ORDER BY created_at DESC, event_id DESC LIMIT ?3",
        )?;
        let rows = statement.query_map(params![owner_id, conversation_id, limit as i64], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Vec<u8>>(1)?,
                row.get::<_, Vec<u8>>(2)?,
                row.get::<_, String>(3)?,
            ))
        })?;
        let encrypted = rows.collect::<rusqlite::Result<Vec<_>>>()?;
        let mut events = encrypted
            .into_iter()
            .map(|(event_id, nonce, ciphertext, created_at)| {
                decrypt_event(
                    &self.key, owner_id, conversation_id, &event_id, &created_at, &nonce, &ciphertext,
                )
            })
            .collect::<Result<Vec<_>>>()?;
        events.reverse();
        Ok(events)
    }

    /// Delete a conversation only within the supplied owner namespace.
    pub fn delete_conversation(&mut self, owner_id: &str, conversation_id: &str) -> Result<bool> {
        validate_identifier(owner_id, MAX_OWNER_ID_BYTES, "owner_id")?;
        validate_identifier(conversation_id, MAX_CONVERSATION_ID_BYTES, "conversation_id")?;
        let deleted = self.connection.execute(
            "DELETE FROM aria_conversations WHERE owner_id = ?1 AND conversation_id = ?2",
            params![owner_id, conversation_id],
        )?;
        Ok(deleted == 1)
    }
}

/// Derive a purpose-specific key so conversation encryption does not reuse the
/// raw key used by Aria's separate personal-memory store.
fn derive_conversation_key(master_key: &[u8; 32]) -> [u8; 32] {
    let mut digest = Sha256::new();
    digest.update(b"janus-aria-conversation-events-v1\\0");
    digest.update(master_key);
    digest.finalize().into()
}

/// Encrypt legacy plaintext conversation rows and scope event IDs to their owner.
/// The migration is transactional: existing history is retained or the old table remains.
fn migrate_encrypted_event_schema(connection: &mut Connection, key: &[u8; 32]) -> Result<()> {
    let mut statement = connection.prepare("PRAGMA table_info(aria_conversation_events)")?;
    let rows = statement.query_map([], |row| row.get::<_, String>(1))?;
    let columns = rows.collect::<rusqlite::Result<Vec<_>>>()?;
    drop(statement);

    if columns.iter().any(|column| column == "content") {
        let mut statement = connection.prepare(
            "SELECT event_id, owner_id, conversation_id, role, content, provider, model, created_at
             FROM aria_conversation_events ORDER BY owner_id, created_at, event_id",
        )?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, Option<String>>(5)?,
                row.get::<_, Option<String>>(6)?,
                row.get::<_, String>(7)?,
            ))
        })?;
        let legacy = rows.collect::<rusqlite::Result<Vec<_>>>()?;
        drop(statement);

        let mut encrypted_rows = Vec::with_capacity(legacy.len());
        for (event_id, owner_id, conversation_id, role, content, provider, model, created_at) in legacy {
            let event = ConversationEvent {
                event_id: event_id.clone(),
                role,
                content,
                provider,
                model,
                created_at: parse_timestamp(&created_at)?,
            };
            let (nonce, ciphertext) = encrypt_event(
                key, &owner_id, &conversation_id, &event_id, &created_at, &event,
            )?;
            encrypted_rows.push((event_id, owner_id, conversation_id, nonce, ciphertext, created_at));
        }

        let tx = connection.transaction()?;
        tx.execute_batch(
            "DROP INDEX IF EXISTS idx_aria_conversation_events_owner_time;
             ALTER TABLE aria_conversation_events RENAME TO aria_conversation_events_legacy;
             CREATE TABLE aria_conversation_events (
                event_id TEXT NOT NULL,
                owner_id TEXT NOT NULL,
                conversation_id TEXT NOT NULL,
                nonce BLOB NOT NULL,
                ciphertext BLOB NOT NULL,
                created_at TEXT NOT NULL,
                FOREIGN KEY(owner_id, conversation_id)
                    REFERENCES aria_conversations(owner_id, conversation_id)
                    ON DELETE CASCADE,
                PRIMARY KEY(owner_id, event_id)
             );",
        )?;
        for (event_id, owner_id, conversation_id, nonce, ciphertext, created_at) in encrypted_rows {
            tx.execute(
                "INSERT INTO aria_conversation_events
                 (event_id, owner_id, conversation_id, nonce, ciphertext, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![event_id, owner_id, conversation_id, nonce, ciphertext, created_at],
            )?;
        }
        tx.execute_batch(
            "DROP TABLE aria_conversation_events_legacy;
             CREATE INDEX idx_aria_conversation_events_owner_time
                ON aria_conversation_events(owner_id, conversation_id, created_at, event_id);",
        )?;
        tx.commit()?;
    } else {
        let mut statement = connection.prepare("PRAGMA table_info(aria_conversation_events)")?;
        let rows = statement.query_map([], |row| {
            Ok((row.get::<_, String>(1)?, row.get::<_, i64>(5)?))
        })?;
        let mut primary_key_columns = Vec::new();
        for row in rows {
            let (name, position) = row?;
            if position > 0 {
                primary_key_columns.push((position, name));
            }
        }
        primary_key_columns.sort_by_key(|(position, _)| *position);
        let primary_key_columns = primary_key_columns
            .into_iter()
            .map(|(_, name)| name)
            .collect::<Vec<_>>();
        drop(statement);
        if primary_key_columns.len() != 2
            || primary_key_columns[0] != "owner_id"
            || primary_key_columns[1] != "event_id"
            || !columns.iter().any(|column| column == "nonce")
            || !columns.iter().any(|column| column == "ciphertext")
        {
            anyhow::bail!("unsupported aria_conversation_events schema");
        }
    }
    Ok(())
}

fn event_aad(
    owner_id: &str,
    conversation_id: &str,
    event_id: &str,
    created_at: &str,
) -> Result<Vec<u8>> {
    Ok(serde_json::to_vec(&(owner_id, conversation_id, event_id, created_at))?)
}

fn encrypt_event(
    key: &[u8; 32],
    owner_id: &str,
    conversation_id: &str,
    event_id: &str,
    created_at: &str,
    event: &ConversationEvent,
) -> Result<(Vec<u8>, Vec<u8>)> {
    let cipher = XChaCha20Poly1305::new_from_slice(key)
        .map_err(|_| anyhow!("invalid Aria conversation encryption key"))?;
    let mut nonce = [0u8; 24];
    OsRng.fill_bytes(&mut nonce);
    let plaintext = serde_json::to_vec(event).context("serializing Aria conversation event")?;
    let aad = event_aad(owner_id, conversation_id, event_id, created_at)?;
    let ciphertext = cipher
        .encrypt(XNonce::from_slice(&nonce), Payload { msg: &plaintext, aad: &aad })
        .map_err(|_| anyhow!("encrypting Aria conversation event failed"))?;
    Ok((nonce.to_vec(), ciphertext))
}

fn decrypt_event(
    key: &[u8; 32],
    owner_id: &str,
    conversation_id: &str,
    event_id: &str,
    created_at: &str,
    nonce: &[u8],
    ciphertext: &[u8],
) -> Result<ConversationEvent> {
    if nonce.len() != 24 {
        anyhow::bail!("stored Aria conversation nonce has invalid length");
    }
    let cipher = XChaCha20Poly1305::new_from_slice(key)
        .map_err(|_| anyhow!("invalid Aria conversation encryption key"))?;
    let aad = event_aad(owner_id, conversation_id, event_id, created_at)?;
    let plaintext = cipher
        .decrypt(XNonce::from_slice(nonce), Payload { msg: ciphertext, aad: &aad })
        .map_err(|_| anyhow!("Aria conversation authentication failed"))?;
    let event: ConversationEvent = serde_json::from_slice(&plaintext)
        .context("decoding encrypted Aria conversation event")?;
    if event.event_id != event_id || event.created_at.to_rfc3339() != created_at {
        anyhow::bail!("encrypted Aria conversation metadata does not match its database index");
    }
    Ok(event)
}

fn validate_identifier(value: &str, max_bytes: usize, name: &str) -> Result<()> {
    if value.trim().is_empty() || value.len() > max_bytes {
        return Err(anyhow!("{name} must contain 1..={max_bytes} bytes"));
    }
    Ok(())
}

fn parse_timestamp(value: &str) -> Result<DateTime<Utc>> {
    Ok(DateTime::parse_from_rfc3339(value)
        .with_context(|| format!("invalid stored timestamp: {value}"))?
        .with_timezone(&Utc))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    fn test_key() -> [u8; 32] {
        [0x42; 32]
    }

    fn event(id: &str, role: &str, content: &str, at: DateTime<Utc>) -> ConversationEvent {
        ConversationEvent {
            event_id: id.into(),
            role: role.into(),
            content: content.into(),
            provider: Some("local-openai-compatible".into()),
            model: Some("test-model".into()),
            created_at: at,
        }
    }

    #[test]
    fn conversation_events_are_durable_and_owner_scoped() {
        let mut store = ConversationStore::in_memory(&test_key()).unwrap();
        let now = Utc::now();
        store.append_event("alice", "chat-1", &event("e1", "user", "hello", now)).unwrap();
        store.append_event("alice", "chat-1", &event("e2", "assistant", "hi", now + Duration::seconds(1))).unwrap();
        store.append_event("bob", "chat-1", &event("e3", "user", "private to bob", now)).unwrap();

        assert_eq!(store.list_events("alice", "chat-1", 20).unwrap().len(), 2);
        assert_eq!(store.list_events("bob", "chat-1", 20).unwrap().len(), 1);
        assert!(store.list_events("mallory", "chat-1", 20).unwrap().is_empty());
        assert_eq!(store.list_conversations("alice", 20).unwrap()[0].event_count, 2);
    }

    #[test]
    fn get_event_is_scoped_to_owner_and_conversation() {
        let mut store = ConversationStore::in_memory(&test_key()).unwrap();
        let now = Utc::now();
        store.append_event("alice", "chat-a", &event("shared", "user", "alice", now)).unwrap();
        store.append_event("bob", "chat-b", &event("shared", "user", "bob", now)).unwrap();

        assert_eq!(store.get_event("alice", "chat-a", "shared").unwrap().unwrap().content, "alice");
        assert_eq!(store.get_event("bob", "chat-b", "shared").unwrap().unwrap().content, "bob");
        assert!(store.get_event("alice", "chat-b", "shared").unwrap().is_none());
    }

    #[test]
    fn recent_events_returns_newest_events_in_chronological_order() {
        let mut store = ConversationStore::in_memory(&test_key()).unwrap();
        let now = Utc::now();
        for index in 0..3 {
            store.append_event(
                "alice",
                "chat-a",
                &event(&format!("event-{index}"), "user", &format!("message-{index}"), now + chrono::Duration::seconds(index)),
            ).unwrap();
        }

        let recent = store.list_recent_events("alice", "chat-a", 2).unwrap();
        assert_eq!(recent.iter().map(|event| event.content.as_str()).collect::<Vec<_>>(), vec!["message-1", "message-2"]);
    }

    #[test]
    fn event_cursor_pages_are_stable_and_owner_scoped() {
        let mut store = ConversationStore::in_memory(&test_key()).unwrap();
        let now = Utc::now();
        for index in 0..5 {
            store.append_event(
                "alice",
                "paged-chat",
                &event(&format!("event-{index}"), "user", &format!("message-{index}"),
                    now + chrono::Duration::seconds(index)),
            ).unwrap();
        }

        let first = store.list_events_after("alice", "paged-chat", None, 2).unwrap();
        assert_eq!(first.iter().map(|item| item.event_id.as_str()).collect::<Vec<_>>(),
            vec!["event-0", "event-1"]);
        let second = store.list_events_after("alice", "paged-chat",
            Some(&first.last().unwrap().event_id), 2).unwrap();
        assert_eq!(second.iter().map(|item| item.event_id.as_str()).collect::<Vec<_>>(),
            vec!["event-2", "event-3"]);
        let third = store.list_events_after("alice", "paged-chat",
            Some(&second.last().unwrap().event_id), 2).unwrap();
        assert_eq!(third.iter().map(|item| item.event_id.as_str()).collect::<Vec<_>>(),
            vec!["event-4"]);
        assert!(store.list_events_after("bob", "paged-chat",
            Some("event-1"), 2).is_err());
        assert!(store.list_events_after("alice", "other-chat",
            Some("event-1"), 2).is_err());
    }

    #[test]
    fn event_identifiers_are_scoped_to_their_owner() {
        let mut store = ConversationStore::in_memory(&test_key()).unwrap();
        let now = Utc::now();
        store.append_event("alice", "chat-a", &event("shared-event", "user", "alice", now)).unwrap();
        store.append_event("bob", "chat-b", &event("shared-event", "user", "bob", now)).unwrap();

        assert_eq!(store.list_events("alice", "chat-a", 20).unwrap()[0].content, "alice");
        assert_eq!(store.list_events("bob", "chat-b", 20).unwrap()[0].content, "bob");
    }

    #[test]
    fn migrates_legacy_global_event_ids_without_losing_history() {
        use std::time::{SystemTime, UNIX_EPOCH};

        let unique = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "janus-aria-conversation-migration-{}-{unique}", std::process::id()
        ));
        std::fs::create_dir_all(&directory).unwrap();
        let database_path = directory.join("conversations.sqlite");
        {
            let connection = Connection::open(&database_path).unwrap();
            connection.execute_batch(
                "PRAGMA foreign_keys=ON;
                 CREATE TABLE aria_conversations (
                    owner_id TEXT NOT NULL, conversation_id TEXT NOT NULL,
                    created_at TEXT NOT NULL, updated_at TEXT NOT NULL,
                    PRIMARY KEY(owner_id, conversation_id)
                 );
                 CREATE TABLE aria_conversation_events (
                    event_id TEXT PRIMARY KEY NOT NULL, owner_id TEXT NOT NULL,
                    conversation_id TEXT NOT NULL, role TEXT NOT NULL, content TEXT NOT NULL,
                    provider TEXT, model TEXT, created_at TEXT NOT NULL,
                    FOREIGN KEY(owner_id, conversation_id)
                    REFERENCES aria_conversations(owner_id, conversation_id) ON DELETE CASCADE
                 );
                 CREATE INDEX idx_aria_conversation_events_owner_time
                    ON aria_conversation_events(owner_id, conversation_id, created_at, event_id);
                 INSERT INTO aria_conversations VALUES ('alice','chat-a','2026-01-01T00:00:00+00:00','2026-01-01T00:00:00+00:00');
                 INSERT INTO aria_conversation_events VALUES ('shared-event','alice','chat-a','user','preserved',NULL,NULL,'2026-01-01T00:00:00+00:00');"
            ).unwrap();
        }

        let mut store = ConversationStore::open(&database_path, &test_key()).unwrap();
        let now = Utc::now();
        store.append_event("bob", "chat-b", &event("shared-event", "user", "new owner", now)).unwrap();
        assert_eq!(store.list_events("alice", "chat-a", 20).unwrap()[0].content, "preserved");
        assert_eq!(store.list_events("bob", "chat-b", 20).unwrap()[0].content, "new owner");

        drop(store);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn deletion_is_owner_scoped_and_cascades_events() {
        let mut store = ConversationStore::in_memory(&test_key()).unwrap();
        let now = Utc::now();
        store.append_event("alice", "same-id", &event("alice-event", "user", "alice", now)).unwrap();
        store.append_event("bob", "same-id", &event("bob-event", "user", "bob", now)).unwrap();

        assert!(!store.delete_conversation("mallory", "same-id").unwrap());
        assert!(store.delete_conversation("alice", "same-id").unwrap());
        assert!(store.list_events("alice", "same-id", 20).unwrap().is_empty());
        assert_eq!(store.list_events("bob", "same-id", 20).unwrap().len(), 1);
    }

    #[test]
    fn conversation_events_survive_database_reopen() {
        use std::time::{SystemTime, UNIX_EPOCH};

        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "janus-aria-conversation-{}-{unique}",
            std::process::id()
        ));
        std::fs::create_dir_all(&directory).unwrap();
        let database_path = directory.join("conversations.sqlite");
        let now = Utc::now();

        {
            let mut store = ConversationStore::open(&database_path, &test_key()).unwrap();
            store
                .append_event(
                    "alice",
                    "persistent-chat",
                    &event("persistent-event", "assistant", "persisted reply", now.clone()),
                )
                .unwrap();
        }

        let reopened = ConversationStore::open(&database_path, &test_key()).unwrap();
        let events = reopened.list_events("alice", "persistent-chat", 20).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_id, "persistent-event");
        assert_eq!(events[0].content, "persisted reply");
        assert_eq!(events[0].provider.as_deref(), Some("local-openai-compatible"));
        assert_eq!(events[0].model.as_deref(), Some("test-model"));
        assert_eq!(events[0].created_at, now);

        drop(reopened);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn conversation_content_is_encrypted_and_requires_the_correct_key() {
        use std::time::{SystemTime, UNIX_EPOCH};

        let unique = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "janus-aria-conversation-encryption-{}-{unique}", std::process::id()
        ));
        std::fs::create_dir_all(&directory).unwrap();
        let database_path = directory.join("conversations.sqlite");
        let secret = "distinctive-private-conversation-plaintext";
        {
            let mut store = ConversationStore::open(&database_path, &test_key()).unwrap();
            store.append_event("alice", "private-chat", &event("private-event", "user", secret, Utc::now())).unwrap();
        }

        let database_bytes = std::fs::read(&database_path).unwrap();
        assert!(!database_bytes.windows(secret.len()).any(|window| window == secret.as_bytes()));

        let wrong_key = [0x24; 32];
        let wrong_key_store = ConversationStore::open(&database_path, &wrong_key).unwrap();
        assert!(wrong_key_store.list_events("alice", "private-chat", 20).is_err());

        let correct_key_store = ConversationStore::open(&database_path, &test_key()).unwrap();
        assert_eq!(correct_key_store.list_events("alice", "private-chat", 20).unwrap()[0].content, secret);

        drop(wrong_key_store);
        drop(correct_key_store);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn rejects_invalid_roles_and_oversized_content() {
        let mut store = ConversationStore::in_memory(&test_key()).unwrap();
        let now = Utc::now();
        assert!(store.append_event("alice", "chat", &event("e1", "developer", "hello", now)).is_err());
        assert!(store.append_event("alice", "chat", &event("e2", "user", &"x".repeat(MAX_CONTENT_BYTES + 1), now)).is_err());
    }
}

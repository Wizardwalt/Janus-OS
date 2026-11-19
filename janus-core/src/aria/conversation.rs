//! Durable, owner-scoped Aria conversation event storage.
//!
//! This store persists conversation history separately from long-term memory.
//! The host must authenticate the caller and supply the verified owner ID.

use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use std::path::Path;

const MAX_OWNER_ID_BYTES: usize = 160;
const MAX_CONVERSATION_ID_BYTES: usize = 160;
const MAX_EVENT_ID_BYTES: usize = 160;
const MAX_CONTENT_BYTES: usize = 64 * 1024;
const MAX_PROVIDER_FIELD_BYTES: usize = 128;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversationEvent {
    pub event_id: String,
    pub role: String,
    pub content: String,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
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
}

impl ConversationStore {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let connection = Connection::open(path).context("opening Aria conversation database")?;
        Self::from_connection(connection)
    }

    pub fn in_memory() -> Result<Self> {
        Self::from_connection(Connection::open_in_memory()?)
    }

    fn from_connection(connection: Connection) -> Result<Self> {
        connection.pragma_update(None, "foreign_keys", "ON")?;
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
                event_id TEXT PRIMARY KEY NOT NULL,
                owner_id TEXT NOT NULL,
                conversation_id TEXT NOT NULL,
                role TEXT NOT NULL,
                content TEXT NOT NULL,
                provider TEXT,
                model TEXT,
                created_at TEXT NOT NULL,
                FOREIGN KEY(owner_id, conversation_id)
                    REFERENCES aria_conversations(owner_id, conversation_id)
                    ON DELETE CASCADE
            );
            CREATE INDEX IF NOT EXISTS idx_aria_conversation_events_owner_time
                ON aria_conversation_events(owner_id, conversation_id, created_at, event_id);
            CREATE INDEX IF NOT EXISTS idx_aria_conversations_owner_updated
                ON aria_conversations(owner_id, updated_at DESC);",
        )?;
        Ok(Self { connection })
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
             (event_id, owner_id, conversation_id, role, content, provider, model, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                event.event_id,
                owner_id,
                conversation_id,
                event.role,
                event.content,
                event.provider,
                event.model,
                timestamp
            ],
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
            "SELECT event_id, role, content, provider, model, created_at
             FROM aria_conversation_events
             WHERE owner_id = ?1 AND conversation_id = ?2
             ORDER BY created_at ASC, event_id ASC LIMIT ?3",
        )?;
        let rows = statement.query_map(params![owner_id, conversation_id, limit as i64], |row| {
            let created: String = row.get(5)?;
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, Option<String>>(4)?,
                created,
            ))
        })?;
        rows.map(|row| {
            let (event_id, role, content, provider, model, created) = row?;
            Ok(ConversationEvent {
                event_id,
                role,
                content,
                provider,
                model,
                created_at: parse_timestamp(&created)?,
            })
        }).collect()
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
        let mut store = ConversationStore::in_memory().unwrap();
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
    fn deletion_is_owner_scoped_and_cascades_events() {
        let mut store = ConversationStore::in_memory().unwrap();
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
            let mut store = ConversationStore::open(&database_path).unwrap();
            store
                .append_event(
                    "alice",
                    "persistent-chat",
                    &event("persistent-event", "assistant", "persisted reply", now.clone()),
                )
                .unwrap();
        }

        let reopened = ConversationStore::open(&database_path).unwrap();
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
    fn rejects_invalid_roles_and_oversized_content() {
        let mut store = ConversationStore::in_memory().unwrap();
        let now = Utc::now();
        assert!(store.append_event("alice", "chat", &event("e1", "developer", "hello", now)).is_err());
        assert!(store.append_event("alice", "chat", &event("e2", "user", &"x".repeat(MAX_CONTENT_BYTES + 1), now)).is_err());
    }
}

//! SQLite-backed append-only knowledge storage for Aria.
//!
//! Original artifacts are not stored or mutated here; source references point
//! back to their preserving host (for example, OmniVault).

use super::{Hat, KnowledgeRecord};
use anyhow::{anyhow, Context, Result};
use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension};
use std::collections::HashSet;
use std::path::Path;

/// Local durable store. A connection is intentionally owned by the caller;
/// hosts should place it behind their own async/task boundary as appropriate.
pub struct KnowledgeStore {
    connection: Connection,
}

/// Read boundary supplied by the host after authorization. This store enforces
/// the scope; it does not authenticate users or authorize callers.
#[derive(Debug, Clone)]
pub struct KnowledgeAccessScope {
    allowed_handling: HashSet<String>,
    allowed_projects: HashSet<String>,
    include_unscoped: bool,
}

impl KnowledgeAccessScope {
    /// Default-safe scope: public/internal records not assigned to a project.
    pub fn public_internal() -> Self {
        Self {
            allowed_handling: ["public".to_owned(), "internal".to_owned()].into_iter().collect(),
            allowed_projects: HashSet::new(),
            include_unscoped: true,
        }
    }

    /// Call only after the host authorizes these handling classes and projects.
    /// This is an enforcement input, not an authentication mechanism.
    pub fn host_authorized(handling: &[&str], project_ids: &[&str], include_unscoped: bool) -> Self {
        let allowed_handling = handling.iter()
            .filter(|v| matches!(**v, "public" | "internal" | "private" | "sensitive"))
            .map(|v| (*v).to_owned()).collect();
        Self {
            allowed_handling,
            allowed_projects: project_ids.iter().map(|v| (*v).to_owned()).collect(),
            include_unscoped,
        }
    }

    fn allows(&self, record: &KnowledgeRecord) -> bool {
        if record.expires_at.is_some_and(|expires_at| expires_at <= Utc::now()) {
            return false;
        }
        if !self.allowed_handling.contains(&record.handling) { return false; }
        if record.project_ids.is_empty() { return self.include_unscoped; }
        record.project_ids.iter().any(|id| self.allowed_projects.contains(id))
    }
}

impl KnowledgeStore {
    /// Open or create a persistent SQLite knowledge database.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let connection = Connection::open(path).context("opening Aria knowledge database")?;
        Self::from_connection(connection)
    }

    /// Create an in-memory store, primarily useful for tests and ephemeral tasks.
    pub fn in_memory() -> Result<Self> {
        Self::from_connection(Connection::open_in_memory()?)
    }

    fn from_connection(connection: Connection) -> Result<Self> {
        connection.pragma_update(None, "foreign_keys", "ON")?;
        connection.busy_timeout(std::time::Duration::from_secs(5))?;
        connection.execute_batch(
            "CREATE TABLE IF NOT EXISTS aria_knowledge_records (
                id TEXT PRIMARY KEY NOT NULL,
                title TEXT NOT NULL,
                content TEXT NOT NULL,
                record_json TEXT NOT NULL,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                epistemic_status TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_aria_knowledge_updated
                ON aria_knowledge_records(updated_at DESC);
            CREATE INDEX IF NOT EXISTS idx_aria_knowledge_title
                ON aria_knowledge_records(title);
            CREATE TABLE IF NOT EXISTS aria_knowledge_supersessions (
                new_record_id TEXT NOT NULL REFERENCES aria_knowledge_records(id),
                old_record_id TEXT NOT NULL UNIQUE REFERENCES aria_knowledge_records(id),
                PRIMARY KEY(new_record_id, old_record_id),
                CHECK(new_record_id <> old_record_id)
            );
            CREATE INDEX IF NOT EXISTS idx_aria_knowledge_supersessions_old
                ON aria_knowledge_supersessions(old_record_id);"
        ).context("initializing Aria knowledge schema")?;
        Ok(Self { connection })
    }

    /// Append a validated record. Existing IDs are never overwritten.
    ///
    /// Supersession is represented as a separate immutable relation: previous
    /// records remain available for audit, while normal searches return only
    /// records that have not been superseded.
    pub fn insert(&mut self, record: &KnowledgeRecord) -> Result<()> {
        record.validate().map_err(|e| anyhow!("invalid knowledge record: {e}"))?;
        if record.supersedes.iter().any(|id| id == &record.id) {
            return Err(anyhow!("a knowledge record cannot supersede itself"));
        }
        let mut seen = std::collections::HashSet::new();
        if record.supersedes.iter().any(|id| !seen.insert(id)) {
            return Err(anyhow!("supersedes contains duplicate record IDs"));
        }
        let json = serde_json::to_string(record).context("serializing knowledge record")?;
        let tx = self.connection.transaction()?;
        tx.execute(
            "INSERT INTO aria_knowledge_records
             (id, title, content, record_json, created_at, updated_at, epistemic_status)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                record.id,
                record.title,
                record.content,
                json,
                record.created_at.to_rfc3339(),
                record.updated_at.to_rfc3339(),
                serde_json::to_string(&record.epistemic_status)?
            ],
        ).with_context(|| format!("inserting knowledge record {}", record.id))?;

        for old_id in &record.supersedes {
            let exists: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM aria_knowledge_records WHERE id = ?1)",
                [old_id],
                |row| row.get(0),
            )?;
            if !exists {
                return Err(anyhow!("cannot supersede missing knowledge record: {old_id}"));
            }
            let already_superseded: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM aria_knowledge_supersessions WHERE old_record_id = ?1)",
                [old_id],
                |row| row.get(0),
            )?;
            if already_superseded {
                return Err(anyhow!("knowledge record is already superseded: {old_id}"));
            }
            tx.execute(
                "INSERT INTO aria_knowledge_supersessions(new_record_id, old_record_id)
                 VALUES (?1, ?2)",
                params![record.id, old_id],
            )?;
        }
        tx.commit().context("committing knowledge record")?;
        Ok(())
    }

    /// Retrieve a record by stable ID. Superseded versions are hidden unless
    /// explicitly requested for audit or history views.
    pub fn get(&self, id: &str, include_superseded: bool) -> Result<Option<KnowledgeRecord>> {
        self.get_with_scope(id, include_superseded, &KnowledgeAccessScope::public_internal())
    }

    pub fn get_with_scope(&self, id: &str, include_superseded: bool, scope: &KnowledgeAccessScope) -> Result<Option<KnowledgeRecord>> {
        let sql = if include_superseded {
            "SELECT record_json FROM aria_knowledge_records WHERE id = ?1"
        } else {
            "SELECT r.record_json FROM aria_knowledge_records r
             WHERE r.id = ?1 AND NOT EXISTS (
                 SELECT 1 FROM aria_knowledge_supersessions s WHERE s.old_record_id = r.id
             )"
        };
        let json: Option<String> = self.connection.query_row(sql, [id], |row| row.get(0)).optional()?;
        let record = json.map(|value| serde_json::from_str::<KnowledgeRecord>(&value)
            .context("decoding stored knowledge record")).transpose()?;
        Ok(record.filter(|item| scope.allows(item)))
    }

    /// Search title and content using a literal, case-insensitive substring.
    /// Results are stable-sorted by most recently updated, then ID.
    pub fn search(&self, query: &str, hat: Option<Hat>, limit: usize) -> Result<Vec<KnowledgeRecord>> {
        self.search_with_scope(query, hat, limit, &KnowledgeAccessScope::public_internal())
    }

    pub fn search_with_scope(&self, query: &str, hat: Option<Hat>, limit: usize, scope: &KnowledgeAccessScope) -> Result<Vec<KnowledgeRecord>> {
        if limit == 0 { return Ok(Vec::new()); }
        let needle = query.trim().to_lowercase();
        let mut statement = self.connection.prepare(
            "SELECT r.record_json FROM aria_knowledge_records r
             WHERE NOT EXISTS (
                 SELECT 1 FROM aria_knowledge_supersessions s WHERE s.old_record_id = r.id
             )
             ORDER BY r.updated_at DESC, r.id ASC"
        )?;
        let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
        let mut matches = Vec::new();
        for row in rows {
            let json = row?;
            let record: KnowledgeRecord = serde_json::from_str(&json)
                .context("decoding knowledge record during search")?;
            if !scope.allows(&record) { continue; }
            let text_matches = needle.is_empty()
                || record.title.to_lowercase().contains(&needle)
                || record.content.to_lowercase().contains(&needle)
                || record.tags.iter().any(|tag| tag.to_lowercase().contains(&needle));
            let hat_matches = hat.map(|wanted| record.hat_ids.contains(&wanted)).unwrap_or(true);
            if text_matches && hat_matches {
                matches.push(record);
                if matches.len() >= limit { break; }
            }
        }
        Ok(matches)
    }

    /// Count active records, excluding superseded history.
    pub fn active_count(&self) -> Result<usize> {
        let count: i64 = self.connection.query_row(
            "SELECT COUNT(*) FROM aria_knowledge_records r
             WHERE NOT EXISTS (
                 SELECT 1 FROM aria_knowledge_supersessions s WHERE s.old_record_id = r.id
             )",
            [],
            |row| row.get(0),
        )?;
        Ok(count as usize)
    }

    /// Count all versions, including superseded records.
    pub fn total_count(&self) -> Result<usize> {
        let count: i64 = self.connection.query_row(
            "SELECT COUNT(*) FROM aria_knowledge_records",
            [],
            |row| row.get(0),
        )?;
        Ok(count as usize)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::aria::{EpistemicStatus, KnowledgeContentType};
    use chrono::Utc;

    fn record(id: &str, title: &str, content: &str, hats: Vec<Hat>) -> KnowledgeRecord {
        let now = Utc::now();
        KnowledgeRecord {
            id: id.into(),
            schema_version: "1.0.0".into(),
            title: title.into(),
            content: content.into(),
            content_type: KnowledgeContentType::Concept,
            hat_ids: hats,
            tags: vec!["test-tag".into()],
            epistemic_status: EpistemicStatus::Unverified,
            confidence: None,
            confidence_method: None,
            sources: vec![],
            created_at: now,
            updated_at: now,
            version: 1,
            supersedes: vec![],
            handling: "internal".into(),
            project_ids: vec![],
            review_notes: None,
            expires_at: None,
        }
    }


    #[test]
    fn default_scope_hides_private_and_project_records() {
        let mut store = KnowledgeStore::in_memory().unwrap();
        store.insert(&record("global", "Public note", "Visible", vec![Hat::White])).unwrap();
        let mut private = record("private", "Private note", "Hidden", vec![Hat::White]);
        private.handling = "private".into();
        store.insert(&private).unwrap();
        let mut project = record("project", "Janus note", "Project only", vec![Hat::Green]);
        project.project_ids = vec!["janus".into()];
        store.insert(&project).unwrap();
        assert!(store.get("global", false).unwrap().is_some());
        assert!(store.get("private", false).unwrap().is_none());
        assert!(store.get("project", false).unwrap().is_none());
        assert_eq!(store.search("", None, 10).unwrap().len(), 1);
    }

    #[test]
    fn host_scope_enforces_handling_and_project_boundaries() {
        let mut store = KnowledgeStore::in_memory().unwrap();
        let mut janus = record("janus-private", "Janus private", "Authorized", vec![Hat::Green]);
        janus.handling = "private".into();
        janus.project_ids = vec!["janus".into()];
        store.insert(&janus).unwrap();
        let mut vault = record("vault-private", "Vault private", "Not authorized", vec![Hat::Blue]);
        vault.handling = "private".into();
        vault.project_ids = vec!["omnivault".into()];
        store.insert(&vault).unwrap();
        let scope = KnowledgeAccessScope::host_authorized(&["private"], &["janus"], false);
        assert!(store.get_with_scope("janus-private", false, &scope).unwrap().is_some());
        assert!(store.get_with_scope("vault-private", false, &scope).unwrap().is_none());
        assert_eq!(store.search_with_scope("", None, 10, &scope).unwrap().len(), 1);
    }


    #[test]
    fn expired_records_are_not_returned_even_with_explicit_project_scope() {
        let mut store = KnowledgeStore::in_memory().unwrap();
        let mut expired = record("expired", "Expired note", "No longer current", vec![Hat::White]);
        expired.handling = "private".into();
        expired.project_ids = vec!["janus".into()];
        expired.expires_at = Some(Utc::now() - chrono::Duration::seconds(1));
        store.insert(&expired).unwrap();

        let scope = KnowledgeAccessScope::host_authorized(&["private"], &["janus"], false);
        assert!(store.get_with_scope("expired", true, &scope).unwrap().is_none());
        assert!(store.search_with_scope("", None, 10, &scope).unwrap().is_empty());
    }

    #[test]
    fn inserts_and_retrieves_record() {
        let mut store = KnowledgeStore::in_memory().unwrap();
        let item = record("k-1", "Circuit basics", "A resistor limits current.", vec![Hat::Red]);
        store.insert(&item).unwrap();
        assert_eq!(store.get("k-1", false).unwrap().unwrap(), item);
        assert_eq!(store.active_count().unwrap(), 1);
    }

    #[test]
    fn refuses_to_overwrite_duplicate_id() {
        let mut store = KnowledgeStore::in_memory().unwrap();
        let item = record("k-1", "First", "Original content.", vec![Hat::White]);
        store.insert(&item).unwrap();
        let replacement = record("k-1", "Changed", "Must not overwrite.", vec![Hat::White]);
        assert!(store.insert(&replacement).is_err());
        assert_eq!(store.get("k-1", false).unwrap().unwrap().title, "First");
    }

    #[test]
    fn searches_case_insensitively_and_filters_by_hat() {
        let mut store = KnowledgeStore::in_memory().unwrap();
        store.insert(&record("k-red", "Circuit design", "Ohm's law.", vec![Hat::Red])).unwrap();
        store.insert(&record("k-green", "Rust programming", "Memory safety.", vec![Hat::Green])).unwrap();
        assert_eq!(store.search("CIRCUIT", None, 10).unwrap().len(), 1);
        assert_eq!(store.search("", Some(Hat::Green), 10).unwrap()[0].id, "k-green");
    }

    #[test]
    fn supersession_preserves_history_and_hides_old_version_from_normal_search() {
        let mut store = KnowledgeStore::in_memory().unwrap();
        let old = record("k-old", "Old fact", "Outdated text.", vec![Hat::White]);
        store.insert(&old).unwrap();
        let mut new = record("k-new", "Corrected fact", "Updated text.", vec![Hat::White]);
        new.supersedes = vec!["k-old".into()];
        store.insert(&new).unwrap();
        assert!(store.get("k-old", false).unwrap().is_none());
        assert_eq!(store.get("k-old", true).unwrap().unwrap(), old);
        assert_eq!(store.active_count().unwrap(), 1);
        assert_eq!(store.total_count().unwrap(), 2);
    }

    #[test]
    fn rejects_missing_or_already_superseded_records() {
        let mut store = KnowledgeStore::in_memory().unwrap();
        let mut missing = record("k-new", "New", "Content", vec![Hat::White]);
        missing.supersedes = vec!["missing".into()];
        assert!(store.insert(&missing).is_err());
        store.insert(&record("k-old", "Old", "Content", vec![Hat::White])).unwrap();
        let mut first = record("k-first", "First correction", "Content", vec![Hat::White]);
        first.supersedes = vec!["k-old".into()];
        store.insert(&first).unwrap();
        let mut second = record("k-second", "Second correction", "Content", vec![Hat::White]);
        second.supersedes = vec!["k-old".into()];
        assert!(store.insert(&second).is_err());
    }
}

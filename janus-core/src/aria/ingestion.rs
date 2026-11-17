//! Source-preserving UTF-8 text ingestion for Aria Stage 2.
//! Original bytes are retained unchanged; chunks are derived, reproducible data.

use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{collections::HashSet, path::Path};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TextArtifactInput {
    pub artifact_id: String,
    pub title: String,
    pub media_type: String,
    pub source_type: String,
    pub handling: String,
    pub project_ids: Vec<String>,
    pub imported_at: DateTime<Utc>,
    pub expires_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ArtifactManifest {
    pub artifact_id: String,
    pub title: String,
    pub media_type: String,
    pub source_type: String,
    pub sha256: String,
    pub byte_length: u64,
    pub handling: String,
    pub project_ids: Vec<String>,
    pub imported_at: DateTime<Utc>,
    pub expires_at: Option<DateTime<Utc>>,
    pub chunk_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SourcePassage {
    pub artifact_id: String,
    pub title: String,
    pub sha256: String,
    pub start_byte: usize,
    pub end_byte: usize,
    pub ordinal: usize,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TextChunk {
    pub artifact_id: String,
    pub ordinal: usize,
    pub start_byte: usize,
    pub end_byte: usize,
    pub text: String,
}

#[derive(Debug, Clone)]
pub struct IngestionAccessScope {
    handling: HashSet<String>,
    projects: HashSet<String>,
    include_unscoped: bool,
}

impl IngestionAccessScope {
    pub fn public_internal() -> Self {
        Self {
            handling: ["public".to_owned(), "internal".to_owned()].into_iter().collect(),
            projects: HashSet::new(),
            include_unscoped: true,
        }
    }

    pub fn host_authorized(handling: &[&str], projects: &[&str], include_unscoped: bool) -> Self {
        Self {
            handling: handling.iter()
                .filter(|v| matches!(**v, "public" | "internal" | "private" | "sensitive"))
                .map(|v| (*v).to_owned()).collect(),
            projects: projects.iter().map(|v| (*v).to_owned()).collect(),
            include_unscoped,
        }
    }

    fn allows(&self, handling: &str, projects: &[String], expiry: Option<DateTime<Utc>>) -> bool {
        if expiry.is_some_and(|v| v <= Utc::now()) || !self.handling.contains(handling) {
            return false;
        }
        if projects.is_empty() {
            return self.include_unscoped && matches!(handling,"public"|"internal");
        }
        projects.iter().any(|id| self.projects.contains(id))
    }
}

pub struct AriaIngestionStore {
    connection: Connection,
}

impl AriaIngestionStore {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        Self::from_connection(Connection::open(path)?)
    }

    pub fn in_memory() -> Result<Self> {
        Self::from_connection(Connection::open_in_memory()?)
    }

    fn from_connection(connection: Connection) -> Result<Self> {
        connection.pragma_update(None, "foreign_keys", "ON")?;
        connection.busy_timeout(std::time::Duration::from_secs(5))?;
        connection.execute_batch(
            "CREATE TABLE IF NOT EXISTS aria_source_artifacts (
                artifact_id TEXT PRIMARY KEY NOT NULL,
                title TEXT NOT NULL, media_type TEXT NOT NULL, source_type TEXT NOT NULL,
                original_bytes BLOB NOT NULL, sha256 TEXT NOT NULL, byte_length INTEGER NOT NULL,
                handling TEXT NOT NULL, project_ids_json TEXT NOT NULL,
                imported_at TEXT NOT NULL, expires_at TEXT
            );
            CREATE TABLE IF NOT EXISTS aria_source_chunks (
                artifact_id TEXT NOT NULL REFERENCES aria_source_artifacts(artifact_id),
                ordinal INTEGER NOT NULL, start_byte INTEGER NOT NULL,
                end_byte INTEGER NOT NULL, chunk_text TEXT NOT NULL,
                PRIMARY KEY(artifact_id, ordinal)
            );
            CREATE INDEX IF NOT EXISTS idx_aria_source_chunks_artifact
                ON aria_source_chunks(artifact_id, ordinal);"
        ).context("initializing Aria source ingestion schema")?;
        Ok(Self { connection })
    }

    /// Store original UTF-8 bytes unchanged and create deterministic derived chunks.
    /// Format-specific extraction for PDF, DOCX, HTML, etc. must happen in adapters.
    pub fn import_text(
        &mut self, input: &TextArtifactInput, original_bytes: &[u8],
        max_chars: usize, overlap_chars: usize,
    ) -> Result<ArtifactManifest> {
        validate_input(input, original_bytes, max_chars, overlap_chars)?;
        let text = std::str::from_utf8(original_bytes)
            .context("text ingestion requires UTF-8; use a format-specific extractor first")?;
        let chunks = chunk_text(&input.artifact_id, text, max_chars, overlap_chars);
        let digest = format!("{:x}", Sha256::digest(original_bytes));
        let projects = serde_json::to_string(&input.project_ids)?;
        let expires = input.expires_at.map(|v| v.to_rfc3339());
        let byte_length = i64::try_from(original_bytes.len())
            .map_err(|_| anyhow!("artifact too large for SQLite byte-length storage"))?;

        let tx = self.connection.transaction()?;
        tx.execute(
            "INSERT INTO aria_source_artifacts
             (artifact_id,title,media_type,source_type,original_bytes,sha256,byte_length,
              handling,project_ids_json,imported_at,expires_at)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
            params![input.artifact_id,input.title,input.media_type,input.source_type,
                original_bytes,digest,byte_length,input.handling,projects,
                input.imported_at.to_rfc3339(),expires],
        ).with_context(|| format!("importing source artifact {}", input.artifact_id))?;

        for chunk in &chunks {
            tx.execute(
                "INSERT INTO aria_source_chunks
                 (artifact_id,ordinal,start_byte,end_byte,chunk_text) VALUES (?1,?2,?3,?4,?5)",
                params![chunk.artifact_id,i64::try_from(chunk.ordinal)?,
                    i64::try_from(chunk.start_byte)?,i64::try_from(chunk.end_byte)?,chunk.text],
            )?;
        }
        tx.commit().context("committing source artifact and chunks")?;

        Ok(ArtifactManifest {
            artifact_id: input.artifact_id.clone(), title: input.title.clone(),
            media_type: input.media_type.clone(), source_type: input.source_type.clone(),
            sha256: digest, byte_length: original_bytes.len() as u64,
            handling: input.handling.clone(), project_ids: input.project_ids.clone(),
            imported_at: input.imported_at, expires_at: input.expires_at,
            chunk_count: chunks.len(),
        })
    }

    /// Preserve an arbitrary binary source exactly without pretending it has
    /// been extracted. Binary artifacts have no text chunks until a format
    /// adapter produces derived text with its own provenance and locators.
    pub fn import_binary(
        &mut self,
        input: &TextArtifactInput,
        original_bytes: &[u8],
    ) -> Result<ArtifactManifest> {
        validate_binary_input(input, original_bytes)?;
        let digest = format!("{:x}", Sha256::digest(original_bytes));
        let projects = serde_json::to_string(&input.project_ids)?;
        let expires = input.expires_at.map(|v| v.to_rfc3339());
        let byte_length = i64::try_from(original_bytes.len())
            .map_err(|_| anyhow!("artifact too large for SQLite byte-length storage"))?;

        let tx = self.connection.transaction()?;
        tx.execute(
            "INSERT INTO aria_source_artifacts
             (artifact_id,title,media_type,source_type,original_bytes,sha256,byte_length,
              handling,project_ids_json,imported_at,expires_at)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
            params![input.artifact_id,input.title,input.media_type,input.source_type,
                original_bytes,digest,byte_length,input.handling,projects,
                input.imported_at.to_rfc3339(),expires],
        ).with_context(|| format!("preserving binary source artifact {}", input.artifact_id))?;
        tx.commit().context("committing binary source artifact")?;

        Ok(ArtifactManifest {
            artifact_id: input.artifact_id.clone(),
            title: input.title.clone(),
            media_type: input.media_type.clone(),
            source_type: input.source_type.clone(),
            sha256: digest,
            byte_length: original_bytes.len() as u64,
            handling: input.handling.clone(),
            project_ids: input.project_ids.clone(),
            imported_at: input.imported_at,
            expires_at: input.expires_at,
            chunk_count: 0,
        })
    }

    pub fn original_bytes(&self, id: &str, scope: &IngestionAccessScope) -> Result<Option<Vec<u8>>> {
        let row: Option<(Vec<u8>,String,String,Option<String>,String)> = self.connection.query_row(
            "SELECT original_bytes,handling,project_ids_json,expires_at,sha256
             FROM aria_source_artifacts WHERE artifact_id=?1",
            [id], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))
        ).optional()?;
        let Some((bytes,handling,projects,expiry,expected_hash)) = row else { return Ok(None) };
        let projects: Vec<String> = serde_json::from_str(&projects)?;
        let expiry = parse_expiry(expiry)?;
        if !scope.allows(&handling,&projects,expiry) { return Ok(None); }
        let actual_hash = format!("{:x}", Sha256::digest(&bytes));
        if actual_hash != expected_hash { return Err(anyhow!("stored source artifact SHA-256 mismatch")); }
        Ok(Some(bytes))
    }

    pub fn manifest(&self, id: &str, scope: &IngestionAccessScope) -> Result<Option<ArtifactManifest>> {
        let row: Option<(String,String,String,String,i64,String,String,String,Option<String>,i64)> =
            self.connection.query_row(
                "SELECT title,media_type,source_type,sha256,byte_length,handling,project_ids_json,
                 imported_at,expires_at,(SELECT COUNT(*) FROM aria_source_chunks c
                 WHERE c.artifact_id=a.artifact_id) FROM aria_source_artifacts a WHERE artifact_id=?1",
                [id], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,
                    r.get(5)?,r.get(6)?,r.get(7)?,r.get(8)?,r.get(9)?))
            ).optional()?;
        let Some((title,media,source,hash,len,handling,projects,imported,expiry,count)) = row else {
            return Ok(None);
        };
        let projects: Vec<String> = serde_json::from_str(&projects)?;
        let expiry = parse_expiry(expiry)?;
        if !scope.allows(&handling,&projects,expiry) { return Ok(None); }
        Ok(Some(ArtifactManifest {
            artifact_id:id.to_owned(),title,media_type:media,source_type:source,sha256:hash,
            byte_length:u64::try_from(len)?,handling,project_ids:projects,
            imported_at:DateTime::parse_from_rfc3339(&imported)?.with_timezone(&Utc),
            expires_at:expiry,chunk_count:usize::try_from(count)?,
        }))
    }

    pub fn chunks(&self, id: &str, scope: &IngestionAccessScope) -> Result<Vec<TextChunk>> {
        if self.manifest(id,scope)?.is_none() { return Ok(Vec::new()); }
        let mut stmt = self.connection.prepare(
            "SELECT ordinal,start_byte,end_byte,chunk_text FROM aria_source_chunks
             WHERE artifact_id=?1 ORDER BY ordinal"
        )?;
        let rows = stmt.query_map([id], |r| Ok((
            r.get::<_,i64>(0)?,r.get::<_,i64>(1)?,r.get::<_,i64>(2)?,r.get::<_,String>(3)?
        )))?;
        rows.map(|row| {
            let (ordinal,start,end,text) = row?;
            Ok(TextChunk { artifact_id:id.to_owned(), ordinal:usize::try_from(ordinal)?,
                start_byte:usize::try_from(start)?, end_byte:usize::try_from(end)?, text })
        }).collect()
    }
    /// Search extracted chunks without losing their exact source locators.
    /// Access checks are applied before any matching passage is returned.
    pub fn search_chunks(
        &self, query: &str, scope: &IngestionAccessScope, limit: usize,
    ) -> Result<Vec<SourcePassage>> {
        let needle = query.trim().to_lowercase();
        if needle.is_empty() || limit == 0 { return Ok(Vec::new()); }
        let mut statement = self.connection.prepare(
            "SELECT a.artifact_id,a.title,a.sha256,a.handling,a.project_ids_json,
                    a.expires_at,c.start_byte,c.end_byte,c.chunk_text,c.ordinal
             FROM aria_source_artifacts a
             JOIN aria_source_chunks c ON c.artifact_id=a.artifact_id
             ORDER BY a.imported_at DESC,a.artifact_id ASC,c.ordinal ASC"
        )?;
        let rows = statement.query_map([], |row| Ok((
            row.get::<_,String>(0)?, row.get::<_,String>(1)?, row.get::<_,String>(2)?,
            row.get::<_,String>(3)?, row.get::<_,String>(4)?, row.get::<_,Option<String>>(5)?,
            row.get::<_,i64>(6)?, row.get::<_,i64>(7)?, row.get::<_,String>(8)?,
            row.get::<_,i64>(9)?
        )))?;
        let mut passages = Vec::new();
        for row in rows {
            let (artifact_id,title,sha256,handling,projects,expiry,start,end,text,ordinal) = row?;
            let projects: Vec<String> = serde_json::from_str(&projects)?;
            if !scope.allows(&handling,&projects,parse_expiry(expiry)?) { continue; }
            if !text.to_lowercase().contains(&needle) { continue; }
            passages.push(SourcePassage {
                artifact_id,title,sha256,start_byte:usize::try_from(start)?,
                end_byte:usize::try_from(end)?,ordinal:usize::try_from(ordinal)?,text,
            });
            if passages.len() >= limit { break; }
        }
        Ok(passages)
    }

}

fn parse_expiry(value: Option<String>) -> Result<Option<DateTime<Utc>>> {
    value.map(|v| DateTime::parse_from_rfc3339(&v)
        .map(|d| d.with_timezone(&Utc)).context("parsing artifact expiry")).transpose()
}

fn validate_binary_input(input: &TextArtifactInput, bytes: &[u8]) -> Result<()> {
    for (name, value) in [
        ("artifact_id", &input.artifact_id),
        ("title", &input.title),
        ("media_type", &input.media_type),
        ("source_type", &input.source_type),
    ] {
        if value.trim().is_empty() {
            return Err(anyhow!("{name} must not be empty"));
        }
    }
    if !matches!(input.handling.as_str(), "public" | "internal" | "private" | "sensitive") {
        return Err(anyhow!("invalid handling classification"));
    }
    if input.artifact_id.len() > 200 || input.title.len() > 500
        || input.media_type.len() > 200 || input.source_type.len() > 100 {
        return Err(anyhow!("artifact metadata exceeds supported length limits"));
    }
    if input.project_ids.iter().any(|id| id.trim().is_empty() || id.len() > 160) {
        return Err(anyhow!("project IDs must be non-empty and at most 160 bytes"));
    }
    let unique: HashSet<_> = input.project_ids.iter().collect();
    if unique.len() != input.project_ids.len() {
        return Err(anyhow!("duplicate project IDs"));
    }
    if bytes.is_empty() {
        return Err(anyhow!("source artifact must not be empty"));
    }
    const MAX_ARTIFACT_BYTES: usize = 32 * 1024 * 1024;
    if bytes.len() > MAX_ARTIFACT_BYTES {
        return Err(anyhow!("binary artifact exceeds the 32 MiB ingestion limit"));
    }
    if input.expires_at.is_some_and(|v| v <= input.imported_at) {
        return Err(anyhow!("expires_at must be later than imported_at"));
    }
    Ok(())
}

fn validate_input(input: &TextArtifactInput, bytes: &[u8], max: usize, overlap: usize) -> Result<()> {
    for (name,value) in [("artifact_id",&input.artifact_id),("title",&input.title),
        ("media_type",&input.media_type),("source_type",&input.source_type)] {
        if value.trim().is_empty() { return Err(anyhow!("{name} must not be empty")); }
    }
    if !matches!(input.handling.as_str(),"public"|"internal"|"private"|"sensitive") {
        return Err(anyhow!("invalid handling classification"));
    }
    if input.artifact_id.len()>200 || input.title.len()>500
        || input.media_type.len()>200 || input.source_type.len()>100 {
        return Err(anyhow!("artifact metadata exceeds supported length limits"));
    }
    if input.project_ids.iter().any(|id|id.trim().is_empty() || id.len()>160) {
        return Err(anyhow!("project IDs must be non-empty and at most 160 bytes"));
    }
    let unique: HashSet<_> = input.project_ids.iter().collect();
    if unique.len()!=input.project_ids.len() { return Err(anyhow!("duplicate project IDs")); }
    if bytes.is_empty() { return Err(anyhow!("source artifact must not be empty")); }
    const MAX_ARTIFACT_BYTES:usize=32*1024*1024;
    if bytes.len()>MAX_ARTIFACT_BYTES {
        return Err(anyhow!("text artifact exceeds the 32 MiB ingestion limit"));
    }
    if max==0 || overlap>=max { return Err(anyhow!("require max_chars > overlap_chars and max_chars > 0")); }
    if input.expires_at.is_some_and(|v| v<=input.imported_at) {
        return Err(anyhow!("expires_at must be later than imported_at"));
    }
    Ok(())
}

/// Split on Unicode scalar boundaries; locators are byte offsets into original UTF-8.
pub fn chunk_text(id: &str, text: &str, max: usize, overlap: usize) -> Vec<TextChunk> {
    if text.is_empty() || max==0 || overlap>=max { return Vec::new(); }
    let bounds: Vec<usize> = text.char_indices().map(|(i,_)|i)
        .chain(std::iter::once(text.len())).collect();
    let count=bounds.len()-1;
    let mut result=Vec::new();
    let mut start=0;
    while start<count {
        let end=(start+max).min(count);
        let a=bounds[start]; let b=bounds[end];
        result.push(TextChunk { artifact_id:id.to_owned(), ordinal:result.len(),
            start_byte:a,end_byte:b,text:text[a..b].to_owned() });
        if end==count { break; }
        start=end-overlap;
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    fn input(id:&str)->TextArtifactInput {
        TextArtifactInput { artifact_id:id.into(),title:"Manual excerpt".into(),
            media_type:"text/plain; charset=utf-8".into(),source_type:"file".into(),
            handling:"internal".into(),project_ids:vec!["janus".into()],
            imported_at:Utc::now(),expires_at:None }
    }

    #[test]
    fn chunks_are_utf8_safe_deterministic_and_overlapping() {
        let text="Aé🙂BCDEF"; let chunks=chunk_text("a",text,4,1);
        assert_eq!(chunks.iter().map(|c|c.text.as_str()).collect::<Vec<_>>(),
            vec!["Aé🙂B","BCDE","EF"]);
        for c in chunks { assert_eq!(&text[c.start_byte..c.end_byte],c.text); }
    }

    #[test]
    fn import_preserves_exact_original_and_sha256() {
        let mut store=AriaIngestionStore::in_memory().unwrap();
        let bytes="Ohm's law: V = I × R.".as_bytes();
        let manifest=store.import_text(&input("manual"),bytes,8,2).unwrap();
        assert_eq!(manifest.sha256,format!("{:x}",Sha256::digest(bytes)));
        let scope=IngestionAccessScope::host_authorized(&["internal"],&["janus"],false);
        assert_eq!(store.original_bytes("manual",&scope).unwrap().unwrap(),bytes);
        assert!(!store.chunks("manual",&scope).unwrap().is_empty());
    }

    #[test]
    fn binary_import_preserves_non_utf8_bytes_without_creating_text_chunks() {
        let mut store = AriaIngestionStore::in_memory().unwrap();
        let bytes = [0x25, 0x50, 0x44, 0x46, 0xff, 0x00];
        let mut item = input("binary-pdf");
        item.media_type = "application/pdf".into();
        let manifest = store.import_binary(&item, &bytes).unwrap();
        assert_eq!(manifest.sha256, format!("{:x}", Sha256::digest(bytes)));
        assert_eq!(manifest.byte_length, bytes.len() as u64);
        assert_eq!(manifest.chunk_count, 0);
        let scope = IngestionAccessScope::host_authorized(&["internal"], &["janus"], false);
        assert_eq!(store.original_bytes("binary-pdf", &scope).unwrap().unwrap(), bytes);
        assert!(store.chunks("binary-pdf", &scope).unwrap().is_empty());
    }

    #[test]
    fn binary_import_obeys_scope_and_rejects_duplicate_ids() {
        let mut store = AriaIngestionStore::in_memory().unwrap();
        let mut item = input("private-binary");
        item.handling = "private".into();
        let bytes = [0, 0xff, 0x80, 1];
        store.import_binary(&item, &bytes).unwrap();
        assert!(store.original_bytes("private-binary", &IngestionAccessScope::public_internal())
            .unwrap().is_none());
        let allowed = IngestionAccessScope::host_authorized(&["private"], &["janus"], false);
        assert_eq!(store.original_bytes("private-binary", &allowed).unwrap().unwrap(), bytes);
        assert!(store.import_binary(&item, b"replacement").is_err());
    }

    #[test]
    fn private_project_scope_is_enforced() {
        let mut store=AriaIngestionStore::in_memory().unwrap();
        let mut item=input("private"); item.handling="private".into();
        store.import_text(&item,b"private text",20,0).unwrap();
        assert!(store.original_bytes("private",&IngestionAccessScope::public_internal())
            .unwrap().is_none());
        let allowed=IngestionAccessScope::host_authorized(&["private"],&["janus"],false);
        assert_eq!(store.original_bytes("private",&allowed).unwrap().unwrap(),b"private text");
        let wrong=IngestionAccessScope::host_authorized(&["private"],&["omnivault"],false);
        assert!(store.original_bytes("private",&wrong).unwrap().is_none());
    }

    #[test]
    fn duplicate_ids_invalid_utf8_and_bad_chunking_are_rejected() {
        let mut store=AriaIngestionStore::in_memory().unwrap(); let item=input("one");
        store.import_text(&item,b"valid",10,0).unwrap();
        assert!(store.import_text(&item,b"replacement",10,0).is_err());
        assert!(store.import_text(&input("binary"),&[0xff],10,0).is_err());
        assert!(store.import_text(&input("bad"),b"text",4,4).is_err());
    }

    #[test]
    fn expired_artifacts_are_hidden() {
        let mut store=AriaIngestionStore::in_memory().unwrap();
        let mut item=input("expired");
        item.imported_at=Utc::now()-chrono::Duration::seconds(10);
        item.expires_at=Some(Utc::now()-chrono::Duration::seconds(1));
        store.import_text(&item,b"short lived",20,0).unwrap();
        let scope=IngestionAccessScope::host_authorized(&["internal"],&["janus"],false);
        assert!(store.original_bytes("expired",&scope).unwrap().is_none());
        assert!(store.manifest("expired",&scope).unwrap().is_none());
        assert!(store.chunks("expired",&scope).unwrap().is_empty());
    }
    #[test]
    fn chunk_search_returns_citable_source_ranges_and_respects_scope() {
        let mut store=AriaIngestionStore::in_memory().unwrap();
        store.import_text(&input("manual-a"),b"Ohm's law relates voltage and current.",12,2).unwrap();
        let allowed=IngestionAccessScope::host_authorized(&["internal"],&["janus"],false);
        let hits=store.search_chunks("Ohm's",&allowed,5).unwrap();
        assert!(!hits.is_empty());
        assert!(hits.iter().all(|hit|hit.artifact_id=="manual-a" && hit.sha256.len()==64));
        for hit in hits {
            let original=store.original_bytes("manual-a",&allowed).unwrap().unwrap();
            assert_eq!(&String::from_utf8(original).unwrap()[hit.start_byte..hit.end_byte],hit.text);
        }
        let denied=IngestionAccessScope::host_authorized(&["private"],&["janus"],false);
        assert!(store.search_chunks("Ohm's",&denied,5).unwrap().is_empty());
    }


    #[test]
    fn rejects_blank_project_scope_and_oversized_artifact() {
        let mut store=AriaIngestionStore::in_memory().unwrap();
        let mut item=input("bad-project");
        item.project_ids=vec![" ".into()];
        assert!(store.import_text(&item,b"text",10,0).is_err());
        let item=input("too-large");
        let bytes=vec![b'x';32*1024*1024+1];
        assert!(store.import_text(&item,&bytes,10,0).is_err());
    }


    #[test]
    fn broad_host_scope_cannot_read_unscoped_private_artifacts() {
        let mut store=AriaIngestionStore::in_memory().unwrap();
        let mut public=input("public-global");
        public.handling="public".into();
        public.project_ids.clear();
        store.import_text(&public,b"shared text",100,0).unwrap();
        let mut private=input("private-global");
        private.handling="private".into();
        private.project_ids.clear();
        store.import_text(&private,b"private text",100,0).unwrap();
        let scope=IngestionAccessScope::host_authorized(
            &["public","internal","private"],&["owner:alice"],true
        );
        assert!(store.original_bytes("public-global",&scope).unwrap().is_some());
        assert!(store.original_bytes("private-global",&scope).unwrap().is_none());
    }


}

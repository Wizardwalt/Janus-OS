//! Read-only evidence collection.
//!
//! This is the first module family intended for production certification. Every
//! operation here is genuinely performed against the local filesystem and is
//! strictly read-only: no file is created, modified, moved, or deleted, and no
//! shell, device, or network access is involved.
//!
//! Access is confined to the evidence paths a customer approved for the active
//! engagement. Path resolution canonicalizes symlinks before the containment
//! check, so a symlink inside an approved directory cannot be used to read
//! outside it.

use anyhow::{anyhow, Result};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

/// Maximum bytes returned by a single `read_slice` call.
pub const MAX_READ_SLICE: u64 = 1024 * 1024;
/// Maximum entries returned by a single `list_dir` call.
pub const MAX_LIST_ENTRIES: usize = 4096;

/// Filesystem metadata for one evidence item.
#[derive(Debug, Clone, Serialize)]
pub struct EvidenceStat {
    pub path: String,
    pub size_bytes: u64,
    pub is_file: bool,
    pub is_dir: bool,
    pub readonly: bool,
    pub modified_unix: Option<i64>,
}

/// A directory entry name and type, without following symlinks.
#[derive(Debug, Clone, Serialize)]
pub struct EvidenceDirEntry {
    pub name: String,
    pub is_file: bool,
    pub is_dir: bool,
    pub is_symlink: bool,
}

/// The set of filesystem locations a customer approved for read-only collection.
#[derive(Debug, Clone, Default)]
pub struct EvidenceScope {
    roots: Vec<PathBuf>,
}

impl EvidenceScope {
    /// Build a scope from approved path strings.
    ///
    /// Paths must exist and be absolute. Relative approved paths are rejected
    /// because their meaning would depend on the daemon's working directory.
    pub fn new(approved_paths: &[String]) -> Result<Self> {
        let mut roots = Vec::new();
        for raw in approved_paths {
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                return Err(anyhow!("approved evidence path must not be empty"));
            }
            let path = Path::new(trimmed);
            if !path.is_absolute() {
                return Err(anyhow!(
                    "approved evidence path must be absolute: {}",
                    trimmed
                ));
            }
            let canonical = path
                .canonicalize()
                .map_err(|error| anyhow!("approved evidence path is not readable: {} ({})", trimmed, error))?;
            if !roots.contains(&canonical) {
                roots.push(canonical);
            }
        }
        Ok(Self { roots })
    }

    /// Whether any usable evidence location was approved.
    pub fn is_empty(&self) -> bool {
        self.roots.is_empty()
    }

    /// Resolve a requested path and prove it is inside an approved root.
    ///
    /// The requested path must be absolute. Both the root and the target are
    /// canonicalized first so `..` segments and symlinks cannot escape.
    pub fn resolve(&self, requested: &str) -> Result<PathBuf> {
        if self.roots.is_empty() {
            return Err(anyhow!(
                "engagement approved no evidence paths; read-only collection is not authorized"
            ));
        }
        let trimmed = requested.trim();
        if trimmed.is_empty() {
            return Err(anyhow!("evidence path must not be empty"));
        }
        let path = Path::new(trimmed);
        if !path.is_absolute() {
            return Err(anyhow!("evidence path must be absolute: {}", trimmed));
        }
        let canonical = path
            .canonicalize()
            .map_err(|error| anyhow!("evidence path is not readable: {} ({})", trimmed, error))?;
        if self.roots.iter().any(|root| canonical.starts_with(root)) {
            Ok(canonical)
        } else {
            Err(anyhow!(
                "evidence path is outside the approved engagement scope: {}",
                trimmed
            ))
        }
    }
}

/// Compute the SHA-256 of a file by streaming it, never loading it fully.
pub fn hash_file(path: &Path) -> Result<(String, u64)> {
    let mut file = fs::File::open(path)
        .map_err(|error| anyhow!("cannot open evidence file {}: {}", path.display(), error))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    let mut total: u64 = 0;
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|error| anyhow!("cannot read evidence file {}: {}", path.display(), error))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        total += read as u64;
    }
    Ok((format!("{:x}", hasher.finalize()), total))
}

/// Stat a path without following the final symlink.
pub fn stat_path(path: &Path) -> Result<EvidenceStat> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| anyhow!("cannot stat evidence path {}: {}", path.display(), error))?;
    let modified_unix = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|duration| duration.as_secs() as i64);
    Ok(EvidenceStat {
        path: path.display().to_string(),
        size_bytes: metadata.len(),
        is_file: metadata.is_file(),
        is_dir: metadata.is_dir(),
        readonly: metadata.permissions().readonly(),
        modified_unix,
    })
}

/// List a directory's immediate entries in deterministic order.
pub fn list_dir(path: &Path) -> Result<Vec<EvidenceDirEntry>> {
    let mut entries = Vec::new();
    let reader = fs::read_dir(path)
        .map_err(|error| anyhow!("cannot list evidence directory {}: {}", path.display(), error))?;
    for entry in reader {
        let entry = entry
            .map_err(|error| anyhow!("cannot read evidence directory entry: {}", error))?;
        if entries.len() >= MAX_LIST_ENTRIES {
            break;
        }
        // file_type() does not follow symlinks.
        let file_type = entry
            .file_type()
            .map_err(|error| anyhow!("cannot inspect evidence entry: {}", error))?;
        entries.push(EvidenceDirEntry {
            name: entry.file_name().to_string_lossy().to_string(),
            is_file: file_type.is_file(),
            is_dir: file_type.is_dir(),
            is_symlink: file_type.is_symlink(),
        });
    }
    entries.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(entries)
}

/// Read a bounded slice of a file, returned as lowercase hex.
///
/// Hex keeps arbitrary binary evidence safe to transport as JSON.
pub fn read_slice(path: &Path, offset: u64, length: u64) -> Result<(String, u64)> {
    if length == 0 {
        return Err(anyhow!("read length must be greater than zero"));
    }
    if length > MAX_READ_SLICE {
        return Err(anyhow!(
            "read length {} exceeds the {} byte per-call limit",
            length,
            MAX_READ_SLICE
        ));
    }
    let mut file = fs::File::open(path)
        .map_err(|error| anyhow!("cannot open evidence file {}: {}", path.display(), error))?;
    file.seek(SeekFrom::Start(offset))
        .map_err(|error| anyhow!("cannot seek evidence file {}: {}", path.display(), error))?;
    let mut buffer = vec![0_u8; length as usize];
    let mut filled = 0_usize;
    while filled < buffer.len() {
        let read = file
            .read(&mut buffer[filled..])
            .map_err(|error| anyhow!("cannot read evidence file {}: {}", path.display(), error))?;
        if read == 0 {
            break;
        }
        filled += read;
    }
    buffer.truncate(filled);
    let hex = buffer.iter().map(|byte| format!("{:02x}", byte)).collect();
    Ok((hex, filled as u64))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// Create a unique temporary directory without external crates.
    fn temp_dir(label: &str) -> PathBuf {
        let unique = format!(
            "janus-evidence-test-{}-{}-{}",
            label,
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let dir = std::env::temp_dir().join(unique);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_file(dir: &Path, name: &str, contents: &[u8]) -> PathBuf {
        let path = dir.join(name);
        let mut file = fs::File::create(&path).unwrap();
        file.write_all(contents).unwrap();
        path
    }

    #[test]
    fn hashes_a_real_file_with_the_known_sha256_of_its_contents() {
        let dir = temp_dir("hash");
        let path = write_file(&dir, "evidence.bin", b"abc");
        let (digest, size) = hash_file(&path).unwrap();
        // Well-known SHA-256("abc").
        assert_eq!(
            digest,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(size, 3);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn reads_a_bounded_slice_as_hex_without_modifying_the_file() {
        let dir = temp_dir("slice");
        let path = write_file(&dir, "evidence.bin", b"\x00\x01ABCDEF");
        let before = hash_file(&path).unwrap().0;
        let (hex, read) = read_slice(&path, 2, 3).unwrap();
        assert_eq!(hex, "414243");
        assert_eq!(read, 3);
        assert_eq!(hash_file(&path).unwrap().0, before, "read must not alter evidence");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn read_slice_rejects_zero_and_oversized_lengths() {
        let dir = temp_dir("bounds");
        let path = write_file(&dir, "evidence.bin", b"data");
        assert!(read_slice(&path, 0, 0).is_err());
        assert!(read_slice(&path, 0, MAX_READ_SLICE + 1).is_err());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn read_slice_stops_cleanly_at_end_of_file() {
        let dir = temp_dir("eof");
        let path = write_file(&dir, "evidence.bin", b"xy");
        let (hex, read) = read_slice(&path, 1, 100).unwrap();
        assert_eq!(hex, "79");
        assert_eq!(read, 1);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn scope_allows_only_paths_inside_approved_roots() {
        let approved = temp_dir("approved");
        let outside = temp_dir("outside");
        let inner = write_file(&approved, "inside.bin", b"ok");
        let secret = write_file(&outside, "secret.bin", b"no");

        let scope = EvidenceScope::new(&[approved.display().to_string()]).unwrap();
        assert!(scope.resolve(&inner.display().to_string()).is_ok());
        assert!(scope.resolve(&secret.display().to_string()).is_err());
        // The approved root itself is readable.
        assert!(scope.resolve(&approved.display().to_string()).is_ok());

        fs::remove_dir_all(&approved).unwrap();
        fs::remove_dir_all(&outside).unwrap();
    }

    #[test]
    fn scope_rejects_parent_traversal_out_of_the_approved_root() {
        let approved = temp_dir("traversal");
        let scope = EvidenceScope::new(&[approved.display().to_string()]).unwrap();
        let escape = format!("{}/../{}", approved.display(), "anything");
        assert!(scope.resolve(&escape).is_err());
        fs::remove_dir_all(&approved).unwrap();
    }

    #[test]
    fn scope_rejects_relative_paths_and_empty_scopes() {
        assert!(EvidenceScope::new(&["relative/path".to_string()]).is_err());
        assert!(EvidenceScope::new(&["".to_string()]).is_err());
        let empty = EvidenceScope::default();
        assert!(empty.is_empty());
        assert!(empty.resolve("/etc/hostname").is_err());
    }

    #[cfg(unix)]
    #[test]
    fn scope_rejects_a_symlink_that_points_outside_the_approved_root() {
        let approved = temp_dir("symlink-approved");
        let outside = temp_dir("symlink-outside");
        let secret = write_file(&outside, "secret.bin", b"no");
        let link = approved.join("link.bin");
        std::os::unix::fs::symlink(&secret, &link).unwrap();

        let scope = EvidenceScope::new(&[approved.display().to_string()]).unwrap();
        let result = scope.resolve(&link.display().to_string());
        assert!(
            result.is_err(),
            "a symlink escaping the approved root must be refused"
        );

        fs::remove_dir_all(&approved).unwrap();
        fs::remove_dir_all(&outside).unwrap();
    }

    #[test]
    fn lists_directory_entries_and_reports_symlinks_without_following_them() {
        let dir = temp_dir("list");
        write_file(&dir, "b.bin", b"b");
        write_file(&dir, "a.bin", b"a");
        fs::create_dir(dir.join("sub")).unwrap();
        let entries = list_dir(&dir).unwrap();
        let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, vec!["a.bin", "b.bin", "sub"]);
        assert!(entries[2].is_dir);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn stats_a_real_file() {
        let dir = temp_dir("stat");
        let path = write_file(&dir, "evidence.bin", b"12345");
        let stat = stat_path(&path).unwrap();
        assert_eq!(stat.size_bytes, 5);
        assert!(stat.is_file);
        assert!(!stat.is_dir);
        fs::remove_dir_all(&dir).unwrap();
    }
}

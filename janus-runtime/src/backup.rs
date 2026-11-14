//! Encrypted SQLite snapshot and restore support. The recovery key is never stored in Janus.
use anyhow::{bail, Context, Result};
use chacha20poly1305::{aead::{Aead, KeyInit, Payload}, XChaCha20Poly1305, XNonce};
use rand_core::{OsRng, RngCore};
use rusqlite::{backup::Backup, Connection};
use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    time::Duration,
};
use tracing::{error, info, warn};

const MAGIC: &[u8; 8] = b"JANUSBK1";
const AAD: &[u8] = b"JANUS-ARIA-SQLITE-BACKUP-V1";
const NONCE_LEN: usize = 24;
const MAX_DATABASE_BYTES: u64 = 512 * 1024 * 1024;

pub fn create_encrypted_backup(source: &Path, destination: &Path, key: &[u8; 32]) -> Result<()> {
    if !source.is_file() {
        bail!("source database does not exist or is not a regular file");
    }
    if destination.exists() {
        bail!("backup destination already exists");
    }

    let source_db = Connection::open(source)
        .with_context(|| format!("opening {}", source.display()))?;
    let snapshot_path = sibling_temp_path(&std::env::temp_dir().join("janus-snapshot.sqlite"), "snapshot");
    let snapshot_result = (|| -> Result<Vec<u8>> {
        // Create the snapshot file with owner-only permissions before SQLite opens it.
        drop(create_new_private_file(&snapshot_path)?);
        let mut snapshot = Connection::open(&snapshot_path)?;
        {
            let mut backup = Backup::new(&source_db, &mut snapshot)?;
            backup.run_to_completion(100, Duration::from_millis(10), None)?;
        }
        snapshot.close().map_err(|(_, error)| error)?;
        let size = fs::metadata(&snapshot_path)?.len();
        if size == 0 || size > MAX_DATABASE_BYTES {
            bail!("database snapshot must be between 1 byte and 512 MiB");
        }
        Ok(fs::read(&snapshot_path)?)
    })();
    let _ = fs::remove_file(&snapshot_path);
    let plaintext = snapshot_result?;

    let cipher = XChaCha20Poly1305::new(key.into());
    let mut nonce_bytes = [0u8; NONCE_LEN];
    OsRng.fill_bytes(&mut nonce_bytes);
    let ciphertext = cipher
        .encrypt(
            XNonce::from_slice(&nonce_bytes),
            Payload { msg: &plaintext, aad: AAD },
        )
        .map_err(|_| anyhow::anyhow!("backup encryption failed"))?;
    let mut encoded = Vec::with_capacity(MAGIC.len() + NONCE_LEN + ciphertext.len());
    encoded.extend_from_slice(MAGIC);
    encoded.extend_from_slice(&nonce_bytes);
    encoded.extend_from_slice(&ciphertext);
    write_new_file_atomically(destination, &encoded)?;
    info!("Encrypted Janus database backup created: {}", destination.display());
    Ok(())
}

pub fn restore_encrypted_backup(
    backup_path: &Path,
    destination: &Path,
    key: &[u8; 32],
) -> Result<()> {
    if destination.exists() {
        bail!("restore destination already exists; refusing to overwrite data");
    }
    let encoded = fs::read(backup_path)?;
    if encoded.len() < MAGIC.len() + NONCE_LEN + 16 || &encoded[..MAGIC.len()] != MAGIC {
        bail!("not a supported Janus encrypted backup");
    }
    let nonce = XNonce::from_slice(&encoded[MAGIC.len()..MAGIC.len() + NONCE_LEN]);
    let cipher = XChaCha20Poly1305::new(key.into());
    let plaintext = cipher
        .decrypt(
            nonce,
            Payload {
                msg: &encoded[MAGIC.len() + NONCE_LEN..],
                aad: AAD,
            },
        )
        .map_err(|_| anyhow::anyhow!("backup authentication failed; wrong key or damaged file"))?;
    if plaintext.is_empty() || plaintext.len() as u64 > MAX_DATABASE_BYTES {
        bail!("decrypted database exceeds supported size");
    }

    let staging = sibling_temp_path(destination, "restore");
    let result = (|| -> Result<()> {
        write_new_file(&staging, &plaintext)?;
        let check = Connection::open(&staging)?;
        // A replacement host must not inherit active sessions or physical-device trust.
        // Preserve device ownership records, but require every device to prove possession
        // of its key and complete enrollment again on the recovered host.
        check.execute_batch("BEGIN IMMEDIATE")?;
        for table in ["sessions", "device_enrollment_challenges", "owner_device_keys"] {
            let exists: i64 = check.query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1)",
                [table],
                |row| row.get(0),
            )?;
            if exists == 1 {
                check.execute(&format!("DELETE FROM {table}"), [])?;
            }
        }
        check.execute_batch("COMMIT")?;
        let integrity: String =
            check.query_row("PRAGMA integrity_check", [], |row| row.get(0))?;
        if integrity != "ok" {
            bail!("restored SQLite database failed integrity_check: {integrity}");
        }
        check.close().map_err(|(_, error)| error)?;
        fs::hard_link(&staging, destination)
            .with_context(|| format!("installing restored database at {}", destination.display()))?;
        Ok(())
    })();
    let _ = fs::remove_file(&staging);
    result?;
    info!(
        "Encrypted Janus database restored and verified: {}",
        destination.display()
    );
    Ok(())
}

/// Restore a backup using JANUS_BACKUP_KEY (Base64 for exactly 32 random bytes).
/// This is used by the offline recovery CLI before the runtime or API server starts.
pub fn restore_from_environment(backup_path: &Path, destination: &Path) -> Result<()> {
    use base64::Engine;
    let encoded = std::env::var("JANUS_BACKUP_KEY")
        .context("JANUS_BACKUP_KEY is required for recovery")?;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(encoded.trim())
        .context("JANUS_BACKUP_KEY must be Base64 for exactly 32 random bytes")?;
    let key: [u8; 32] = bytes
        .try_into()
        .map_err(|_| anyhow::anyhow!("JANUS_BACKUP_KEY must decode to exactly 32 bytes"))?;
    restore_encrypted_backup(backup_path, destination, &key)
}

fn sibling_temp_path(path: &Path, suffix: &str) -> PathBuf {
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    path.with_file_name(format!(".{name}.{suffix}-{}", uuid::Uuid::new_v4()))
}

fn create_new_private_file(path: &Path) -> Result<File> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    // SQLite snapshots and restore staging files contain plaintext database contents.
    // On Unix, create them as owner-only from the first filesystem operation.
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    Ok(options.open(path)?)
}

fn write_new_file_atomically(path: &Path, bytes: &[u8]) -> Result<()> {
    let temp = sibling_temp_path(path, "backup");
    let result = (|| -> Result<()> {
        write_new_file(&temp, bytes)?;
        fs::hard_link(&temp, path)?;
        Ok(())
    })();
    let _ = fs::remove_file(temp);
    result
}

fn write_new_file(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut file = create_new_private_file(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}

/// Enable immediate then periodic backups when JANUS_BACKUP_KEY contains Base64 for 32 random bytes.
pub fn start_automatic_backups(database_path: PathBuf) -> Result<()> {
    let encoded = match std::env::var("JANUS_BACKUP_KEY") {
        Ok(value) if !value.trim().is_empty() => value,
        _ => {
            warn!("Automatic encrypted backups disabled: JANUS_BACKUP_KEY is not configured");
            return Ok(());
        }
    };
    use base64::Engine;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(encoded.trim())
        .context("JANUS_BACKUP_KEY must be Base64 for exactly 32 random bytes")?;
    let key: [u8; 32] = bytes
        .try_into()
        .map_err(|_| anyhow::anyhow!("JANUS_BACKUP_KEY must decode to exactly 32 bytes"))?;
    let directory = std::env::var_os("JANUS_BACKUP_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("./backups"));
    fs::create_dir_all(&directory)?;
    let minutes = std::env::var("JANUS_BACKUP_INTERVAL_MINUTES")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(60);
    if !(5..=1440).contains(&minutes) {
        bail!("JANUS_BACKUP_INTERVAL_MINUTES must be between 5 and 1440");
    }
    info!(
        "Automatic encrypted backups enabled at {} every {} minutes",
        directory.display(),
        minutes
    );
    tokio::spawn(async move {
        let mut timer = tokio::time::interval(Duration::from_secs(minutes * 60));
        loop {
            timer.tick().await;
            let source = database_path.clone();
            let output = directory.join(format!(
                "janus-{}-{}.janusbk",
                chrono::Utc::now().format("%Y%m%dT%H%M%SZ"),
                uuid::Uuid::new_v4()
            ));
            match tokio::task::spawn_blocking(move || {
                create_encrypted_backup(&source, &output, &key)
            })
            .await
            {
                Ok(Ok(())) => {}
                Ok(Err(error)) => error!("Automatic Janus backup failed: {error:#}"),
                Err(error) => error!("Backup worker failed: {error}"),
            }
        }
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::params;

    fn path(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!("janus-{tag}-{}.db", uuid::Uuid::new_v4()))
    }

    #[test]
    fn encrypted_backup_restores_consistent_database() {
        let src = path("src");
        let enc = path("enc");
        let dst = path("dst");
        let key = [42u8; 32];
        {
            let db = Connection::open(&src).unwrap();
            db.execute(
                "CREATE TABLE memories (id TEXT PRIMARY KEY, body TEXT NOT NULL)",
                [],
            )
            .unwrap();
            db.execute(
                "INSERT INTO memories VALUES (?1,?2)",
                params!["m1", "backup roundtrip"],
            )
            .unwrap();
        }
        create_encrypted_backup(&src, &enc, &key).unwrap();
        assert_ne!(fs::read(&src).unwrap(), fs::read(&enc).unwrap());
        restore_encrypted_backup(&enc, &dst, &key).unwrap();
        let db = Connection::open(&dst).unwrap();
        let body: String = db
            .query_row("SELECT body FROM memories WHERE id='m1'", [], |row| row.get(0))
            .unwrap();
        assert_eq!(body, "backup roundtrip");
        drop(db);
        for path in [src, enc, dst] {
            let _ = fs::remove_file(path);
        }
    }

    #[test]
    fn rejects_wrong_key_and_tampering_without_creating_destination() {
        let src = path("bad-src");
        let enc = path("bad-enc");
        let dst = path("bad-dst");
        Connection::open(&src)
            .unwrap()
            .execute("CREATE TABLE t(id)", [])
            .unwrap();
        create_encrypted_backup(&src, &enc, &[7u8; 32]).unwrap();
        assert!(restore_encrypted_backup(&enc, &dst, &[8u8; 32]).is_err());
        let mut bytes = fs::read(&enc).unwrap();
        let last = bytes.len() - 1;
        bytes[last] ^= 0x80;
        fs::write(&enc, bytes).unwrap();
        assert!(restore_encrypted_backup(&enc, &dst, &[7u8; 32]).is_err());
        assert!(!dst.exists());
        for path in [src, enc, dst] {
            let _ = fs::remove_file(path);
        }
    }

    #[test]
    fn restore_refuses_to_overwrite_existing_file() {
        let src = path("keep-src");
        let enc = path("keep-enc");
        let dst = path("keep-dst");
        Connection::open(&src)
            .unwrap()
            .execute("CREATE TABLE t(id)", [])
            .unwrap();
        Connection::open(&dst)
            .unwrap()
            .execute("CREATE TABLE keep(id)", [])
            .unwrap();
        create_encrypted_backup(&src, &enc, &[9u8; 32]).unwrap();
        assert!(restore_encrypted_backup(&enc, &dst, &[9u8; 32]).is_err());
        for path in [src, enc, dst] {
            let _ = fs::remove_file(path);
        }
    }

    #[cfg(unix)]
    #[test]
    fn plaintext_snapshot_and_restore_staging_are_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let source = path("permissions-src");
        let backup = path("permissions-backup");
        let restored = path("permissions-restored");
        Connection::open(&source)
            .unwrap()
            .execute("CREATE TABLE private_data(value TEXT)", [])
            .unwrap();
        create_encrypted_backup(&source, &backup, &[3u8; 32]).unwrap();
        restore_encrypted_backup(&backup, &restored, &[3u8; 32]).unwrap();
        assert_eq!(fs::metadata(&restored).unwrap().permissions().mode() & 0o777, 0o600);
        for path in [source, backup, restored] {
            let _ = fs::remove_file(path);
        }
    }

    #[test]
    fn missing_source_database_is_not_silently_created() {
        let source = path("missing-source");
        let backup = path("missing-backup");
        assert!(create_encrypted_backup(&source, &backup, &[1u8; 32]).is_err());
        assert!(!source.exists());
        assert!(!backup.exists());
    }
    #[test]
    fn recovery_invalidates_sessions_challenges_and_device_key_trust() {
        let source = path("recovery-trust-src");
        let encrypted = path("recovery-trust-enc");
        let restored = path("recovery-trust-restored");
        {
            let db = Connection::open(&source).unwrap();
            db.execute_batch(
                "CREATE TABLE sessions(token_hash TEXT PRIMARY KEY);
                 CREATE TABLE device_enrollment_challenges(challenge_id TEXT PRIMARY KEY);
                 CREATE TABLE owner_device_keys(device_id TEXT PRIMARY KEY);",
            ).unwrap();
            db.execute("INSERT INTO sessions VALUES ('old-session')", []).unwrap();
            db.execute("INSERT INTO device_enrollment_challenges VALUES ('old-challenge')", []).unwrap();
            db.execute("INSERT INTO owner_device_keys VALUES ('old-device')", []).unwrap();
        }
        create_encrypted_backup(&source, &encrypted, &[6u8; 32]).unwrap();
        restore_encrypted_backup(&encrypted, &restored, &[6u8; 32]).unwrap();
        let db = Connection::open(&restored).unwrap();
        for table in ["sessions", "device_enrollment_challenges", "owner_device_keys"] {
            let count: i64 = db.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| row.get(0)).unwrap();
            assert_eq!(count, 0, "{table} must be cleared on recovery");
        }
        drop(db);
        for file in [source, encrypted, restored] {
            let _ = fs::remove_file(file);
        }
    }

}

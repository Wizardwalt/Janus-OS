//! Encrypted SQLite snapshot and restore support. The recovery key is never stored in Janus.
use anyhow::{bail, Context, Result};
use chacha20poly1305::{aead::{Aead, KeyInit, Payload}, XChaCha20Poly1305, XNonce};
use rand_core::{OsRng, RngCore};
use rusqlite::{backup::Backup, Connection};
use std::{fs::{self, OpenOptions}, io::Write, path::{Path, PathBuf}, time::Duration};
use tracing::{error, info, warn};
const MAGIC: &[u8; 8] = b"JANUSBK1";
const AAD: &[u8] = b"JANUS-ARIA-SQLITE-BACKUP-V1";
const NONCE_LEN: usize = 24;
const MAX_DATABASE_BYTES: u64 = 512 * 1024 * 1024;

pub fn create_encrypted_backup(source: &Path, destination: &Path, key: &[u8; 32]) -> Result<()> {
    if destination.exists() { bail!("backup destination already exists"); }
    let source_db = Connection::open(source).with_context(|| format!("opening {}", source.display()))?;
    let snapshot_path = std::env::temp_dir().join(format!("janus-snapshot-{}.sqlite", uuid::Uuid::new_v4()));
    let snapshot_result = (|| -> Result<Vec<u8>> {
        let mut snapshot = Connection::open(&snapshot_path)?;
        { let mut backup = Backup::new(&source_db, &mut snapshot)?; backup.run_to_completion(100, Duration::from_millis(10), None)?; }
        snapshot.close().map_err(|(_, error)| error)?;
        let size = fs::metadata(&snapshot_path)?.len();
        if size == 0 || size > MAX_DATABASE_BYTES { bail!("database snapshot must be between 1 byte and 512 MiB"); }
        Ok(fs::read(&snapshot_path)?)
    })();
    let _ = fs::remove_file(&snapshot_path);
    let plaintext = snapshot_result?;
    let cipher = XChaCha20Poly1305::new(key.into());
    let mut nonce_bytes = [0u8; NONCE_LEN]; OsRng.fill_bytes(&mut nonce_bytes);
    let ciphertext = cipher.encrypt(XNonce::from_slice(&nonce_bytes), Payload { msg: &plaintext, aad: AAD })
        .map_err(|_| anyhow::anyhow!("backup encryption failed"))?;
    let mut encoded = Vec::with_capacity(MAGIC.len() + NONCE_LEN + ciphertext.len());
    encoded.extend_from_slice(MAGIC); encoded.extend_from_slice(&nonce_bytes); encoded.extend_from_slice(&ciphertext);
    write_new_file_atomically(destination, &encoded)?;
    info!("Encrypted Janus database backup created: {}", destination.display()); Ok(())
}

pub fn restore_encrypted_backup(backup_path: &Path, destination: &Path, key: &[u8; 32]) -> Result<()> {
    if destination.exists() { bail!("restore destination already exists; refusing to overwrite data"); }
    let encoded = fs::read(backup_path)?;
    if encoded.len() < MAGIC.len() + NONCE_LEN + 16 || &encoded[..MAGIC.len()] != MAGIC { bail!("not a supported Janus encrypted backup"); }
    let nonce = XNonce::from_slice(&encoded[MAGIC.len()..MAGIC.len() + NONCE_LEN]);
    let cipher = XChaCha20Poly1305::new(key.into());
    let plaintext = cipher.decrypt(nonce, Payload { msg: &encoded[MAGIC.len() + NONCE_LEN..], aad: AAD })
        .map_err(|_| anyhow::anyhow!("backup authentication failed; wrong key or damaged file"))?;
    if plaintext.is_empty() || plaintext.len() as u64 > MAX_DATABASE_BYTES { bail!("decrypted database exceeds supported size"); }
    let staging = sibling_temp_path(destination, "restore");
    let result = (|| -> Result<()> {
        write_new_file(&staging, &plaintext)?;
        let mut check = Connection::open(&staging)?;
        // A recovered host must not inherit live sessions or trust bindings from the lost host.
        // Device public-key enrollment is intentionally cleared so every physical device re-enrolls.
        check.execute_batch(
            "BEGIN IMMEDIATE;\n             DELETE FROM sessions;\n             DELETE FROM device_enrollment_challenges;\n             DELETE FROM owner_device_keys;\n             COMMIT;"
        )?;
        let integrity: String = check.query_row("PRAGMA integrity_check", [], |row| row.get(0))?;
        if integrity != "ok" { bail!("restored SQLite database failed integrity_check: {integrity}"); }
        check.close().map_err(|(_, error)| error)?;
        fs::hard_link(&staging, destination).with_context(|| format!("installing restored database at {}", destination.display()))?;
        Ok(())
    })();
    let _ = fs::remove_file(&staging); result?;
    info!("Encrypted Janus database restored and verified: {}", destination.display()); Ok(())
}

fn sibling_temp_path(path: &Path, suffix: &str) -> PathBuf {
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    path.with_file_name(format!(".{name}.{suffix}-{}", uuid::Uuid::new_v4()))
}
fn write_new_file_atomically(path: &Path, bytes: &[u8]) -> Result<()> {
    let temp = sibling_temp_path(path, "backup");
    let result = (|| -> Result<()> { write_new_file(&temp, bytes)?; fs::hard_link(&temp, path)?; Ok(()) })();
    let _ = fs::remove_file(temp); result
}
fn write_new_file(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
    file.write_all(bytes)?; file.sync_all()?; Ok(())
}

/// Enable immediate then periodic backups when JANUS_BACKUP_KEY contains Base64 for 32 random bytes.
pub fn start_automatic_backups(database_path: PathBuf) -> Result<()> {
    let encoded = match std::env::var("JANUS_BACKUP_KEY") {
        Ok(value) if !value.trim().is_empty() => value,
        _ => { warn!("Automatic encrypted backups disabled: JANUS_BACKUP_KEY is not configured"); return Ok(()); }
    };
    use base64::Engine;
    let bytes = base64::engine::general_purpose::STANDARD.decode(encoded.trim()).context("JANUS_BACKUP_KEY must be Base64 for exactly 32 random bytes")?;
    let key: [u8; 32] = bytes.try_into().map_err(|_| anyhow::anyhow!("JANUS_BACKUP_KEY must decode to exactly 32 bytes"))?;
    let directory = std::env::var_os("JANUS_BACKUP_DIR").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("./backups"));
    fs::create_dir_all(&directory)?;
    let minutes = std::env::var("JANUS_BACKUP_INTERVAL_MINUTES").ok().and_then(|v| v.parse::<u64>().ok()).unwrap_or(60);
    if !(5..=1440).contains(&minutes) { bail!("JANUS_BACKUP_INTERVAL_MINUTES must be between 5 and 1440"); }
    info!("Automatic encrypted backups enabled at {} every {} minutes", directory.display(), minutes);
    tokio::spawn(async move {
        let mut timer = tokio::time::interval(Duration::from_secs(minutes * 60));
        loop {
            timer.tick().await;
            let source = database_path.clone();
            let output = directory.join(format!("janus-{}-{}.janusbk", chrono::Utc::now().format("%Y%m%dT%H%M%SZ"), uuid::Uuid::new_v4()));
            match tokio::task::spawn_blocking(move || create_encrypted_backup(&source, &output, &key)).await {
                Ok(Ok(())) => {}, Ok(Err(e)) => error!("Automatic Janus backup failed: {e:#}"), Err(e) => error!("Backup worker failed: {e}"),
            }
        }
    }); Ok(())
}

#[cfg(test)] mod tests {
    use super::*;
    use rusqlite::params;
    fn path(tag: &str) -> PathBuf { std::env::temp_dir().join(format!("janus-{tag}-{}.db", uuid::Uuid::new_v4())) }
    #[test] fn encrypted_backup_restores_consistent_database() {
        let src=path("src"); let enc=path("enc"); let dst=path("dst"); let key=[42u8;32];
        { let db=Connection::open(&src).unwrap(); db.execute("CREATE TABLE memories (id TEXT PRIMARY KEY, body TEXT NOT NULL)",[]).unwrap(); db.execute("INSERT INTO memories VALUES (?1,?2)",params!["m1","backup roundtrip"]).unwrap(); }
        create_encrypted_backup(&src,&enc,&key).unwrap(); assert_ne!(fs::read(&src).unwrap(),fs::read(&enc).unwrap());
        restore_encrypted_backup(&enc,&dst,&key).unwrap(); let db=Connection::open(&dst).unwrap();
        let body:String=db.query_row("SELECT body FROM memories WHERE id='m1'",[],|r|r.get(0)).unwrap(); assert_eq!(body,"backup roundtrip"); drop(db);
        for p in [src,enc,dst] { let _=fs::remove_file(p); }
    }
    #[test] fn rejects_wrong_key_and_tampering_without_creating_destination() {
        let src=path("bad-src"); let enc=path("bad-enc"); let dst=path("bad-dst"); Connection::open(&src).unwrap().execute("CREATE TABLE t(id)",[]).unwrap();
        create_encrypted_backup(&src,&enc,&[7u8;32]).unwrap(); assert!(restore_encrypted_backup(&enc,&dst,&[8u8;32]).is_err());
        let mut bytes=fs::read(&enc).unwrap(); let last=bytes.len()-1; bytes[last]^=0x80; fs::write(&enc,bytes).unwrap();
        assert!(restore_encrypted_backup(&enc,&dst,&[7u8;32]).is_err()); assert!(!dst.exists());
        for p in [src,enc,dst] { let _=fs::remove_file(p); }
    }
    #[test] fn restore_clears_sessions_and_device_trust_bindings() {
        let src=path("sanitize-src"); let enc=path("sanitize-enc"); let dst=path("sanitize-dst");
        {
            let db=Connection::open(&src).unwrap();
            db.execute_batch("CREATE TABLE sessions(token_hash TEXT PRIMARY KEY); CREATE TABLE device_enrollment_challenges(challenge_id TEXT PRIMARY KEY); CREATE TABLE owner_device_keys(device_id TEXT PRIMARY KEY);").unwrap();
            db.execute("INSERT INTO sessions VALUES ('old-session')",[]).unwrap();
            db.execute("INSERT INTO device_enrollment_challenges VALUES ('old-challenge')",[]).unwrap();
            db.execute("INSERT INTO owner_device_keys VALUES ('old-device')",[]).unwrap();
        }
        create_encrypted_backup(&src,&enc,&[5u8;32]).unwrap();
        restore_encrypted_backup(&enc,&dst,&[5u8;32]).unwrap();
        let db=Connection::open(&dst).unwrap();
        for table in ["sessions","device_enrollment_challenges","owner_device_keys"] {
            let count:i64=db.query_row(&format!("SELECT COUNT(*) FROM {table}"),[],|r|r.get(0)).unwrap();
            assert_eq!(count,0,"{table} should be cleared on recovery");
        }
        drop(db); for p in [src,enc,dst] { let _=fs::remove_file(p); }
    }
    #[test] fn restore_refuses_to_overwrite_existing_file() {
        let src=path("keep-src"); let enc=path("keep-enc"); let dst=path("keep-dst"); Connection::open(&src).unwrap().execute("CREATE TABLE t(id)",[]).unwrap();
        Connection::open(&dst).unwrap().execute("CREATE TABLE keep(id)",[]).unwrap(); create_encrypted_backup(&src,&enc,&[9u8;32]).unwrap();
        assert!(restore_encrypted_backup(&enc,&dst,&[9u8;32]).is_err()); for p in [src,enc,dst] { let _=fs::remove_file(p); }
    }
}

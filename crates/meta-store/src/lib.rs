use std::cell::RefCell;
use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::time::Duration;

use argon2::{Algorithm, Argon2, Params, Version};
use chacha20poly1305::{
    aead::{Aead, KeyInit, Payload},
    XChaCha20Poly1305, XNonce,
};
pub use core_domain::{
    Candidate, CandidateId, ContactHash, Document, DocumentId, DocumentStatus, EntityMention,
    EntityMentionId, EntityType, FileExtension, ImportTaskId, IndexStateStatus, IngestJobId,
    IngestJobKind, IngestJobStatus, ResumeVersion, ResumeVersionId, ResumeVisibility, SectionId,
    UnixTimestamp,
};
use rusqlite::{params, params_from_iter, types::Value, Connection, OptionalExtension, Row};

mod classification;

pub use classification::{
    ClassificationStatus, DocumentClassificationCounts, DocumentClassificationRecord, ReasonCode,
    ReviewDisposition,
};
pub use resume_classifier::{
    classify as classify_resume, ClassificationResult, ClassifierInput, CLASSIFIER_EPOCH,
};

const SCHEMA_VERSION_V1: u32 = 1;
const SCHEMA_VERSION_V2: u32 = 2;
const SCHEMA_VERSION_V3: u32 = 3;
const SCHEMA_VERSION_V4: u32 = 4;
const SCHEMA_VERSION_V5: u32 = 5;
const SCHEMA_VERSION_V6: u32 = 6;
const SCHEMA_VERSION_V7: u32 = 7;
const SCHEMA_VERSION_V8: u32 = 8;
const SCHEMA_VERSION_V9: u32 = 9;
const SCHEMA_VERSION_V10: u32 = 10;
const SCHEMA_VERSION_V11: u32 = 11;
const SCHEMA_VERSION_V12: u32 = 12;
const SCHEMA_VERSION_V13: u32 = 13;
const SCHEMA_VERSION_V14: u32 = 14;
const SCHEMA_VERSION_V15: u32 = 15;
const SCHEMA_VERSION_V16: u32 = 16;
const SCHEMA_VERSION_V17: u32 = 17;
const SCHEMA_VERSION_V18: u32 = 18;
const SCHEMA_VERSION_V19: u32 = 19;
const SCHEMA_VERSION_V20: u32 = 20;
const SCHEMA_VERSION_V21: u32 = 21;
const SCHEMA_VERSION_V22: u32 = 22;
const QUERY_OBSERVATION_RETENTION_ROWS: i64 = 10_000;
const METADATA_STORE_FILE: &str = "metadata.sqlite3";
const METADATA_ENCRYPTION_KEY_LEN: usize = 32;
const METADATA_ENCRYPTION_KEY_HEX_LEN: usize = METADATA_ENCRYPTION_KEY_LEN * 2;
const METADATA_ENCRYPTION_KEY_PATH: &[&str] = &["metadata-secrets", "metadata-sqlcipher-key-v1"];
const METADATA_ENCRYPTION_KEY_BACKUP_SCHEMA_VERSION: &str =
    "resume-ir-metadata-sqlcipher-key-backup-v1";
const BACKUP_PASSPHRASE_MIN_BYTES: usize = 12;
const BACKUP_SALT_LEN: usize = 16;
const BACKUP_NONCE_LEN: usize = 24;
const BACKUP_KDF_MEMORY_KIB: u32 = 19 * 1024;
const BACKUP_KDF_ITERATIONS: u32 = 2;
const BACKUP_KDF_PARALLELISM: u32 = 1;
const INDEX_STATE_KEY: &str = "default";
const CANDIDATE_COLUMNS: &str = "\
    id, primary_name, phone_hash, email_hash, dedupe_key, merge_confidence, version_count";
const DOCUMENT_COLUMNS: &str = "\
    id, source_uri, normalized_path, file_name, extension, byte_size, mtime_seconds, \
    content_hash, text_hash, is_deleted, created_at_seconds, updated_at_seconds, status";
const RESUME_VERSION_COLUMNS: &str = "\
    id, document_id, candidate_id, parse_version, schema_version, language_set_json, \
    page_count, raw_text, clean_text, quality_score, visibility";
const INGEST_JOB_COLUMNS: &str = "\
    id, document_id, resume_version_id, kind, status, attempt_count, max_attempts, \
    queued_at_seconds, started_at_seconds, finished_at_seconds, updated_at_seconds, \
    failure_kind";
const INGEST_JOB_COLUMNS_JOB_ALIAS: &str = "\
    job.id, job.document_id, job.resume_version_id, job.kind, job.status, \
    job.attempt_count, job.max_attempts, job.queued_at_seconds, \
    job.started_at_seconds, job.finished_at_seconds, job.updated_at_seconds, \
    job.failure_kind";
const IMPORT_TASK_COLUMNS: &str = "\
    id, root_path, status, queued_at_seconds, started_at_seconds, finished_at_seconds, \
    updated_at_seconds";
const ENTITY_MENTION_COLUMNS: &str = "\
    id, resume_version_id, section_id, entity_type, raw_value, normalized_value, \
    span_start, span_end, confidence, extractor";
const OCR_PAGE_CACHE_COLUMNS: &str = "\
    file_content_hash, page_no, render_dpi, ocr_lang, ocr_profile, text, confidence, \
    engine_profile, duration_ms, status, error_kind, updated_at_seconds, word_boxes_json";
const WORKER_TASK_CONTROL_COLUMNS: &str = "task_kind, paused, updated_at_seconds";
const IMPORT_SCAN_SCOPE_COLUMNS: &str = "\
    import_task_id, root_kind, root_preset, scan_profile, requested_root_path, \
    canonical_root_path, files_discovered, ignored_entries, scan_errors, \
    searchable_documents, ocr_required_documents, ocr_jobs_queued, failed_documents, \
    deleted_documents, scan_budget_kind, scan_budget_limit, scan_budget_observed, \
    scan_budget_exhausted, updated_at_seconds";
const IMPORT_SCAN_ERROR_COLUMNS: &str = "\
    import_task_id, error_index, kind, operation, path_digest, updated_at_seconds";

pub fn crate_name() -> &'static str {
    "meta-store"
}

pub type Result<T> = std::result::Result<T, MetaStoreError>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PendingImportTaskByRootDiagnostic {
    QueryFailure,
    RowMaterializationFailure,
}

pub fn metadata_store_path(data_dir: &Path) -> PathBuf {
    data_dir.join(METADATA_STORE_FILE)
}

pub fn metadata_encryption_key_path(data_dir: &Path) -> PathBuf {
    METADATA_ENCRYPTION_KEY_PATH
        .iter()
        .fold(data_dir.to_path_buf(), |path, component| {
            path.join(component)
        })
}

pub fn backup_metadata_encryption_key(
    data_dir: &Path,
    backup_path: &Path,
    passphrase: &[u8],
) -> Result<MetadataEncryptionKeyBackup> {
    validate_backup_passphrase(passphrase)?;
    let metadata_key = read_metadata_encryption_key(&metadata_encryption_key_path(data_dir))?;
    create_private_file_parent(backup_path)?;

    let mut salt = [0_u8; BACKUP_SALT_LEN];
    getrandom::getrandom(&mut salt).map_err(|_| MetaStoreError::random())?;
    let mut nonce = [0_u8; BACKUP_NONCE_LEN];
    getrandom::getrandom(&mut nonce).map_err(|_| MetaStoreError::random())?;
    let encryption_key = derive_backup_encryption_key(passphrase, &salt)?;
    let ciphertext = encrypt_metadata_key_backup(&encryption_key, &nonce, &metadata_key)?;

    let backup = format!(
        "\
{METADATA_ENCRYPTION_KEY_BACKUP_SCHEMA_VERSION}
kdf=argon2id
kdf_memory_kib={BACKUP_KDF_MEMORY_KIB}
kdf_iterations={BACKUP_KDF_ITERATIONS}
kdf_parallelism={BACKUP_KDF_PARALLELISM}
cipher=xchacha20poly1305
salt={}
nonce={}
ciphertext={}
",
        encode_hex(&salt),
        encode_hex(&nonce),
        encode_hex(&ciphertext)
    );
    write_new_private_file(backup_path, backup.as_bytes()).map_err(MetaStoreError::io_storage)?;
    restrict_private_file_permissions(backup_path)?;

    Ok(MetadataEncryptionKeyBackup { _private: () })
}

pub fn restore_metadata_encryption_key(
    data_dir: &Path,
    backup_path: &Path,
    passphrase: &[u8],
) -> Result<MetadataEncryptionKeyRestore> {
    validate_backup_passphrase(passphrase)?;
    let key_path = metadata_encryption_key_path(data_dir);
    if key_path.try_exists().map_err(MetaStoreError::io_storage)? {
        return Err(MetaStoreError::key_already_exists());
    }

    let metadata_key = read_backup_metadata_encryption_key(backup_path, passphrase)?;
    let parent = key_path
        .parent()
        .ok_or_else(|| MetaStoreError::invalid_value("metadata.encryption_key_path"))?;
    fs::create_dir_all(parent).map_err(MetaStoreError::io_storage)?;
    write_new_private_file(&key_path, encode_hex(&metadata_key).as_bytes())
        .map_err(MetaStoreError::io_storage)?;
    restrict_private_file_permissions(&key_path)?;

    Ok(MetadataEncryptionKeyRestore { _private: () })
}

pub fn rotate_metadata_encryption_key(data_dir: &Path) -> Result<MetadataEncryptionKeyRotation> {
    let key_path = metadata_encryption_key_path(data_dir);
    let old_key = read_metadata_encryption_key(&key_path)?;
    let db_path = metadata_store_path(data_dir);

    let connection = Connection::open(&db_path).map_err(MetaStoreError::storage)?;
    apply_sqlcipher_key(&connection, &old_key)?;
    verify_sqlcipher_key(&connection)?;

    let new_key = random_metadata_encryption_key()?;
    apply_sqlcipher_rekey(&connection, &new_key)?;
    verify_sqlcipher_key(&connection)?;

    replace_private_file(&key_path, encode_hex(&new_key).as_bytes())
        .map_err(MetaStoreError::io_storage)?;
    restrict_private_file_permissions(&key_path)?;

    Ok(MetadataEncryptionKeyRotation { _private: () })
}

fn validate_metadata_encryption_key(key: &[u8]) -> Result<()> {
    if key.len() != METADATA_ENCRYPTION_KEY_LEN {
        return Err(MetaStoreError::invalid_value("metadata.encryption_key"));
    }

    Ok(())
}

fn apply_sqlcipher_key(connection: &Connection, key: &[u8]) -> Result<()> {
    let key_hex = encode_hex(key);
    connection
        .execute_batch(&format!("PRAGMA key = \"x'{key_hex}'\";"))
        .map_err(MetaStoreError::storage)
}

fn apply_sqlcipher_rekey(connection: &Connection, key: &[u8]) -> Result<()> {
    let key_hex = encode_hex(key);
    connection
        .execute_batch(&format!("PRAGMA rekey = \"x'{key_hex}'\";"))
        .map_err(MetaStoreError::storage)
}

fn verify_sqlcipher_key(connection: &Connection) -> Result<()> {
    connection
        .query_row("SELECT count(*) FROM sqlite_master", [], |row| {
            row.get::<_, i64>(0)
        })
        .map(|_| ())
        .map_err(MetaStoreError::storage)
}

fn encode_hex(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write;
        let _ = write!(&mut output, "{byte:02x}");
    }
    output
}

fn decode_metadata_key_hex(value: &str) -> Result<[u8; METADATA_ENCRYPTION_KEY_LEN]> {
    if value.len() != METADATA_ENCRYPTION_KEY_HEX_LEN {
        return Err(MetaStoreError::invalid_value("metadata.encryption_key"));
    }

    let mut key = [0_u8; METADATA_ENCRYPTION_KEY_LEN];
    for (index, slot) in key.iter_mut().enumerate() {
        let start = index * 2;
        *slot = u8::from_str_radix(&value[start..start + 2], 16)
            .map_err(|_| MetaStoreError::invalid_value("metadata.encryption_key"))?;
    }

    Ok(key)
}

fn load_or_create_metadata_encryption_key(
    data_dir: &Path,
) -> Result<[u8; METADATA_ENCRYPTION_KEY_LEN]> {
    let key_path = metadata_encryption_key_path(data_dir);
    if key_path.exists() {
        return read_metadata_encryption_key(&key_path);
    }

    let parent = key_path
        .parent()
        .ok_or_else(|| MetaStoreError::invalid_value("metadata.encryption_key_path"))?;
    fs::create_dir_all(parent).map_err(MetaStoreError::io_storage)?;

    let key = random_metadata_encryption_key()?;
    match write_new_private_file(&key_path, encode_hex(&key).as_bytes()) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            return read_metadata_encryption_key(&key_path);
        }
        Err(error) => return Err(MetaStoreError::io_storage(error)),
    }
    restrict_private_file_permissions(&key_path)?;

    Ok(key)
}

fn random_metadata_encryption_key() -> Result<[u8; METADATA_ENCRYPTION_KEY_LEN]> {
    let mut key = [0_u8; METADATA_ENCRYPTION_KEY_LEN];
    getrandom::getrandom(&mut key).map_err(|_| MetaStoreError::random())?;
    Ok(key)
}

fn read_metadata_encryption_key(path: &Path) -> Result<[u8; METADATA_ENCRYPTION_KEY_LEN]> {
    restrict_private_file_permissions(path)?;
    let key_hex = fs::read_to_string(path).map_err(MetaStoreError::io_storage)?;
    decode_metadata_key_hex(key_hex.trim())
}

fn read_backup_metadata_encryption_key(
    backup_path: &Path,
    passphrase: &[u8],
) -> Result<[u8; METADATA_ENCRYPTION_KEY_LEN]> {
    let backup = fs::read_to_string(backup_path).map_err(MetaStoreError::io_storage)?;
    let mut lines = backup.lines();
    if lines.next() != Some(METADATA_ENCRYPTION_KEY_BACKUP_SCHEMA_VERSION) {
        return Err(MetaStoreError::invalid_backup());
    }
    let fields = parse_backup_fields(lines)?;
    require_backup_field(&fields, "kdf", "argon2id")?;
    require_backup_field(
        &fields,
        "kdf_memory_kib",
        &BACKUP_KDF_MEMORY_KIB.to_string(),
    )?;
    require_backup_field(
        &fields,
        "kdf_iterations",
        &BACKUP_KDF_ITERATIONS.to_string(),
    )?;
    require_backup_field(
        &fields,
        "kdf_parallelism",
        &BACKUP_KDF_PARALLELISM.to_string(),
    )?;
    require_backup_field(&fields, "cipher", "xchacha20poly1305")?;

    let salt = decode_fixed_backup_hex::<BACKUP_SALT_LEN>(required_backup_value(&fields, "salt")?)?;
    let nonce =
        decode_fixed_backup_hex::<BACKUP_NONCE_LEN>(required_backup_value(&fields, "nonce")?)?;
    let ciphertext = decode_backup_hex(required_backup_value(&fields, "ciphertext")?)?;
    let encryption_key = derive_backup_encryption_key(passphrase, &salt)?;

    decrypt_metadata_key_backup(&encryption_key, &nonce, &ciphertext)
}

fn create_private_file_parent(path: &Path) -> Result<()> {
    let Some(parent) = path.parent() else {
        return Ok(());
    };
    if parent.as_os_str().is_empty() {
        return Ok(());
    }

    fs::create_dir_all(parent).map_err(MetaStoreError::io_storage)
}

fn validate_backup_passphrase(passphrase: &[u8]) -> Result<()> {
    if passphrase.len() < BACKUP_PASSPHRASE_MIN_BYTES
        || passphrase.iter().all(u8::is_ascii_whitespace)
    {
        return Err(MetaStoreError::weak_passphrase());
    }

    Ok(())
}

fn derive_backup_encryption_key(
    passphrase: &[u8],
    salt: &[u8; BACKUP_SALT_LEN],
) -> Result<[u8; METADATA_ENCRYPTION_KEY_LEN]> {
    let params = Params::new(
        BACKUP_KDF_MEMORY_KIB,
        BACKUP_KDF_ITERATIONS,
        BACKUP_KDF_PARALLELISM,
        Some(METADATA_ENCRYPTION_KEY_LEN),
    )
    .map_err(|_| MetaStoreError::crypto())?;
    let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let mut key = [0_u8; METADATA_ENCRYPTION_KEY_LEN];
    argon2
        .hash_password_into(passphrase, salt, &mut key)
        .map_err(|_| MetaStoreError::crypto())?;
    Ok(key)
}

fn encrypt_metadata_key_backup(
    encryption_key: &[u8; METADATA_ENCRYPTION_KEY_LEN],
    nonce: &[u8; BACKUP_NONCE_LEN],
    metadata_key: &[u8; METADATA_ENCRYPTION_KEY_LEN],
) -> Result<Vec<u8>> {
    let cipher =
        XChaCha20Poly1305::new_from_slice(encryption_key).map_err(|_| MetaStoreError::crypto())?;
    cipher
        .encrypt(
            XNonce::from_slice(nonce),
            Payload {
                msg: metadata_key,
                aad: METADATA_ENCRYPTION_KEY_BACKUP_SCHEMA_VERSION.as_bytes(),
            },
        )
        .map_err(|_| MetaStoreError::crypto())
}

fn decrypt_metadata_key_backup(
    encryption_key: &[u8; METADATA_ENCRYPTION_KEY_LEN],
    nonce: &[u8; BACKUP_NONCE_LEN],
    ciphertext: &[u8],
) -> Result<[u8; METADATA_ENCRYPTION_KEY_LEN]> {
    let cipher = XChaCha20Poly1305::new_from_slice(encryption_key)
        .map_err(|_| MetaStoreError::invalid_backup())?;
    let plaintext = cipher
        .decrypt(
            XNonce::from_slice(nonce),
            Payload {
                msg: ciphertext,
                aad: METADATA_ENCRYPTION_KEY_BACKUP_SCHEMA_VERSION.as_bytes(),
            },
        )
        .map_err(|_| MetaStoreError::invalid_backup())?;
    plaintext
        .try_into()
        .map_err(|_| MetaStoreError::invalid_backup())
}

fn parse_backup_fields<'a>(
    lines: impl Iterator<Item = &'a str>,
) -> Result<BTreeMap<&'a str, &'a str>> {
    let mut fields = BTreeMap::new();
    for line in lines {
        if line.is_empty() {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            return Err(MetaStoreError::invalid_backup());
        };
        if key.is_empty() || value.is_empty() || fields.insert(key, value).is_some() {
            return Err(MetaStoreError::invalid_backup());
        }
    }

    Ok(fields)
}

fn require_backup_field(
    fields: &BTreeMap<&str, &str>,
    key: &'static str,
    expected: &str,
) -> Result<()> {
    if required_backup_value(fields, key)? != expected {
        return Err(MetaStoreError::invalid_backup());
    }

    Ok(())
}

fn required_backup_value<'a>(
    fields: &'a BTreeMap<&str, &str>,
    key: &'static str,
) -> Result<&'a str> {
    fields
        .get(key)
        .copied()
        .ok_or_else(MetaStoreError::invalid_backup)
}

fn decode_fixed_backup_hex<const N: usize>(value: &str) -> Result<[u8; N]> {
    let bytes = decode_backup_hex(value)?;
    bytes
        .try_into()
        .map_err(|_| MetaStoreError::invalid_backup())
}

fn decode_backup_hex(value: &str) -> Result<Vec<u8>> {
    if !value.len().is_multiple_of(2) {
        return Err(MetaStoreError::invalid_backup());
    }
    let mut bytes = Vec::with_capacity(value.len() / 2);
    let mut index = 0;
    while index < value.len() {
        let byte = u8::from_str_radix(&value[index..index + 2], 16)
            .map_err(|_| MetaStoreError::invalid_backup())?;
        bytes.push(byte);
        index += 2;
    }

    Ok(bytes)
}

fn metadata_store_has_plaintext_header(path: &Path) -> Result<bool> {
    if !path.try_exists().map_err(MetaStoreError::io_storage)? {
        return Ok(false);
    }

    let mut file = fs::File::open(path).map_err(MetaStoreError::io_storage)?;
    let mut header = [0_u8; 16];
    let bytes_read = file.read(&mut header).map_err(MetaStoreError::io_storage)?;
    Ok(bytes_read == header.len() && header.starts_with(b"SQLite format 3"))
}

fn migrate_plaintext_metadata_store_to_encrypted(
    db_path: &Path,
    key: &[u8; METADATA_ENCRYPTION_KEY_LEN],
) -> Result<()> {
    let encrypted_temp_path =
        private_replacement_path(db_path).map_err(MetaStoreError::io_storage)?;
    let plaintext_backup_path =
        private_replacement_path(db_path).map_err(MetaStoreError::io_storage)?;

    export_plaintext_metadata_store_to_encrypted(db_path, &encrypted_temp_path, key)?;
    if let Err(error) =
        replace_plaintext_metadata_store(db_path, &encrypted_temp_path, &plaintext_backup_path)
    {
        let _ = fs::remove_file(&encrypted_temp_path);
        return Err(MetaStoreError::io_storage(error));
    }
    let _ = fs::remove_file(&plaintext_backup_path);
    remove_sqlite_sidecars(&plaintext_backup_path);

    Ok(())
}

fn export_plaintext_metadata_store_to_encrypted(
    db_path: &Path,
    encrypted_temp_path: &Path,
    key: &[u8; METADATA_ENCRYPTION_KEY_LEN],
) -> Result<()> {
    let connection = Connection::open(db_path).map_err(MetaStoreError::storage)?;
    connection
        .execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")
        .map_err(MetaStoreError::storage)?;
    let encrypted_temp_literal = sql_string_literal(encrypted_temp_path)?;
    let key_hex = encode_hex(key);
    connection
        .execute_batch(&format!(
            "\
            ATTACH DATABASE {encrypted_temp_literal} AS encrypted KEY \"x'{key_hex}'\";
            SELECT sqlcipher_export('encrypted');
            DETACH DATABASE encrypted;
            "
        ))
        .map_err(MetaStoreError::storage)?;

    let encrypted_connection =
        Connection::open(encrypted_temp_path).map_err(MetaStoreError::storage)?;
    apply_sqlcipher_key(&encrypted_connection, key)?;
    verify_sqlcipher_key(&encrypted_connection)?;
    encrypted_connection
        .query_row(
            "SELECT COALESCE(MAX(version), 0) FROM schema_migrations",
            [],
            |row| row.get::<_, i64>(0),
        )
        .map_err(MetaStoreError::storage)?;
    drop(encrypted_connection);

    Ok(())
}

fn replace_plaintext_metadata_store(
    db_path: &Path,
    encrypted_temp_path: &Path,
    plaintext_backup_path: &Path,
) -> io::Result<()> {
    fs::rename(db_path, plaintext_backup_path)?;
    remove_sqlite_sidecars(db_path);

    if let Err(error) = fs::rename(encrypted_temp_path, db_path) {
        let _ = fs::rename(plaintext_backup_path, db_path);
        return Err(error);
    }

    Ok(())
}

fn remove_sqlite_sidecars(path: &Path) {
    let _ = fs::remove_file(path.with_extension(format!(
        "{}-wal",
        path.extension().and_then(|value| value.to_str()).unwrap_or("")
    )));
    let _ = fs::remove_file(path.with_extension(format!(
        "{}-shm",
        path.extension().and_then(|value| value.to_str()).unwrap_or("")
    )));
    let _ = fs::remove_file(format!("{}-wal", path.display()));
    let _ = fs::remove_file(format!("{}-shm", path.display()));
}

fn sql_string_literal(path: &Path) -> Result<String> {
    let value = path
        .to_str()
        .ok_or_else(|| MetaStoreError::invalid_value("metadata.store_path"))?;
    Ok(format!("'{}'", value.replace('\'', "''")))
}

fn replace_private_file(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let temp_path = private_replacement_path(path)?;
    write_new_private_file(&temp_path, bytes)?;
    let replacement = replace_existing_file(&temp_path, path);
    if replacement.is_err() {
        let _ = fs::remove_file(&temp_path);
    }
    replacement
}

fn private_replacement_path(path: &Path) -> io::Result<PathBuf> {
    let parent = path.parent().unwrap_or_else(|| Path::new(""));
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "invalid private file path"))?;
    let mut suffix = [0_u8; 8];
    getrandom::getrandom(&mut suffix)
        .map_err(|error| io::Error::other(format!("private replacement random failed: {error}")))?;

    Ok(parent.join(format!(".{file_name}.tmp-{}", encode_hex(&suffix))))
}

fn replace_existing_file(source: &Path, target: &Path) -> io::Result<()> {
    #[cfg(windows)]
    if target.exists() {
        fs::remove_file(target)?;
    }

    fs::rename(source, target)
}

fn write_new_private_file(path: &Path, bytes: &[u8]) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;

        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true).mode(0o600);
        let mut file = options.open(path)?;
        file.write_all(bytes)?;
        file.write_all(b"\n")?;
        file.sync_all()?;
        Ok(())
    }
    #[cfg(not(unix))]
    {
        use std::io::Write;

        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)?;
        file.write_all(bytes)?;
        file.write_all(b"\n")?;
        file.sync_all()?;
        Ok(())
    }
}

fn restrict_private_file_permissions(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        fs::set_permissions(path, fs::Permissions::from_mode(0o600))
            .map_err(MetaStoreError::io_storage)?;
    }

    Ok(())
}

#[derive(Clone, PartialEq, Eq)]
pub struct MetadataEncryptionKeyBackup {
    _private: (),
}

impl fmt::Debug for MetadataEncryptionKeyBackup {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MetadataEncryptionKeyBackup")
            .field("key", &"<redacted>")
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct MetadataEncryptionKeyRestore {
    _private: (),
}

impl fmt::Debug for MetadataEncryptionKeyRestore {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MetadataEncryptionKeyRestore")
            .field("key", &"<redacted>")
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct MetadataEncryptionKeyRotation {
    _private: (),
}

impl fmt::Debug for MetadataEncryptionKeyRotation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MetadataEncryptionKeyRotation")
            .field("key", &"<redacted>")
            .finish()
    }
}

#[derive(Clone, Copy)]
pub struct SearchableImportDocument<'a> {
    pub document: &'a Document,
    pub version: &'a ResumeVersion,
    pub mentions: &'a [EntityMention],
    pub email_hash: Option<&'a ContactHash>,
    pub phone_hash: Option<&'a ContactHash>,
}

#[derive(Clone, Copy)]
pub struct OcrAttemptPublication<'a> {
    pub document: &'a Document,
    pub classification: &'a DocumentClassificationRecord,
    pub version: Option<&'a ResumeVersion>,
    pub mentions: &'a [EntityMention],
    pub email_hash: Option<&'a ContactHash>,
    pub phone_hash: Option<&'a ContactHash>,
    pub index_state: &'a IndexState,
}

impl fmt::Debug for OcrAttemptPublication<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("OcrAttemptPublication(<redacted>)")
    }
}

impl fmt::Debug for SearchableImportDocument<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SearchableImportDocument")
            .field("document", self.document)
            .field("version", self.version)
            .field("mention_count", &self.mentions.len())
            .field("email_hash", &self.email_hash.map(|_| "<redacted>"))
            .field("phone_hash", &self.phone_hash.map(|_| "<redacted>"))
            .finish()
    }
}

pub struct MetaStore {
    connection: RefCell<Connection>,
    metadata_encryption_state: MetadataEncryptionState,
}

impl MetaStore {
    pub fn open_data_dir(data_dir: &Path) -> Result<Self> {
        fs::create_dir_all(data_dir).map_err(MetaStoreError::io_storage)?;
        let db_path = metadata_store_path(data_dir);
        let key = load_or_create_metadata_encryption_key(data_dir)?;
        if metadata_store_has_plaintext_header(&db_path)? {
            migrate_plaintext_metadata_store_to_encrypted(&db_path, &key)?;
        }

        Self::open_encrypted(db_path, &key)
    }

    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let connection = Connection::open(path).map_err(MetaStoreError::storage)?;
        Self::from_connection(connection, true, MetadataEncryptionState::Plaintext)
    }

    pub fn open_encrypted(path: impl AsRef<Path>, key: &[u8]) -> Result<Self> {
        validate_metadata_encryption_key(key)?;
        let connection = Connection::open(path).map_err(MetaStoreError::storage)?;
        apply_sqlcipher_key(&connection, key)?;
        verify_sqlcipher_key(&connection)?;
        Self::from_connection(connection, true, MetadataEncryptionState::SqlCipher)
    }

    pub fn open_in_memory() -> Result<Self> {
        let connection = Connection::open_in_memory().map_err(MetaStoreError::storage)?;
        Self::from_connection(connection, false, MetadataEncryptionState::Plaintext)
    }

    fn from_connection(
        connection: Connection,
        file_backed: bool,
        metadata_encryption_state: MetadataEncryptionState,
    ) -> Result<Self> {
        connection
            .busy_timeout(Duration::from_millis(5_000))
            .map_err(MetaStoreError::storage)?;
        connection
            .execute_batch("PRAGMA foreign_keys = ON;")
            .map_err(MetaStoreError::storage)?;
        if file_backed {
            let journal_mode = connection
                .query_row("PRAGMA journal_mode", [], |row| row.get::<_, String>(0))
                .map_err(MetaStoreError::storage)?;
            if !journal_mode.eq_ignore_ascii_case("wal") {
                connection
                    .pragma_update(None, "journal_mode", "WAL")
                    .map_err(MetaStoreError::storage)?;
            }
        }

        Ok(Self {
            connection: RefCell::new(connection),
            metadata_encryption_state,
        })
    }

    pub fn run_migrations(&self) -> Result<MigrationReport> {
        let mut connection = self.connection.borrow_mut();
        connection
            .execute_batch(
                "\
                CREATE TABLE IF NOT EXISTS schema_migrations (
                    version INTEGER PRIMARY KEY,
                    applied_at_seconds INTEGER NOT NULL
                );",
            )
            .map_err(MetaStoreError::migration)?;

        let mut applied_versions = Vec::new();

        for (version, schema) in [
            (SCHEMA_VERSION_V1, SCHEMA_V1),
            (SCHEMA_VERSION_V2, SCHEMA_V2),
            (SCHEMA_VERSION_V3, SCHEMA_V3),
            (SCHEMA_VERSION_V4, SCHEMA_V4),
            (SCHEMA_VERSION_V5, SCHEMA_V5),
            (SCHEMA_VERSION_V6, SCHEMA_V6),
            (SCHEMA_VERSION_V7, SCHEMA_V7),
            (SCHEMA_VERSION_V8, SCHEMA_V8),
            (SCHEMA_VERSION_V9, SCHEMA_V9),
            (SCHEMA_VERSION_V10, SCHEMA_V10),
            (SCHEMA_VERSION_V11, SCHEMA_V11),
            (SCHEMA_VERSION_V12, SCHEMA_V12),
            (SCHEMA_VERSION_V13, SCHEMA_V13),
            (SCHEMA_VERSION_V14, SCHEMA_V14),
            (SCHEMA_VERSION_V15, SCHEMA_V15),
            (SCHEMA_VERSION_V16, SCHEMA_V16),
            (SCHEMA_VERSION_V17, SCHEMA_V17),
            (SCHEMA_VERSION_V18, SCHEMA_V18),
            (SCHEMA_VERSION_V19, SCHEMA_V19),
            (SCHEMA_VERSION_V20, SCHEMA_V20),
            (SCHEMA_VERSION_V21, SCHEMA_V21),
            (SCHEMA_VERSION_V22, SCHEMA_V22),
        ] {
            if !migration_applied(&connection, version)? {
                let transaction = connection
                    .transaction()
                    .map_err(MetaStoreError::migration)?;
                transaction
                    .execute_batch(schema)
                    .map_err(MetaStoreError::migration)?;
                transaction
                    .execute(
                        "INSERT INTO schema_migrations (version, applied_at_seconds) VALUES (?1, ?2)",
                        params![i64::from(version), 0_i64],
                    )
                    .map_err(MetaStoreError::migration)?;
                transaction.commit().map_err(MetaStoreError::migration)?;
                applied_versions.push(version);
            }
        }

        Ok(MigrationReport { applied_versions })
    }

    pub fn schema_version(&self) -> Result<u32> {
        if !self.schema_table_exists("schema_migrations")? {
            return Ok(0);
        }

        let connection = self.connection.borrow();
        let version = connection
            .query_row(
                "SELECT COALESCE(MAX(version), 0) FROM schema_migrations",
                [],
                |row| row.get::<_, i64>(0),
            )
            .map_err(MetaStoreError::storage)?;

        u32::try_from(version)
            .map_err(|_| MetaStoreError::invalid_value("schema_migrations.version"))
    }

    pub fn metadata_encryption_state(&self) -> MetadataEncryptionState {
        self.metadata_encryption_state
    }

    pub fn schema_table_exists(&self, table_name: &str) -> Result<bool> {
        let connection = self.connection.borrow();
        let exists = connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1)",
                params![table_name],
                |row| row.get::<_, i64>(0),
            )
            .map_err(MetaStoreError::storage)?;

        Ok(exists == 1)
    }

    pub fn foreign_keys_enabled(&self) -> Result<bool> {
        let connection = self.connection.borrow();
        let enabled = connection
            .query_row("PRAGMA foreign_keys", [], |row| row.get::<_, i64>(0))
            .map_err(MetaStoreError::storage)?;

        Ok(enabled == 1)
    }

    pub fn busy_timeout_millis(&self) -> Result<u64> {
        let connection = self.connection.borrow();
        let timeout = connection
            .query_row("PRAGMA busy_timeout", [], |row| row.get::<_, i64>(0))
            .map_err(MetaStoreError::storage)?;

        u64::try_from(timeout).map_err(|_| MetaStoreError::invalid_value("pragma.busy_timeout"))
    }

    pub fn journal_mode(&self) -> Result<String> {
        let connection = self.connection.borrow();
        connection
            .query_row("PRAGMA journal_mode", [], |row| row.get::<_, String>(0))
            .map_err(MetaStoreError::storage)
            .map(|mode| mode.to_ascii_lowercase())
    }

    pub fn upsert_document(&self, document: &Document) -> Result<()> {
        let connection = self.connection.borrow();
        upsert_document_in_connection(&connection, document)
    }

    pub fn document_by_id(&self, id: &DocumentId) -> Result<Option<Document>> {
        let connection = self.connection.borrow();
        let sql = format!("SELECT {DOCUMENT_COLUMNS} FROM document WHERE id = ?1");
        let mut statement = connection.prepare(&sql).map_err(MetaStoreError::storage)?;
        let mut rows = statement
            .query(params![id.as_str()])
            .map_err(MetaStoreError::storage)?;

        match rows.next().map_err(MetaStoreError::storage)? {
            Some(row) => Ok(Some(read_document(row)?)),
            None => Ok(None),
        }
    }

    pub fn visible_documents(&self) -> Result<Vec<Document>> {
        let connection = self.connection.borrow();
        let sql = format!(
            "SELECT {DOCUMENT_COLUMNS} FROM document WHERE is_deleted = 0 AND status <> ?1 ORDER BY id"
        );
        let mut statement = connection.prepare(&sql).map_err(MetaStoreError::storage)?;
        let mut rows = statement
            .query(params![document_status_to_storage(DocumentStatus::Deleted)])
            .map_err(MetaStoreError::storage)?;
        let mut documents = Vec::new();

        while let Some(row) = rows.next().map_err(MetaStoreError::storage)? {
            documents.push(read_document(row)?);
        }

        Ok(documents)
    }

    pub fn visible_document_count(&self) -> Result<u64> {
        let connection = self.connection.borrow();
        let count = connection
            .query_row(
                "SELECT COUNT(*) FROM document WHERE is_deleted = 0 AND status <> ?1",
                params![document_status_to_storage(DocumentStatus::Deleted)],
                |row| row.get::<_, i64>(0),
            )
            .map_err(MetaStoreError::storage)?;
        i64_to_u64(count, "document.visible_document_count")
    }

    pub fn searchable_document_ids(&self) -> Result<Vec<DocumentId>> {
        let connection = self.connection.borrow();
        let mut statement = connection
            .prepare("SELECT id FROM document WHERE is_deleted = 0 AND status = ?1 ORDER BY id")
            .map_err(MetaStoreError::storage)?;
        let mut rows = statement
            .query(params![document_status_to_storage(
                DocumentStatus::Searchable
            )])
            .map_err(MetaStoreError::storage)?;
        let mut document_ids = Vec::new();

        while let Some(row) = rows.next().map_err(MetaStoreError::storage)? {
            document_ids.push(read_id::<DocumentId>(row, 0, "document.id")?);
        }

        Ok(document_ids)
    }

    pub fn mark_document_deleted(
        &self,
        id: &DocumentId,
        updated_at: UnixTimestamp,
    ) -> Result<Option<Document>> {
        let mut connection = self.connection.borrow_mut();
        let transaction = connection.transaction().map_err(MetaStoreError::storage)?;
        let current_document = {
            let sql = format!("SELECT {DOCUMENT_COLUMNS} FROM document WHERE id = ?1");
            let mut statement = transaction.prepare(&sql).map_err(MetaStoreError::storage)?;
            let mut rows = statement
                .query(params![id.as_str()])
                .map_err(MetaStoreError::storage)?;

            match rows.next().map_err(MetaStoreError::storage)? {
                Some(row) => Some(read_document(row)?),
                None => None,
            }
        };

        let Some(mut document) = current_document else {
            transaction.commit().map_err(MetaStoreError::storage)?;
            return Ok(None);
        };

        transaction
            .execute(
                "\
                UPDATE document
                SET is_deleted = 1, status = ?1, updated_at_seconds = ?2
                WHERE id = ?3",
                params![
                    document_status_to_storage(DocumentStatus::Deleted),
                    updated_at.as_unix_seconds(),
                    id.as_str(),
                ],
            )
            .map_err(MetaStoreError::storage)?;
        transaction
            .execute(
                "\
                UPDATE resume_version
                SET visibility = ?1
                WHERE document_id = ?2",
                params![
                    resume_visibility_to_storage(ResumeVisibility::Hidden),
                    id.as_str()
                ],
            )
            .map_err(MetaStoreError::storage)?;
        transaction.commit().map_err(MetaStoreError::storage)?;

        document.is_deleted = true;
        document.status = DocumentStatus::Deleted;
        document.updated_at = updated_at;
        Ok(Some(document))
    }

    pub fn deleted_document_ids(&self) -> Result<Vec<DocumentId>> {
        let connection = self.connection.borrow();
        deleted_document_ids_from_connection(&connection)
    }

    pub fn quarantine_document_searchability(&self, document_id: &DocumentId) -> Result<()> {
        let mut connection = self.connection.borrow_mut();
        let transaction = connection.transaction().map_err(MetaStoreError::storage)?;
        for table in ["candidate_contact_conflict", "entity_mention"] {
            transaction
                .execute(
                    &format!(
                        "DELETE FROM {table} WHERE resume_version_id IN (
                            SELECT id FROM resume_version WHERE document_id = ?1
                         )"
                    ),
                    params![document_id.as_str()],
                )
                .map_err(MetaStoreError::storage)?;
        }
        transaction
            .execute(
                "UPDATE resume_version
                 SET visibility = 'hidden', candidate_id = NULL
                 WHERE document_id = ?1",
                params![document_id.as_str()],
            )
            .map_err(MetaStoreError::storage)?;
        transaction
            .execute_batch(
                "UPDATE candidate SET version_count = (
                    SELECT COUNT(*) FROM resume_version
                    WHERE resume_version.candidate_id = candidate.id
                 );
                 DELETE FROM candidate WHERE version_count = 0;",
            )
            .map_err(MetaStoreError::storage)?;
        transaction.commit().map_err(MetaStoreError::storage)
    }

    pub fn purge_deleted_documents(&self) -> Result<usize> {
        let mut connection = self.connection.borrow_mut();
        let document_ids = deleted_document_ids_from_connection(&connection)?;
        if document_ids.is_empty() {
            return Ok(0);
        }

        let placeholders = (0..document_ids.len())
            .map(|index| format!("?{}", index + 1))
            .collect::<Vec<_>>()
            .join(", ");
        let delete_sql = format!("DELETE FROM document WHERE id IN ({placeholders})");
        let delete_params = document_ids
            .iter()
            .map(|document_id| Value::Text(document_id.as_str().to_string()))
            .collect::<Vec<_>>();

        let transaction = connection.transaction().map_err(MetaStoreError::storage)?;
        let deleted = transaction
            .execute(&delete_sql, params_from_iter(delete_params))
            .map_err(MetaStoreError::storage)?;
        transaction
            .execute(
                "\
                UPDATE candidate
                SET version_count = (
                    SELECT COUNT(*)
                    FROM resume_version
                    WHERE resume_version.candidate_id = candidate.id
                )",
                [],
            )
            .map_err(MetaStoreError::storage)?;
        transaction
            .execute("DELETE FROM candidate WHERE version_count = 0", [])
            .map_err(MetaStoreError::storage)?;
        transaction.commit().map_err(MetaStoreError::storage)?;

        connection
            .execute_batch("PRAGMA wal_checkpoint(TRUNCATE); VACUUM;")
            .map_err(MetaStoreError::storage)?;
        Ok(deleted)
    }

    pub fn purge_import_tasks_for_deleted_document_roots(
        &self,
        document_ids: &[DocumentId],
    ) -> Result<ImportTaskPurge> {
        if document_ids.is_empty() {
            return Ok(ImportTaskPurge::empty());
        }

        let mut connection = self.connection.borrow_mut();
        let transaction = connection.transaction().map_err(MetaStoreError::storage)?;
        let import_tasks =
            import_tasks_for_deleted_document_roots_from_connection(&transaction, document_ids)?;
        let task_ids = import_tasks
            .into_iter()
            .map(|task| task.id)
            .collect::<Vec<_>>();

        if task_ids.is_empty() {
            transaction.commit().map_err(MetaStoreError::storage)?;
            return Ok(ImportTaskPurge::empty());
        }

        let scan_scopes = count_import_task_child_rows(
            &transaction,
            "import_scan_scope",
            "import_task_id",
            &task_ids,
        )?;
        let scan_errors = count_import_task_child_rows(
            &transaction,
            "import_scan_error",
            "import_task_id",
            &task_ids,
        )?;
        let cancellations = count_import_task_child_rows(
            &transaction,
            "import_task_cancellation",
            "import_task_id",
            &task_ids,
        )?;
        let placeholders = import_task_id_placeholders(task_ids.len());
        let delete_sql = format!("DELETE FROM import_task WHERE id IN ({placeholders})");
        let delete_params = task_ids
            .iter()
            .map(|task_id| Value::Text(task_id.as_str().to_string()))
            .collect::<Vec<_>>();
        let tasks = transaction
            .execute(&delete_sql, params_from_iter(delete_params))
            .map_err(MetaStoreError::storage)?;
        transaction.commit().map_err(MetaStoreError::storage)?;

        Ok(ImportTaskPurge {
            tasks,
            scan_scopes,
            scan_errors,
            cancellations,
        })
    }

    pub fn import_root_markers_for_deleted_document_roots(
        &self,
        document_ids: &[DocumentId],
    ) -> Result<Vec<String>> {
        if document_ids.is_empty() {
            return Ok(Vec::new());
        }

        let connection = self.connection.borrow();
        let purge_candidates =
            import_tasks_for_deleted_document_roots_from_connection(&connection, document_ids)?;
        let mut markers = Vec::new();

        for task in purge_candidates {
            markers.push(task.root_path.clone());
            let sql = format!(
                "SELECT {IMPORT_SCAN_SCOPE_COLUMNS} FROM import_scan_scope WHERE import_task_id = ?1"
            );
            let mut statement = connection.prepare(&sql).map_err(MetaStoreError::storage)?;
            let mut rows = statement
                .query(params![task.id.as_str()])
                .map_err(MetaStoreError::storage)?;
            if let Some(row) = rows.next().map_err(MetaStoreError::storage)? {
                let scope = read_import_scan_scope(row)?;
                markers.push(scope.requested_root_path);
                markers.push(scope.canonical_root_path);
            }
        }

        Ok(markers)
    }

    pub fn purge_ingest_jobs_for_documents(
        &self,
        document_ids: &[DocumentId],
    ) -> Result<IngestJobPurge> {
        if document_ids.is_empty() {
            return Ok(IngestJobPurge::empty());
        }

        let placeholders = (0..document_ids.len())
            .map(|index| format!("?{}", index + 1))
            .collect::<Vec<_>>()
            .join(", ");
        let connection = self.connection.borrow();
        let embedding_specs = {
            let count_sql = format!(
                "\
                SELECT COUNT(*)
                FROM embedding_job_spec AS spec
                JOIN ingest_job AS job ON job.id = spec.ingest_job_id
                WHERE job.document_id IN ({placeholders})"
            );
            let count_params = document_ids
                .iter()
                .map(|document_id| Value::Text(document_id.as_str().to_string()))
                .collect::<Vec<_>>();
            let count = connection
                .query_row(&count_sql, params_from_iter(count_params), |row| {
                    row.get::<_, i64>(0)
                })
                .map_err(MetaStoreError::storage)?;
            i64_to_usize(count, "embedding_job_spec.count")?
        };
        let delete_sql = format!("DELETE FROM ingest_job WHERE document_id IN ({placeholders})");
        let delete_params = document_ids
            .iter()
            .map(|document_id| Value::Text(document_id.as_str().to_string()))
            .collect::<Vec<_>>();

        let jobs = connection
            .execute(&delete_sql, params_from_iter(delete_params))
            .map_err(MetaStoreError::storage)?;

        Ok(IngestJobPurge {
            jobs,
            embedding_specs,
        })
    }

    pub fn purge_ocr_page_cache_by_content_hashes(
        &self,
        content_hashes: &[String],
    ) -> Result<OcrPageCachePurge> {
        if content_hashes.is_empty() {
            return Ok(OcrPageCachePurge::empty());
        }

        let placeholders = (0..content_hashes.len())
            .map(|index| format!("?{}", index + 1))
            .collect::<Vec<_>>()
            .join(", ");
        let connection = self.connection.borrow();
        let word_boxes = {
            let query_params = content_hashes
                .iter()
                .map(|content_hash| Value::Text(content_hash.clone()))
                .collect::<Vec<_>>();
            let select_sql = format!(
                "SELECT word_boxes_json FROM ocr_page_cache WHERE file_content_hash IN ({placeholders})"
            );
            let mut statement = connection
                .prepare(&select_sql)
                .map_err(MetaStoreError::storage)?;
            let mut rows = statement
                .query(params_from_iter(query_params))
                .map_err(MetaStoreError::storage)?;
            let mut word_boxes = 0;
            while let Some(row) = rows.next().map_err(MetaStoreError::storage)? {
                word_boxes +=
                    read_ocr_word_boxes_json(read_optional_string(row, 0)?.as_deref())?.len();
            }
            word_boxes
        };
        let delete_sql =
            format!("DELETE FROM ocr_page_cache WHERE file_content_hash IN ({placeholders})");
        let delete_params = content_hashes
            .iter()
            .map(|content_hash| Value::Text(content_hash.clone()))
            .collect::<Vec<_>>();

        let entries = connection
            .execute(&delete_sql, params_from_iter(delete_params))
            .map_err(MetaStoreError::storage)?;

        Ok(OcrPageCachePurge {
            entries,
            word_boxes,
        })
    }

    pub fn upsert_candidate(&self, candidate: &Candidate) -> Result<()> {
        validate_candidate(candidate)?;
        let connection = self.connection.borrow();
        upsert_candidate_in_connection(&connection, candidate)
    }

    pub fn candidate_by_id(&self, id: &CandidateId) -> Result<Option<Candidate>> {
        let connection = self.connection.borrow();
        candidate_by_id_from_connection(&connection, id)
    }

    pub fn candidate_by_contact_hash(
        &self,
        contact_hash: &ContactHash,
    ) -> Result<Option<Candidate>> {
        let connection = self.connection.borrow();
        candidate_by_contact_hash_from_connection(&connection, contact_hash)
    }

    pub fn candidate_contact_conflicts(&self) -> Result<Vec<CandidateContactConflict>> {
        let connection = self.connection.borrow();
        candidate_contact_conflicts_from_connection(&connection)
    }

    pub fn assign_candidate_to_version(
        &self,
        version_id: &ResumeVersionId,
        candidate_id: &CandidateId,
    ) -> Result<Option<Candidate>> {
        let mut connection = self.connection.borrow_mut();
        let transaction = connection.transaction().map_err(MetaStoreError::storage)?;
        let Some(candidate) = candidate_by_id_from_connection(&transaction, candidate_id)? else {
            return Err(MetaStoreError::invalid_value("resume_version.candidate_id"));
        };
        let updated = transaction
            .execute(
                "UPDATE resume_version SET candidate_id = ?1 WHERE id = ?2",
                params![candidate.id.as_str(), version_id.as_str()],
            )
            .map_err(MetaStoreError::storage)?;
        if updated == 0 {
            transaction.commit().map_err(MetaStoreError::storage)?;
            return Ok(None);
        }

        refresh_candidate_version_count_in_connection(&transaction, &candidate.id)?;
        let assigned = candidate_by_id_from_connection(&transaction, &candidate.id)?;
        transaction.commit().map_err(MetaStoreError::storage)?;
        Ok(assigned)
    }

    pub fn unassign_candidate_versions(&self, candidate_id: &CandidateId) -> Result<usize> {
        let mut connection = self.connection.borrow_mut();
        let transaction = connection.transaction().map_err(MetaStoreError::storage)?;
        if candidate_by_id_from_connection(&transaction, candidate_id)?.is_none() {
            return Err(MetaStoreError::invalid_value("candidate.id"));
        }
        let updated = transaction
            .execute(
                "UPDATE resume_version SET candidate_id = NULL WHERE candidate_id = ?1",
                params![candidate_id.as_str()],
            )
            .map_err(MetaStoreError::storage)?;
        refresh_candidate_version_count_in_connection(&transaction, candidate_id)?;
        transaction.commit().map_err(MetaStoreError::storage)?;
        Ok(updated)
    }

    pub fn assign_candidate_from_hashed_contacts(
        &self,
        version_id: &ResumeVersionId,
        email_hash: Option<&ContactHash>,
        phone_hash: Option<&ContactHash>,
    ) -> Result<Option<Candidate>> {
        if email_hash.is_none() && phone_hash.is_none() {
            return Ok(None);
        }

        let mut connection = self.connection.borrow_mut();
        let transaction = connection.transaction().map_err(MetaStoreError::storage)?;
        let assigned = assign_candidate_from_hashed_contacts_in_connection(
            &transaction,
            version_id,
            email_hash,
            phone_hash,
            UnixTimestamp::from_unix_seconds(0),
        )?;
        transaction.commit().map_err(MetaStoreError::storage)?;
        Ok(assigned)
    }

    pub fn upsert_searchable_import_document(
        &self,
        import: SearchableImportDocument<'_>,
    ) -> Result<Option<Candidate>> {
        let mut connection = self.connection.borrow_mut();
        let transaction = connection.transaction().map_err(MetaStoreError::storage)?;
        upsert_document_in_connection(&transaction, import.document)?;

        let mut version = import.version.clone();
        if version.candidate_id.is_none() {
            version.candidate_id = resume_version_by_id_from_connection(&transaction, &version.id)?
                .and_then(|existing| existing.candidate_id);
        }
        upsert_resume_version_in_connection(&transaction, &version)?;
        replace_entity_mentions_in_connection(&transaction, &version.id, import.mentions)?;
        let assigned = assign_candidate_from_hashed_contacts_in_connection(
            &transaction,
            &version.id,
            import.email_hash,
            import.phone_hash,
            UnixTimestamp::from_unix_seconds(0),
        )?;

        transaction.commit().map_err(MetaStoreError::storage)?;
        Ok(assigned)
    }

    pub fn upsert_resume_version(&self, version: &ResumeVersion) -> Result<()> {
        let connection = self.connection.borrow();
        upsert_resume_version_in_connection(&connection, version)
    }

    pub fn resume_version_by_id(&self, id: &ResumeVersionId) -> Result<Option<ResumeVersion>> {
        let connection = self.connection.borrow();
        let sql = format!("SELECT {RESUME_VERSION_COLUMNS} FROM resume_version WHERE id = ?1");
        let mut statement = connection.prepare(&sql).map_err(MetaStoreError::storage)?;
        let mut rows = statement
            .query(params![id.as_str()])
            .map_err(MetaStoreError::storage)?;

        match rows.next().map_err(MetaStoreError::storage)? {
            Some(row) => Ok(Some(read_resume_version(row)?)),
            None => Ok(None),
        }
    }

    pub fn resume_versions_for_document(
        &self,
        document_id: &DocumentId,
    ) -> Result<Vec<ResumeVersion>> {
        let connection = self.connection.borrow();
        let sql = format!(
            "\
            SELECT {RESUME_VERSION_COLUMNS}
            FROM resume_version
            WHERE document_id = ?1
            ORDER BY id"
        );
        let mut statement = connection.prepare(&sql).map_err(MetaStoreError::storage)?;
        let mut rows = statement
            .query(params![document_id.as_str()])
            .map_err(MetaStoreError::storage)?;
        let mut versions = Vec::new();

        while let Some(row) = rows.next().map_err(MetaStoreError::storage)? {
            versions.push(read_resume_version(row)?);
        }

        Ok(versions)
    }

    pub fn latest_visible_resume_version_for_document(
        &self,
        document_id: &DocumentId,
    ) -> Result<Option<ResumeVersion>> {
        let connection = self.connection.borrow();
        let sql = format!(
            "\
            SELECT {RESUME_VERSION_COLUMNS}
            FROM resume_version
            WHERE document_id = ?1 AND visibility <> ?2
            ORDER BY rowid DESC
            LIMIT 1"
        );
        let mut statement = connection.prepare(&sql).map_err(MetaStoreError::storage)?;
        let mut rows = statement
            .query(params![
                document_id.as_str(),
                resume_visibility_to_storage(ResumeVisibility::Hidden)
            ])
            .map_err(MetaStoreError::storage)?;

        match rows.next().map_err(MetaStoreError::storage)? {
            Some(row) => Ok(Some(read_resume_version(row)?)),
            None => Ok(None),
        }
    }

    pub fn replace_entity_mentions(
        &self,
        version_id: &ResumeVersionId,
        mentions: &[EntityMention],
    ) -> Result<()> {
        let mut connection = self.connection.borrow_mut();
        let transaction = connection.transaction().map_err(MetaStoreError::storage)?;
        replace_entity_mentions_in_connection(&transaction, version_id, mentions)?;
        transaction.commit().map_err(MetaStoreError::storage)?;
        Ok(())
    }

    pub fn entity_mentions_for_version(
        &self,
        version_id: &ResumeVersionId,
    ) -> Result<Vec<EntityMention>> {
        let connection = self.connection.borrow();
        let sql = format!(
            "\
            SELECT {ENTITY_MENTION_COLUMNS}
            FROM entity_mention
            WHERE resume_version_id = ?1
            ORDER BY span_start IS NULL, span_start, rowid"
        );
        let mut statement = connection.prepare(&sql).map_err(MetaStoreError::storage)?;
        let mut rows = statement
            .query(params![version_id.as_str()])
            .map_err(MetaStoreError::storage)?;
        let mut mentions = Vec::new();

        while let Some(row) = rows.next().map_err(MetaStoreError::storage)? {
            mentions.push(read_entity_mention(row)?);
        }

        Ok(mentions)
    }

    pub fn visible_entity_type_counts_for_document(
        &self,
        document_id: &DocumentId,
    ) -> Result<Vec<(EntityType, usize)>> {
        let connection = self.connection.borrow();
        let sql = "\
            SELECT mention.entity_type, COUNT(*)
            FROM entity_mention AS mention
            JOIN resume_version AS version ON version.id = mention.resume_version_id
            WHERE version.document_id = ?1
                AND version.visibility <> ?2
            GROUP BY mention.entity_type
            ORDER BY mention.entity_type";
        let mut statement = connection.prepare(sql).map_err(MetaStoreError::storage)?;
        let mut rows = statement
            .query(params![
                document_id.as_str(),
                resume_visibility_to_storage(ResumeVisibility::Hidden),
            ])
            .map_err(MetaStoreError::storage)?;
        let mut counts = Vec::new();

        while let Some(row) = rows.next().map_err(MetaStoreError::storage)? {
            let entity_type = entity_type_from_storage(&read_string(row, 0)?)?;
            let count = i64_to_usize(read_i64(row, 1)?, "entity_mention.count")?;
            counts.push((entity_type, count));
        }

        Ok(counts)
    }

    pub fn searchable_document_ids_with_entity_values(
        &self,
        entity_type: EntityType,
        normalized_values: &[String],
        min_confidence: f32,
        case_insensitive: bool,
    ) -> Result<Vec<DocumentId>> {
        if normalized_values.is_empty() {
            return Ok(Vec::new());
        }
        validate_confidence_threshold(min_confidence, "entity_mention.confidence")?;

        let value_placeholders = (0..normalized_values.len())
            .map(|index| format!("?{}", index + 3))
            .collect::<Vec<_>>()
            .join(", ");
        let value_expression = if case_insensitive {
            "LOWER(mention.normalized_value)"
        } else {
            "mention.normalized_value"
        };
        let sql = format!(
            "\
            SELECT DISTINCT version.document_id
            FROM entity_mention AS mention
            JOIN resume_version AS version ON version.id = mention.resume_version_id
            JOIN document AS document ON document.id = version.document_id
            WHERE document.is_deleted = 0
                AND document.status IN ('indexed_partial', 'searchable')
                AND version.visibility = 'searchable'
                AND mention.entity_type = ?1
                AND mention.confidence >= ?2
                AND {value_expression} IN ({value_placeholders})
            ORDER BY version.document_id"
        );
        let mut values = vec![
            Value::Text(entity_type_to_storage(&entity_type).to_string()),
            Value::Real(f64::from(min_confidence)),
        ];
        for value in normalized_values {
            values.push(Value::Text(if case_insensitive {
                value.to_ascii_lowercase()
            } else {
                value.clone()
            }));
        }

        let connection = self.connection.borrow();
        let mut statement = connection.prepare(&sql).map_err(MetaStoreError::storage)?;
        let mut rows = statement
            .query(params_from_iter(values))
            .map_err(MetaStoreError::storage)?;
        let mut document_ids = Vec::new();

        while let Some(row) = rows.next().map_err(MetaStoreError::storage)? {
            document_ids.push(read_id::<DocumentId>(row, 0, "document.id")?);
        }

        Ok(document_ids)
    }

    pub fn searchable_document_ids_with_numeric_entity_min(
        &self,
        entity_type: EntityType,
        min_value: f32,
        min_confidence: f32,
    ) -> Result<Vec<DocumentId>> {
        if !min_value.is_finite() {
            return Err(MetaStoreError::invalid_value(
                "entity_mention.normalized_value",
            ));
        }
        validate_confidence_threshold(min_confidence, "entity_mention.confidence")?;

        let connection = self.connection.borrow();
        let mut statement = connection
            .prepare(
                "\
                SELECT DISTINCT version.document_id
                FROM entity_mention AS mention
                JOIN resume_version AS version ON version.id = mention.resume_version_id
                JOIN document AS document ON document.id = version.document_id
                WHERE document.is_deleted = 0
                    AND document.status IN ('indexed_partial', 'searchable')
                    AND version.visibility = 'searchable'
                    AND mention.entity_type = ?1
                    AND mention.confidence >= ?2
                    AND CAST(mention.normalized_value AS REAL) >= ?3
                ORDER BY version.document_id",
            )
            .map_err(MetaStoreError::storage)?;
        let mut rows = statement
            .query(params![
                entity_type_to_storage(&entity_type),
                f64::from(min_confidence),
                f64::from(min_value),
            ])
            .map_err(MetaStoreError::storage)?;
        let mut document_ids = Vec::new();

        while let Some(row) = rows.next().map_err(MetaStoreError::storage)? {
            document_ids.push(read_id::<DocumentId>(row, 0, "document.id")?);
        }

        Ok(document_ids)
    }

    pub fn searchable_document_ids_with_date_range_overlap(
        &self,
        start_month: i32,
        end_month: Option<i32>,
        min_confidence: f32,
    ) -> Result<Vec<DocumentId>> {
        validate_confidence_threshold(min_confidence, "entity_mention.confidence")?;
        let end_month = end_month.unwrap_or(i32::MAX);
        if start_month < 1900 * 12 + 1 || end_month < start_month {
            return Err(MetaStoreError::invalid_value("date_range.filter"));
        }

        let connection = self.connection.borrow();
        let mut statement = connection
            .prepare(
                "\
                SELECT DISTINCT version.document_id
                FROM entity_mention AS mention
                JOIN resume_version AS version ON version.id = mention.resume_version_id
                JOIN document AS document ON document.id = version.document_id
                WHERE document.is_deleted = 0
                    AND document.status IN ('indexed_partial', 'searchable')
                    AND version.visibility = 'searchable'
                    AND mention.entity_type = 'date_range'
                    AND mention.confidence >= ?1
                    AND mention.normalized_value IS NOT NULL
                    AND (
                        mention.normalized_value GLOB
                            '[0-9][0-9][0-9][0-9]-[0-9][0-9]/[0-9][0-9][0-9][0-9]-[0-9][0-9]'
                        OR mention.normalized_value GLOB
                            '[0-9][0-9][0-9][0-9]-[0-9][0-9]/PRESENT'
                    )
                    AND (
                        CAST(substr(mention.normalized_value, 1, 4) AS INTEGER) * 12
                            + CAST(substr(mention.normalized_value, 6, 2) AS INTEGER)
                    ) <= ?2
                    AND (
                        CASE
                            WHEN substr(mention.normalized_value, 9) = 'PRESENT' THEN 2147483647
                            ELSE CAST(substr(mention.normalized_value, 9, 4) AS INTEGER) * 12
                                + CAST(substr(mention.normalized_value, 14, 2) AS INTEGER)
                        END
                    ) >= ?3
                ORDER BY version.document_id",
            )
            .map_err(MetaStoreError::storage)?;
        let mut rows = statement
            .query(params![
                f64::from(min_confidence),
                i64::from(end_month),
                i64::from(start_month),
            ])
            .map_err(MetaStoreError::storage)?;
        let mut document_ids = Vec::new();

        while let Some(row) = rows.next().map_err(MetaStoreError::storage)? {
            document_ids.push(read_id::<DocumentId>(row, 0, "document.id")?);
        }

        Ok(document_ids)
    }

    pub fn searchable_document_ids_with_contact_hashes(
        &self,
        contact_hashes: &[ContactHash],
    ) -> Result<Vec<DocumentId>> {
        if contact_hashes.is_empty() {
            return Ok(Vec::new());
        }

        let placeholders = (0..contact_hashes.len())
            .map(|index| format!("?{}", index + 1))
            .collect::<Vec<_>>()
            .join(", ");
        let sql = format!(
            "\
            SELECT DISTINCT version.document_id
            FROM candidate AS candidate
            JOIN resume_version AS version ON version.candidate_id = candidate.id
            JOIN document AS document ON document.id = version.document_id
            WHERE document.is_deleted = 0
                AND document.status IN ('indexed_partial', 'searchable')
                AND version.visibility = 'searchable'
                AND (
                    candidate.email_hash IN ({placeholders})
                    OR candidate.phone_hash IN ({placeholders})
                )
            ORDER BY version.document_id"
        );
        let values = contact_hashes
            .iter()
            .map(|contact_hash| Value::Text(contact_hash.as_str().to_string()))
            .collect::<Vec<_>>();

        let connection = self.connection.borrow();
        let mut statement = connection.prepare(&sql).map_err(MetaStoreError::storage)?;
        let mut rows = statement
            .query(params_from_iter(values))
            .map_err(MetaStoreError::storage)?;
        let mut document_ids = Vec::new();

        while let Some(row) = rows.next().map_err(MetaStoreError::storage)? {
            document_ids.push(read_id::<DocumentId>(row, 0, "document.id")?);
        }

        Ok(document_ids)
    }

    pub fn searchable_document_ids_without_entity_type(
        &self,
        entity_type: EntityType,
        min_confidence: f32,
    ) -> Result<Vec<DocumentId>> {
        validate_confidence_threshold(min_confidence, "entity_mention.confidence")?;

        let connection = self.connection.borrow();
        let mut statement = connection
            .prepare(
                "\
                SELECT DISTINCT document.id
                FROM document AS document
                JOIN resume_version AS version ON version.document_id = document.id
                WHERE document.is_deleted = 0
                    AND document.status IN ('indexed_partial', 'searchable')
                    AND version.visibility = 'searchable'
                    AND NOT EXISTS (
                        SELECT 1
                        FROM entity_mention AS mention
                        WHERE mention.resume_version_id = version.id
                            AND mention.entity_type = ?1
                            AND mention.confidence >= ?2
                    )
                ORDER BY document.file_name",
            )
            .map_err(MetaStoreError::storage)?;
        let mut rows = statement
            .query(params![
                entity_type_to_storage(&entity_type),
                f64::from(min_confidence),
            ])
            .map_err(MetaStoreError::storage)?;
        let mut document_ids = Vec::new();

        while let Some(row) = rows.next().map_err(MetaStoreError::storage)? {
            document_ids.push(read_id::<DocumentId>(row, 0, "document.id")?);
        }

        Ok(document_ids)
    }

    pub fn upsert_ocr_page_cache_entry(&self, entry: &OcrPageCacheEntry) -> Result<()> {
        validate_ocr_page_cache_entry(entry)?;
        let connection = self.connection.borrow();
        connection
            .execute(
                "\
                INSERT INTO ocr_page_cache (
                    file_content_hash, page_no, render_dpi, ocr_lang, ocr_profile, text,
                    confidence, engine_profile, duration_ms, status, error_kind, updated_at_seconds,
                    word_boxes_json
                )
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
                ON CONFLICT(file_content_hash, page_no, render_dpi, ocr_lang, ocr_profile)
                DO UPDATE SET
                    text = excluded.text,
                    confidence = excluded.confidence,
                    engine_profile = excluded.engine_profile,
                    duration_ms = excluded.duration_ms,
                    status = excluded.status,
                    error_kind = excluded.error_kind,
                    updated_at_seconds = excluded.updated_at_seconds,
                    word_boxes_json = excluded.word_boxes_json",
                params![
                    entry.key.file_content_hash.as_str(),
                    u32_to_i64(entry.key.page_no),
                    u32_to_i64(entry.key.render_dpi),
                    entry.key.ocr_lang.as_str(),
                    entry.key.ocr_profile.as_str(),
                    entry.text.as_deref(),
                    entry.confidence.map(f64::from),
                    entry.engine_profile.as_deref(),
                    entry
                        .duration_ms
                        .map(|value| u64_to_i64(value, "ocr_page_cache.duration_ms"))
                        .transpose()?,
                    ocr_page_cache_status_to_storage(entry.status),
                    entry.error_kind.as_deref(),
                    entry.updated_at.as_unix_seconds(),
                    ocr_word_boxes_json_for_storage(entry)?,
                ],
            )
            .map_err(MetaStoreError::storage)?;

        Ok(())
    }

    pub fn ocr_page_cache_entry(&self, key: &OcrPageCacheKey) -> Result<Option<OcrPageCacheEntry>> {
        let connection = self.connection.borrow();
        let sql = format!(
            "\
            SELECT {OCR_PAGE_CACHE_COLUMNS}
            FROM ocr_page_cache
            WHERE file_content_hash = ?1
                AND page_no = ?2
                AND render_dpi = ?3
                AND ocr_lang = ?4
                AND ocr_profile = ?5"
        );
        let mut statement = connection.prepare(&sql).map_err(MetaStoreError::storage)?;
        let mut rows = statement
            .query(params![
                key.file_content_hash.as_str(),
                u32_to_i64(key.page_no),
                u32_to_i64(key.render_dpi),
                key.ocr_lang.as_str(),
                key.ocr_profile.as_str(),
            ])
            .map_err(MetaStoreError::storage)?;

        match rows.next().map_err(MetaStoreError::storage)? {
            Some(row) => Ok(Some(read_ocr_page_cache_entry(row)?)),
            None => Ok(None),
        }
    }

    pub fn ocr_page_cache_entries_for_content_hashes(
        &self,
        content_hashes: &[String],
    ) -> Result<Vec<OcrPageCacheEntry>> {
        if content_hashes.is_empty() {
            return Ok(Vec::new());
        }

        let placeholders = (0..content_hashes.len())
            .map(|index| format!("?{}", index + 1))
            .collect::<Vec<_>>()
            .join(", ");
        let sql = format!(
            "\
            SELECT {OCR_PAGE_CACHE_COLUMNS}
            FROM ocr_page_cache
            WHERE file_content_hash IN ({placeholders})
            ORDER BY file_content_hash, page_no, render_dpi, ocr_lang, ocr_profile"
        );
        let query_params = content_hashes
            .iter()
            .map(|content_hash| Value::Text(content_hash.clone()))
            .collect::<Vec<_>>();
        let connection = self.connection.borrow();
        let mut statement = connection.prepare(&sql).map_err(MetaStoreError::storage)?;
        let mut rows = statement
            .query(params_from_iter(query_params))
            .map_err(MetaStoreError::storage)?;
        let mut entries = Vec::new();

        while let Some(row) = rows.next().map_err(MetaStoreError::storage)? {
            entries.push(read_ocr_page_cache_entry(row)?);
        }

        Ok(entries)
    }

    pub fn worker_task_control(&self, task: WorkerTaskKind) -> Result<WorkerTaskControl> {
        let connection = self.connection.borrow();
        let sql = format!(
            "\
            SELECT {WORKER_TASK_CONTROL_COLUMNS}
            FROM worker_task_control
            WHERE task_kind = ?1"
        );
        let mut statement = connection.prepare(&sql).map_err(MetaStoreError::storage)?;
        let mut rows = statement
            .query(params![worker_task_kind_to_storage(task)])
            .map_err(MetaStoreError::storage)?;

        match rows.next().map_err(MetaStoreError::storage)? {
            Some(row) => read_worker_task_control(row),
            None => Ok(WorkerTaskControl {
                task,
                paused: false,
                updated_at: UnixTimestamp::from_unix_seconds(0),
            }),
        }
    }

    pub fn set_worker_task_paused(
        &self,
        task: WorkerTaskKind,
        paused: bool,
        updated_at: UnixTimestamp,
    ) -> Result<()> {
        let connection = self.connection.borrow();
        connection
            .execute(
                "\
                INSERT INTO worker_task_control (
                    task_kind, paused, updated_at_seconds
                )
                VALUES (?1, ?2, ?3)
                ON CONFLICT(task_kind) DO UPDATE SET
                    paused = excluded.paused,
                    updated_at_seconds = excluded.updated_at_seconds",
                params![
                    worker_task_kind_to_storage(task),
                    bool_to_i64(paused),
                    updated_at.as_unix_seconds(),
                ],
            )
            .map_err(MetaStoreError::storage)?;

        Ok(())
    }

    pub fn insert_ingest_job(&self, job: &IngestJob) -> Result<()> {
        let connection = self.connection.borrow();
        connection
            .execute(
                "\
                INSERT INTO ingest_job (
                    id, document_id, resume_version_id, kind, status, attempt_count, max_attempts,
                    queued_at_seconds, started_at_seconds, finished_at_seconds, updated_at_seconds,
                    failure_kind
                )
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
                params![
                    job.id.as_str(),
                    job.document_id.as_str(),
                    job.resume_version_id.as_ref().map(ResumeVersionId::as_str),
                    ingest_job_kind_to_storage(job.kind),
                    ingest_job_status_to_storage(job.status),
                    u32_to_i64(job.attempt_count),
                    u32_to_i64(job.max_attempts),
                    job.queued_at.as_unix_seconds(),
                    job.started_at.map(UnixTimestamp::as_unix_seconds),
                    job.finished_at.map(UnixTimestamp::as_unix_seconds),
                    job.updated_at.as_unix_seconds(),
                    job.failure_kind.map(ingest_job_failure_kind_to_storage),
                ],
            )
            .map_err(MetaStoreError::storage)?;

        Ok(())
    }

    pub fn ingest_job_by_id(&self, id: &IngestJobId) -> Result<Option<IngestJob>> {
        let connection = self.connection.borrow();
        let sql = format!("SELECT {INGEST_JOB_COLUMNS} FROM ingest_job WHERE id = ?1");
        let mut statement = connection.prepare(&sql).map_err(MetaStoreError::storage)?;
        let mut rows = statement
            .query(params![id.as_str()])
            .map_err(MetaStoreError::storage)?;

        match rows.next().map_err(MetaStoreError::storage)? {
            Some(row) => Ok(Some(read_ingest_job(row)?)),
            None => Ok(None),
        }
    }

    pub fn enqueue_ocr_job_for_document(
        &self,
        document_id: &DocumentId,
        queued_at: UnixTimestamp,
    ) -> Result<EnqueuedIngestJob> {
        let id = IngestJobId::from_non_secret_parts(&["ocr-document", document_id.as_str()]);
        let job = IngestJob {
            id,
            document_id: document_id.clone(),
            resume_version_id: None,
            kind: IngestJobKind::OcrDocument,
            status: IngestJobStatus::Queued,
            attempt_count: 0,
            max_attempts: 3,
            queued_at,
            started_at: None,
            finished_at: None,
            updated_at: queued_at,
            failure_kind: None,
        };
        let scheduled = {
            let mut connection = self.connection.borrow_mut();
            let transaction = connection.transaction().map_err(MetaStoreError::storage)?;
            let existing = {
                let mut statement = transaction
                    .prepare(
                        "\
                        SELECT id, attempt_count
                        FROM ingest_job
                        WHERE document_id = ?1 AND kind = ?2
                        ORDER BY rowid
                        LIMIT 1",
                    )
                    .map_err(MetaStoreError::storage)?;
                let mut rows = statement
                    .query(params![
                        document_id.as_str(),
                        ingest_job_kind_to_storage(IngestJobKind::OcrDocument),
                    ])
                    .map_err(MetaStoreError::storage)?;
                match rows.next().map_err(MetaStoreError::storage)? {
                    Some(row) => Some((
                        read_string(row, 0)?,
                        i64_to_u32(
                            row.get(1).map_err(MetaStoreError::storage)?,
                            "ingest_job.attempt_count",
                        )?,
                    )),
                    None => None,
                }
            };

            let scheduled = if let Some((existing_id, attempt_count)) = existing {
                let renewed_max_attempts = attempt_count
                    .checked_add(3)
                    .ok_or_else(|| MetaStoreError::invalid_value("ingest_job.max_attempts"))?;
                transaction
                    .execute(
                        "UPDATE ingest_job
                         SET status = ?1, max_attempts = ?2,
                             queued_at_seconds = ?3, started_at_seconds = NULL,
                             finished_at_seconds = NULL, updated_at_seconds = ?3,
                             failure_kind = NULL
                         WHERE id = ?4 AND (
                             status IN (?5, ?6)
                             OR (status IN (?7, ?8) AND attempt_count >= max_attempts)
                         ) AND EXISTS (
                             SELECT 1
                             FROM document
                             JOIN document_classification AS classification
                               ON classification.document_id = document.id
                             WHERE document.id = ?9 AND document.is_deleted = 0
                               AND document.status = ?10
                               AND classification.status = ?11
                               AND classification.classifier_epoch = ?12
                         )",
                        params![
                            ingest_job_status_to_storage(IngestJobStatus::Queued),
                            u32_to_i64(renewed_max_attempts),
                            queued_at.as_unix_seconds(),
                            existing_id,
                            ingest_job_status_to_storage(IngestJobStatus::Completed),
                            ingest_job_status_to_storage(IngestJobStatus::FailedPermanent),
                            ingest_job_status_to_storage(IngestJobStatus::FailedRetryable),
                            ingest_job_status_to_storage(IngestJobStatus::Interrupted),
                            document_id.as_str(),
                            document_status_to_storage(DocumentStatus::OcrRequired),
                            ClassificationStatus::OcrBacklog.as_str(),
                            CLASSIFIER_EPOCH,
                        ],
                    )
                    .map_err(MetaStoreError::storage)?
                    == 1
            } else {
                transaction
                    .execute(
                        "\
                        INSERT INTO ingest_job (
                            id, document_id, resume_version_id, kind, status, attempt_count,
                            max_attempts, queued_at_seconds, started_at_seconds,
                            finished_at_seconds, updated_at_seconds, failure_kind
                        )
                        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
                        params![
                            job.id.as_str(),
                            job.document_id.as_str(),
                            job.resume_version_id.as_ref().map(ResumeVersionId::as_str),
                            ingest_job_kind_to_storage(job.kind),
                            ingest_job_status_to_storage(job.status),
                            u32_to_i64(job.attempt_count),
                            u32_to_i64(job.max_attempts),
                            job.queued_at.as_unix_seconds(),
                            job.started_at.map(UnixTimestamp::as_unix_seconds),
                            job.finished_at.map(UnixTimestamp::as_unix_seconds),
                            job.updated_at.as_unix_seconds(),
                            job.failure_kind.map(ingest_job_failure_kind_to_storage),
                        ],
                    )
                    .map_err(MetaStoreError::storage)?;
                true
            };
            transaction.commit().map_err(MetaStoreError::storage)?;
            scheduled
        };

        let job = self
            .ocr_job_for_document(document_id)?
            .ok_or_else(|| MetaStoreError::not_found("ingest_job"))?;
        Ok(EnqueuedIngestJob { job, scheduled })
    }

    pub fn enqueue_embedding_job_for_resume_version(
        &self,
        document_id: &DocumentId,
        resume_version_id: &ResumeVersionId,
        model_id: &str,
        dimension: usize,
        queued_at: UnixTimestamp,
    ) -> Result<EnqueuedIngestJob> {
        validate_embedding_job_spec(model_id, dimension)?;
        let version = self
            .resume_version_by_id(resume_version_id)?
            .ok_or_else(|| MetaStoreError::not_found("resume_version"))?;
        if &version.document_id != document_id {
            return Err(MetaStoreError::invalid_value(
                "ingest_job.resume_version_id",
            ));
        }

        let dimension_label = dimension.to_string();
        let id = IngestJobId::from_non_secret_parts(&[
            "embedding-version",
            document_id.as_str(),
            resume_version_id.as_str(),
            model_id,
            dimension_label.as_str(),
        ]);
        let job = IngestJob {
            id,
            document_id: document_id.clone(),
            resume_version_id: Some(resume_version_id.clone()),
            kind: IngestJobKind::UpdateIndex,
            status: IngestJobStatus::Queued,
            attempt_count: 0,
            max_attempts: 3,
            queued_at,
            started_at: None,
            finished_at: None,
            updated_at: queued_at,
            failure_kind: None,
        };
        let inserted = {
            let mut connection = self.connection.borrow_mut();
            let transaction = connection.transaction().map_err(MetaStoreError::storage)?;
            let existing_id = {
                let mut statement = transaction
                    .prepare(
                        "\
                        SELECT job.id
                        FROM ingest_job AS job
                        JOIN embedding_job_spec AS spec ON spec.ingest_job_id = job.id
                        WHERE spec.resume_version_id = ?1
                            AND spec.model_id = ?2
                            AND spec.dimension = ?3
                            AND job.kind = ?4
                        LIMIT 1",
                    )
                    .map_err(MetaStoreError::storage)?;
                let mut rows = statement
                    .query(params![
                        resume_version_id.as_str(),
                        model_id,
                        usize_to_i64(dimension, "embedding_job_spec.dimension")?,
                        ingest_job_kind_to_storage(IngestJobKind::UpdateIndex),
                    ])
                    .map_err(MetaStoreError::storage)?;

                match rows.next().map_err(MetaStoreError::storage)? {
                    Some(row) => Some(read_string(row, 0)?),
                    None => None,
                }
            };

            if existing_id.is_some() {
                transaction.commit().map_err(MetaStoreError::storage)?;
                false
            } else {
                transaction
                    .execute(
                        "\
                        INSERT INTO ingest_job (
                            id, document_id, resume_version_id, kind, status, attempt_count,
                            max_attempts, queued_at_seconds, started_at_seconds,
                            finished_at_seconds, updated_at_seconds, failure_kind
                        )
                        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
                        params![
                            job.id.as_str(),
                            job.document_id.as_str(),
                            job.resume_version_id.as_ref().map(ResumeVersionId::as_str),
                            ingest_job_kind_to_storage(job.kind),
                            ingest_job_status_to_storage(job.status),
                            u32_to_i64(job.attempt_count),
                            u32_to_i64(job.max_attempts),
                            job.queued_at.as_unix_seconds(),
                            job.started_at.map(UnixTimestamp::as_unix_seconds),
                            job.finished_at.map(UnixTimestamp::as_unix_seconds),
                            job.updated_at.as_unix_seconds(),
                            job.failure_kind.map(ingest_job_failure_kind_to_storage),
                        ],
                    )
                    .map_err(MetaStoreError::storage)?;
                transaction
                    .execute(
                        "\
                        INSERT INTO embedding_job_spec (
                            ingest_job_id, resume_version_id, model_id, dimension,
                            updated_at_seconds
                        )
                        VALUES (?1, ?2, ?3, ?4, ?5)",
                        params![
                            job.id.as_str(),
                            resume_version_id.as_str(),
                            model_id,
                            usize_to_i64(dimension, "embedding_job_spec.dimension")?,
                            queued_at.as_unix_seconds(),
                        ],
                    )
                    .map_err(MetaStoreError::storage)?;
                transaction.commit().map_err(MetaStoreError::storage)?;
                true
            }
        };

        let job = self
            .embedding_job_for_resume_version(resume_version_id, model_id, dimension)?
            .ok_or_else(|| MetaStoreError::not_found("ingest_job"))?;
        Ok(EnqueuedIngestJob {
            job,
            scheduled: inserted,
        })
    }

    pub fn ocr_job_for_document(&self, document_id: &DocumentId) -> Result<Option<IngestJob>> {
        let connection = self.connection.borrow();
        let sql = format!(
            "\
            SELECT {INGEST_JOB_COLUMNS}
            FROM ingest_job
            WHERE document_id = ?1 AND kind = ?2
            ORDER BY rowid
            LIMIT 1"
        );
        let mut statement = connection.prepare(&sql).map_err(MetaStoreError::storage)?;
        let mut rows = statement
            .query(params![
                document_id.as_str(),
                ingest_job_kind_to_storage(IngestJobKind::OcrDocument),
            ])
            .map_err(MetaStoreError::storage)?;

        match rows.next().map_err(MetaStoreError::storage)? {
            Some(row) => Ok(Some(read_ingest_job(row)?)),
            None => Ok(None),
        }
    }

    fn embedding_job_for_resume_version(
        &self,
        resume_version_id: &ResumeVersionId,
        model_id: &str,
        dimension: usize,
    ) -> Result<Option<IngestJob>> {
        validate_embedding_job_spec(model_id, dimension)?;
        let connection = self.connection.borrow();
        let sql = format!(
            "\
            SELECT {INGEST_JOB_COLUMNS_JOB_ALIAS}
            FROM ingest_job AS job
            JOIN embedding_job_spec AS spec ON spec.ingest_job_id = job.id
            WHERE spec.resume_version_id = ?1
                AND spec.model_id = ?2
                AND spec.dimension = ?3
                AND job.kind = ?4
            LIMIT 1"
        );
        let mut statement = connection.prepare(&sql).map_err(MetaStoreError::storage)?;
        let mut rows = statement
            .query(params![
                resume_version_id.as_str(),
                model_id,
                usize_to_i64(dimension, "embedding_job_spec.dimension")?,
                ingest_job_kind_to_storage(IngestJobKind::UpdateIndex),
            ])
            .map_err(MetaStoreError::storage)?;

        match rows.next().map_err(MetaStoreError::storage)? {
            Some(row) => Ok(Some(read_ingest_job(row)?)),
            None => Ok(None),
        }
    }

    pub fn requeue_completed_embedding_jobs_for_model(
        &self,
        model_id: &str,
        dimension: usize,
        queued_at: UnixTimestamp,
    ) -> Result<usize> {
        validate_embedding_job_spec(model_id, dimension)?;
        let queued_at_seconds = queued_at.as_unix_seconds();
        let mut connection = self.connection.borrow_mut();
        let transaction = connection.transaction().map_err(MetaStoreError::storage)?;
        let changed = transaction
            .execute(
                "\
                UPDATE ingest_job
                SET
                    status = ?1,
                    attempt_count = 0,
                    queued_at_seconds = ?2,
                    started_at_seconds = NULL,
                    finished_at_seconds = NULL,
                    updated_at_seconds = ?2,
                    failure_kind = NULL
                WHERE id IN (
                    SELECT job.id
                    FROM ingest_job AS job
                    JOIN embedding_job_spec AS spec ON spec.ingest_job_id = job.id
                    WHERE job.status = ?3
                        AND job.kind = ?4
                        AND job.resume_version_id IS NOT NULL
                        AND spec.model_id = ?5
                        AND spec.dimension = ?6
                )",
                params![
                    ingest_job_status_to_storage(IngestJobStatus::Queued),
                    queued_at_seconds,
                    ingest_job_status_to_storage(IngestJobStatus::Completed),
                    ingest_job_kind_to_storage(IngestJobKind::UpdateIndex),
                    model_id,
                    usize_to_i64(dimension, "embedding_job_spec.dimension")?,
                ],
            )
            .map_err(MetaStoreError::storage)?;
        transaction
            .execute(
                "\
                UPDATE embedding_job_spec
                SET updated_at_seconds = ?1
                WHERE model_id = ?2
                    AND dimension = ?3
                    AND ingest_job_id IN (
                        SELECT id
                        FROM ingest_job
                        WHERE status = ?4
                            AND kind = ?5
                            AND resume_version_id IS NOT NULL
                            AND updated_at_seconds = ?1
                    )",
                params![
                    queued_at_seconds,
                    model_id,
                    usize_to_i64(dimension, "embedding_job_spec.dimension")?,
                    ingest_job_status_to_storage(IngestJobStatus::Queued),
                    ingest_job_kind_to_storage(IngestJobKind::UpdateIndex),
                ],
            )
            .map_err(MetaStoreError::storage)?;
        transaction.commit().map_err(MetaStoreError::storage)?;
        Ok(changed)
    }

    pub fn update_job_status(
        &self,
        id: &IngestJobId,
        status: IngestJobStatus,
        updated_at: UnixTimestamp,
    ) -> Result<()> {
        self.update_job_status_with_failure_kind(id, status, None, updated_at)
    }

    pub fn update_job_status_with_failure_kind(
        &self,
        id: &IngestJobId,
        status: IngestJobStatus,
        failure_kind: Option<IngestJobFailureKind>,
        updated_at: UnixTimestamp,
    ) -> Result<()> {
        if failure_kind.is_some()
            && !matches!(
                status,
                IngestJobStatus::FailedRetryable | IngestJobStatus::FailedPermanent
            )
        {
            return Err(MetaStoreError::invalid_value("ingest_job.failure_kind"));
        }

        let mut connection = self.connection.borrow_mut();
        let transaction = connection.transaction().map_err(MetaStoreError::storage)?;
        let current_status = {
            let mut statement = transaction
                .prepare("SELECT status FROM ingest_job WHERE id = ?1")
                .map_err(MetaStoreError::storage)?;
            let mut rows = statement
                .query(params![id.as_str()])
                .map_err(MetaStoreError::storage)?;

            match rows.next().map_err(MetaStoreError::storage)? {
                Some(row) => ingest_job_status_from_storage(&read_string(row, 0)?)?,
                None => return Err(MetaStoreError::not_found("ingest_job")),
            }
        };

        if !job_status_transition_allowed(current_status, status) {
            return Err(MetaStoreError::invalid_transition());
        }

        let updated_at_seconds = updated_at.as_unix_seconds();
        let changed = transaction
            .execute(
                "\
                UPDATE ingest_job
                SET
                    status = ?1,
                    started_at_seconds = CASE
                        WHEN ?1 = ?2 THEN ?5
                        ELSE started_at_seconds
                    END,
                    finished_at_seconds = CASE
                        WHEN ?1 = ?2 THEN NULL
                        WHEN ?1 IN (?3, ?4, ?6) THEN ?5
                        ELSE finished_at_seconds
                    END,
                    updated_at_seconds = ?5,
                    failure_kind = CASE
                        WHEN ?1 IN (?4, ?6) THEN ?9
                        ELSE NULL
                    END
                WHERE id = ?7 AND status = ?8",
                params![
                    ingest_job_status_to_storage(status),
                    ingest_job_status_to_storage(IngestJobStatus::Running),
                    ingest_job_status_to_storage(IngestJobStatus::Completed),
                    ingest_job_status_to_storage(IngestJobStatus::FailedRetryable),
                    updated_at_seconds,
                    ingest_job_status_to_storage(IngestJobStatus::FailedPermanent),
                    id.as_str(),
                    ingest_job_status_to_storage(current_status),
                    failure_kind.map(ingest_job_failure_kind_to_storage),
                ],
            )
            .map_err(MetaStoreError::storage)?;

        if changed == 0 {
            return Err(MetaStoreError::invalid_transition());
        }

        transaction.commit().map_err(MetaStoreError::storage)?;
        Ok(())
    }

    pub fn finish_ocr_attempt_failure(
        &self,
        claimed: &ClaimedOcrJob,
        failure: OcrAttemptFailure,
        now: UnixTimestamp,
    ) -> Result<OcrAttemptFailureOutcome> {
        let claimed_job = &claimed.job;
        if claimed_job.kind != IngestJobKind::OcrDocument
            || claimed_job.status != IngestJobStatus::Running
            || claimed_job.attempt_count == 0
        {
            return Err(MetaStoreError::invalid_value("ingest_job.ocr_attempt"));
        }
        let mut connection = self.connection.borrow_mut();
        let transaction = connection.transaction().map_err(MetaStoreError::storage)?;
        let terminal = failure == OcrAttemptFailure::Permanent
            || claimed_job.attempt_count >= claimed_job.max_attempts;
        let failure_kind = match failure {
            OcrAttemptFailure::RetryableWithKind(kind) => Some(kind),
            OcrAttemptFailure::Retryable | OcrAttemptFailure::Permanent => None,
        };
        let next_status = if terminal {
            IngestJobStatus::FailedPermanent
        } else {
            IngestJobStatus::FailedRetryable
        };
        let changed = transaction
            .execute(
                "UPDATE ingest_job
                 SET status = ?1, finished_at_seconds = ?2, updated_at_seconds = ?2,
                     failure_kind = ?3
                 WHERE id = ?4 AND document_id = ?5 AND kind = ?6
                   AND status = ?7 AND attempt_count = ?8 AND max_attempts = ?9",
                params![
                    ingest_job_status_to_storage(next_status),
                    now.as_unix_seconds(),
                    failure_kind.map(ingest_job_failure_kind_to_storage),
                    claimed_job.id.as_str(),
                    claimed_job.document_id.as_str(),
                    ingest_job_kind_to_storage(IngestJobKind::OcrDocument),
                    ingest_job_status_to_storage(IngestJobStatus::Running),
                    u32_to_i64(claimed_job.attempt_count),
                    u32_to_i64(claimed_job.max_attempts),
                ],
            )
            .map_err(MetaStoreError::storage)?;
        if changed == 0 {
            transaction.commit().map_err(MetaStoreError::storage)?;
            return Ok(OcrAttemptFailureOutcome::Superseded);
        }
        if terminal {
            classification::transition_current_ocr_backlog_to_failed(
                &transaction,
                &claimed_job.document_id,
                now,
            )?;
            transaction
                .execute(
                    "UPDATE document SET status = ?1, updated_at_seconds = ?2
                     WHERE id = ?3 AND is_deleted = 0 AND status = ?4",
                    params![
                        document_status_to_storage(DocumentStatus::FailedPermanent),
                        now.as_unix_seconds(),
                        claimed_job.document_id.as_str(),
                        document_status_to_storage(DocumentStatus::OcrRequired),
                    ],
                )
                .map_err(MetaStoreError::storage)?;
        }
        transaction.commit().map_err(MetaStoreError::storage)?;
        Ok(if terminal {
            OcrAttemptFailureOutcome::FailedPermanent
        } else {
            OcrAttemptFailureOutcome::Retryable
        })
    }

    pub fn claim_next_ocr_job(&self, now: UnixTimestamp) -> Result<Option<ClaimedOcrJob>> {
        let Some(job) =
            self.claim_next_job_matching(Some(IngestJobKind::OcrDocument), false, now)?
        else {
            return Ok(None);
        };
        let source_fingerprint = self
            .connection
            .borrow()
            .query_row(
                "SELECT document.content_hash FROM ingest_job AS job
                 JOIN document ON document.id = job.document_id
                 WHERE job.id = ?1 AND job.status = ?2
                   AND job.attempt_count = ?3 AND job.max_attempts = ?4",
                params![
                    job.id.as_str(),
                    ingest_job_status_to_storage(IngestJobStatus::Running),
                    u32_to_i64(job.attempt_count),
                    u32_to_i64(job.max_attempts),
                ],
                |row| row.get::<_, Option<String>>(0),
            )
            .optional()
            .map_err(MetaStoreError::storage)?
            .flatten();
        Ok(source_fingerprint.map(|source_fingerprint| ClaimedOcrJob {
            job,
            source_fingerprint,
        }))
    }

    pub fn ocr_claim_is_current(&self, claimed: &ClaimedOcrJob) -> Result<bool> {
        ocr_claim_is_current_in_connection(&self.connection.borrow(), claimed)
    }

    pub fn finish_ocr_attempt_success(
        &self,
        claimed: &ClaimedOcrJob,
        publication: OcrAttemptPublication<'_>,
        now: UnixTimestamp,
    ) -> Result<OcrAttemptSuccessOutcome> {
        let job = &claimed.job;
        let candidate = publication.classification.status == ClassificationStatus::ResumeCandidate;
        let valid = job.kind == IngestJobKind::OcrDocument
            && job.status == IngestJobStatus::Running
            && job.attempt_count > 0
            && publication.document.id == job.document_id
            && publication.document.content_hash.as_deref() == Some(claimed.source_fingerprint())
            && publication.classification.document_id == job.document_id
            && publication.classification.classifier_epoch == CLASSIFIER_EPOCH
            && publication.classification.status != ClassificationStatus::OcrBacklog
            && (candidate == (publication.document.status == DocumentStatus::Searchable))
            && (candidate != (publication.document.status == DocumentStatus::OcrDone))
            && publication.version.is_none_or(|version| {
                version.document_id == job.document_id
                    && (candidate == (version.visibility == ResumeVisibility::Searchable))
            })
            && (!candidate || publication.version.is_some())
            && (publication.version.is_some() || publication.mentions.is_empty())
            && (candidate || publication.mentions.is_empty())
            && (candidate || publication.email_hash.is_none() && publication.phone_hash.is_none());
        if !valid {
            return Err(MetaStoreError::invalid_value("ingest_job.ocr_publication"));
        }

        let mut connection = self.connection.borrow_mut();
        let transaction = connection.transaction().map_err(MetaStoreError::storage)?;
        let current = ocr_claim_is_current_in_connection(&transaction, claimed)?;
        if !current {
            transaction.commit().map_err(MetaStoreError::storage)?;
            return Ok(OcrAttemptSuccessOutcome::Superseded);
        }

        upsert_document_in_connection(&transaction, publication.document)?;
        classification::upsert_document_classification_in_connection(
            &transaction,
            publication.classification,
        )?;
        if let Some(version) = publication.version {
            let mut version = version.clone();
            if version.candidate_id.is_none() {
                version.candidate_id =
                    resume_version_by_id_from_connection(&transaction, &version.id)?
                        .and_then(|existing| existing.candidate_id);
            }
            upsert_resume_version_in_connection(&transaction, &version)?;
            replace_entity_mentions_in_connection(&transaction, &version.id, publication.mentions)?;
            assign_candidate_from_hashed_contacts_in_connection(
                &transaction,
                &version.id,
                publication.email_hash,
                publication.phone_hash,
                now,
            )?;
        }
        upsert_index_state_in_connection(&transaction, publication.index_state)?;
        let changed = transaction
            .execute(
                "UPDATE ingest_job SET status = ?1, finished_at_seconds = ?2,
                 updated_at_seconds = ?2, failure_kind = NULL
                 WHERE id = ?3 AND status = ?4 AND attempt_count = ?5
                   AND max_attempts = ?6",
                params![
                    ingest_job_status_to_storage(IngestJobStatus::Completed),
                    now.as_unix_seconds(),
                    job.id.as_str(),
                    ingest_job_status_to_storage(IngestJobStatus::Running),
                    u32_to_i64(job.attempt_count),
                    u32_to_i64(job.max_attempts),
                ],
            )
            .map_err(MetaStoreError::storage)?;
        if changed != 1 {
            return Err(MetaStoreError::invalid_transition());
        }
        transaction.commit().map_err(MetaStoreError::storage)?;
        Ok(OcrAttemptSuccessOutcome::Completed)
    }

    pub fn claim_next_job(&self, now: UnixTimestamp) -> Result<Option<IngestJob>> {
        self.claim_next_job_matching(None, false, now)
    }

    pub fn claim_next_job_by_kind(
        &self,
        kind: IngestJobKind,
        now: UnixTimestamp,
    ) -> Result<Option<IngestJob>> {
        self.claim_next_job_matching(Some(kind), false, now)
    }

    pub fn claim_next_embedding_job(
        &self,
        model_id: &str,
        dimension: usize,
        now: UnixTimestamp,
    ) -> Result<Option<IngestJob>> {
        validate_embedding_job_spec(model_id, dimension)?;
        let claimed_id = {
            let mut connection = self.connection.borrow_mut();
            let transaction = connection.transaction().map_err(MetaStoreError::storage)?;
            let candidate_id = {
                let mut statement = transaction
                    .prepare(
                        "\
                        SELECT job.id
                        FROM ingest_job AS job
                        JOIN embedding_job_spec AS spec ON spec.ingest_job_id = job.id
                        WHERE (
                                job.status IN (?1, ?2)
                                OR (job.status = ?3 AND job.attempt_count < job.max_attempts)
                            )
                            AND job.kind = ?4
                            AND job.resume_version_id IS NOT NULL
                            AND spec.model_id = ?5
                            AND spec.dimension = ?6
                        ORDER BY job.queued_at_seconds, job.rowid
                        LIMIT 1",
                    )
                    .map_err(MetaStoreError::storage)?;
                let mut rows = statement
                    .query(params![
                        ingest_job_status_to_storage(IngestJobStatus::Queued),
                        ingest_job_status_to_storage(IngestJobStatus::Interrupted),
                        ingest_job_status_to_storage(IngestJobStatus::FailedRetryable),
                        ingest_job_kind_to_storage(IngestJobKind::UpdateIndex),
                        model_id,
                        usize_to_i64(dimension, "embedding_job_spec.dimension")?,
                    ])
                    .map_err(MetaStoreError::storage)?;

                match rows.next().map_err(MetaStoreError::storage)? {
                    Some(row) => Some(read_string(row, 0)?),
                    None => None,
                }
            };

            let Some(candidate_id) = candidate_id else {
                transaction.commit().map_err(MetaStoreError::storage)?;
                return Ok(None);
            };

            let now_seconds = now.as_unix_seconds();
            let changed = transaction
                .execute(
                    "\
                    UPDATE ingest_job
                    SET
                        status = ?1,
                        attempt_count = attempt_count + 1,
                        started_at_seconds = ?2,
                        finished_at_seconds = NULL,
                        updated_at_seconds = ?2,
                        failure_kind = NULL
                    WHERE id = ?3
                        AND (
                            status IN (?4, ?5)
                            OR (status = ?6 AND attempt_count < max_attempts)
                        )
                        AND kind = ?7
                        AND resume_version_id IS NOT NULL",
                    params![
                        ingest_job_status_to_storage(IngestJobStatus::Running),
                        now_seconds,
                        candidate_id,
                        ingest_job_status_to_storage(IngestJobStatus::Queued),
                        ingest_job_status_to_storage(IngestJobStatus::Interrupted),
                        ingest_job_status_to_storage(IngestJobStatus::FailedRetryable),
                        ingest_job_kind_to_storage(IngestJobKind::UpdateIndex),
                    ],
                )
                .map_err(MetaStoreError::storage)?;

            if changed == 0 {
                transaction.commit().map_err(MetaStoreError::storage)?;
                return Ok(None);
            }

            transaction.commit().map_err(MetaStoreError::storage)?;
            candidate_id
        };

        let claimed_id = IngestJobId::from_str(&claimed_id)
            .map_err(|_| MetaStoreError::invalid_value("ingest_job.id"))?;

        self.ingest_job_by_id(&claimed_id)?
            .ok_or_else(|| MetaStoreError::not_found("ingest_job"))
            .map(Some)
    }

    fn claim_next_job_matching(
        &self,
        kind: Option<IngestJobKind>,
        require_resume_version_id: bool,
        now: UnixTimestamp,
    ) -> Result<Option<IngestJob>> {
        let kind_filter = kind.map(ingest_job_kind_to_storage);
        let claimed_id = {
            let mut connection = self.connection.borrow_mut();
            let transaction = connection.transaction().map_err(MetaStoreError::storage)?;
            let candidate_id = {
                let mut statement = transaction
                    .prepare(
                        "\
                        SELECT id
                        FROM ingest_job
                        WHERE (
                                status = ?1
                                OR (status IN (?2, ?3) AND attempt_count < max_attempts)
                            )
                            AND (?4 IS NULL OR kind = ?4)
                            AND (?5 = 0 OR resume_version_id IS NOT NULL)
                            AND (kind <> ?6 OR EXISTS (
                                SELECT 1 FROM document
                                JOIN document_classification AS classification
                                  ON classification.document_id = document.id
                                WHERE document.id = ingest_job.document_id
                                  AND document.is_deleted = 0 AND document.status = 'ocr_required'
                                  AND document.content_hash IS NOT NULL
                                  AND classification.status = 'ocr_backlog'
                                  AND classification.classifier_epoch = ?7
                            ))
                        ORDER BY queued_at_seconds, rowid
                        LIMIT 1",
                    )
                    .map_err(MetaStoreError::storage)?;
                let mut rows = statement
                    .query(params![
                        ingest_job_status_to_storage(IngestJobStatus::Queued),
                        ingest_job_status_to_storage(IngestJobStatus::Interrupted),
                        ingest_job_status_to_storage(IngestJobStatus::FailedRetryable),
                        kind_filter,
                        bool_to_i64(require_resume_version_id),
                        ingest_job_kind_to_storage(IngestJobKind::OcrDocument),
                        CLASSIFIER_EPOCH,
                    ])
                    .map_err(MetaStoreError::storage)?;

                match rows.next().map_err(MetaStoreError::storage)? {
                    Some(row) => Some(read_string(row, 0)?),
                    None => None,
                }
            };

            let Some(candidate_id) = candidate_id else {
                transaction.commit().map_err(MetaStoreError::storage)?;
                return Ok(None);
            };

            let now_seconds = now.as_unix_seconds();
            let changed = transaction
                .execute(
                    "\
                    UPDATE ingest_job
                    SET
                        status = ?1,
                        attempt_count = attempt_count + 1,
                        started_at_seconds = ?2,
                        finished_at_seconds = NULL,
                        updated_at_seconds = ?2,
                        failure_kind = NULL
                    WHERE id = ?3
                        AND (
                            status = ?4
                            OR (status IN (?5, ?6) AND attempt_count < max_attempts)
                        )
                        AND (?7 IS NULL OR kind = ?7)
                        AND (?8 = 0 OR resume_version_id IS NOT NULL)",
                    params![
                        ingest_job_status_to_storage(IngestJobStatus::Running),
                        now_seconds,
                        candidate_id,
                        ingest_job_status_to_storage(IngestJobStatus::Queued),
                        ingest_job_status_to_storage(IngestJobStatus::Interrupted),
                        ingest_job_status_to_storage(IngestJobStatus::FailedRetryable),
                        kind_filter,
                        bool_to_i64(require_resume_version_id),
                    ],
                )
                .map_err(MetaStoreError::storage)?;

            if changed == 0 {
                transaction.commit().map_err(MetaStoreError::storage)?;
                return Ok(None);
            }

            transaction.commit().map_err(MetaStoreError::storage)?;
            candidate_id
        };

        let claimed_id = IngestJobId::from_str(&claimed_id)
            .map_err(|_| MetaStoreError::invalid_value("ingest_job.id"))?;

        self.ingest_job_by_id(&claimed_id)?
            .ok_or_else(|| MetaStoreError::not_found("ingest_job"))
            .map(Some)
    }

    pub fn retryable_jobs(&self) -> Result<Vec<IngestJob>> {
        self.query_jobs(
            "\
            WHERE status = ?1
                OR (status IN (?2, ?3) AND attempt_count < max_attempts)
            ORDER BY rowid",
            params![
                ingest_job_status_to_storage(IngestJobStatus::Queued),
                ingest_job_status_to_storage(IngestJobStatus::Interrupted),
                ingest_job_status_to_storage(IngestJobStatus::FailedRetryable),
            ],
        )
    }

    pub fn jobs_requiring_recovery(&self) -> Result<Vec<IngestJob>> {
        self.query_jobs(
            "\
            WHERE status = ?1
                OR (status IN (?2, ?3) AND attempt_count < max_attempts)
            ORDER BY rowid",
            params![
                ingest_job_status_to_storage(IngestJobStatus::Running),
                ingest_job_status_to_storage(IngestJobStatus::Interrupted),
                ingest_job_status_to_storage(IngestJobStatus::FailedRetryable),
            ],
        )
    }

    pub fn recover_stale_running_ingest_jobs(
        &self,
        now: UnixTimestamp,
        stale_before: UnixTimestamp,
    ) -> Result<usize> {
        let mut terminalized = 0_usize;
        let exhausted_ocr = self.query_jobs(
            "WHERE kind = ?1 AND status = ?2 AND attempt_count >= max_attempts
               AND updated_at_seconds <= ?3 ORDER BY rowid",
            params![
                ingest_job_kind_to_storage(IngestJobKind::OcrDocument),
                ingest_job_status_to_storage(IngestJobStatus::Running),
                stale_before.as_unix_seconds(),
            ],
        )?;
        for job in exhausted_ocr {
            if let Some(claimed) = self.claimed_ocr_job_from_job(job)? {
                if self.finish_ocr_attempt_failure(&claimed, OcrAttemptFailure::Permanent, now)?
                    == OcrAttemptFailureOutcome::FailedPermanent
                {
                    terminalized += 1;
                }
            }
        }
        let changed = self
            .connection
            .borrow()
            .execute(
                "\
                UPDATE ingest_job
                SET
                    status = ?1,
                    updated_at_seconds = ?2,
                    finished_at_seconds = NULL,
                    failure_kind = NULL
                WHERE status = ?3
                    AND updated_at_seconds <= ?4",
                params![
                    ingest_job_status_to_storage(IngestJobStatus::Interrupted),
                    now.as_unix_seconds(),
                    ingest_job_status_to_storage(IngestJobStatus::Running),
                    stale_before.as_unix_seconds(),
                ],
            )
            .map_err(MetaStoreError::storage)?;
        Ok(terminalized.saturating_add(changed))
    }

    fn claimed_ocr_job_from_job(&self, job: IngestJob) -> Result<Option<ClaimedOcrJob>> {
        let source_fingerprint = self
            .connection
            .borrow()
            .query_row(
                "SELECT document.content_hash FROM ingest_job AS job
                 JOIN document ON document.id = job.document_id WHERE job.id = ?1",
                params![job.id.as_str()],
                |row| row.get::<_, Option<String>>(0),
            )
            .optional()
            .map_err(MetaStoreError::storage)?
            .flatten();
        Ok(source_fingerprint.map(|source_fingerprint| ClaimedOcrJob {
            job,
            source_fingerprint,
        }))
    }

    pub fn ingest_jobs(&self) -> Result<Vec<IngestJob>> {
        self.query_jobs("ORDER BY rowid", params![])
    }

    pub fn upsert_index_state(&self, state: &IndexState) -> Result<()> {
        let connection = self.connection.borrow();
        upsert_index_state_in_connection(&connection, state)
    }

    pub fn index_state(&self) -> Result<Option<IndexState>> {
        let connection = self.connection.borrow();
        let mut statement = connection
            .prepare(
                "\
                SELECT
                    manifest_version,
                    snapshot_token,
                    status,
                    updated_at_seconds,
                    visible_epoch,
                    manifest_document_count
                FROM index_state
                WHERE state_key = ?1",
            )
            .map_err(MetaStoreError::storage)?;
        let mut rows = statement
            .query(params![INDEX_STATE_KEY])
            .map_err(MetaStoreError::storage)?;

        match rows.next().map_err(MetaStoreError::storage)? {
            Some(row) => Ok(Some(read_index_state(row)?)),
            None => Ok(None),
        }
    }

    pub fn insert_import_task(&self, task: &ImportTask) -> Result<()> {
        validate_import_task(task)?;

        let connection = self.connection.borrow();
        connection
            .execute(
                "\
                INSERT INTO import_task (
                    id, root_path, status, queued_at_seconds, started_at_seconds,
                    finished_at_seconds, updated_at_seconds
                )
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    task.id.as_str(),
                    task.root_path,
                    import_task_status_to_storage(task.status),
                    task.queued_at.as_unix_seconds(),
                    task.started_at.map(UnixTimestamp::as_unix_seconds),
                    task.finished_at.map(UnixTimestamp::as_unix_seconds),
                    task.updated_at.as_unix_seconds(),
                ],
            )
            .map_err(MetaStoreError::storage)?;

        Ok(())
    }

    pub fn insert_import_task_with_scan_scope(
        &self,
        task: &ImportTask,
        scope: &ImportScanScope,
    ) -> Result<()> {
        validate_import_task(task)?;
        validate_import_scan_scope(scope)?;
        if scope.import_task_id != task.id {
            return Err(MetaStoreError::invalid_value(
                "import_scan_scope.import_task_id",
            ));
        }
        if scope.canonical_root_path != task.root_path {
            return Err(MetaStoreError::invalid_value("import_task.root_path"));
        }

        let mut connection = self.connection.borrow_mut();
        let transaction = connection.transaction().map_err(MetaStoreError::storage)?;
        transaction
            .execute(
                "\
                INSERT INTO import_task (
                    id, root_path, status, queued_at_seconds, started_at_seconds,
                    finished_at_seconds, updated_at_seconds
                )
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    task.id.as_str(),
                    task.root_path,
                    import_task_status_to_storage(task.status),
                    task.queued_at.as_unix_seconds(),
                    task.started_at.map(UnixTimestamp::as_unix_seconds),
                    task.finished_at.map(UnixTimestamp::as_unix_seconds),
                    task.updated_at.as_unix_seconds(),
                ],
            )
            .map_err(MetaStoreError::storage)?;
        transaction
            .execute(
                "\
                INSERT INTO import_scan_scope (
                    import_task_id, root_kind, root_preset, scan_profile, requested_root_path,
                    canonical_root_path, files_discovered, ignored_entries, scan_errors,
                    searchable_documents, ocr_required_documents, ocr_jobs_queued,
                    failed_documents, deleted_documents, scan_budget_kind, scan_budget_limit,
                    scan_budget_observed, scan_budget_exhausted, updated_at_seconds
                )
                VALUES (
                    ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17,
                    ?18, ?19
                )",
                params![
                    scope.import_task_id.as_str(),
                    import_root_kind_to_storage(scope.root_kind),
                    scope.root_preset.map(import_root_preset_to_storage),
                    import_scan_profile_to_storage(scope.scan_profile),
                    scope.requested_root_path.as_str(),
                    scope.canonical_root_path.as_str(),
                    u64_to_i64(scope.files_discovered, "import_scan_scope.files_discovered")?,
                    u64_to_i64(scope.ignored_entries, "import_scan_scope.ignored_entries")?,
                    u64_to_i64(scope.scan_errors, "import_scan_scope.scan_errors")?,
                    u64_to_i64(
                        scope.searchable_documents,
                        "import_scan_scope.searchable_documents"
                    )?,
                    u64_to_i64(
                        scope.ocr_required_documents,
                        "import_scan_scope.ocr_required_documents"
                    )?,
                    u64_to_i64(scope.ocr_jobs_queued, "import_scan_scope.ocr_jobs_queued")?,
                    u64_to_i64(scope.failed_documents, "import_scan_scope.failed_documents")?,
                    u64_to_i64(
                        scope.deleted_documents,
                        "import_scan_scope.deleted_documents"
                    )?,
                    scope
                        .scan_budget_kind
                        .map(import_scan_budget_kind_to_storage),
                    scope
                        .scan_budget_limit
                        .map(|value| u64_to_i64(value, "import_scan_scope.scan_budget_limit"))
                        .transpose()?,
                    scope
                        .scan_budget_observed
                        .map(|value| u64_to_i64(value, "import_scan_scope.scan_budget_observed"))
                        .transpose()?,
                    bool_to_i64(scope.scan_budget_exhausted),
                    scope.updated_at.as_unix_seconds(),
                ],
            )
            .map_err(MetaStoreError::storage)?;
        transaction.commit().map_err(MetaStoreError::storage)?;

        Ok(())
    }

    pub fn import_task_by_id(&self, id: &ImportTaskId) -> Result<Option<ImportTask>> {
        let connection = self.connection.borrow();
        let sql = format!("SELECT {IMPORT_TASK_COLUMNS} FROM import_task WHERE id = ?1");
        let mut statement = connection.prepare(&sql).map_err(MetaStoreError::storage)?;
        let mut rows = statement
            .query(params![id.as_str()])
            .map_err(MetaStoreError::storage)?;

        match rows.next().map_err(MetaStoreError::storage)? {
            Some(row) => Ok(Some(read_import_task(row)?)),
            None => Ok(None),
        }
    }

    pub fn cancel_import_task(
        &self,
        id: &ImportTaskId,
        requested_at: UnixTimestamp,
    ) -> Result<bool> {
        let mut connection = self.connection.borrow_mut();
        let transaction = connection.transaction().map_err(MetaStoreError::storage)?;
        let current_task = {
            let sql = format!("SELECT {IMPORT_TASK_COLUMNS} FROM import_task WHERE id = ?1");
            let mut statement = transaction.prepare(&sql).map_err(MetaStoreError::storage)?;
            let mut rows = statement
                .query(params![id.as_str()])
                .map_err(MetaStoreError::storage)?;

            match rows.next().map_err(MetaStoreError::storage)? {
                Some(row) => read_import_task(row)?,
                None => return Err(MetaStoreError::not_found("import_task")),
            }
        };

        if !matches!(
            current_task.status,
            ImportTaskStatus::Queued
                | ImportTaskStatus::Running
                | ImportTaskStatus::FailedRetryable
        ) {
            return Err(MetaStoreError::invalid_transition());
        }
        if requested_at.as_unix_seconds() < current_task.updated_at.as_unix_seconds() {
            return Err(MetaStoreError::invalid_value("import_task.timestamps"));
        }

        let requested_at_seconds = requested_at.as_unix_seconds();
        transaction
            .execute(
                "\
                UPDATE import_task
                SET updated_at_seconds = ?1
                WHERE id = ?2",
                params![requested_at_seconds, id.as_str()],
            )
            .map_err(MetaStoreError::storage)?;
        let inserted = transaction
            .execute(
                "\
                INSERT OR IGNORE INTO import_task_cancellation (
                    import_task_id, requested_at_seconds
                )
                VALUES (?1, ?2)",
                params![id.as_str(), requested_at_seconds],
            )
            .map_err(MetaStoreError::storage)?;

        transaction.commit().map_err(MetaStoreError::storage)?;
        Ok(inserted > 0)
    }

    pub fn is_import_task_cancelled(&self, id: &ImportTaskId) -> Result<bool> {
        let connection = self.connection.borrow();
        let exists = connection
            .query_row(
                "\
                SELECT EXISTS(
                    SELECT 1
                    FROM import_task_cancellation
                    WHERE import_task_id = ?1
                )",
                params![id.as_str()],
                |row| row.get::<_, i64>(0),
            )
            .map_err(MetaStoreError::storage)?;
        Ok(exists == 1)
    }

    pub fn latest_import_task_by_root(&self, root_path: &str) -> Result<Option<ImportTask>> {
        let connection = self.connection.borrow();
        let sql = format!(
            "\
            SELECT {IMPORT_TASK_COLUMNS}
            FROM import_task
            WHERE root_path = ?1
            ORDER BY updated_at_seconds DESC, rowid DESC
            LIMIT 1"
        );
        let mut statement = connection.prepare(&sql).map_err(MetaStoreError::storage)?;
        let mut rows = statement
            .query(params![root_path])
            .map_err(MetaStoreError::storage)?;

        match rows.next().map_err(MetaStoreError::storage)? {
            Some(row) => Ok(Some(read_import_task(row)?)),
            None => Ok(None),
        }
    }

    pub fn pending_import_task_by_root(&self, root_path: &str) -> Result<Option<ImportTask>> {
        let connection = self.connection.borrow();
        let sql = pending_import_task_by_root_sql();
        let mut statement = connection.prepare(&sql).map_err(MetaStoreError::storage)?;
        let mut rows = statement
            .query(params![
                root_path,
                import_task_status_to_storage(ImportTaskStatus::Queued),
                import_task_status_to_storage(ImportTaskStatus::Running),
                import_task_status_to_storage(ImportTaskStatus::FailedRetryable),
            ])
            .map_err(MetaStoreError::storage)?;

        match rows.next().map_err(MetaStoreError::storage)? {
            Some(row) => Ok(Some(read_import_task(row)?)),
            None => Ok(None),
        }
    }

    pub fn diagnose_pending_import_task_by_root(
        &self,
        root_path: &str,
    ) -> std::result::Result<(), PendingImportTaskByRootDiagnostic> {
        let connection = self.connection.borrow();
        let sql = pending_import_task_by_root_sql();
        let mut statement = connection
            .prepare(&sql)
            .map_err(|_| PendingImportTaskByRootDiagnostic::QueryFailure)?;
        let mut rows = statement
            .query(params![
                root_path,
                import_task_status_to_storage(ImportTaskStatus::Queued),
                import_task_status_to_storage(ImportTaskStatus::Running),
                import_task_status_to_storage(ImportTaskStatus::FailedRetryable),
            ])
            .map_err(|_| PendingImportTaskByRootDiagnostic::QueryFailure)?;

        match rows
            .next()
            .map_err(|_| PendingImportTaskByRootDiagnostic::QueryFailure)?
        {
            Some(row) => read_import_task(row)
                .map(|_| ())
                .map_err(|_| PendingImportTaskByRootDiagnostic::RowMaterializationFailure),
            None => Ok(()),
        }
    }

    pub fn completed_import_scan_scopes_due_for_requeue(
        &self,
        finished_at_or_before: UnixTimestamp,
    ) -> Result<Vec<ImportScanScope>> {
        let connection = self.connection.borrow();
        let sql = "\
            SELECT
                scope.import_task_id,
                scope.root_kind,
                scope.root_preset,
                scope.scan_profile,
                scope.requested_root_path,
                scope.canonical_root_path,
                scope.files_discovered,
                scope.ignored_entries,
                scope.scan_errors,
                scope.searchable_documents,
                scope.ocr_required_documents,
                scope.ocr_jobs_queued,
                scope.failed_documents,
                scope.deleted_documents,
                scope.scan_budget_kind,
                scope.scan_budget_limit,
                scope.scan_budget_observed,
                scope.scan_budget_exhausted,
                scope.updated_at_seconds
            FROM import_scan_scope AS scope
            JOIN import_task AS task ON task.id = scope.import_task_id
            WHERE task.status = ?1
                AND task.finished_at_seconds <= ?2
                AND task.rowid = (
                    SELECT latest.rowid
                    FROM import_task AS latest
                    WHERE latest.root_path = task.root_path
                        AND latest.status = ?1
                    ORDER BY latest.finished_at_seconds DESC, latest.rowid DESC
                    LIMIT 1
                )
                AND NOT EXISTS (
                    SELECT 1
                    FROM import_task AS pending
                    WHERE pending.root_path = task.root_path
                        AND pending.status IN (?3, ?4, ?5)
                        AND NOT EXISTS (
                            SELECT 1
                            FROM import_task_cancellation AS cancellation
                            WHERE cancellation.import_task_id = pending.id
                        )
                )
            ORDER BY task.finished_at_seconds, task.rowid";
        let mut statement = connection.prepare(sql).map_err(MetaStoreError::storage)?;
        let mut rows = statement
            .query(params![
                import_task_status_to_storage(ImportTaskStatus::Completed),
                finished_at_or_before.as_unix_seconds(),
                import_task_status_to_storage(ImportTaskStatus::Queued),
                import_task_status_to_storage(ImportTaskStatus::Running),
                import_task_status_to_storage(ImportTaskStatus::FailedRetryable),
            ])
            .map_err(MetaStoreError::storage)?;
        let mut scopes = Vec::new();

        while let Some(row) = rows.next().map_err(MetaStoreError::storage)? {
            scopes.push(read_import_scan_scope(row)?);
        }

        Ok(scopes)
    }

    pub fn claim_next_import_task_for_worker(
        &self,
        updated_at: UnixTimestamp,
    ) -> Result<Option<ImportTask>> {
        self.claim_next_import_task_for_worker_excluding(updated_at, &[])
    }

    pub fn claim_next_import_task_for_worker_excluding(
        &self,
        updated_at: UnixTimestamp,
        excluded_ids: &[ImportTaskId],
    ) -> Result<Option<ImportTask>> {
        self.claim_next_import_task_for_worker_excluding_due_at(
            updated_at,
            updated_at,
            excluded_ids,
        )
    }

    pub fn claim_next_import_task_for_worker_excluding_due_at(
        &self,
        updated_at: UnixTimestamp,
        retryable_updated_at_or_before: UnixTimestamp,
        excluded_ids: &[ImportTaskId],
    ) -> Result<Option<ImportTask>> {
        let connection = self.connection.borrow();
        let excluded_clause = if excluded_ids.is_empty() {
            String::new()
        } else {
            format!(
                " AND id NOT IN ({})",
                std::iter::repeat_n("?", excluded_ids.len())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        };
        let sql = format!(
            "\
            UPDATE import_task
            SET
                status = ?,
                started_at_seconds = ?,
                finished_at_seconds = NULL,
                updated_at_seconds = ?
            WHERE rowid = (
                SELECT rowid
                FROM import_task
                WHERE (
                        (status = ? AND updated_at_seconds <= ?)
                        OR (status = ? AND updated_at_seconds <= ?)
                    )
                    AND NOT EXISTS (
                        SELECT 1
                        FROM import_task_cancellation AS cancellation
                        WHERE cancellation.import_task_id = import_task.id
                    )
                    {excluded_clause}
                ORDER BY CASE WHEN status = ? THEN 0 ELSE 1 END, queued_at_seconds, rowid
                LIMIT 1
            )
            RETURNING {IMPORT_TASK_COLUMNS}"
        );
        let mut statement = connection.prepare(&sql).map_err(MetaStoreError::storage)?;
        let updated_at_seconds = updated_at.as_unix_seconds();
        let retryable_due_seconds = retryable_updated_at_or_before.as_unix_seconds();
        let queued = import_task_status_to_storage(ImportTaskStatus::Queued);
        let retryable = import_task_status_to_storage(ImportTaskStatus::FailedRetryable);
        let mut values = vec![
            Value::Text(import_task_status_to_storage(ImportTaskStatus::Running).to_string()),
            Value::Integer(updated_at_seconds),
            Value::Integer(updated_at_seconds),
            Value::Text(queued.to_string()),
            Value::Integer(updated_at_seconds),
            Value::Text(retryable.to_string()),
            Value::Integer(retryable_due_seconds),
        ];
        values.extend(
            excluded_ids
                .iter()
                .map(|id| Value::Text(id.as_str().to_string())),
        );
        values.push(Value::Text(queued.to_string()));
        let mut rows = statement
            .query(params_from_iter(values))
            .map_err(MetaStoreError::storage)?;

        match rows.next().map_err(MetaStoreError::storage)? {
            Some(row) => Ok(Some(read_import_task(row)?)),
            None => Ok(None),
        }
    }

    pub fn recover_stale_running_import_tasks(
        &self,
        updated_at: UnixTimestamp,
        running_updated_at_or_before: UnixTimestamp,
    ) -> Result<usize> {
        let connection = self.connection.borrow();
        let updated_at_seconds = updated_at.as_unix_seconds();
        let changed = connection
            .execute(
                "\
                UPDATE import_task
                SET
                    status = ?1,
                    finished_at_seconds = ?2,
                    updated_at_seconds = ?2
                WHERE status = ?3 AND updated_at_seconds <= ?4",
                params![
                    import_task_status_to_storage(ImportTaskStatus::FailedRetryable),
                    updated_at_seconds,
                    import_task_status_to_storage(ImportTaskStatus::Running),
                    running_updated_at_or_before.as_unix_seconds(),
                ],
            )
            .map_err(MetaStoreError::storage)?;
        Ok(changed)
    }

    pub fn heartbeat_running_import_task(
        &self,
        id: &ImportTaskId,
        updated_at: UnixTimestamp,
    ) -> Result<bool> {
        let connection = self.connection.borrow();
        let updated_at_seconds = updated_at.as_unix_seconds();
        let changed = connection
            .execute(
                "\
                UPDATE import_task
                SET updated_at_seconds = ?1
                WHERE id = ?2 AND status = ?3 AND updated_at_seconds <= ?1",
                params![
                    updated_at_seconds,
                    id.as_str(),
                    import_task_status_to_storage(ImportTaskStatus::Running),
                ],
            )
            .map_err(MetaStoreError::storage)?;
        Ok(changed > 0)
    }

    pub fn upsert_import_scan_scope(&self, scope: &ImportScanScope) -> Result<()> {
        validate_import_scan_scope(scope)?;

        let connection = self.connection.borrow();
        connection
            .execute(
                "\
                INSERT INTO import_scan_scope (
                    import_task_id, root_kind, root_preset, scan_profile, requested_root_path,
                    canonical_root_path, files_discovered, ignored_entries, scan_errors,
                    searchable_documents, ocr_required_documents, ocr_jobs_queued,
                    failed_documents, deleted_documents, scan_budget_kind, scan_budget_limit,
                    scan_budget_observed, scan_budget_exhausted, updated_at_seconds
                )
                VALUES (
                    ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17,
                    ?18, ?19
                )
                ON CONFLICT(import_task_id) DO UPDATE SET
                    root_kind = excluded.root_kind,
                    root_preset = excluded.root_preset,
                    scan_profile = excluded.scan_profile,
                    requested_root_path = excluded.requested_root_path,
                    canonical_root_path = excluded.canonical_root_path,
                    files_discovered = excluded.files_discovered,
                    ignored_entries = excluded.ignored_entries,
                    scan_errors = excluded.scan_errors,
                    searchable_documents = excluded.searchable_documents,
                    ocr_required_documents = excluded.ocr_required_documents,
                    ocr_jobs_queued = excluded.ocr_jobs_queued,
                    failed_documents = excluded.failed_documents,
                    deleted_documents = excluded.deleted_documents,
                    scan_budget_kind = excluded.scan_budget_kind,
                    scan_budget_limit = excluded.scan_budget_limit,
                    scan_budget_observed = excluded.scan_budget_observed,
                    scan_budget_exhausted = excluded.scan_budget_exhausted,
                    updated_at_seconds = excluded.updated_at_seconds",
                params![
                    scope.import_task_id.as_str(),
                    import_root_kind_to_storage(scope.root_kind),
                    scope.root_preset.map(import_root_preset_to_storage),
                    import_scan_profile_to_storage(scope.scan_profile),
                    scope.requested_root_path.as_str(),
                    scope.canonical_root_path.as_str(),
                    u64_to_i64(scope.files_discovered, "import_scan_scope.files_discovered")?,
                    u64_to_i64(scope.ignored_entries, "import_scan_scope.ignored_entries")?,
                    u64_to_i64(scope.scan_errors, "import_scan_scope.scan_errors")?,
                    u64_to_i64(
                        scope.searchable_documents,
                        "import_scan_scope.searchable_documents"
                    )?,
                    u64_to_i64(
                        scope.ocr_required_documents,
                        "import_scan_scope.ocr_required_documents"
                    )?,
                    u64_to_i64(scope.ocr_jobs_queued, "import_scan_scope.ocr_jobs_queued")?,
                    u64_to_i64(scope.failed_documents, "import_scan_scope.failed_documents")?,
                    u64_to_i64(
                        scope.deleted_documents,
                        "import_scan_scope.deleted_documents"
                    )?,
                    scope
                        .scan_budget_kind
                        .map(import_scan_budget_kind_to_storage),
                    scope
                        .scan_budget_limit
                        .map(|value| u64_to_i64(value, "import_scan_scope.scan_budget_limit"))
                        .transpose()?,
                    scope
                        .scan_budget_observed
                        .map(|value| u64_to_i64(value, "import_scan_scope.scan_budget_observed"))
                        .transpose()?,
                    bool_to_i64(scope.scan_budget_exhausted),
                    scope.updated_at.as_unix_seconds(),
                ],
            )
            .map_err(MetaStoreError::storage)?;

        Ok(())
    }

    pub fn import_scan_scope_by_task_id(
        &self,
        id: &ImportTaskId,
    ) -> Result<Option<ImportScanScope>> {
        let connection = self.connection.borrow();
        let sql = format!(
            "SELECT {IMPORT_SCAN_SCOPE_COLUMNS} FROM import_scan_scope WHERE import_task_id = ?1"
        );
        let mut statement = connection.prepare(&sql).map_err(MetaStoreError::storage)?;
        let mut rows = statement
            .query(params![id.as_str()])
            .map_err(MetaStoreError::storage)?;

        match rows.next().map_err(MetaStoreError::storage)? {
            Some(row) => Ok(Some(read_import_scan_scope(row)?)),
            None => Ok(None),
        }
    }

    pub fn latest_import_scan_scope(&self) -> Result<Option<ImportScanScope>> {
        let connection = self.connection.borrow();
        let sql = format!(
            "\
            SELECT {IMPORT_SCAN_SCOPE_COLUMNS}
            FROM import_scan_scope
            ORDER BY updated_at_seconds DESC, rowid DESC
            LIMIT 1"
        );
        let mut statement = connection.prepare(&sql).map_err(MetaStoreError::storage)?;
        let mut rows = statement.query([]).map_err(MetaStoreError::storage)?;

        match rows.next().map_err(MetaStoreError::storage)? {
            Some(row) => Ok(Some(read_import_scan_scope(row)?)),
            None => Ok(None),
        }
    }

    pub fn replace_import_scan_errors(
        &self,
        task_id: &ImportTaskId,
        errors: &[ImportScanError],
    ) -> Result<()> {
        for error in errors {
            validate_import_scan_error(task_id, error)?;
        }

        let mut connection = self.connection.borrow_mut();
        let transaction = connection.transaction().map_err(MetaStoreError::storage)?;
        transaction
            .execute(
                "DELETE FROM import_scan_error WHERE import_task_id = ?1",
                params![task_id.as_str()],
            )
            .map_err(MetaStoreError::storage)?;

        {
            let mut statement = transaction
                .prepare(
                    "\
                    INSERT INTO import_scan_error (
                        import_task_id, error_index, kind, operation, path_digest,
                        updated_at_seconds
                    )
                    VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                )
                .map_err(MetaStoreError::storage)?;

            for error in errors {
                statement
                    .execute(params![
                        error.import_task_id.as_str(),
                        u64_to_i64(error.error_index, "import_scan_error.error_index")?,
                        import_scan_error_kind_to_storage(error.kind),
                        import_scan_error_operation_to_storage(error.operation),
                        error.path_digest.as_deref(),
                        error.updated_at.as_unix_seconds(),
                    ])
                    .map_err(MetaStoreError::storage)?;
            }
        }

        transaction.commit().map_err(MetaStoreError::storage)?;
        Ok(())
    }

    pub fn import_scan_errors_for_task(
        &self,
        task_id: &ImportTaskId,
    ) -> Result<Vec<ImportScanError>> {
        let connection = self.connection.borrow();
        let sql = format!(
            "\
            SELECT {IMPORT_SCAN_ERROR_COLUMNS}
            FROM import_scan_error
            WHERE import_task_id = ?1
            ORDER BY error_index"
        );
        let mut statement = connection.prepare(&sql).map_err(MetaStoreError::storage)?;
        let mut rows = statement
            .query(params![task_id.as_str()])
            .map_err(MetaStoreError::storage)?;
        let mut errors = Vec::new();

        while let Some(row) = rows.next().map_err(MetaStoreError::storage)? {
            errors.push(read_import_scan_error(row)?);
        }

        Ok(errors)
    }

    pub fn import_scan_error_breakdown(&self) -> Result<Vec<ImportScanErrorSummary>> {
        let connection = self.connection.borrow();
        let mut statement = connection
            .prepare(
                "\
                SELECT kind, operation, COUNT(*)
                FROM import_scan_error
                GROUP BY kind, operation
                ORDER BY
                    CASE kind
                        WHEN 'permission_denied' THEN 0
                        WHEN 'source_unavailable' THEN 1
                        WHEN 'locked_or_unreadable' THEN 2
                        WHEN 'io' THEN 3
                        ELSE 4
                    END,
                    CASE operation
                        WHEN 'normalize_path' THEN 0
                        WHEN 'read_directory' THEN 1
                        WHEN 'read_metadata' THEN 2
                        WHEN 'fingerprint' THEN 3
                        ELSE 4
                    END",
            )
            .map_err(MetaStoreError::storage)?;
        let mut rows = statement.query([]).map_err(MetaStoreError::storage)?;
        let mut summaries = Vec::new();

        while let Some(row) = rows.next().map_err(MetaStoreError::storage)? {
            summaries.push(ImportScanErrorSummary {
                kind: import_scan_error_kind_from_storage(&read_string(row, 0)?)?,
                operation: import_scan_error_operation_from_storage(&read_string(row, 1)?)?,
                count: i64_to_u64(read_i64(row, 2)?, "import_scan_error.count")?,
            });
        }

        Ok(summaries)
    }

    pub fn update_import_task_status(
        &self,
        id: &ImportTaskId,
        status: ImportTaskStatus,
        updated_at: UnixTimestamp,
    ) -> Result<()> {
        let mut connection = self.connection.borrow_mut();
        let transaction = connection.transaction().map_err(MetaStoreError::storage)?;
        let current_task = {
            let sql = format!("SELECT {IMPORT_TASK_COLUMNS} FROM import_task WHERE id = ?1");
            let mut statement = transaction.prepare(&sql).map_err(MetaStoreError::storage)?;
            let mut rows = statement
                .query(params![id.as_str()])
                .map_err(MetaStoreError::storage)?;

            match rows.next().map_err(MetaStoreError::storage)? {
                Some(row) => read_import_task(row)?,
                None => return Err(MetaStoreError::not_found("import_task")),
            }
        };
        let current_status = current_task.status;

        if updated_at.as_unix_seconds() < current_task.updated_at.as_unix_seconds() {
            return Err(MetaStoreError::invalid_value("import_task.timestamps"));
        }

        if !import_task_status_transition_allowed(current_status, status) {
            return Err(MetaStoreError::invalid_transition());
        }
        let next_task = next_import_task_state(&current_task, status, updated_at);
        validate_import_task(&next_task)?;

        let updated_at_seconds = updated_at.as_unix_seconds();
        let changed = transaction
            .execute(
                "\
                UPDATE import_task
                SET
                    status = ?1,
                    started_at_seconds = CASE
                        WHEN ?1 = ?2 THEN ?5
                        ELSE started_at_seconds
                    END,
                    finished_at_seconds = CASE
                        WHEN ?1 = ?2 THEN NULL
                        WHEN ?1 IN (?3, ?4, ?6) THEN ?5
                        ELSE finished_at_seconds
                    END,
                    updated_at_seconds = ?5
                WHERE id = ?7 AND status = ?8",
                params![
                    import_task_status_to_storage(status),
                    import_task_status_to_storage(ImportTaskStatus::Running),
                    import_task_status_to_storage(ImportTaskStatus::Completed),
                    import_task_status_to_storage(ImportTaskStatus::FailedRetryable),
                    updated_at_seconds,
                    import_task_status_to_storage(ImportTaskStatus::FailedPermanent),
                    id.as_str(),
                    import_task_status_to_storage(current_status),
                ],
            )
            .map_err(MetaStoreError::storage)?;

        if changed == 0 {
            return Err(MetaStoreError::invalid_transition());
        }

        transaction.commit().map_err(MetaStoreError::storage)?;
        Ok(())
    }

    pub fn status_summary(&self) -> Result<StoreStatusSummary> {
        let connection = self.connection.borrow();
        let document_counts = connection
            .query_row(
                "\
                SELECT
                    COALESCE(SUM(CASE WHEN status IN ('indexed_partial', 'searchable') THEN 1 ELSE 0 END), 0),
                    COALESCE(SUM(CASE WHEN status = 'searchable' THEN 1 ELSE 0 END), 0),
                    COALESCE(SUM(CASE WHEN status = 'indexed_partial' THEN 1 ELSE 0 END), 0),
                    COALESCE(SUM(CASE WHEN status = 'failed_retryable' THEN 1 ELSE 0 END), 0),
                    COALESCE(SUM(CASE WHEN status = 'failed_permanent' THEN 1 ELSE 0 END), 0),
                    COALESCE(SUM(CASE WHEN status = 'ocr_required' THEN 1 ELSE 0 END), 0)
                FROM document
                WHERE is_deleted = 0 AND status <> 'deleted'",
                [],
                |row| {
                    Ok(DocumentStatusCounts {
                        indexed_documents: row.get(0)?,
                        searchable_documents: row.get(1)?,
                        partial_documents: row.get(2)?,
                        failed_retryable: row.get(3)?,
                        failed_permanent: row.get(4)?,
                        ocr_queue_depth: row.get(5)?,
                    })
                },
            )
            .map_err(MetaStoreError::storage)?;
        let recovery_queue_depth = connection
            .query_row(
                "\
                SELECT COUNT(*)
                FROM ingest_job
                WHERE status = ?1
                    OR (status IN (?2, ?3) AND attempt_count < max_attempts)",
                params![
                    ingest_job_status_to_storage(IngestJobStatus::Running),
                    ingest_job_status_to_storage(IngestJobStatus::Interrupted),
                    ingest_job_status_to_storage(IngestJobStatus::FailedRetryable),
                ],
                |row| row.get::<_, i64>(0),
            )
            .map_err(MetaStoreError::storage)?;
        let ocr_jobs_queued = connection
            .query_row(
                "\
                SELECT COUNT(*)
                FROM ingest_job
                WHERE kind = ?1
                    AND (
                        status = ?2
                        OR (status IN (?3, ?4) AND attempt_count < max_attempts)
                    )",
                params![
                    ingest_job_kind_to_storage(IngestJobKind::OcrDocument),
                    ingest_job_status_to_storage(IngestJobStatus::Queued),
                    ingest_job_status_to_storage(IngestJobStatus::Interrupted),
                    ingest_job_status_to_storage(IngestJobStatus::FailedRetryable),
                ],
                |row| row.get::<_, i64>(0),
            )
            .map_err(MetaStoreError::storage)?;
        let ocr_page_budget_blocked = connection
            .query_row(
                "\
                SELECT COUNT(*)
                FROM ingest_job AS job
                JOIN document AS document ON document.id = job.document_id
                WHERE job.kind = ?1
                    AND job.status IN (?2, ?3)
                    AND job.failure_kind = ?4
                    AND document.is_deleted = 0
                    AND document.status <> ?5",
                params![
                    ingest_job_kind_to_storage(IngestJobKind::OcrDocument),
                    ingest_job_status_to_storage(IngestJobStatus::FailedRetryable),
                    ingest_job_status_to_storage(IngestJobStatus::FailedPermanent),
                    ingest_job_failure_kind_to_storage(IngestJobFailureKind::OcrPageBudgetExceeded),
                    document_status_to_storage(DocumentStatus::Deleted),
                ],
                |row| row.get::<_, i64>(0),
            )
            .map_err(MetaStoreError::storage)?;
        let ocr_language_unavailable = connection
            .query_row(
                "\
                SELECT COUNT(DISTINCT job.id)
                FROM ingest_job AS job
                JOIN document AS document ON document.id = job.document_id
                WHERE job.kind = ?1
                    AND job.status IN (?2, ?3)
                    AND document.is_deleted = 0
                    AND document.status <> ?4
                    AND EXISTS (
                        SELECT 1
                        FROM ocr_page_cache AS cache
                        WHERE cache.file_content_hash = document.content_hash
                            AND cache.status = ?5
                            AND cache.error_kind = ?6
                    )",
                params![
                    ingest_job_kind_to_storage(IngestJobKind::OcrDocument),
                    ingest_job_status_to_storage(IngestJobStatus::FailedRetryable),
                    ingest_job_status_to_storage(IngestJobStatus::FailedPermanent),
                    document_status_to_storage(DocumentStatus::Deleted),
                    ocr_page_cache_status_to_storage(OcrPageCacheStatus::FailedRetryable),
                    "LanguageUnavailable",
                ],
                |row| row.get::<_, i64>(0),
            )
            .map_err(MetaStoreError::storage)?;
        let embedding_jobs_queued = connection
            .query_row(
                "\
                SELECT COUNT(*)
                FROM ingest_job AS job
                JOIN embedding_job_spec AS spec ON spec.ingest_job_id = job.id
                JOIN document AS document ON document.id = job.document_id
                JOIN resume_version AS version ON version.id = job.resume_version_id
                WHERE job.kind = ?1
                    AND job.resume_version_id IS NOT NULL
                    AND document.is_deleted = 0
                    AND document.status <> ?2
                    AND version.visibility <> ?3
                    AND (
                        job.status IN (?4, ?5)
                        OR (job.status = ?6 AND job.attempt_count < job.max_attempts)
                    )",
                params![
                    ingest_job_kind_to_storage(IngestJobKind::UpdateIndex),
                    document_status_to_storage(DocumentStatus::Deleted),
                    resume_visibility_to_storage(ResumeVisibility::Hidden),
                    ingest_job_status_to_storage(IngestJobStatus::Queued),
                    ingest_job_status_to_storage(IngestJobStatus::Interrupted),
                    ingest_job_status_to_storage(IngestJobStatus::FailedRetryable),
                ],
                |row| row.get::<_, i64>(0),
            )
            .map_err(MetaStoreError::storage)?;
        let import_tasks_queued = connection
            .query_row(
                "\
                SELECT COUNT(*)
                FROM import_task
                WHERE status = ?1
                    AND NOT EXISTS (
                        SELECT 1
                        FROM import_task_cancellation AS cancellation
                        WHERE cancellation.import_task_id = import_task.id
                    )",
                params![import_task_status_to_storage(ImportTaskStatus::Queued)],
                |row| row.get::<_, i64>(0),
            )
            .map_err(MetaStoreError::storage)?;
        let import_tasks_recoverable = connection
            .query_row(
                "\
                SELECT COUNT(*)
                FROM import_task
                WHERE status IN (?1, ?2)
                    AND NOT EXISTS (
                        SELECT 1
                        FROM import_task_cancellation AS cancellation
                        WHERE cancellation.import_task_id = import_task.id
                    )",
                params![
                    import_task_status_to_storage(ImportTaskStatus::Running),
                    import_task_status_to_storage(ImportTaskStatus::FailedRetryable),
                ],
                |row| row.get::<_, i64>(0),
            )
            .map_err(MetaStoreError::storage)?;
        let import_tasks_cancelled = connection
            .query_row("SELECT COUNT(*) FROM import_task_cancellation", [], |row| {
                row.get::<_, i64>(0)
            })
            .map_err(MetaStoreError::storage)?;
        let import_scan_scopes = connection
            .query_row("SELECT COUNT(*) FROM import_scan_scope", [], |row| {
                row.get::<_, i64>(0)
            })
            .map_err(MetaStoreError::storage)?;
        let import_scan_errors = connection
            .query_row("SELECT COUNT(*) FROM import_scan_error", [], |row| {
                row.get::<_, i64>(0)
            })
            .map_err(MetaStoreError::storage)?;
        let entity_mentions = connection
            .query_row(
                "\
                SELECT COUNT(*)
                FROM entity_mention AS mention
                JOIN resume_version AS version ON version.id = mention.resume_version_id
                JOIN document AS document ON document.id = version.document_id
                WHERE document.is_deleted = 0
                    AND document.status <> ?1
                    AND version.visibility <> ?2",
                params![
                    document_status_to_storage(DocumentStatus::Deleted),
                    resume_visibility_to_storage(ResumeVisibility::Hidden),
                ],
                |row| row.get::<_, i64>(0),
            )
            .map_err(MetaStoreError::storage)?;
        let index_state = connection
            .query_row(
                "\
                SELECT status, snapshot_token
                FROM index_state
                WHERE state_key = ?1",
                params![INDEX_STATE_KEY],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?)),
            )
            .optional()
            .map_err(MetaStoreError::storage)?;
        let (index_health, last_snapshot_id) = match index_state {
            Some((status, snapshot_token)) => {
                (index_state_status_from_storage(&status)?, snapshot_token)
            }
            None => (IndexStateStatus::Empty, None),
        };

        Ok(StoreStatusSummary {
            indexed_documents: i64_to_u64(
                document_counts.indexed_documents,
                "status.indexed_documents",
            )?,
            searchable_documents: i64_to_u64(
                document_counts.searchable_documents,
                "status.searchable_documents",
            )?,
            partial_documents: i64_to_u64(
                document_counts.partial_documents,
                "status.partial_documents",
            )?,
            failed_retryable: i64_to_u64(
                document_counts.failed_retryable,
                "status.failed_retryable",
            )?,
            failed_permanent: i64_to_u64(
                document_counts.failed_permanent,
                "status.failed_permanent",
            )?,
            ocr_queue_depth: i64_to_u64(document_counts.ocr_queue_depth, "status.ocr_queue_depth")?,
            embedding_queue_depth: i64_to_u64(
                embedding_jobs_queued,
                "status.embedding_queue_depth",
            )?,
            recovery_queue_depth: i64_to_u64(recovery_queue_depth, "status.recovery_queue_depth")?,
            import_tasks_queued: i64_to_u64(import_tasks_queued, "status.import_tasks_queued")?,
            import_tasks_recoverable: i64_to_u64(
                import_tasks_recoverable,
                "status.import_tasks_recoverable",
            )?,
            import_tasks_cancelled: i64_to_u64(
                import_tasks_cancelled,
                "status.import_tasks_cancelled",
            )?,
            import_scan_scopes: i64_to_u64(import_scan_scopes, "status.import_scan_scopes")?,
            import_scan_errors: i64_to_u64(import_scan_errors, "status.import_scan_errors")?,
            ocr_jobs_queued: i64_to_u64(ocr_jobs_queued, "status.ocr_jobs_queued")?,
            ocr_page_budget_blocked: i64_to_u64(
                ocr_page_budget_blocked,
                "status.ocr_page_budget_blocked",
            )?,
            ocr_language_unavailable: i64_to_u64(
                ocr_language_unavailable,
                "status.ocr_language_unavailable",
            )?,
            entity_mentions: i64_to_u64(entity_mentions, "status.entity_mentions")?,
            query_latency: query_latency_summary(&connection)?,
            index_health,
            last_snapshot_id,
        })
    }

    pub fn record_query_observation(
        &self,
        mode: &str,
        duration: Duration,
        result_count: usize,
        observed_at: UnixTimestamp,
    ) -> Result<()> {
        let mode = validate_query_observation_mode(mode)?;
        let duration_ms = u64::try_from(duration.as_millis())
            .ok()
            .and_then(|value| u64_to_i64(value, "query_observation.duration_ms").ok())
            .ok_or_else(|| MetaStoreError::invalid_value("query_observation.duration_ms"))?;
        let result_count = usize_to_i64(result_count, "query_observation.result_count")?;
        let connection = self.connection.borrow_mut();
        connection
            .execute(
                "\
                INSERT INTO query_observation (
                    observed_at_seconds, mode, duration_ms, result_count
                )
                VALUES (?1, ?2, ?3, ?4)",
                params![
                    observed_at.as_unix_seconds(),
                    mode,
                    duration_ms,
                    result_count,
                ],
            )
            .map_err(MetaStoreError::storage)?;
        connection
            .execute(
                "\
                DELETE FROM query_observation
                WHERE rowid NOT IN (
                    SELECT rowid
                    FROM query_observation
                    ORDER BY observed_at_seconds DESC, rowid DESC
                    LIMIT ?1
                )",
                params![QUERY_OBSERVATION_RETENTION_ROWS],
            )
            .map_err(MetaStoreError::storage)?;
        Ok(())
    }

    fn query_jobs<P>(&self, filter_clause: &str, params: P) -> Result<Vec<IngestJob>>
    where
        P: rusqlite::Params,
    {
        let connection = self.connection.borrow();
        let sql = format!("SELECT {INGEST_JOB_COLUMNS} FROM ingest_job {filter_clause}");
        let mut statement = connection.prepare(&sql).map_err(MetaStoreError::storage)?;
        let mut rows = statement.query(params).map_err(MetaStoreError::storage)?;
        let mut jobs = Vec::new();

        while let Some(row) = rows.next().map_err(MetaStoreError::storage)? {
            jobs.push(read_ingest_job(row)?);
        }

        Ok(jobs)
    }
}

impl fmt::Debug for MetaStore {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MetaStore")
            .field("connection", &"<redacted>")
            .finish()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MigrationReport {
    applied_versions: Vec<u32>,
}

impl MigrationReport {
    pub fn applied_versions(&self) -> &[u32] {
        &self.applied_versions
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct OcrPageCacheKey {
    file_content_hash: String,
    page_no: u32,
    render_dpi: u32,
    ocr_lang: String,
    ocr_profile: String,
}

impl OcrPageCacheKey {
    pub fn new(
        file_content_hash: impl Into<String>,
        page_no: u32,
        render_dpi: u32,
        ocr_lang: impl Into<String>,
        ocr_profile: impl Into<String>,
    ) -> Result<Self> {
        let file_content_hash = file_content_hash.into();
        let ocr_lang = ocr_lang.into();
        let ocr_profile = ocr_profile.into();
        if file_content_hash.trim().is_empty()
            || page_no == 0
            || render_dpi == 0
            || ocr_lang.trim().is_empty()
            || ocr_profile.trim().is_empty()
        {
            return Err(MetaStoreError::invalid_value("ocr_page_cache.key"));
        }

        Ok(Self {
            file_content_hash,
            page_no,
            render_dpi,
            ocr_lang,
            ocr_profile,
        })
    }

    pub fn page_no(&self) -> u32 {
        self.page_no
    }

    pub fn render_dpi(&self) -> u32 {
        self.render_dpi
    }

    pub fn ocr_lang(&self) -> &str {
        &self.ocr_lang
    }

    pub fn ocr_profile(&self) -> &str {
        &self.ocr_profile
    }
}

impl fmt::Debug for OcrPageCacheKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OcrPageCacheKey")
            .field("file_content_hash", &"<redacted>")
            .field("page_no", &self.page_no)
            .field("render_dpi", &self.render_dpi)
            .field("ocr_lang", &self.ocr_lang)
            .field("ocr_profile", &self.ocr_profile)
            .finish()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OcrPageCacheStatus {
    Succeeded,
    FailedRetryable,
    FailedPermanent,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct IngestJobPurge {
    jobs: usize,
    embedding_specs: usize,
}

impl IngestJobPurge {
    fn empty() -> Self {
        Self {
            jobs: 0,
            embedding_specs: 0,
        }
    }

    pub fn jobs(self) -> usize {
        self.jobs
    }

    pub fn embedding_specs(self) -> usize {
        self.embedding_specs
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ImportTaskPurge {
    tasks: usize,
    scan_scopes: usize,
    scan_errors: usize,
    cancellations: usize,
}

impl ImportTaskPurge {
    fn empty() -> Self {
        Self {
            tasks: 0,
            scan_scopes: 0,
            scan_errors: 0,
            cancellations: 0,
        }
    }

    pub fn tasks(self) -> usize {
        self.tasks
    }

    pub fn scan_scopes(self) -> usize {
        self.scan_scopes
    }

    pub fn scan_errors(self) -> usize {
        self.scan_errors
    }

    pub fn cancellations(self) -> usize {
        self.cancellations
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct OcrPageCachePurge {
    entries: usize,
    word_boxes: usize,
}

impl OcrPageCachePurge {
    fn empty() -> Self {
        Self {
            entries: 0,
            word_boxes: 0,
        }
    }

    pub fn entries(self) -> usize {
        self.entries
    }

    pub fn word_boxes(self) -> usize {
        self.word_boxes
    }
}

#[derive(Clone, PartialEq)]
pub struct OcrWordBox {
    text: String,
    left: u32,
    top: u32,
    width: u32,
    height: u32,
    confidence: f32,
}

impl OcrWordBox {
    pub fn new(
        text: impl Into<String>,
        left: u32,
        top: u32,
        width: u32,
        height: u32,
        confidence: f32,
    ) -> Result<Self> {
        let text = text.into();
        if text.trim().is_empty()
            || width == 0
            || height == 0
            || !confidence.is_finite()
            || !(0.0..=1.0).contains(&confidence)
        {
            return Err(MetaStoreError::invalid_value("ocr_page_cache.word_box"));
        }

        Ok(Self {
            text,
            left,
            top,
            width,
            height,
            confidence,
        })
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn left(&self) -> u32 {
        self.left
    }

    pub fn top(&self) -> u32 {
        self.top
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    pub fn confidence(&self) -> f32 {
        self.confidence
    }
}

impl fmt::Debug for OcrWordBox {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OcrWordBox")
            .field("text", &"<redacted>")
            .field("text_bytes", &self.text.len())
            .field("left", &self.left)
            .field("top", &self.top)
            .field("width", &self.width)
            .field("height", &self.height)
            .field("confidence", &self.confidence)
            .finish()
    }
}

#[derive(Clone, PartialEq)]
pub struct OcrPageCacheEntry {
    key: OcrPageCacheKey,
    text: Option<String>,
    word_boxes: Vec<OcrWordBox>,
    confidence: Option<f32>,
    engine_profile: Option<String>,
    duration_ms: Option<u64>,
    status: OcrPageCacheStatus,
    error_kind: Option<String>,
    updated_at: UnixTimestamp,
}

impl OcrPageCacheEntry {
    pub fn succeeded(
        key: OcrPageCacheKey,
        text: impl Into<String>,
        confidence: f32,
        engine_profile: impl Into<String>,
        duration_ms: u64,
        updated_at: UnixTimestamp,
    ) -> Result<Self> {
        Self::succeeded_with_word_boxes(
            key,
            text,
            confidence,
            engine_profile,
            duration_ms,
            Vec::new(),
            updated_at,
        )
    }

    pub fn succeeded_with_word_boxes(
        key: OcrPageCacheKey,
        text: impl Into<String>,
        confidence: f32,
        engine_profile: impl Into<String>,
        duration_ms: u64,
        word_boxes: Vec<OcrWordBox>,
        updated_at: UnixTimestamp,
    ) -> Result<Self> {
        let engine_profile = engine_profile.into();
        if !confidence.is_finite()
            || !(0.0..=1.0).contains(&confidence)
            || engine_profile.trim().is_empty()
        {
            return Err(MetaStoreError::invalid_value("ocr_page_cache.success"));
        }

        Ok(Self {
            key,
            text: Some(text.into()),
            word_boxes,
            confidence: Some(confidence),
            engine_profile: Some(engine_profile),
            duration_ms: Some(duration_ms),
            status: OcrPageCacheStatus::Succeeded,
            error_kind: None,
            updated_at,
        })
    }

    pub fn failed_retryable(
        key: OcrPageCacheKey,
        error_kind: impl Into<String>,
        updated_at: UnixTimestamp,
    ) -> Result<Self> {
        Self::failed(
            key,
            error_kind,
            OcrPageCacheStatus::FailedRetryable,
            updated_at,
        )
    }

    pub fn failed_permanent(
        key: OcrPageCacheKey,
        error_kind: impl Into<String>,
        updated_at: UnixTimestamp,
    ) -> Result<Self> {
        Self::failed(
            key,
            error_kind,
            OcrPageCacheStatus::FailedPermanent,
            updated_at,
        )
    }

    fn failed(
        key: OcrPageCacheKey,
        error_kind: impl Into<String>,
        status: OcrPageCacheStatus,
        updated_at: UnixTimestamp,
    ) -> Result<Self> {
        let error_kind = error_kind.into();
        if error_kind.trim().is_empty() {
            return Err(MetaStoreError::invalid_value("ocr_page_cache.error_kind"));
        }

        Ok(Self {
            key,
            text: None,
            word_boxes: Vec::new(),
            confidence: None,
            engine_profile: None,
            duration_ms: None,
            status,
            error_kind: Some(error_kind),
            updated_at,
        })
    }

    pub fn key(&self) -> &OcrPageCacheKey {
        &self.key
    }

    pub fn text(&self) -> Option<&str> {
        self.text.as_deref()
    }

    pub fn word_boxes(&self) -> &[OcrWordBox] {
        &self.word_boxes
    }

    pub fn confidence(&self) -> Option<f32> {
        self.confidence
    }

    pub fn engine_profile(&self) -> Option<&str> {
        self.engine_profile.as_deref()
    }

    pub fn duration_ms(&self) -> Option<u64> {
        self.duration_ms
    }

    pub fn status(&self) -> OcrPageCacheStatus {
        self.status
    }

    pub fn error_kind(&self) -> Option<&str> {
        self.error_kind.as_deref()
    }
}

impl fmt::Debug for OcrPageCacheEntry {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OcrPageCacheEntry")
            .field("key", &self.key)
            .field("text", &"<redacted>")
            .field("text_bytes", &self.text.as_ref().map(String::len))
            .field("word_box_count", &self.word_boxes.len())
            .field("confidence", &self.confidence)
            .field("engine_profile", &self.engine_profile)
            .field("duration_ms", &self.duration_ms)
            .field("status", &self.status)
            .field("error_kind", &"<redacted>")
            .field("updated_at", &self.updated_at)
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct IngestJob {
    pub id: IngestJobId,
    pub document_id: DocumentId,
    pub resume_version_id: Option<ResumeVersionId>,
    pub kind: IngestJobKind,
    pub status: IngestJobStatus,
    pub attempt_count: u32,
    pub max_attempts: u32,
    pub queued_at: UnixTimestamp,
    pub started_at: Option<UnixTimestamp>,
    pub finished_at: Option<UnixTimestamp>,
    pub updated_at: UnixTimestamp,
    pub failure_kind: Option<IngestJobFailureKind>,
}

#[derive(Clone, PartialEq, Eq)]
pub struct ClaimedOcrJob {
    pub job: IngestJob,
    source_fingerprint: String,
}

impl ClaimedOcrJob {
    pub fn source_fingerprint(&self) -> &str {
        &self.source_fingerprint
    }
}

impl fmt::Debug for ClaimedOcrJob {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ClaimedOcrJob(<redacted>)")
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IngestJobFailureKind {
    OcrPageBudgetExceeded,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OcrAttemptFailure {
    Retryable,
    RetryableWithKind(IngestJobFailureKind),
    Permanent,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OcrAttemptFailureOutcome {
    Retryable,
    FailedPermanent,
    Superseded,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OcrAttemptSuccessOutcome {
    Completed,
    Superseded,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EnqueuedIngestJob {
    pub job: IngestJob,
    pub scheduled: bool,
}

#[derive(Clone, PartialEq, Eq)]
pub struct ImportTask {
    pub id: ImportTaskId,
    pub root_path: String,
    pub status: ImportTaskStatus,
    pub queued_at: UnixTimestamp,
    pub started_at: Option<UnixTimestamp>,
    pub finished_at: Option<UnixTimestamp>,
    pub updated_at: UnixTimestamp,
}

impl fmt::Debug for ImportTask {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ImportTask")
            .field("id", &self.id)
            .field("root_path", &"<redacted>")
            .field("status", &self.status)
            .field("queued_at", &self.queued_at)
            .field("started_at", &self.started_at)
            .field("finished_at", &self.finished_at)
            .field("updated_at", &self.updated_at)
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct ImportScanScope {
    pub import_task_id: ImportTaskId,
    pub root_kind: ImportRootKind,
    pub root_preset: Option<ImportRootPreset>,
    pub scan_profile: ImportScanProfile,
    pub requested_root_path: String,
    pub canonical_root_path: String,
    pub files_discovered: u64,
    pub ignored_entries: u64,
    pub scan_errors: u64,
    pub searchable_documents: u64,
    pub ocr_required_documents: u64,
    pub ocr_jobs_queued: u64,
    pub failed_documents: u64,
    pub deleted_documents: u64,
    pub scan_budget_kind: Option<ImportScanBudgetKind>,
    pub scan_budget_limit: Option<u64>,
    pub scan_budget_observed: Option<u64>,
    pub scan_budget_exhausted: bool,
    pub updated_at: UnixTimestamp,
}

impl fmt::Debug for ImportScanScope {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ImportScanScope")
            .field("import_task_id", &self.import_task_id)
            .field("root_kind", &self.root_kind)
            .field("root_preset", &self.root_preset)
            .field("scan_profile", &self.scan_profile)
            .field("requested_root_path", &"<redacted>")
            .field("canonical_root_path", &"<redacted>")
            .field("files_discovered", &self.files_discovered)
            .field("ignored_entries", &self.ignored_entries)
            .field("scan_errors", &self.scan_errors)
            .field("searchable_documents", &self.searchable_documents)
            .field("ocr_required_documents", &self.ocr_required_documents)
            .field("ocr_jobs_queued", &self.ocr_jobs_queued)
            .field("failed_documents", &self.failed_documents)
            .field("deleted_documents", &self.deleted_documents)
            .field("scan_budget_kind", &self.scan_budget_kind)
            .field("scan_budget_limit", &self.scan_budget_limit)
            .field("scan_budget_observed", &self.scan_budget_observed)
            .field("scan_budget_exhausted", &self.scan_budget_exhausted)
            .field("updated_at", &self.updated_at)
            .finish()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImportRootKind {
    Explicit,
    Preset,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImportRootPreset {
    LocalDiscovery,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImportScanProfile {
    Explicit,
    Discovery,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImportScanBudgetKind {
    Files,
}

#[derive(Clone, PartialEq, Eq)]
pub struct ImportScanError {
    pub import_task_id: ImportTaskId,
    pub error_index: u64,
    pub kind: ImportScanErrorKind,
    pub operation: ImportScanErrorOperation,
    pub path_digest: Option<String>,
    pub updated_at: UnixTimestamp,
}

impl fmt::Debug for ImportScanError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ImportScanError")
            .field("import_task_id", &self.import_task_id)
            .field("error_index", &self.error_index)
            .field("kind", &self.kind)
            .field("operation", &self.operation)
            .field(
                "path_digest",
                &self.path_digest.as_ref().map(|_| "<redacted>"),
            )
            .field("updated_at", &self.updated_at)
            .finish()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImportScanErrorSummary {
    pub kind: ImportScanErrorKind,
    pub operation: ImportScanErrorOperation,
    pub count: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CandidateContactConflict {
    pub resume_version_id: ResumeVersionId,
    pub email_candidate_id: CandidateId,
    pub phone_candidate_id: CandidateId,
    pub updated_at: UnixTimestamp,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImportScanErrorKind {
    PermissionDenied,
    SourceUnavailable,
    LockedOrUnreadable,
    Io,
}

impl ImportScanErrorKind {
    pub fn label(self) -> &'static str {
        import_scan_error_kind_to_storage(self)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImportScanErrorOperation {
    NormalizePath,
    ReadDirectory,
    ReadMetadata,
    Fingerprint,
}

impl ImportScanErrorOperation {
    pub fn label(self) -> &'static str {
        import_scan_error_operation_to_storage(self)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImportTaskStatus {
    Queued,
    Running,
    Completed,
    FailedRetryable,
    FailedPermanent,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkerTaskKind {
    Ocr,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkerTaskControl {
    pub task: WorkerTaskKind,
    pub paused: bool,
    pub updated_at: UnixTimestamp,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoreStatusSummary {
    pub indexed_documents: u64,
    pub searchable_documents: u64,
    pub partial_documents: u64,
    pub failed_retryable: u64,
    pub failed_permanent: u64,
    pub ocr_queue_depth: u64,
    pub embedding_queue_depth: u64,
    pub recovery_queue_depth: u64,
    pub import_tasks_queued: u64,
    pub import_tasks_recoverable: u64,
    pub import_tasks_cancelled: u64,
    pub import_scan_scopes: u64,
    pub import_scan_errors: u64,
    pub ocr_jobs_queued: u64,
    pub ocr_page_budget_blocked: u64,
    pub ocr_language_unavailable: u64,
    pub entity_mentions: u64,
    pub query_latency: QueryLatencySummary,
    pub index_health: IndexStateStatus,
    pub last_snapshot_id: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct QueryLatencySummary {
    pub sample_count: u64,
    pub p50_ms: Option<u64>,
    pub p95_ms: Option<u64>,
    pub p99_ms: Option<u64>,
    pub last_result_count: Option<u64>,
    pub last_observed_at: Option<UnixTimestamp>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MetadataEncryptionState {
    Plaintext,
    SqlCipher,
}

impl MetadataEncryptionState {
    pub fn label(self) -> &'static str {
        match self {
            MetadataEncryptionState::Plaintext => "plaintext",
            MetadataEncryptionState::SqlCipher => "sqlcipher",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct DocumentStatusCounts {
    indexed_documents: i64,
    searchable_documents: i64,
    partial_documents: i64,
    failed_retryable: i64,
    failed_permanent: i64,
    ocr_queue_depth: i64,
}

impl fmt::Debug for IngestJob {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("IngestJob")
            .field("id", &self.id)
            .field("document_id", &self.document_id)
            .field("resume_version_id", &self.resume_version_id)
            .field("kind", &self.kind)
            .field("status", &self.status)
            .field("attempt_count", &self.attempt_count)
            .field("max_attempts", &self.max_attempts)
            .field("queued_at", &self.queued_at)
            .field("started_at", &self.started_at)
            .field("finished_at", &self.finished_at)
            .field("updated_at", &self.updated_at)
            .field("failure_kind", &self.failure_kind)
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct IndexState {
    pub manifest_version: String,
    pub snapshot_token: Option<String>,
    pub status: IndexStateStatus,
    pub updated_at: UnixTimestamp,
    pub visible_epoch: u64,
    pub manifest_document_count: u64,
}

impl fmt::Debug for IndexState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("IndexState")
            .field("manifest_version", &"<redacted>")
            .field(
                "snapshot_token",
                &self.snapshot_token.as_ref().map(|_| "<redacted>"),
            )
            .field("status", &self.status)
            .field("updated_at", &self.updated_at)
            .field("visible_epoch", &self.visible_epoch)
            .field("manifest_document_count", &self.manifest_document_count)
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct MetaStoreError {
    kind: MetaStoreErrorKind,
}

impl MetaStoreError {
    fn storage(_error: rusqlite::Error) -> Self {
        Self {
            kind: MetaStoreErrorKind::Storage,
        }
    }

    fn io_storage(_error: io::Error) -> Self {
        Self {
            kind: MetaStoreErrorKind::Storage,
        }
    }

    fn random() -> Self {
        Self {
            kind: MetaStoreErrorKind::Storage,
        }
    }

    fn weak_passphrase() -> Self {
        Self {
            kind: MetaStoreErrorKind::WeakPassphrase,
        }
    }

    fn invalid_backup() -> Self {
        Self {
            kind: MetaStoreErrorKind::InvalidBackup,
        }
    }

    fn crypto() -> Self {
        Self {
            kind: MetaStoreErrorKind::Crypto,
        }
    }

    fn key_already_exists() -> Self {
        Self {
            kind: MetaStoreErrorKind::KeyAlreadyExists,
        }
    }

    fn migration(_error: rusqlite::Error) -> Self {
        Self {
            kind: MetaStoreErrorKind::Migration,
        }
    }

    fn invalid_value(field: &'static str) -> Self {
        Self {
            kind: MetaStoreErrorKind::InvalidPersistedValue { field },
        }
    }

    fn not_found(entity: &'static str) -> Self {
        Self {
            kind: MetaStoreErrorKind::NotFound { entity },
        }
    }

    fn invalid_transition() -> Self {
        Self {
            kind: MetaStoreErrorKind::InvalidTransition,
        }
    }
}

impl fmt::Debug for MetaStoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MetaStoreError")
            .field("kind", &self.kind)
            .finish()
    }
}

impl fmt::Display for MetaStoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.kind {
            MetaStoreErrorKind::Storage => formatter.write_str("metadata store operation failed"),
            MetaStoreErrorKind::Migration => {
                formatter.write_str("metadata schema migration failed")
            }
            MetaStoreErrorKind::InvalidPersistedValue { field } => {
                write!(
                    formatter,
                    "metadata store contains an invalid value for {field}"
                )
            }
            MetaStoreErrorKind::NotFound { entity } => {
                write!(
                    formatter,
                    "metadata store record was not found for {entity}"
                )
            }
            MetaStoreErrorKind::InvalidTransition => {
                formatter.write_str("metadata store job status transition is invalid")
            }
            MetaStoreErrorKind::WeakPassphrase => {
                formatter.write_str("metadata key backup passphrase is too weak")
            }
            MetaStoreErrorKind::InvalidBackup => {
                formatter.write_str("metadata key backup is invalid or cannot be decrypted")
            }
            MetaStoreErrorKind::Crypto => formatter.write_str("metadata key backup crypto failed"),
            MetaStoreErrorKind::KeyAlreadyExists => {
                formatter.write_str("metadata encryption key already exists")
            }
        }
    }
}

impl std::error::Error for MetaStoreError {}

#[derive(Clone, Debug, PartialEq, Eq)]
enum MetaStoreErrorKind {
    Storage,
    Migration,
    InvalidPersistedValue { field: &'static str },
    NotFound { entity: &'static str },
    InvalidTransition,
    WeakPassphrase,
    InvalidBackup,
    Crypto,
    KeyAlreadyExists,
}

const SCHEMA_V1: &str = r#"
CREATE TABLE document (
    id TEXT PRIMARY KEY,
    source_uri TEXT NOT NULL,
    normalized_path TEXT NOT NULL,
    file_name TEXT NOT NULL,
    extension TEXT NOT NULL,
    byte_size INTEGER NOT NULL CHECK (byte_size >= 0),
    mtime_seconds INTEGER NOT NULL,
    content_hash TEXT,
    text_hash TEXT,
    is_deleted INTEGER NOT NULL DEFAULT 0 CHECK (is_deleted IN (0, 1)),
    created_at_seconds INTEGER NOT NULL,
    updated_at_seconds INTEGER NOT NULL,
    status TEXT NOT NULL CHECK (status IN (
        'discovered',
        'fingerprinted',
        'parse_queued',
        'parse_running',
        'text_extracted',
        'ocr_required',
        'ocr_running',
        'ocr_done',
        'text_cleaned',
        'fields_extracted',
        'embedding_done',
        'indexed_partial',
        'searchable',
        'failed_retryable',
        'failed_permanent',
        'deleted'
    ))
);

CREATE TABLE resume_version (
    id TEXT PRIMARY KEY,
    document_id TEXT NOT NULL,
    candidate_id TEXT,
    parse_version TEXT NOT NULL,
    schema_version TEXT NOT NULL,
    language_set_json TEXT NOT NULL DEFAULT '[]',
    page_count INTEGER CHECK (page_count IS NULL OR page_count >= 0),
    raw_text TEXT,
    clean_text TEXT,
    quality_score REAL CHECK (quality_score IS NULL OR quality_score BETWEEN 0 AND 1),
    visibility TEXT NOT NULL CHECK (visibility IN ('searchable', 'partial', 'hidden')),
    FOREIGN KEY (document_id) REFERENCES document(id) ON DELETE CASCADE
);

CREATE TABLE ingest_job (
    id TEXT PRIMARY KEY,
    document_id TEXT NOT NULL,
    resume_version_id TEXT,
    kind TEXT NOT NULL CHECK (kind IN (
        'discover_document',
        'fingerprint_document',
        'parse_document',
        'clean_text',
        'extract_fields',
        'update_index'
    )),
    status TEXT NOT NULL CHECK (status IN (
        'queued',
        'running',
        'interrupted',
        'completed',
        'failed_retryable',
        'failed_permanent'
    )),
    attempt_count INTEGER NOT NULL DEFAULT 0 CHECK (attempt_count >= 0),
    max_attempts INTEGER NOT NULL DEFAULT 3 CHECK (max_attempts > 0),
    queued_at_seconds INTEGER NOT NULL,
    started_at_seconds INTEGER,
    finished_at_seconds INTEGER,
    updated_at_seconds INTEGER NOT NULL,
    FOREIGN KEY (document_id) REFERENCES document(id) ON DELETE CASCADE,
    FOREIGN KEY (resume_version_id) REFERENCES resume_version(id) ON DELETE SET NULL
);

CREATE TABLE index_state (
    state_key TEXT PRIMARY KEY,
    manifest_version TEXT NOT NULL,
    snapshot_token TEXT,
    status TEXT NOT NULL CHECK (status IN ('empty', 'building', 'ready', 'stale')),
    updated_at_seconds INTEGER NOT NULL,
    CHECK (state_key = 'default')
);

CREATE INDEX ingest_job_recovery_idx
    ON ingest_job(status, attempt_count, max_attempts);
CREATE INDEX resume_version_document_idx
    ON resume_version(document_id);
"#;

const SCHEMA_V2: &str = r#"
CREATE TABLE import_task (
    id TEXT PRIMARY KEY,
    root_path TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN (
        'queued',
        'running',
        'completed',
        'failed_retryable',
        'failed_permanent'
    )),
    queued_at_seconds INTEGER NOT NULL,
    started_at_seconds INTEGER,
    finished_at_seconds INTEGER,
    updated_at_seconds INTEGER NOT NULL,
    CHECK (queued_at_seconds <= updated_at_seconds),
    CHECK (
        started_at_seconds IS NULL
        OR (queued_at_seconds <= started_at_seconds AND started_at_seconds <= updated_at_seconds)
    ),
    CHECK (
        finished_at_seconds IS NULL
        OR (
            started_at_seconds IS NOT NULL
            AND started_at_seconds <= finished_at_seconds
            AND finished_at_seconds <= updated_at_seconds
        )
    ),
    CHECK (
        (
            status = 'queued'
            AND started_at_seconds IS NULL
            AND finished_at_seconds IS NULL
        )
        OR (
            status = 'running'
            AND started_at_seconds IS NOT NULL
            AND finished_at_seconds IS NULL
        )
        OR (
            status IN ('completed', 'failed_retryable', 'failed_permanent')
            AND started_at_seconds IS NOT NULL
            AND finished_at_seconds IS NOT NULL
        )
    )
);

CREATE INDEX import_task_status_idx
    ON import_task(status, queued_at_seconds);
"#;

const SCHEMA_V3: &str = r#"
CREATE TABLE ingest_job_v3 (
    id TEXT PRIMARY KEY,
    document_id TEXT NOT NULL,
    resume_version_id TEXT,
    kind TEXT NOT NULL CHECK (kind IN (
        'discover_document',
        'fingerprint_document',
        'parse_document',
        'ocr_document',
        'clean_text',
        'extract_fields',
        'update_index'
    )),
    status TEXT NOT NULL CHECK (status IN (
        'queued',
        'running',
        'interrupted',
        'completed',
        'failed_retryable',
        'failed_permanent'
    )),
    attempt_count INTEGER NOT NULL DEFAULT 0 CHECK (attempt_count >= 0),
    max_attempts INTEGER NOT NULL DEFAULT 3 CHECK (max_attempts > 0),
    queued_at_seconds INTEGER NOT NULL,
    started_at_seconds INTEGER,
    finished_at_seconds INTEGER,
    updated_at_seconds INTEGER NOT NULL,
    FOREIGN KEY (document_id) REFERENCES document(id) ON DELETE CASCADE,
    FOREIGN KEY (resume_version_id) REFERENCES resume_version(id) ON DELETE SET NULL
);

INSERT INTO ingest_job_v3 (
    id, document_id, resume_version_id, kind, status, attempt_count, max_attempts,
    queued_at_seconds, started_at_seconds, finished_at_seconds, updated_at_seconds
)
SELECT
    id, document_id, resume_version_id, kind, status, attempt_count, max_attempts,
    queued_at_seconds, started_at_seconds, finished_at_seconds, updated_at_seconds
FROM ingest_job;

DROP TABLE ingest_job;
ALTER TABLE ingest_job_v3 RENAME TO ingest_job;

CREATE INDEX ingest_job_recovery_idx
    ON ingest_job(status, attempt_count, max_attempts);
CREATE UNIQUE INDEX ingest_job_ocr_document_unique_idx
    ON ingest_job(document_id, kind)
    WHERE kind = 'ocr_document';
"#;

const SCHEMA_V4: &str = r#"
CREATE TABLE entity_mention (
    id TEXT PRIMARY KEY,
    resume_version_id TEXT NOT NULL,
    section_id TEXT,
    entity_type TEXT NOT NULL CHECK (
        entity_type IN (
            'name',
            'email',
            'phone',
            'school',
            'school_tier',
            'degree',
            'major',
            'company',
            'title',
            'education',
            'skills',
            'skill',
            'certificate',
            'date',
            'date_range',
            'years_experience',
            'location'
        )
        OR entity_type LIKE 'other:%'
    ),
    raw_value TEXT NOT NULL,
    normalized_value TEXT,
    span_start INTEGER CHECK (span_start IS NULL OR span_start >= 0),
    span_end INTEGER CHECK (span_end IS NULL OR span_end >= 0),
    confidence REAL NOT NULL CHECK (confidence >= 0 AND confidence <= 1),
    extractor TEXT NOT NULL,
    CHECK (
        span_start IS NULL
        OR span_end IS NULL
        OR span_start <= span_end
    ),
    FOREIGN KEY (resume_version_id) REFERENCES resume_version(id) ON DELETE CASCADE
);

CREATE INDEX entity_mention_version_idx
    ON entity_mention(resume_version_id, entity_type);
CREATE INDEX entity_mention_type_value_idx
    ON entity_mention(entity_type, normalized_value, confidence);
"#;

const SCHEMA_V5: &str = r#"
CREATE TABLE candidate (
    id TEXT PRIMARY KEY,
    primary_name TEXT,
    phone_hash TEXT CHECK (phone_hash IS NULL OR length(phone_hash) = 64),
    email_hash TEXT CHECK (email_hash IS NULL OR length(email_hash) = 64),
    dedupe_key TEXT,
    merge_confidence REAL CHECK (merge_confidence IS NULL OR merge_confidence BETWEEN 0 AND 1),
    version_count INTEGER NOT NULL DEFAULT 0 CHECK (version_count >= 0)
);

CREATE UNIQUE INDEX candidate_phone_hash_unique_idx
    ON candidate(phone_hash)
    WHERE phone_hash IS NOT NULL;
CREATE UNIQUE INDEX candidate_email_hash_unique_idx
    ON candidate(email_hash)
    WHERE email_hash IS NOT NULL;
CREATE INDEX resume_version_candidate_idx
    ON resume_version(candidate_id)
    WHERE candidate_id IS NOT NULL;
"#;

const SCHEMA_V6: &str = r#"
UPDATE entity_mention
SET raw_value = '<redacted:email>',
    normalized_value = NULL
WHERE entity_type = 'email';

UPDATE entity_mention
SET raw_value = '<redacted:phone>',
    normalized_value = NULL
WHERE entity_type = 'phone';
"#;

const SCHEMA_V7: &str = r#"
CREATE TABLE ocr_page_cache (
    file_content_hash TEXT NOT NULL,
    page_no INTEGER NOT NULL CHECK (page_no > 0),
    render_dpi INTEGER NOT NULL CHECK (render_dpi > 0),
    ocr_lang TEXT NOT NULL,
    ocr_profile TEXT NOT NULL,
    text TEXT,
    confidence REAL CHECK (confidence IS NULL OR confidence BETWEEN 0 AND 1),
    engine_profile TEXT,
    duration_ms INTEGER CHECK (duration_ms IS NULL OR duration_ms >= 0),
    status TEXT NOT NULL CHECK (
        status IN ('succeeded', 'failed_retryable', 'failed_permanent')
    ),
    error_kind TEXT,
    updated_at_seconds INTEGER NOT NULL,
    PRIMARY KEY (file_content_hash, page_no, render_dpi, ocr_lang, ocr_profile),
    CHECK (
        (
            status = 'succeeded'
            AND text IS NOT NULL
            AND confidence IS NOT NULL
            AND engine_profile IS NOT NULL
            AND duration_ms IS NOT NULL
            AND error_kind IS NULL
        )
        OR (
            status IN ('failed_retryable', 'failed_permanent')
            AND text IS NULL
            AND confidence IS NULL
            AND engine_profile IS NULL
            AND duration_ms IS NULL
            AND error_kind IS NOT NULL
        )
    )
);

CREATE INDEX ocr_page_cache_content_idx
    ON ocr_page_cache(file_content_hash, status, updated_at_seconds);
"#;

const SCHEMA_V8: &str = r#"
CREATE TABLE worker_task_control (
    task_kind TEXT PRIMARY KEY CHECK (task_kind IN ('ocr')),
    paused INTEGER NOT NULL CHECK (paused IN (0, 1)),
    updated_at_seconds INTEGER NOT NULL
);
"#;

const SCHEMA_V9: &str = r#"
CREATE TABLE import_scan_scope (
    import_task_id TEXT PRIMARY KEY,
    root_kind TEXT NOT NULL CHECK (root_kind IN ('explicit', 'preset')),
    root_preset TEXT CHECK (root_preset IS NULL OR root_preset IN ('local_discovery')),
    scan_profile TEXT NOT NULL CHECK (scan_profile IN ('explicit', 'discovery')),
    requested_root_path TEXT NOT NULL,
    canonical_root_path TEXT NOT NULL,
    files_discovered INTEGER NOT NULL DEFAULT 0 CHECK (files_discovered >= 0),
    ignored_entries INTEGER NOT NULL DEFAULT 0 CHECK (ignored_entries >= 0),
    scan_errors INTEGER NOT NULL DEFAULT 0 CHECK (scan_errors >= 0),
    searchable_documents INTEGER NOT NULL DEFAULT 0 CHECK (searchable_documents >= 0),
    ocr_required_documents INTEGER NOT NULL DEFAULT 0 CHECK (ocr_required_documents >= 0),
    ocr_jobs_queued INTEGER NOT NULL DEFAULT 0 CHECK (ocr_jobs_queued >= 0),
    failed_documents INTEGER NOT NULL DEFAULT 0 CHECK (failed_documents >= 0),
    deleted_documents INTEGER NOT NULL DEFAULT 0 CHECK (deleted_documents >= 0),
    updated_at_seconds INTEGER NOT NULL,
    FOREIGN KEY (import_task_id) REFERENCES import_task(id) ON DELETE CASCADE,
    CHECK (
        (
            root_kind = 'explicit'
            AND root_preset IS NULL
        )
        OR (
            root_kind = 'preset'
            AND root_preset IS NOT NULL
        )
    )
);

CREATE INDEX import_scan_scope_updated_idx
    ON import_scan_scope(updated_at_seconds);
"#;

const SCHEMA_V10: &str = r#"
ALTER TABLE import_scan_scope
    ADD COLUMN scan_budget_kind TEXT CHECK (
        scan_budget_kind IS NULL OR scan_budget_kind IN ('files')
    );

ALTER TABLE import_scan_scope
    ADD COLUMN scan_budget_limit INTEGER CHECK (
        scan_budget_limit IS NULL OR scan_budget_limit >= 0
    );

ALTER TABLE import_scan_scope
    ADD COLUMN scan_budget_observed INTEGER CHECK (
        scan_budget_observed IS NULL OR scan_budget_observed >= 0
    );

ALTER TABLE import_scan_scope
    ADD COLUMN scan_budget_exhausted INTEGER NOT NULL DEFAULT 0 CHECK (
        scan_budget_exhausted IN (0, 1)
    );
"#;

const SCHEMA_V11: &str = r#"
CREATE TABLE import_scan_error (
    import_task_id TEXT NOT NULL,
    error_index INTEGER NOT NULL CHECK (error_index >= 0),
    kind TEXT NOT NULL CHECK (
        kind IN ('permission_denied', 'source_unavailable', 'locked_or_unreadable', 'io')
    ),
    operation TEXT NOT NULL CHECK (
        operation IN ('normalize_path', 'read_directory', 'read_metadata', 'fingerprint')
    ),
    path_digest TEXT CHECK (path_digest IS NULL OR length(path_digest) > 0),
    updated_at_seconds INTEGER NOT NULL,
    PRIMARY KEY (import_task_id, error_index),
    FOREIGN KEY (import_task_id) REFERENCES import_task(id) ON DELETE CASCADE
);
"#;

const SCHEMA_V12: &str = r#"
CREATE UNIQUE INDEX ingest_job_embedding_version_unique_idx
    ON ingest_job(resume_version_id, kind)
    WHERE kind = 'update_index' AND resume_version_id IS NOT NULL;

CREATE INDEX ingest_job_embedding_queue_idx
    ON ingest_job(kind, status, attempt_count, max_attempts, queued_at_seconds)
    WHERE kind = 'update_index' AND resume_version_id IS NOT NULL;
"#;

const SCHEMA_V13: &str = r#"
DROP INDEX IF EXISTS ingest_job_embedding_version_unique_idx;

CREATE TABLE embedding_job_spec (
    ingest_job_id TEXT PRIMARY KEY,
    resume_version_id TEXT NOT NULL,
    model_id TEXT NOT NULL CHECK (
        length(trim(model_id)) > 0
        AND instr(model_id, char(10)) = 0
        AND instr(model_id, char(13)) = 0
        AND instr(model_id, char(9)) = 0
    ),
    dimension INTEGER NOT NULL CHECK (dimension > 0),
    updated_at_seconds INTEGER NOT NULL,
    FOREIGN KEY (ingest_job_id) REFERENCES ingest_job(id) ON DELETE CASCADE,
    FOREIGN KEY (resume_version_id) REFERENCES resume_version(id) ON DELETE CASCADE
);

CREATE UNIQUE INDEX embedding_job_spec_unique_idx
    ON embedding_job_spec(resume_version_id, model_id, dimension);

CREATE INDEX embedding_job_spec_model_idx
    ON embedding_job_spec(model_id, dimension, resume_version_id);
"#;

const SCHEMA_V14: &str = r#"
CREATE TABLE import_task_cancellation (
    import_task_id TEXT PRIMARY KEY,
    requested_at_seconds INTEGER NOT NULL,
    FOREIGN KEY (import_task_id) REFERENCES import_task(id) ON DELETE CASCADE
);

CREATE INDEX import_task_cancellation_requested_idx
    ON import_task_cancellation(requested_at_seconds);
"#;

const SCHEMA_V15: &str = r#"
ALTER TABLE ocr_page_cache ADD COLUMN word_boxes_json TEXT;
"#;

const SCHEMA_V16: &str = r#"
ALTER TABLE ingest_job
    ADD COLUMN failure_kind TEXT CHECK (
        failure_kind IS NULL OR failure_kind IN ('ocr_page_budget_exceeded')
    );
"#;

const SCHEMA_V17: &str = r#"
CREATE TABLE query_observation (
    observed_at_seconds INTEGER NOT NULL,
    mode TEXT NOT NULL CHECK (mode IN ('fulltext', 'semantic', 'hybrid')),
    duration_ms INTEGER NOT NULL CHECK (duration_ms >= 0),
    result_count INTEGER NOT NULL CHECK (result_count >= 0)
);

CREATE INDEX query_observation_observed_idx
    ON query_observation(observed_at_seconds);
CREATE INDEX query_observation_duration_idx
    ON query_observation(duration_ms);
"#;

const SCHEMA_V18: &str = r#"
CREATE TABLE candidate_contact_conflict (
    resume_version_id TEXT PRIMARY KEY,
    email_candidate_id TEXT NOT NULL,
    phone_candidate_id TEXT NOT NULL,
    updated_at_seconds INTEGER NOT NULL,
    FOREIGN KEY (resume_version_id) REFERENCES resume_version(id) ON DELETE CASCADE,
    FOREIGN KEY (email_candidate_id) REFERENCES candidate(id) ON DELETE CASCADE,
    FOREIGN KEY (phone_candidate_id) REFERENCES candidate(id) ON DELETE CASCADE,
    CHECK (email_candidate_id <> phone_candidate_id)
);

CREATE INDEX candidate_contact_conflict_updated_idx
    ON candidate_contact_conflict(updated_at_seconds);
"#;

const SCHEMA_V19: &str = r#"
ALTER TABLE entity_mention RENAME TO entity_mention_v18;

CREATE TABLE entity_mention (
    id TEXT PRIMARY KEY,
    resume_version_id TEXT NOT NULL,
    section_id TEXT,
    entity_type TEXT NOT NULL CHECK (
        entity_type IN (
            'name',
            'email',
            'phone',
            'school',
            'school_tier',
            'degree',
            'major',
            'company',
            'title',
            'education',
            'skills',
            'skill',
            'certificate',
            'date',
            'date_range',
            'years_experience',
            'location'
        )
        OR entity_type LIKE 'other:%'
    ),
    raw_value TEXT NOT NULL,
    normalized_value TEXT,
    span_start INTEGER CHECK (span_start IS NULL OR span_start >= 0),
    span_end INTEGER CHECK (span_end IS NULL OR span_end >= 0),
    confidence REAL NOT NULL CHECK (confidence >= 0 AND confidence <= 1),
    extractor TEXT NOT NULL,
    CHECK (
        span_start IS NULL
        OR span_end IS NULL
        OR span_start <= span_end
    ),
    FOREIGN KEY (resume_version_id) REFERENCES resume_version(id) ON DELETE CASCADE
);

INSERT INTO entity_mention (
    id, resume_version_id, section_id, entity_type, raw_value,
    normalized_value, span_start, span_end, confidence, extractor
)
SELECT
    id, resume_version_id, section_id, entity_type, raw_value,
    normalized_value, span_start, span_end, confidence, extractor
FROM entity_mention_v18;

DROP TABLE entity_mention_v18;

CREATE INDEX entity_mention_version_idx
    ON entity_mention(resume_version_id, entity_type);
CREATE INDEX entity_mention_type_value_idx
    ON entity_mention(entity_type, normalized_value, confidence);
"#;

const SCHEMA_V20: &str = r#"
ALTER TABLE entity_mention RENAME TO entity_mention_v19;

CREATE TABLE entity_mention (
    id TEXT PRIMARY KEY,
    resume_version_id TEXT NOT NULL,
    section_id TEXT,
    entity_type TEXT NOT NULL CHECK (
        entity_type IN (
            'name',
            'email',
            'phone',
            'wechat',
            'school',
            'school_tier',
            'degree',
            'major',
            'company',
            'title',
            'education',
            'skills',
            'skill',
            'certificate',
            'date',
            'date_range',
            'years_experience',
            'location'
        )
        OR entity_type LIKE 'other:%'
    ),
    raw_value TEXT NOT NULL,
    normalized_value TEXT,
    span_start INTEGER CHECK (span_start IS NULL OR span_start >= 0),
    span_end INTEGER CHECK (span_end IS NULL OR span_end >= 0),
    confidence REAL NOT NULL CHECK (confidence >= 0 AND confidence <= 1),
    extractor TEXT NOT NULL,
    CHECK (
        span_start IS NULL
        OR span_end IS NULL
        OR span_start <= span_end
    ),
    FOREIGN KEY (resume_version_id) REFERENCES resume_version(id) ON DELETE CASCADE
);

INSERT INTO entity_mention (
    id, resume_version_id, section_id, entity_type, raw_value,
    normalized_value, span_start, span_end, confidence, extractor
)
SELECT
    id, resume_version_id, section_id, entity_type, raw_value,
    normalized_value, span_start, span_end, confidence, extractor
FROM entity_mention_v19;

DROP TABLE entity_mention_v19;

CREATE INDEX entity_mention_version_idx
    ON entity_mention(resume_version_id, entity_type);
CREATE INDEX entity_mention_type_value_idx
    ON entity_mention(entity_type, normalized_value, confidence);
"#;

const SCHEMA_V21: &str = r#"
ALTER TABLE index_state
    ADD COLUMN visible_epoch INTEGER NOT NULL DEFAULT 0 CHECK (visible_epoch >= 0);

ALTER TABLE index_state
    ADD COLUMN manifest_document_count INTEGER NOT NULL DEFAULT 0 CHECK (manifest_document_count >= 0);
"#;

const SCHEMA_V22: &str = r#"
CREATE TABLE document_classification (
    document_id TEXT PRIMARY KEY NOT NULL,
    status TEXT NOT NULL CHECK (status IN (
        'resume_candidate', 'non_resume', 'needs_review', 'ocr_backlog', 'failed'
    )),
    classifier_epoch TEXT NOT NULL CHECK (
        length(classifier_epoch) BETWEEN 1 AND 64
        AND classifier_epoch NOT GLOB '*[^a-z0-9_]*'
    ),
    classified_at_seconds INTEGER NOT NULL CHECK (classified_at_seconds >= 0),
    review_disposition TEXT NOT NULL CHECK (review_disposition IN ('not_required', 'pending')),
    CHECK (
        (status = 'needs_review' AND review_disposition = 'pending')
        OR (status != 'needs_review' AND review_disposition = 'not_required')
    ),
    FOREIGN KEY (document_id) REFERENCES document(id) ON DELETE CASCADE
);

CREATE TABLE document_classification_reason (
    document_id TEXT NOT NULL,
    ordinal INTEGER NOT NULL CHECK (ordinal BETWEEN 0 AND 7),
    reason_code TEXT NOT NULL CHECK (reason_code IN (
        'profile_heading', 'experience_heading', 'education_heading', 'skills_heading',
        'career_history_detail', 'invoice_heading', 'invoice_terms', 'meeting_heading',
        'meeting_workflow', 'manual_heading', 'manual_instructions',
        'corroborated_resume_signals', 'corroborated_non_resume_signals',
        'conflicting_signal_families', 'insufficient_signal_families',
        'empty_normalized_text', 'ocr_required', 'parser_failed'
    )),
    PRIMARY KEY (document_id, ordinal),
    UNIQUE (document_id, reason_code),
    FOREIGN KEY (document_id) REFERENCES document_classification(document_id) ON DELETE CASCADE
);

CREATE INDEX document_classification_review_idx
    ON document_classification(review_disposition, status);
"#;

fn migration_applied(connection: &Connection, version: u32) -> Result<bool> {
    let exists = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM schema_migrations WHERE version = ?1)",
            params![i64::from(version)],
            |row| row.get::<_, i64>(0),
        )
        .map_err(MetaStoreError::migration)?;

    Ok(exists == 1)
}

fn read_document(row: &Row<'_>) -> Result<Document> {
    let id = read_id::<DocumentId>(row, 0, "document.id")?;
    let byte_size = i64_to_u64(read_i64(row, 5)?, "document.byte_size")?;

    Ok(Document {
        id,
        source_uri: read_string(row, 1)?,
        normalized_path: read_string(row, 2)?,
        file_name: read_string(row, 3)?,
        extension: file_extension_from_storage(&read_string(row, 4)?),
        byte_size,
        mtime: UnixTimestamp::from_unix_seconds(read_i64(row, 6)?),
        content_hash: read_optional_string(row, 7)?,
        text_hash: read_optional_string(row, 8)?,
        is_deleted: read_i64(row, 9)? == 1,
        created_at: UnixTimestamp::from_unix_seconds(read_i64(row, 10)?),
        updated_at: UnixTimestamp::from_unix_seconds(read_i64(row, 11)?),
        status: document_status_from_storage(&read_string(row, 12)?)?,
    })
}

fn read_resume_version(row: &Row<'_>) -> Result<ResumeVersion> {
    let language_set_json = read_string(row, 5)?;
    let language_set = serde_json::from_str::<Vec<String>>(&language_set_json)
        .map_err(|_| MetaStoreError::invalid_value("resume_version.language_set"))?;
    let page_count = read_optional_i64(row, 6)?
        .map(|value| {
            u32::try_from(value)
                .map_err(|_| MetaStoreError::invalid_value("resume_version.page_count"))
        })
        .transpose()?;
    let quality_score = read_optional_f64(row, 9)?.map(|value| value as f32);

    Ok(ResumeVersion {
        id: read_id::<ResumeVersionId>(row, 0, "resume_version.id")?,
        document_id: read_id::<DocumentId>(row, 1, "resume_version.document_id")?,
        candidate_id: read_optional_id::<CandidateId>(row, 2, "resume_version.candidate_id")?,
        parse_version: read_string(row, 3)?,
        schema_version: read_string(row, 4)?,
        language_set,
        page_count,
        raw_text: read_optional_string(row, 7)?,
        clean_text: read_optional_string(row, 8)?,
        quality_score,
        visibility: resume_visibility_from_storage(&read_string(row, 10)?)?,
    })
}

fn read_entity_mention(row: &Row<'_>) -> Result<EntityMention> {
    let span_start = read_optional_i64(row, 6)?
        .map(|value| i64_to_usize(value, "entity_mention.span_start"))
        .transpose()?;
    let span_end = read_optional_i64(row, 7)?
        .map(|value| i64_to_usize(value, "entity_mention.span_end"))
        .transpose()?;

    Ok(EntityMention {
        id: read_id::<EntityMentionId>(row, 0, "entity_mention.id")?,
        resume_version_id: read_id::<ResumeVersionId>(row, 1, "entity_mention.resume_version_id")?,
        section_id: read_optional_id::<SectionId>(row, 2, "entity_mention.section_id")?,
        entity_type: entity_type_from_storage(&read_string(row, 3)?)?,
        raw_value: read_string(row, 4)?,
        normalized_value: read_optional_string(row, 5)?,
        span_start,
        span_end,
        confidence: row.get::<_, f64>(8).map_err(MetaStoreError::storage)? as f32,
        extractor: read_string(row, 9)?,
    })
}

fn read_ingest_job(row: &Row<'_>) -> Result<IngestJob> {
    let attempt_count = i64_to_u32(read_i64(row, 5)?, "ingest_job.attempt_count")?;
    let max_attempts = i64_to_u32(read_i64(row, 6)?, "ingest_job.max_attempts")?;

    Ok(IngestJob {
        id: read_id::<IngestJobId>(row, 0, "ingest_job.id")?,
        document_id: read_id::<DocumentId>(row, 1, "ingest_job.document_id")?,
        resume_version_id: read_optional_id::<ResumeVersionId>(
            row,
            2,
            "ingest_job.resume_version_id",
        )?,
        kind: ingest_job_kind_from_storage(&read_string(row, 3)?)?,
        status: ingest_job_status_from_storage(&read_string(row, 4)?)?,
        attempt_count,
        max_attempts,
        queued_at: UnixTimestamp::from_unix_seconds(read_i64(row, 7)?),
        started_at: read_optional_timestamp(row, 8)?,
        finished_at: read_optional_timestamp(row, 9)?,
        updated_at: UnixTimestamp::from_unix_seconds(read_i64(row, 10)?),
        failure_kind: read_optional_string(row, 11)?
            .as_deref()
            .map(ingest_job_failure_kind_from_storage)
            .transpose()?,
    })
}

fn read_index_state(row: &Row<'_>) -> Result<IndexState> {
    Ok(IndexState {
        manifest_version: read_string(row, 0)?,
        snapshot_token: read_optional_string(row, 1)?,
        status: index_state_status_from_storage(&read_string(row, 2)?)?,
        updated_at: UnixTimestamp::from_unix_seconds(read_i64(row, 3)?),
        visible_epoch: i64_to_u64(read_i64(row, 4)?, "index_state.visible_epoch")?,
        manifest_document_count: i64_to_u64(
            read_i64(row, 5)?,
            "index_state.manifest_document_count",
        )?,
    })
}

fn read_ocr_page_cache_entry(row: &Row<'_>) -> Result<OcrPageCacheEntry> {
    let page_no = i64_to_u32(read_i64(row, 1)?, "ocr_page_cache.page_no")?;
    let render_dpi = i64_to_u32(read_i64(row, 2)?, "ocr_page_cache.render_dpi")?;
    let key = OcrPageCacheKey::new(
        read_string(row, 0)?,
        page_no,
        render_dpi,
        read_string(row, 3)?,
        read_string(row, 4)?,
    )?;
    let duration_ms = read_optional_i64(row, 8)?
        .map(|value| i64_to_u64(value, "ocr_page_cache.duration_ms"))
        .transpose()?;
    let entry = OcrPageCacheEntry {
        key,
        text: read_optional_string(row, 5)?,
        word_boxes: read_ocr_word_boxes_json(read_optional_string(row, 12)?.as_deref())?,
        confidence: read_optional_f64(row, 6)?.map(|value| value as f32),
        engine_profile: read_optional_string(row, 7)?,
        duration_ms,
        status: ocr_page_cache_status_from_storage(&read_string(row, 9)?)?,
        error_kind: read_optional_string(row, 10)?,
        updated_at: UnixTimestamp::from_unix_seconds(read_i64(row, 11)?),
    };
    validate_ocr_page_cache_entry(&entry)?;
    Ok(entry)
}

fn read_worker_task_control(row: &Row<'_>) -> Result<WorkerTaskControl> {
    Ok(WorkerTaskControl {
        task: worker_task_kind_from_storage(&read_string(row, 0)?)?,
        paused: i64_to_bool(read_i64(row, 1)?, "worker_task_control.paused")?,
        updated_at: UnixTimestamp::from_unix_seconds(read_i64(row, 2)?),
    })
}

fn read_import_task(row: &Row<'_>) -> Result<ImportTask> {
    Ok(ImportTask {
        id: read_id::<ImportTaskId>(row, 0, "import_task.id")?,
        root_path: read_string(row, 1)?,
        status: import_task_status_from_storage(&read_string(row, 2)?)?,
        queued_at: UnixTimestamp::from_unix_seconds(read_i64(row, 3)?),
        started_at: read_optional_timestamp(row, 4)?,
        finished_at: read_optional_timestamp(row, 5)?,
        updated_at: UnixTimestamp::from_unix_seconds(read_i64(row, 6)?),
    })
}

fn pending_import_task_by_root_sql() -> String {
    format!(
        "\
        SELECT {IMPORT_TASK_COLUMNS}
        FROM import_task
        WHERE root_path = ?1 AND status IN (?2, ?3, ?4)
            AND NOT EXISTS (
                SELECT 1
                FROM import_task_cancellation AS cancellation
                WHERE cancellation.import_task_id = import_task.id
            )
        ORDER BY CASE WHEN status = ?3 THEN 0 ELSE 1 END, queued_at_seconds, rowid
        LIMIT 1"
    )
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct DocumentPathRecord {
    source_uri: String,
    normalized_path: String,
}

fn document_paths_for_ids_from_connection(
    connection: &Connection,
    document_ids: &[DocumentId],
) -> Result<Vec<DocumentPathRecord>> {
    if document_ids.is_empty() {
        return Ok(Vec::new());
    }
    let placeholders = (0..document_ids.len())
        .map(|index| format!("?{}", index + 1))
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!(
        "\
        SELECT source_uri, normalized_path
        FROM document
        WHERE id IN ({placeholders})"
    );
    let params = document_ids
        .iter()
        .map(|document_id| Value::Text(document_id.as_str().to_string()))
        .collect::<Vec<_>>();
    let mut statement = connection.prepare(&sql).map_err(MetaStoreError::storage)?;
    let mut rows = statement
        .query(params_from_iter(params))
        .map_err(MetaStoreError::storage)?;
    let mut paths = Vec::new();

    while let Some(row) = rows.next().map_err(MetaStoreError::storage)? {
        paths.push(DocumentPathRecord {
            source_uri: read_string(row, 0)?,
            normalized_path: read_string(row, 1)?,
        });
    }

    Ok(paths)
}

fn visible_document_paths_from_connection(
    connection: &Connection,
) -> Result<Vec<DocumentPathRecord>> {
    let mut statement = connection
        .prepare(
            "\
            SELECT source_uri, normalized_path
            FROM document
            WHERE is_deleted = 0 AND status <> ?1",
        )
        .map_err(MetaStoreError::storage)?;
    let mut rows = statement
        .query(params![document_status_to_storage(DocumentStatus::Deleted)])
        .map_err(MetaStoreError::storage)?;
    let mut paths = Vec::new();

    while let Some(row) = rows.next().map_err(MetaStoreError::storage)? {
        paths.push(DocumentPathRecord {
            source_uri: read_string(row, 0)?,
            normalized_path: read_string(row, 1)?,
        });
    }

    Ok(paths)
}

fn import_tasks_from_connection(connection: &Connection) -> Result<Vec<ImportTask>> {
    let sql = format!("SELECT {IMPORT_TASK_COLUMNS} FROM import_task ORDER BY rowid");
    let mut statement = connection.prepare(&sql).map_err(MetaStoreError::storage)?;
    let mut rows = statement.query([]).map_err(MetaStoreError::storage)?;
    let mut tasks = Vec::new();

    while let Some(row) = rows.next().map_err(MetaStoreError::storage)? {
        tasks.push(read_import_task(row)?);
    }

    Ok(tasks)
}

fn import_tasks_for_deleted_document_roots_from_connection(
    connection: &Connection,
    document_ids: &[DocumentId],
) -> Result<Vec<ImportTask>> {
    if document_ids.is_empty() {
        return Ok(Vec::new());
    }

    let deleted_paths = document_paths_for_ids_from_connection(connection, document_ids)?;
    if deleted_paths.is_empty() {
        return Ok(Vec::new());
    }
    let visible_paths = visible_document_paths_from_connection(connection)?;
    let import_tasks = import_tasks_from_connection(connection)?;

    Ok(import_tasks
        .into_iter()
        .filter(|task| {
            deleted_paths
                .iter()
                .any(|path| import_root_matches_document_path(&task.root_path, path))
                && !visible_paths
                    .iter()
                    .any(|path| import_root_matches_document_path(&task.root_path, path))
        })
        .collect())
}

fn import_root_matches_document_path(root_path: &str, document_path: &DocumentPathRecord) -> bool {
    path_string_is_root_or_child(root_path, &document_path.normalized_path)
        || path_string_is_root_or_child(root_path, &document_path.source_uri)
}

fn path_string_is_root_or_child(root_path: &str, path: &str) -> bool {
    let Some(root) = path_match_key(root_path) else {
        return false;
    };
    let Some(path) = path_match_key(path) else {
        return false;
    };
    if path == root {
        return true;
    }
    path.strip_prefix(&root)
        .is_some_and(|remaining| remaining.starts_with('/'))
}

fn path_match_key(raw: &str) -> Option<String> {
    let mut value = raw.trim();
    if value.is_empty() {
        return None;
    }
    if value.len() >= "file://".len() && value[.."file://".len()].eq_ignore_ascii_case("file://") {
        value = &value["file://".len()..];
    }

    let mut normalized = value.replace('\\', "/");
    if normalized.len() >= "//?/UNC/".len()
        && normalized[.."//?/UNC/".len()].eq_ignore_ascii_case("//?/UNC/")
    {
        normalized = format!("//{}", &normalized["//?/UNC/".len()..]);
    } else if normalized.len() >= "//?/".len()
        && normalized[.."//?/".len()].eq_ignore_ascii_case("//?/")
    {
        normalized = normalized["//?/".len()..].to_string();
    } else if normalized.len() >= "//./".len()
        && normalized[.."//./".len()].eq_ignore_ascii_case("//./")
    {
        normalized = normalized["//./".len()..].to_string();
    }

    if normalized.len() >= 3
        && normalized.as_bytes()[0] == b'/'
        && normalized.as_bytes()[2] == b':'
        && normalized.as_bytes()[1].is_ascii_alphabetic()
    {
        normalized = normalized[1..].to_string();
    }

    let normalized = normalize_path_match_key(&normalized);
    if path_match_key_is_windows_path(&normalized) {
        Some(normalized.to_ascii_lowercase())
    } else {
        Some(normalized)
    }
}

fn normalize_path_match_key(raw: &str) -> String {
    let (drive_prefix, drive_absolute, without_drive) = split_windows_drive_match_key(raw);
    let unc_prefix = drive_prefix.is_none() && without_drive.starts_with("//");
    let absolute = drive_prefix.is_none() && without_drive.starts_with('/') && !unc_prefix;
    let anchored = drive_absolute || absolute || unc_prefix;
    let minimum_parts = if unc_prefix { 2 } else { 0 };
    let mut parts = Vec::<&str>::new();

    for part in without_drive.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                if parts.len() > minimum_parts && parts.last().is_some_and(|last| *last != "..") {
                    parts.pop();
                } else if !anchored {
                    parts.push(part);
                }
            }
            _ => parts.push(part),
        }
    }

    match (
        drive_prefix,
        drive_absolute,
        unc_prefix,
        absolute,
        parts.is_empty(),
    ) {
        (Some(prefix), true, _, _, true) => format!("{prefix}:/"),
        (Some(prefix), true, _, _, false) => format!("{prefix}:/{}", parts.join("/")),
        (Some(prefix), false, _, _, true) => format!("{prefix}:"),
        (Some(prefix), false, _, _, false) => format!("{prefix}:{}", parts.join("/")),
        (None, _, true, _, true) => "//".to_string(),
        (None, _, true, _, false) => format!("//{}", parts.join("/")),
        (None, _, false, true, true) => "/".to_string(),
        (None, _, false, true, false) => format!("/{}", parts.join("/")),
        (None, _, false, false, true) => ".".to_string(),
        (None, _, false, false, false) => parts.join("/"),
    }
}

fn split_windows_drive_match_key(path: &str) -> (Option<char>, bool, &str) {
    let bytes = path.as_bytes();
    if bytes.len() >= 2 && bytes[1] == b':' && bytes[0].is_ascii_alphabetic() {
        let drive = (bytes[0] as char).to_ascii_lowercase();
        let rest = &path[2..];
        return (Some(drive), rest.starts_with('/'), rest);
    }

    (None, false, path)
}

fn path_match_key_is_windows_path(path: &str) -> bool {
    let bytes = path.as_bytes();
    path.starts_with("//")
        || (bytes.len() >= 2 && bytes[1] == b':' && bytes[0].is_ascii_alphabetic())
}

fn count_import_task_child_rows(
    connection: &Connection,
    table_name: &'static str,
    column_name: &'static str,
    task_ids: &[ImportTaskId],
) -> Result<usize> {
    if task_ids.is_empty() {
        return Ok(0);
    }
    let placeholders = import_task_id_placeholders(task_ids.len());
    let sql = format!(
        "\
        SELECT COUNT(*)
        FROM {table_name}
        WHERE {column_name} IN ({placeholders})"
    );
    let params = task_ids
        .iter()
        .map(|task_id| Value::Text(task_id.as_str().to_string()))
        .collect::<Vec<_>>();
    let count = connection
        .query_row(&sql, params_from_iter(params), |row| row.get::<_, i64>(0))
        .map_err(MetaStoreError::storage)?;
    i64_to_usize(count, "import_task_child.count")
}

fn import_task_id_placeholders(count: usize) -> String {
    (0..count)
        .map(|index| format!("?{}", index + 1))
        .collect::<Vec<_>>()
        .join(", ")
}

fn read_import_scan_scope(row: &Row<'_>) -> Result<ImportScanScope> {
    let scope = ImportScanScope {
        import_task_id: read_id::<ImportTaskId>(row, 0, "import_scan_scope.import_task_id")?,
        root_kind: import_root_kind_from_storage(&read_string(row, 1)?)?,
        root_preset: read_optional_string(row, 2)?
            .as_deref()
            .map(import_root_preset_from_storage)
            .transpose()?,
        scan_profile: import_scan_profile_from_storage(&read_string(row, 3)?)?,
        requested_root_path: read_string(row, 4)?,
        canonical_root_path: read_string(row, 5)?,
        files_discovered: i64_to_u64(read_i64(row, 6)?, "import_scan_scope.files_discovered")?,
        ignored_entries: i64_to_u64(read_i64(row, 7)?, "import_scan_scope.ignored_entries")?,
        scan_errors: i64_to_u64(read_i64(row, 8)?, "import_scan_scope.scan_errors")?,
        searchable_documents: i64_to_u64(
            read_i64(row, 9)?,
            "import_scan_scope.searchable_documents",
        )?,
        ocr_required_documents: i64_to_u64(
            read_i64(row, 10)?,
            "import_scan_scope.ocr_required_documents",
        )?,
        ocr_jobs_queued: i64_to_u64(read_i64(row, 11)?, "import_scan_scope.ocr_jobs_queued")?,
        failed_documents: i64_to_u64(read_i64(row, 12)?, "import_scan_scope.failed_documents")?,
        deleted_documents: i64_to_u64(read_i64(row, 13)?, "import_scan_scope.deleted_documents")?,
        scan_budget_kind: read_optional_string(row, 14)?
            .as_deref()
            .map(import_scan_budget_kind_from_storage)
            .transpose()?,
        scan_budget_limit: read_optional_i64(row, 15)?
            .map(|value| i64_to_u64(value, "import_scan_scope.scan_budget_limit"))
            .transpose()?,
        scan_budget_observed: read_optional_i64(row, 16)?
            .map(|value| i64_to_u64(value, "import_scan_scope.scan_budget_observed"))
            .transpose()?,
        scan_budget_exhausted: i64_to_bool(
            read_i64(row, 17)?,
            "import_scan_scope.scan_budget_exhausted",
        )?,
        updated_at: UnixTimestamp::from_unix_seconds(read_i64(row, 18)?),
    };
    validate_import_scan_scope(&scope)?;
    Ok(scope)
}

fn read_import_scan_error(row: &Row<'_>) -> Result<ImportScanError> {
    let error = ImportScanError {
        import_task_id: read_id::<ImportTaskId>(row, 0, "import_scan_error.import_task_id")?,
        error_index: i64_to_u64(read_i64(row, 1)?, "import_scan_error.error_index")?,
        kind: import_scan_error_kind_from_storage(&read_string(row, 2)?)?,
        operation: import_scan_error_operation_from_storage(&read_string(row, 3)?)?,
        path_digest: read_optional_string(row, 4)?,
        updated_at: UnixTimestamp::from_unix_seconds(read_i64(row, 5)?),
    };
    validate_import_scan_error(&error.import_task_id, &error)?;
    Ok(error)
}

fn upsert_document_in_connection(connection: &Connection, document: &Document) -> Result<()> {
    connection
        .execute(
            "\
            INSERT INTO document (
                id, source_uri, normalized_path, file_name, extension, byte_size,
                mtime_seconds, content_hash, text_hash, is_deleted, created_at_seconds,
                updated_at_seconds, status
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
            ON CONFLICT(id) DO UPDATE SET
                source_uri = excluded.source_uri,
                normalized_path = excluded.normalized_path,
                file_name = excluded.file_name,
                extension = excluded.extension,
                byte_size = excluded.byte_size,
                mtime_seconds = excluded.mtime_seconds,
                content_hash = excluded.content_hash,
                text_hash = excluded.text_hash,
                is_deleted = excluded.is_deleted,
                created_at_seconds = excluded.created_at_seconds,
                updated_at_seconds = excluded.updated_at_seconds,
                status = excluded.status",
            params![
                document.id.as_str(),
                document.source_uri,
                document.normalized_path,
                document.file_name,
                file_extension_to_storage(&document.extension),
                u64_to_i64(document.byte_size, "document.byte_size")?,
                document.mtime.as_unix_seconds(),
                document.content_hash,
                document.text_hash,
                bool_to_i64(document.is_deleted),
                document.created_at.as_unix_seconds(),
                document.updated_at.as_unix_seconds(),
                document_status_to_storage(document.status),
            ],
        )
        .map_err(MetaStoreError::storage)?;

    Ok(())
}

fn ocr_claim_is_current_in_connection(
    connection: &Connection,
    claimed: &ClaimedOcrJob,
) -> Result<bool> {
    let job = &claimed.job;
    connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM ingest_job AS job
             JOIN document ON document.id = job.document_id
             JOIN document_classification AS classification
               ON classification.document_id = document.id
             WHERE job.id = ?1 AND job.document_id = ?2 AND job.kind = ?3
               AND job.status = ?4 AND job.attempt_count = ?5 AND job.max_attempts = ?6
               AND document.is_deleted = 0 AND document.status = ?7
               AND document.content_hash = ?8 AND classification.status = ?9
               AND classification.classifier_epoch = ?10)",
            params![
                job.id.as_str(),
                job.document_id.as_str(),
                ingest_job_kind_to_storage(IngestJobKind::OcrDocument),
                ingest_job_status_to_storage(IngestJobStatus::Running),
                u32_to_i64(job.attempt_count),
                u32_to_i64(job.max_attempts),
                document_status_to_storage(DocumentStatus::OcrRequired),
                claimed.source_fingerprint(),
                ClassificationStatus::OcrBacklog.as_str(),
                CLASSIFIER_EPOCH,
            ],
            |row| row.get::<_, i64>(0),
        )
        .map(|exists| exists == 1)
        .map_err(MetaStoreError::storage)
}

fn upsert_index_state_in_connection(connection: &Connection, state: &IndexState) -> Result<()> {
    connection
        .execute(
            "INSERT INTO index_state (
                state_key, manifest_version, snapshot_token, status, updated_at_seconds,
                visible_epoch, manifest_document_count
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(state_key) DO UPDATE SET
                manifest_version = excluded.manifest_version,
                snapshot_token = excluded.snapshot_token, status = excluded.status,
                updated_at_seconds = excluded.updated_at_seconds,
                visible_epoch = excluded.visible_epoch,
                manifest_document_count = excluded.manifest_document_count",
            params![
                INDEX_STATE_KEY,
                state.manifest_version,
                state.snapshot_token,
                index_state_status_to_storage(state.status),
                state.updated_at.as_unix_seconds(),
                u64_to_i64(state.visible_epoch, "index_state.visible_epoch")?,
                u64_to_i64(
                    state.manifest_document_count,
                    "index_state.manifest_document_count",
                )?,
            ],
        )
        .map_err(MetaStoreError::storage)?;
    Ok(())
}

fn upsert_resume_version_in_connection(
    connection: &Connection,
    version: &ResumeVersion,
) -> Result<()> {
    let language_set_json = serde_json::to_string(&version.language_set)
        .map_err(|_| MetaStoreError::invalid_value("resume_version.language_set"))?;
    connection
        .execute(
            "\
            INSERT INTO resume_version (
                id, document_id, candidate_id, parse_version, schema_version,
                language_set_json, page_count, raw_text, clean_text, quality_score, visibility
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
            ON CONFLICT(id) DO UPDATE SET
                document_id = excluded.document_id,
                candidate_id = excluded.candidate_id,
                parse_version = excluded.parse_version,
                schema_version = excluded.schema_version,
                language_set_json = excluded.language_set_json,
                page_count = excluded.page_count,
                raw_text = excluded.raw_text,
                clean_text = excluded.clean_text,
                quality_score = excluded.quality_score,
                visibility = excluded.visibility",
            params![
                version.id.as_str(),
                version.document_id.as_str(),
                version.candidate_id.as_ref().map(CandidateId::as_str),
                version.parse_version,
                version.schema_version,
                language_set_json,
                version.page_count.map(i64::from),
                version.raw_text,
                version.clean_text,
                version.quality_score.map(f64::from),
                resume_visibility_to_storage(version.visibility),
            ],
        )
        .map_err(MetaStoreError::storage)?;

    Ok(())
}

fn replace_entity_mentions_in_connection(
    connection: &Connection,
    version_id: &ResumeVersionId,
    mentions: &[EntityMention],
) -> Result<()> {
    connection
        .execute(
            "DELETE FROM entity_mention WHERE resume_version_id = ?1",
            params![version_id.as_str()],
        )
        .map_err(MetaStoreError::storage)?;

    for mention in mentions {
        validate_entity_mention(version_id, mention)?;
        connection
            .execute(
                "\
                INSERT INTO entity_mention (
                    id, resume_version_id, section_id, entity_type, raw_value,
                    normalized_value, span_start, span_end, confidence, extractor
                )
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                params![
                    mention.id.as_str(),
                    mention.resume_version_id.as_str(),
                    mention.section_id.as_ref().map(SectionId::as_str),
                    entity_type_to_storage(&mention.entity_type),
                    entity_mention_raw_value_for_storage(mention),
                    entity_mention_normalized_value_for_storage(mention),
                    mention
                        .span_start
                        .map(|value| usize_to_i64(value, "entity_mention.span_start"))
                        .transpose()?,
                    mention
                        .span_end
                        .map(|value| usize_to_i64(value, "entity_mention.span_end"))
                        .transpose()?,
                    f64::from(mention.confidence),
                    mention.extractor.as_str(),
                ],
            )
            .map_err(MetaStoreError::storage)?;
    }

    Ok(())
}

fn assign_candidate_from_hashed_contacts_in_connection(
    connection: &Connection,
    version_id: &ResumeVersionId,
    email_hash: Option<&ContactHash>,
    phone_hash: Option<&ContactHash>,
    conflict_updated_at: UnixTimestamp,
) -> Result<Option<Candidate>> {
    if email_hash.is_none() && phone_hash.is_none() {
        return Ok(None);
    }

    let Some(_version) = resume_version_by_id_from_connection(connection, version_id)? else {
        return Ok(None);
    };

    let candidate =
        match candidate_contact_match_from_connection(connection, email_hash, phone_hash)? {
            CandidateContactMatch::Conflict {
                email_candidate,
                phone_candidate,
            } => {
                upsert_candidate_contact_conflict_in_connection(
                    connection,
                    version_id,
                    &email_candidate.id,
                    &phone_candidate.id,
                    conflict_updated_at,
                )?;
                return Ok(None);
            }
            CandidateContactMatch::Single(candidate) => candidate,
            CandidateContactMatch::None => {
                let candidate = Candidate {
                    id: CandidateId::from_non_secret_parts(&[
                        "candidate-assignment-v1",
                        version_id.as_str(),
                    ]),
                    primary_name: None,
                    phone_hash: phone_hash.cloned(),
                    email_hash: email_hash.cloned(),
                    dedupe_key: None,
                    merge_confidence: Some(1.0),
                    version_count: 0,
                };
                upsert_candidate_in_connection(connection, &candidate)?;
                candidate
            }
        };

    connection
        .execute(
            "UPDATE resume_version SET candidate_id = ?1 WHERE id = ?2",
            params![candidate.id.as_str(), version_id.as_str()],
        )
        .map_err(MetaStoreError::storage)?;
    clear_candidate_contact_conflict_in_connection(connection, version_id)?;
    refresh_candidate_version_count_in_connection(connection, &candidate.id)?;
    candidate_by_id_from_connection(connection, &candidate.id)
}

fn upsert_candidate_in_connection(connection: &Connection, candidate: &Candidate) -> Result<()> {
    validate_candidate(candidate)?;
    connection
        .execute(
            "\
            INSERT INTO candidate (
                id, primary_name, phone_hash, email_hash, dedupe_key, merge_confidence,
                version_count
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
            ON CONFLICT(id) DO UPDATE SET
                primary_name = excluded.primary_name,
                phone_hash = excluded.phone_hash,
                email_hash = excluded.email_hash,
                dedupe_key = excluded.dedupe_key,
                merge_confidence = excluded.merge_confidence,
                version_count = excluded.version_count",
            params![
                candidate.id.as_str(),
                candidate.primary_name.as_deref(),
                candidate.phone_hash.as_ref().map(ContactHash::as_str),
                candidate.email_hash.as_ref().map(ContactHash::as_str),
                candidate.dedupe_key.as_deref(),
                candidate.merge_confidence.map(f64::from),
                u32_to_i64(candidate.version_count),
            ],
        )
        .map_err(MetaStoreError::storage)?;

    Ok(())
}

fn candidate_by_id_from_connection(
    connection: &Connection,
    id: &CandidateId,
) -> Result<Option<Candidate>> {
    let sql = format!("SELECT {CANDIDATE_COLUMNS} FROM candidate WHERE id = ?1");
    let mut statement = connection.prepare(&sql).map_err(MetaStoreError::storage)?;
    let mut rows = statement
        .query(params![id.as_str()])
        .map_err(MetaStoreError::storage)?;

    match rows.next().map_err(MetaStoreError::storage)? {
        Some(row) => Ok(Some(read_candidate(row)?)),
        None => Ok(None),
    }
}

fn candidate_by_contact_hash_from_connection(
    connection: &Connection,
    contact_hash: &ContactHash,
) -> Result<Option<Candidate>> {
    let sql = format!(
        "\
        SELECT {CANDIDATE_COLUMNS}
        FROM candidate
        WHERE email_hash = ?1 OR phone_hash = ?1
        ORDER BY id
        LIMIT 2"
    );
    let mut statement = connection.prepare(&sql).map_err(MetaStoreError::storage)?;
    let mut rows = statement
        .query(params![contact_hash.as_str()])
        .map_err(MetaStoreError::storage)?;
    let mut candidates = Vec::new();

    while let Some(row) = rows.next().map_err(MetaStoreError::storage)? {
        candidates.push(read_candidate(row)?);
    }

    match candidates.len() {
        0 => Ok(None),
        1 => Ok(candidates.pop()),
        _ => Err(MetaStoreError::invalid_value("candidate.contact_hash")),
    }
}

enum CandidateContactMatch {
    None,
    Single(Candidate),
    Conflict {
        email_candidate: Candidate,
        phone_candidate: Candidate,
    },
}

fn candidate_contact_match_from_connection(
    connection: &Connection,
    email_hash: Option<&ContactHash>,
    phone_hash: Option<&ContactHash>,
) -> Result<CandidateContactMatch> {
    let email_candidate = email_hash
        .map(|contact_hash| candidate_by_email_hash_from_connection(connection, contact_hash))
        .transpose()?
        .flatten();
    let phone_candidate = phone_hash
        .map(|contact_hash| candidate_by_phone_hash_from_connection(connection, contact_hash))
        .transpose()?
        .flatten();

    match (email_candidate, phone_candidate) {
        (Some(email_candidate), Some(phone_candidate)) => {
            if email_candidate.id == phone_candidate.id {
                Ok(CandidateContactMatch::Single(email_candidate))
            } else {
                Ok(CandidateContactMatch::Conflict {
                    email_candidate,
                    phone_candidate,
                })
            }
        }
        (Some(candidate), None) | (None, Some(candidate)) => {
            Ok(CandidateContactMatch::Single(candidate))
        }
        (None, None) => Ok(CandidateContactMatch::None),
    }
}

fn candidate_by_email_hash_from_connection(
    connection: &Connection,
    contact_hash: &ContactHash,
) -> Result<Option<Candidate>> {
    candidate_by_exact_contact_hash_from_connection(connection, "email_hash", contact_hash)
}

fn candidate_by_phone_hash_from_connection(
    connection: &Connection,
    contact_hash: &ContactHash,
) -> Result<Option<Candidate>> {
    candidate_by_exact_contact_hash_from_connection(connection, "phone_hash", contact_hash)
}

fn candidate_by_exact_contact_hash_from_connection(
    connection: &Connection,
    column_name: &str,
    contact_hash: &ContactHash,
) -> Result<Option<Candidate>> {
    let sql = format!(
        "\
        SELECT {CANDIDATE_COLUMNS}
        FROM candidate
        WHERE {column_name} = ?1"
    );
    let mut statement = connection.prepare(&sql).map_err(MetaStoreError::storage)?;
    let mut rows = statement
        .query(params![contact_hash.as_str()])
        .map_err(MetaStoreError::storage)?;

    match rows.next().map_err(MetaStoreError::storage)? {
        Some(row) => Ok(Some(read_candidate(row)?)),
        None => Ok(None),
    }
}

fn candidate_contact_conflicts_from_connection(
    connection: &Connection,
) -> Result<Vec<CandidateContactConflict>> {
    let mut statement = connection
        .prepare(
            "\
            SELECT resume_version_id, email_candidate_id, phone_candidate_id, updated_at_seconds
            FROM candidate_contact_conflict
            ORDER BY updated_at_seconds DESC, resume_version_id",
        )
        .map_err(MetaStoreError::storage)?;
    let mut rows = statement.query([]).map_err(MetaStoreError::storage)?;
    let mut conflicts = Vec::new();

    while let Some(row) = rows.next().map_err(MetaStoreError::storage)? {
        conflicts.push(read_candidate_contact_conflict(row)?);
    }

    Ok(conflicts)
}

fn upsert_candidate_contact_conflict_in_connection(
    connection: &Connection,
    version_id: &ResumeVersionId,
    email_candidate_id: &CandidateId,
    phone_candidate_id: &CandidateId,
    updated_at: UnixTimestamp,
) -> Result<()> {
    if email_candidate_id == phone_candidate_id {
        return Err(MetaStoreError::invalid_value(
            "candidate_contact_conflict.candidate_id",
        ));
    }

    connection
        .execute(
            "\
            INSERT INTO candidate_contact_conflict (
                resume_version_id, email_candidate_id, phone_candidate_id, updated_at_seconds
            )
            VALUES (?1, ?2, ?3, ?4)
            ON CONFLICT(resume_version_id) DO UPDATE SET
                email_candidate_id = excluded.email_candidate_id,
                phone_candidate_id = excluded.phone_candidate_id,
                updated_at_seconds = excluded.updated_at_seconds",
            params![
                version_id.as_str(),
                email_candidate_id.as_str(),
                phone_candidate_id.as_str(),
                updated_at.as_unix_seconds(),
            ],
        )
        .map_err(MetaStoreError::storage)?;

    Ok(())
}

fn clear_candidate_contact_conflict_in_connection(
    connection: &Connection,
    version_id: &ResumeVersionId,
) -> Result<()> {
    connection
        .execute(
            "DELETE FROM candidate_contact_conflict WHERE resume_version_id = ?1",
            params![version_id.as_str()],
        )
        .map_err(MetaStoreError::storage)?;
    Ok(())
}

fn resume_version_by_id_from_connection(
    connection: &Connection,
    id: &ResumeVersionId,
) -> Result<Option<ResumeVersion>> {
    let sql = format!("SELECT {RESUME_VERSION_COLUMNS} FROM resume_version WHERE id = ?1");
    let mut statement = connection.prepare(&sql).map_err(MetaStoreError::storage)?;
    let mut rows = statement
        .query(params![id.as_str()])
        .map_err(MetaStoreError::storage)?;

    match rows.next().map_err(MetaStoreError::storage)? {
        Some(row) => Ok(Some(read_resume_version(row)?)),
        None => Ok(None),
    }
}

fn refresh_candidate_version_count_in_connection(
    connection: &Connection,
    candidate_id: &CandidateId,
) -> Result<()> {
    connection
        .execute(
            "\
            UPDATE candidate
            SET version_count = (
                SELECT COUNT(*)
                FROM resume_version
                WHERE candidate_id = ?1
            )
            WHERE id = ?1",
            params![candidate_id.as_str()],
        )
        .map_err(MetaStoreError::storage)?;
    Ok(())
}

fn read_candidate(row: &Row<'_>) -> Result<Candidate> {
    let merge_confidence = read_optional_f64(row, 5)?.map(|value| value as f32);
    let version_count = i64_to_u32(read_i64(row, 6)?, "candidate.version_count")?;

    Ok(Candidate {
        id: read_id::<CandidateId>(row, 0, "candidate.id")?,
        primary_name: read_optional_string(row, 1)?,
        phone_hash: read_optional_id::<ContactHash>(row, 2, "candidate.phone_hash")?,
        email_hash: read_optional_id::<ContactHash>(row, 3, "candidate.email_hash")?,
        dedupe_key: read_optional_string(row, 4)?,
        merge_confidence,
        version_count,
    })
}

fn read_candidate_contact_conflict(row: &Row<'_>) -> Result<CandidateContactConflict> {
    let conflict = CandidateContactConflict {
        resume_version_id: read_id::<ResumeVersionId>(
            row,
            0,
            "candidate_contact_conflict.resume_version_id",
        )?,
        email_candidate_id: read_id::<CandidateId>(
            row,
            1,
            "candidate_contact_conflict.email_candidate_id",
        )?,
        phone_candidate_id: read_id::<CandidateId>(
            row,
            2,
            "candidate_contact_conflict.phone_candidate_id",
        )?,
        updated_at: UnixTimestamp::from_unix_seconds(read_i64(row, 3)?),
    };
    if conflict.email_candidate_id == conflict.phone_candidate_id {
        return Err(MetaStoreError::invalid_value(
            "candidate_contact_conflict.candidate_id",
        ));
    }
    Ok(conflict)
}

fn deleted_document_ids_from_connection(connection: &Connection) -> Result<Vec<DocumentId>> {
    let mut statement = connection
        .prepare(
            "\
            SELECT id
            FROM document
            WHERE is_deleted = 1 OR status = 'deleted'
            ORDER BY id",
        )
        .map_err(MetaStoreError::storage)?;
    let mut rows = statement.query([]).map_err(MetaStoreError::storage)?;
    let mut document_ids = Vec::new();

    while let Some(row) = rows.next().map_err(MetaStoreError::storage)? {
        document_ids.push(read_id::<DocumentId>(row, 0, "document.id")?);
    }

    Ok(document_ids)
}

fn validate_candidate(candidate: &Candidate) -> Result<()> {
    if let Some(merge_confidence) = candidate.merge_confidence {
        if !merge_confidence.is_finite() || !(0.0..=1.0).contains(&merge_confidence) {
            return Err(MetaStoreError::invalid_value("candidate.merge_confidence"));
        }
    }

    Ok(())
}

fn validate_embedding_job_spec(model_id: &str, dimension: usize) -> Result<()> {
    if model_id.trim().is_empty()
        || model_id.contains('\n')
        || model_id.contains('\r')
        || model_id.contains('\t')
    {
        return Err(MetaStoreError::invalid_value("embedding_job_spec.model_id"));
    }
    if dimension == 0 {
        return Err(MetaStoreError::invalid_value(
            "embedding_job_spec.dimension",
        ));
    }

    Ok(())
}

fn validate_import_task(task: &ImportTask) -> Result<()> {
    let queued_at = task.queued_at.as_unix_seconds();
    let updated_at = task.updated_at.as_unix_seconds();
    if queued_at > updated_at {
        return Err(MetaStoreError::invalid_value("import_task.timestamps"));
    }

    let started_at = task.started_at.map(UnixTimestamp::as_unix_seconds);
    let finished_at = task.finished_at.map(UnixTimestamp::as_unix_seconds);

    if let Some(started_at) = started_at {
        if started_at < queued_at || started_at > updated_at {
            return Err(MetaStoreError::invalid_value("import_task.timestamps"));
        }
    }

    if let Some(finished_at) = finished_at {
        let Some(started_at) = started_at else {
            return Err(MetaStoreError::invalid_value("import_task.timestamps"));
        };
        if finished_at < started_at || finished_at > updated_at {
            return Err(MetaStoreError::invalid_value("import_task.timestamps"));
        }
    }

    let valid_state = match task.status {
        ImportTaskStatus::Queued => started_at.is_none() && finished_at.is_none(),
        ImportTaskStatus::Running => started_at.is_some() && finished_at.is_none(),
        ImportTaskStatus::Completed
        | ImportTaskStatus::FailedRetryable
        | ImportTaskStatus::FailedPermanent => started_at.is_some() && finished_at.is_some(),
    };

    if !valid_state {
        return Err(MetaStoreError::invalid_value("import_task.lifecycle"));
    }

    Ok(())
}

fn validate_import_scan_scope(scope: &ImportScanScope) -> Result<()> {
    if scope.requested_root_path.trim().is_empty() {
        return Err(MetaStoreError::invalid_value(
            "import_scan_scope.requested_root_path",
        ));
    }
    if scope.canonical_root_path.trim().is_empty() {
        return Err(MetaStoreError::invalid_value(
            "import_scan_scope.canonical_root_path",
        ));
    }

    match (scope.root_kind, scope.root_preset) {
        (ImportRootKind::Explicit, None) | (ImportRootKind::Preset, Some(_)) => {}
        _ => return Err(MetaStoreError::invalid_value("import_scan_scope.root")),
    };

    match (
        scope.scan_budget_kind,
        scope.scan_budget_limit,
        scope.scan_budget_observed,
        scope.scan_budget_exhausted,
    ) {
        (None, None, None, false) | (Some(_), Some(_), Some(_), false | true) => Ok(()),
        _ => Err(MetaStoreError::invalid_value(
            "import_scan_scope.scan_budget",
        )),
    }
}

fn validate_import_scan_error(task_id: &ImportTaskId, error: &ImportScanError) -> Result<()> {
    if &error.import_task_id != task_id {
        return Err(MetaStoreError::invalid_value(
            "import_scan_error.import_task_id",
        ));
    }
    if error.path_digest.as_deref().is_some_and(str::is_empty) {
        return Err(MetaStoreError::invalid_value(
            "import_scan_error.path_digest",
        ));
    }

    Ok(())
}

fn validate_entity_mention(version_id: &ResumeVersionId, mention: &EntityMention) -> Result<()> {
    if &mention.resume_version_id != version_id {
        return Err(MetaStoreError::invalid_value(
            "entity_mention.resume_version_id",
        ));
    }
    if mention.raw_value.trim().is_empty() {
        return Err(MetaStoreError::invalid_value("entity_mention.raw_value"));
    }
    if mention.extractor.trim().is_empty() {
        return Err(MetaStoreError::invalid_value("entity_mention.extractor"));
    }
    if !mention.confidence.is_finite() || !(0.0..=1.0).contains(&mention.confidence) {
        return Err(MetaStoreError::invalid_value("entity_mention.confidence"));
    }
    if let (Some(span_start), Some(span_end)) = (mention.span_start, mention.span_end) {
        if span_start > span_end {
            return Err(MetaStoreError::invalid_value("entity_mention.span"));
        }
    }

    Ok(())
}

fn validate_confidence_threshold(confidence: f32, field: &'static str) -> Result<()> {
    if !confidence.is_finite() || !(0.0..=1.0).contains(&confidence) {
        return Err(MetaStoreError::invalid_value(field));
    }
    Ok(())
}

fn validate_query_observation_mode(mode: &str) -> Result<&'static str> {
    match mode {
        "fulltext" => Ok("fulltext"),
        "semantic" => Ok("semantic"),
        "hybrid" => Ok("hybrid"),
        _ => Err(MetaStoreError::invalid_value("query_observation.mode")),
    }
}

fn query_latency_summary(connection: &Connection) -> Result<QueryLatencySummary> {
    let mut statement = connection
        .prepare("SELECT duration_ms FROM query_observation ORDER BY duration_ms ASC")
        .map_err(MetaStoreError::storage)?;
    let mut rows = statement.query([]).map_err(MetaStoreError::storage)?;
    let mut durations = Vec::new();
    while let Some(row) = rows.next().map_err(MetaStoreError::storage)? {
        durations.push(i64_to_u64(
            row.get::<_, i64>(0).map_err(MetaStoreError::storage)?,
            "query_observation.duration_ms",
        )?);
    }
    drop(rows);
    drop(statement);

    let last = connection
        .query_row(
            "\
            SELECT result_count, observed_at_seconds
            FROM query_observation
            ORDER BY observed_at_seconds DESC, rowid DESC
            LIMIT 1",
            [],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
        )
        .optional()
        .map_err(MetaStoreError::storage)?;
    let (last_result_count, last_observed_at) = match last {
        Some((result_count, observed_at)) => (
            Some(i64_to_u64(result_count, "query_observation.result_count")?),
            Some(UnixTimestamp::from_unix_seconds(observed_at)),
        ),
        None => (None, None),
    };

    Ok(QueryLatencySummary {
        sample_count: u64::try_from(durations.len())
            .map_err(|_| MetaStoreError::invalid_value("query_observation.sample_count"))?,
        p50_ms: percentile_nearest_rank(&durations, 50),
        p95_ms: percentile_nearest_rank(&durations, 95),
        p99_ms: percentile_nearest_rank(&durations, 99),
        last_result_count,
        last_observed_at,
    })
}

fn percentile_nearest_rank(sorted_values: &[u64], percentile: usize) -> Option<u64> {
    if sorted_values.is_empty() {
        return None;
    }
    let rank = sorted_values.len() * percentile;
    let index = rank.div_ceil(100).saturating_sub(1);
    sorted_values.get(index).copied()
}

fn validate_ocr_page_cache_entry(entry: &OcrPageCacheEntry) -> Result<()> {
    match entry.status {
        OcrPageCacheStatus::Succeeded => {
            if entry.text.is_none()
                || entry.engine_profile.as_deref().is_none_or(str::is_empty)
                || entry.duration_ms.is_none()
                || entry.error_kind.is_some()
            {
                return Err(MetaStoreError::invalid_value("ocr_page_cache.success"));
            }
            let Some(confidence) = entry.confidence else {
                return Err(MetaStoreError::invalid_value("ocr_page_cache.confidence"));
            };
            if !confidence.is_finite() || !(0.0..=1.0).contains(&confidence) {
                return Err(MetaStoreError::invalid_value("ocr_page_cache.confidence"));
            }
        }
        OcrPageCacheStatus::FailedRetryable | OcrPageCacheStatus::FailedPermanent => {
            if entry.text.is_some()
                || !entry.word_boxes.is_empty()
                || entry.confidence.is_some()
                || entry.engine_profile.is_some()
                || entry.duration_ms.is_some()
                || entry.error_kind.as_deref().is_none_or(str::is_empty)
            {
                return Err(MetaStoreError::invalid_value("ocr_page_cache.failure"));
            }
        }
    }

    Ok(())
}

fn ocr_word_boxes_json_for_storage(entry: &OcrPageCacheEntry) -> Result<Option<String>> {
    if entry.status != OcrPageCacheStatus::Succeeded {
        return Ok(None);
    }

    let values = entry
        .word_boxes
        .iter()
        .map(|word_box| {
            serde_json::json!({
                "text": word_box.text,
                "left": word_box.left,
                "top": word_box.top,
                "width": word_box.width,
                "height": word_box.height,
                "confidence": word_box.confidence,
            })
        })
        .collect::<Vec<_>>();
    serde_json::to_string(&values)
        .map(Some)
        .map_err(|_| MetaStoreError::invalid_value("ocr_page_cache.word_boxes_json"))
}

fn read_ocr_word_boxes_json(json: Option<&str>) -> Result<Vec<OcrWordBox>> {
    let Some(json) = json else {
        return Ok(Vec::new());
    };
    let value = serde_json::from_str::<serde_json::Value>(json)
        .map_err(|_| MetaStoreError::invalid_value("ocr_page_cache.word_boxes_json"))?;
    let array = value
        .as_array()
        .ok_or_else(|| MetaStoreError::invalid_value("ocr_page_cache.word_boxes_json"))?;

    array.iter().map(read_ocr_word_box_json).collect()
}

fn read_ocr_word_box_json(value: &serde_json::Value) -> Result<OcrWordBox> {
    let object = value
        .as_object()
        .ok_or_else(|| MetaStoreError::invalid_value("ocr_page_cache.word_box"))?;
    let text = object
        .get("text")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| MetaStoreError::invalid_value("ocr_page_cache.word_box.text"))?;
    let left = read_json_u32(object.get("left"), "ocr_page_cache.word_box.left")?;
    let top = read_json_u32(object.get("top"), "ocr_page_cache.word_box.top")?;
    let width = read_json_u32(object.get("width"), "ocr_page_cache.word_box.width")?;
    let height = read_json_u32(object.get("height"), "ocr_page_cache.word_box.height")?;
    let confidence = object
        .get("confidence")
        .and_then(serde_json::Value::as_f64)
        .ok_or_else(|| MetaStoreError::invalid_value("ocr_page_cache.word_box.confidence"))?
        as f32;
    OcrWordBox::new(text, left, top, width, height, confidence)
}

fn read_json_u32(value: Option<&serde_json::Value>, field: &'static str) -> Result<u32> {
    let value = value
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| MetaStoreError::invalid_value(field))?;
    u32::try_from(value).map_err(|_| MetaStoreError::invalid_value(field))
}

fn entity_mention_raw_value_for_storage(mention: &EntityMention) -> &str {
    match mention.entity_type {
        EntityType::Email => "<redacted:email>",
        EntityType::Phone => "<redacted:phone>",
        EntityType::WeChat => "<redacted:wechat>",
        _ => mention.raw_value.as_str(),
    }
}

fn entity_mention_normalized_value_for_storage(mention: &EntityMention) -> Option<&str> {
    match mention.entity_type {
        EntityType::Email | EntityType::Phone | EntityType::WeChat => None,
        _ => mention.normalized_value.as_deref(),
    }
}

fn read_string(row: &Row<'_>, index: usize) -> Result<String> {
    row.get(index).map_err(MetaStoreError::storage)
}

fn read_optional_string(row: &Row<'_>, index: usize) -> Result<Option<String>> {
    row.get(index).map_err(MetaStoreError::storage)
}

fn read_i64(row: &Row<'_>, index: usize) -> Result<i64> {
    row.get(index).map_err(MetaStoreError::storage)
}

fn read_optional_i64(row: &Row<'_>, index: usize) -> Result<Option<i64>> {
    row.get(index).map_err(MetaStoreError::storage)
}

fn read_optional_f64(row: &Row<'_>, index: usize) -> Result<Option<f64>> {
    row.get(index).map_err(MetaStoreError::storage)
}

fn read_optional_timestamp(row: &Row<'_>, index: usize) -> Result<Option<UnixTimestamp>> {
    Ok(read_optional_i64(row, index)?.map(UnixTimestamp::from_unix_seconds))
}

fn read_id<T>(row: &Row<'_>, index: usize, field: &'static str) -> Result<T>
where
    T: FromStr,
{
    let value = read_string(row, index)?;
    T::from_str(&value).map_err(|_| MetaStoreError::invalid_value(field))
}

fn read_optional_id<T>(row: &Row<'_>, index: usize, field: &'static str) -> Result<Option<T>>
where
    T: FromStr,
{
    read_optional_string(row, index)?
        .map(|value| T::from_str(&value).map_err(|_| MetaStoreError::invalid_value(field)))
        .transpose()
}

fn u64_to_i64(value: u64, field: &'static str) -> Result<i64> {
    i64::try_from(value).map_err(|_| MetaStoreError::invalid_value(field))
}

fn usize_to_i64(value: usize, field: &'static str) -> Result<i64> {
    i64::try_from(value).map_err(|_| MetaStoreError::invalid_value(field))
}

fn u32_to_i64(value: u32) -> i64 {
    i64::from(value)
}

fn i64_to_u64(value: i64, field: &'static str) -> Result<u64> {
    u64::try_from(value).map_err(|_| MetaStoreError::invalid_value(field))
}

fn i64_to_u32(value: i64, field: &'static str) -> Result<u32> {
    u32::try_from(value).map_err(|_| MetaStoreError::invalid_value(field))
}

fn i64_to_usize(value: i64, field: &'static str) -> Result<usize> {
    usize::try_from(value).map_err(|_| MetaStoreError::invalid_value(field))
}

fn bool_to_i64(value: bool) -> i64 {
    if value {
        1
    } else {
        0
    }
}

fn i64_to_bool(value: i64, field: &'static str) -> Result<bool> {
    match value {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(MetaStoreError::invalid_value(field)),
    }
}

fn file_extension_to_storage(extension: &FileExtension) -> String {
    match extension {
        FileExtension::Docx => "docx".to_string(),
        FileExtension::Pdf => "pdf".to_string(),
        FileExtension::Doc => "doc".to_string(),
        FileExtension::Txt => "txt".to_string(),
        FileExtension::Image => "image".to_string(),
        FileExtension::Other(value) => format!("other:{value}"),
    }
}

fn file_extension_from_storage(value: &str) -> FileExtension {
    match value {
        "docx" => FileExtension::Docx,
        "pdf" => FileExtension::Pdf,
        "doc" => FileExtension::Doc,
        "txt" => FileExtension::Txt,
        "image" => FileExtension::Image,
        _ => FileExtension::Other(value.strip_prefix("other:").unwrap_or(value).to_string()),
    }
}

fn document_status_to_storage(status: DocumentStatus) -> &'static str {
    match status {
        DocumentStatus::Discovered => "discovered",
        DocumentStatus::Fingerprinted => "fingerprinted",
        DocumentStatus::ParseQueued => "parse_queued",
        DocumentStatus::ParseRunning => "parse_running",
        DocumentStatus::TextExtracted => "text_extracted",
        DocumentStatus::OcrRequired => "ocr_required",
        DocumentStatus::OcrRunning => "ocr_running",
        DocumentStatus::OcrDone => "ocr_done",
        DocumentStatus::TextCleaned => "text_cleaned",
        DocumentStatus::FieldsExtracted => "fields_extracted",
        DocumentStatus::EmbeddingDone => "embedding_done",
        DocumentStatus::IndexedPartial => "indexed_partial",
        DocumentStatus::Searchable => "searchable",
        DocumentStatus::FailedRetryable => "failed_retryable",
        DocumentStatus::FailedPermanent => "failed_permanent",
        DocumentStatus::Deleted => "deleted",
    }
}

fn document_status_from_storage(value: &str) -> Result<DocumentStatus> {
    match value {
        "discovered" => Ok(DocumentStatus::Discovered),
        "fingerprinted" => Ok(DocumentStatus::Fingerprinted),
        "parse_queued" => Ok(DocumentStatus::ParseQueued),
        "parse_running" => Ok(DocumentStatus::ParseRunning),
        "text_extracted" => Ok(DocumentStatus::TextExtracted),
        "ocr_required" => Ok(DocumentStatus::OcrRequired),
        "ocr_running" => Ok(DocumentStatus::OcrRunning),
        "ocr_done" => Ok(DocumentStatus::OcrDone),
        "text_cleaned" => Ok(DocumentStatus::TextCleaned),
        "fields_extracted" => Ok(DocumentStatus::FieldsExtracted),
        "embedding_done" => Ok(DocumentStatus::EmbeddingDone),
        "indexed_partial" => Ok(DocumentStatus::IndexedPartial),
        "searchable" => Ok(DocumentStatus::Searchable),
        "failed_retryable" => Ok(DocumentStatus::FailedRetryable),
        "failed_permanent" => Ok(DocumentStatus::FailedPermanent),
        "deleted" => Ok(DocumentStatus::Deleted),
        _ => Err(MetaStoreError::invalid_value("document.status")),
    }
}

fn resume_visibility_to_storage(visibility: ResumeVisibility) -> &'static str {
    match visibility {
        ResumeVisibility::Searchable => "searchable",
        ResumeVisibility::Partial => "partial",
        ResumeVisibility::Hidden => "hidden",
    }
}

fn resume_visibility_from_storage(value: &str) -> Result<ResumeVisibility> {
    match value {
        "searchable" => Ok(ResumeVisibility::Searchable),
        "partial" => Ok(ResumeVisibility::Partial),
        "hidden" => Ok(ResumeVisibility::Hidden),
        _ => Err(MetaStoreError::invalid_value("resume_version.visibility")),
    }
}

fn entity_type_to_storage(entity_type: &EntityType) -> String {
    match entity_type {
        EntityType::Name => "name".to_string(),
        EntityType::Email => "email".to_string(),
        EntityType::Phone => "phone".to_string(),
        EntityType::WeChat => "wechat".to_string(),
        EntityType::School => "school".to_string(),
        EntityType::SchoolTier => "school_tier".to_string(),
        EntityType::Degree => "degree".to_string(),
        EntityType::Major => "major".to_string(),
        EntityType::Company => "company".to_string(),
        EntityType::Title => "title".to_string(),
        EntityType::Education => "education".to_string(),
        EntityType::Skills => "skills".to_string(),
        EntityType::Skill => "skill".to_string(),
        EntityType::Certificate => "certificate".to_string(),
        EntityType::Date => "date".to_string(),
        EntityType::DateRange => "date_range".to_string(),
        EntityType::YearsExperience => "years_experience".to_string(),
        EntityType::Location => "location".to_string(),
        EntityType::Other(value) => format!("other:{value}"),
    }
}

fn entity_type_from_storage(value: &str) -> Result<EntityType> {
    match value {
        "name" => Ok(EntityType::Name),
        "email" => Ok(EntityType::Email),
        "phone" => Ok(EntityType::Phone),
        "wechat" => Ok(EntityType::WeChat),
        "school" => Ok(EntityType::School),
        "school_tier" => Ok(EntityType::SchoolTier),
        "degree" => Ok(EntityType::Degree),
        "major" => Ok(EntityType::Major),
        "company" => Ok(EntityType::Company),
        "title" => Ok(EntityType::Title),
        "education" => Ok(EntityType::Education),
        "skills" => Ok(EntityType::Skills),
        "skill" => Ok(EntityType::Skill),
        "certificate" => Ok(EntityType::Certificate),
        "date" => Ok(EntityType::Date),
        "date_range" => Ok(EntityType::DateRange),
        "years_experience" => Ok(EntityType::YearsExperience),
        "location" => Ok(EntityType::Location),
        _ => value
            .strip_prefix("other:")
            .map(|value| EntityType::Other(value.to_string()))
            .ok_or_else(|| MetaStoreError::invalid_value("entity_mention.entity_type")),
    }
}

fn ingest_job_kind_to_storage(kind: IngestJobKind) -> &'static str {
    match kind {
        IngestJobKind::DiscoverDocument => "discover_document",
        IngestJobKind::FingerprintDocument => "fingerprint_document",
        IngestJobKind::ParseDocument => "parse_document",
        IngestJobKind::OcrDocument => "ocr_document",
        IngestJobKind::CleanText => "clean_text",
        IngestJobKind::ExtractFields => "extract_fields",
        IngestJobKind::UpdateIndex => "update_index",
    }
}

fn ingest_job_kind_from_storage(value: &str) -> Result<IngestJobKind> {
    match value {
        "discover_document" => Ok(IngestJobKind::DiscoverDocument),
        "fingerprint_document" => Ok(IngestJobKind::FingerprintDocument),
        "parse_document" => Ok(IngestJobKind::ParseDocument),
        "ocr_document" => Ok(IngestJobKind::OcrDocument),
        "clean_text" => Ok(IngestJobKind::CleanText),
        "extract_fields" => Ok(IngestJobKind::ExtractFields),
        "update_index" => Ok(IngestJobKind::UpdateIndex),
        _ => Err(MetaStoreError::invalid_value("ingest_job.kind")),
    }
}

fn ingest_job_status_to_storage(status: IngestJobStatus) -> &'static str {
    match status {
        IngestJobStatus::Queued => "queued",
        IngestJobStatus::Running => "running",
        IngestJobStatus::Interrupted => "interrupted",
        IngestJobStatus::Completed => "completed",
        IngestJobStatus::FailedRetryable => "failed_retryable",
        IngestJobStatus::FailedPermanent => "failed_permanent",
    }
}

fn ingest_job_status_from_storage(value: &str) -> Result<IngestJobStatus> {
    match value {
        "queued" => Ok(IngestJobStatus::Queued),
        "running" => Ok(IngestJobStatus::Running),
        "interrupted" => Ok(IngestJobStatus::Interrupted),
        "completed" => Ok(IngestJobStatus::Completed),
        "failed_retryable" => Ok(IngestJobStatus::FailedRetryable),
        "failed_permanent" => Ok(IngestJobStatus::FailedPermanent),
        _ => Err(MetaStoreError::invalid_value("ingest_job.status")),
    }
}

fn ingest_job_failure_kind_to_storage(kind: IngestJobFailureKind) -> &'static str {
    match kind {
        IngestJobFailureKind::OcrPageBudgetExceeded => "ocr_page_budget_exceeded",
    }
}

fn ingest_job_failure_kind_from_storage(value: &str) -> Result<IngestJobFailureKind> {
    match value {
        "ocr_page_budget_exceeded" => Ok(IngestJobFailureKind::OcrPageBudgetExceeded),
        _ => Err(MetaStoreError::invalid_value("ingest_job.failure_kind")),
    }
}

fn ocr_page_cache_status_to_storage(status: OcrPageCacheStatus) -> &'static str {
    match status {
        OcrPageCacheStatus::Succeeded => "succeeded",
        OcrPageCacheStatus::FailedRetryable => "failed_retryable",
        OcrPageCacheStatus::FailedPermanent => "failed_permanent",
    }
}

fn ocr_page_cache_status_from_storage(value: &str) -> Result<OcrPageCacheStatus> {
    match value {
        "succeeded" => Ok(OcrPageCacheStatus::Succeeded),
        "failed_retryable" => Ok(OcrPageCacheStatus::FailedRetryable),
        "failed_permanent" => Ok(OcrPageCacheStatus::FailedPermanent),
        _ => Err(MetaStoreError::invalid_value("ocr_page_cache.status")),
    }
}

fn worker_task_kind_to_storage(task: WorkerTaskKind) -> &'static str {
    match task {
        WorkerTaskKind::Ocr => "ocr",
    }
}

fn worker_task_kind_from_storage(value: &str) -> Result<WorkerTaskKind> {
    match value {
        "ocr" => Ok(WorkerTaskKind::Ocr),
        _ => Err(MetaStoreError::invalid_value(
            "worker_task_control.task_kind",
        )),
    }
}

fn import_root_kind_to_storage(kind: ImportRootKind) -> &'static str {
    match kind {
        ImportRootKind::Explicit => "explicit",
        ImportRootKind::Preset => "preset",
    }
}

fn import_root_kind_from_storage(value: &str) -> Result<ImportRootKind> {
    match value {
        "explicit" => Ok(ImportRootKind::Explicit),
        "preset" => Ok(ImportRootKind::Preset),
        _ => Err(MetaStoreError::invalid_value("import_scan_scope.root_kind")),
    }
}

fn import_root_preset_to_storage(preset: ImportRootPreset) -> &'static str {
    match preset {
        ImportRootPreset::LocalDiscovery => "local_discovery",
    }
}

fn import_root_preset_from_storage(value: &str) -> Result<ImportRootPreset> {
    match value {
        "local_discovery" => Ok(ImportRootPreset::LocalDiscovery),
        _ => Err(MetaStoreError::invalid_value(
            "import_scan_scope.root_preset",
        )),
    }
}

fn import_scan_profile_to_storage(profile: ImportScanProfile) -> &'static str {
    match profile {
        ImportScanProfile::Explicit => "explicit",
        ImportScanProfile::Discovery => "discovery",
    }
}

fn import_scan_profile_from_storage(value: &str) -> Result<ImportScanProfile> {
    match value {
        "explicit" => Ok(ImportScanProfile::Explicit),
        "discovery" => Ok(ImportScanProfile::Discovery),
        _ => Err(MetaStoreError::invalid_value(
            "import_scan_scope.scan_profile",
        )),
    }
}

fn import_scan_budget_kind_to_storage(kind: ImportScanBudgetKind) -> &'static str {
    match kind {
        ImportScanBudgetKind::Files => "files",
    }
}

fn import_scan_budget_kind_from_storage(value: &str) -> Result<ImportScanBudgetKind> {
    match value {
        "files" => Ok(ImportScanBudgetKind::Files),
        _ => Err(MetaStoreError::invalid_value(
            "import_scan_scope.scan_budget_kind",
        )),
    }
}

fn import_scan_error_kind_to_storage(kind: ImportScanErrorKind) -> &'static str {
    match kind {
        ImportScanErrorKind::PermissionDenied => "permission_denied",
        ImportScanErrorKind::SourceUnavailable => "source_unavailable",
        ImportScanErrorKind::LockedOrUnreadable => "locked_or_unreadable",
        ImportScanErrorKind::Io => "io",
    }
}

fn import_scan_error_kind_from_storage(value: &str) -> Result<ImportScanErrorKind> {
    match value {
        "permission_denied" => Ok(ImportScanErrorKind::PermissionDenied),
        "source_unavailable" => Ok(ImportScanErrorKind::SourceUnavailable),
        "locked_or_unreadable" => Ok(ImportScanErrorKind::LockedOrUnreadable),
        "io" => Ok(ImportScanErrorKind::Io),
        _ => Err(MetaStoreError::invalid_value("import_scan_error.kind")),
    }
}

fn import_scan_error_operation_to_storage(operation: ImportScanErrorOperation) -> &'static str {
    match operation {
        ImportScanErrorOperation::NormalizePath => "normalize_path",
        ImportScanErrorOperation::ReadDirectory => "read_directory",
        ImportScanErrorOperation::ReadMetadata => "read_metadata",
        ImportScanErrorOperation::Fingerprint => "fingerprint",
    }
}

fn import_scan_error_operation_from_storage(value: &str) -> Result<ImportScanErrorOperation> {
    match value {
        "normalize_path" => Ok(ImportScanErrorOperation::NormalizePath),
        "read_directory" => Ok(ImportScanErrorOperation::ReadDirectory),
        "read_metadata" => Ok(ImportScanErrorOperation::ReadMetadata),
        "fingerprint" => Ok(ImportScanErrorOperation::Fingerprint),
        _ => Err(MetaStoreError::invalid_value("import_scan_error.operation")),
    }
}

fn job_status_transition_allowed(current: IngestJobStatus, next: IngestJobStatus) -> bool {
    match current {
        IngestJobStatus::Queued => matches!(
            next,
            IngestJobStatus::Queued | IngestJobStatus::Running | IngestJobStatus::Interrupted
        ),
        IngestJobStatus::Running => matches!(
            next,
            IngestJobStatus::Running
                | IngestJobStatus::Interrupted
                | IngestJobStatus::Completed
                | IngestJobStatus::FailedRetryable
                | IngestJobStatus::FailedPermanent
        ),
        IngestJobStatus::Interrupted => matches!(
            next,
            IngestJobStatus::Interrupted
                | IngestJobStatus::Running
                | IngestJobStatus::FailedPermanent
        ),
        IngestJobStatus::FailedRetryable => matches!(
            next,
            IngestJobStatus::FailedRetryable
                | IngestJobStatus::Running
                | IngestJobStatus::FailedPermanent
        ),
        IngestJobStatus::Completed => matches!(next, IngestJobStatus::Completed),
        IngestJobStatus::FailedPermanent => matches!(next, IngestJobStatus::FailedPermanent),
    }
}

fn import_task_status_transition_allowed(
    current: ImportTaskStatus,
    next: ImportTaskStatus,
) -> bool {
    match current {
        ImportTaskStatus::Queued => matches!(next, ImportTaskStatus::Running),
        ImportTaskStatus::Running => matches!(
            next,
            ImportTaskStatus::Completed
                | ImportTaskStatus::FailedRetryable
                | ImportTaskStatus::FailedPermanent
        ),
        ImportTaskStatus::FailedRetryable => matches!(next, ImportTaskStatus::Running),
        ImportTaskStatus::Completed | ImportTaskStatus::FailedPermanent => false,
    }
}

fn next_import_task_state(
    current: &ImportTask,
    status: ImportTaskStatus,
    updated_at: UnixTimestamp,
) -> ImportTask {
    let mut next = current.clone();
    next.status = status;
    next.updated_at = updated_at;
    match status {
        ImportTaskStatus::Running => {
            next.started_at = Some(updated_at);
            next.finished_at = None;
        }
        ImportTaskStatus::Completed
        | ImportTaskStatus::FailedRetryable
        | ImportTaskStatus::FailedPermanent => {
            if next.started_at.is_none() {
                next.started_at = Some(updated_at);
            }
            next.finished_at = Some(updated_at);
        }
        ImportTaskStatus::Queued => {}
    }
    next
}

fn index_state_status_to_storage(status: IndexStateStatus) -> &'static str {
    match status {
        IndexStateStatus::Empty => "empty",
        IndexStateStatus::Building => "building",
        IndexStateStatus::Ready => "ready",
        IndexStateStatus::Stale => "stale",
    }
}

fn index_state_status_from_storage(value: &str) -> Result<IndexStateStatus> {
    match value {
        "empty" => Ok(IndexStateStatus::Empty),
        "building" => Ok(IndexStateStatus::Building),
        "ready" => Ok(IndexStateStatus::Ready),
        "stale" => Ok(IndexStateStatus::Stale),
        _ => Err(MetaStoreError::invalid_value("index_state.status")),
    }
}

fn import_task_status_to_storage(status: ImportTaskStatus) -> &'static str {
    match status {
        ImportTaskStatus::Queued => "queued",
        ImportTaskStatus::Running => "running",
        ImportTaskStatus::Completed => "completed",
        ImportTaskStatus::FailedRetryable => "failed_retryable",
        ImportTaskStatus::FailedPermanent => "failed_permanent",
    }
}

fn import_task_status_from_storage(value: &str) -> Result<ImportTaskStatus> {
    match value {
        "queued" => Ok(ImportTaskStatus::Queued),
        "running" => Ok(ImportTaskStatus::Running),
        "completed" => Ok(ImportTaskStatus::Completed),
        "failed_retryable" => Ok(ImportTaskStatus::FailedRetryable),
        "failed_permanent" => Ok(ImportTaskStatus::FailedPermanent),
        _ => Err(MetaStoreError::invalid_value("import_task.status")),
    }
}

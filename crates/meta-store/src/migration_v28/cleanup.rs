use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

use tempfile::Builder;

use crate::{
    active_store_manifest::{
        remove_owner_file_if_exists, sync_parent_directory, validate_owner_regular_metadata,
        validate_store_file_name, validate_store_id_digest, ActiveStoreManifest,
    },
    schema_v27, schema_v28, MetaStoreError, Result,
};

pub(super) const ATTEMPT_JOURNAL_FILE: &str = "metadata-v28-migration-attempt.v1";
pub(super) const ATTEMPT_TEMP_PREFIX: &str = ".metadata-v28-migration-attempt.v1.tmp-";
const ATTEMPT_JOURNAL_SCHEMA: &str = "resume-ir.metadata-v28-migration-attempt.v1";
const ATTEMPT_JOURNAL_MAX_BYTES: u64 = 2_048;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct PreviousStore {
    pub(super) file_name: String,
    pub(super) schema_version: u32,
    pub(super) store_id_digest: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct MigrationAttempt {
    pub(super) expected_manifest: Option<ActiveStoreManifest>,
    pub(super) previous: Option<PreviousStore>,
    pub(super) target: ActiveStoreManifest,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum MigrationCleanupFailpoint {
    None,
    #[cfg(test)]
    AfterArtifactRemoval,
    #[cfg(test)]
    AfterArtifactSync,
    #[cfg(test)]
    AfterJournalRemoval,
}

pub(super) fn write_migration_attempt(data_dir: &Path, attempt: &MigrationAttempt) -> Result<()> {
    validate_attempt(attempt)?;
    let expected = manifest_fields(attempt.expected_manifest.as_ref());
    let previous = previous_fields(attempt.previous.as_ref());
    let bytes = format!(
        "{ATTEMPT_JOURNAL_SCHEMA}\nexpected_file={}\nexpected_schema={}\nexpected_digest={}\nprevious_file={}\nprevious_schema={}\nprevious_digest={}\ntarget_file={}\ntarget_schema={}\ntarget_digest={}",
        expected.0,
        expected.1,
        expected.2,
        previous.0,
        previous.1,
        previous.2,
        attempt.target.file_name,
        attempt.target.schema_version,
        attempt.target.store_id_digest,
    );
    let path = data_dir.join(ATTEMPT_JOURNAL_FILE);
    let mut temporary = Builder::new()
        .prefix(ATTEMPT_TEMP_PREFIX)
        .tempfile_in(data_dir)
        .map_err(MetaStoreError::io_storage)?;
    temporary
        .write_all(bytes.as_bytes())
        .and_then(|_| temporary.write_all(b"\n"))
        .map_err(MetaStoreError::io_storage)?;
    temporary
        .as_file()
        .sync_all()
        .map_err(MetaStoreError::io_storage)?;
    crate::restrict_private_file_permissions(temporary.path())?;
    temporary
        .persist_noclobber(&path)
        .map_err(|error| MetaStoreError::io_storage(error.error))?;
    sync_parent_directory(data_dir)
}

/// Resolves the durable attempt journal against the authoritative manifest.
///
/// If the target is not published, only its namespaced artifacts are removed.
/// If the target is published, only the recorded predecessor is retired. Any
/// third manifest value is an ownership conflict and fails closed.
pub(super) fn recover_migration_attempt(
    data_dir: &Path,
    current_manifest: Option<&ActiveStoreManifest>,
) -> Result<()> {
    recover_migration_attempt_inner(data_dir, current_manifest, MigrationCleanupFailpoint::None)
}

#[cfg(test)]
pub(super) fn recover_migration_attempt_at_cleanup_cut(
    data_dir: &Path,
    current_manifest: Option<&ActiveStoreManifest>,
    failpoint: MigrationCleanupFailpoint,
) -> Result<()> {
    recover_migration_attempt_inner(data_dir, current_manifest, failpoint)
}

fn recover_migration_attempt_inner(
    data_dir: &Path,
    current_manifest: Option<&ActiveStoreManifest>,
    _failpoint: MigrationCleanupFailpoint,
) -> Result<()> {
    remove_attempt_temporaries(data_dir)?;
    let path = data_dir.join(ATTEMPT_JOURNAL_FILE);
    let Some(attempt) = read_optional_attempt(&path)? else {
        return Ok(());
    };
    if current_manifest == Some(&attempt.target) {
        if let Some(previous) = attempt.previous.as_ref() {
            remove_store_artifacts(data_dir, &previous.file_name)?;
        }
    } else if current_manifest == attempt.expected_manifest.as_ref() {
        remove_store_artifacts(data_dir, &attempt.target.file_name)?;
    } else {
        return Err(MetaStoreError::storage_invariant());
    }
    #[cfg(test)]
    if _failpoint == MigrationCleanupFailpoint::AfterArtifactRemoval {
        return Err(MetaStoreError::storage_invariant());
    }
    sync_parent_directory(data_dir)?;
    #[cfg(test)]
    if _failpoint == MigrationCleanupFailpoint::AfterArtifactSync {
        return Err(MetaStoreError::storage_invariant());
    }
    remove_owner_file_if_exists(&path)?;
    #[cfg(test)]
    if _failpoint == MigrationCleanupFailpoint::AfterJournalRemoval {
        return Err(MetaStoreError::storage_invariant());
    }
    sync_parent_directory(data_dir)
}

/// Returns the recorded predecessor only when the supplied v28 manifest is the
/// durably published target. The caller must fence legacy writers before
/// invoking `recover_migration_attempt`, which retires that predecessor.
pub(super) fn published_previous_store(
    data_dir: &Path,
    current_manifest: &ActiveStoreManifest,
) -> Result<Option<PreviousStore>> {
    let path = data_dir.join(ATTEMPT_JOURNAL_FILE);
    let Some(attempt) = read_optional_attempt(&path)? else {
        return Ok(None);
    };
    if attempt.target != *current_manifest {
        return Err(MetaStoreError::storage_invariant());
    }
    Ok(attempt.previous)
}

pub(super) fn pending_migration_attempt(data_dir: &Path) -> Result<Option<MigrationAttempt>> {
    read_optional_attempt(&data_dir.join(ATTEMPT_JOURNAL_FILE))
}

fn read_optional_attempt(path: &Path) -> Result<Option<MigrationAttempt>> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(MetaStoreError::io_storage(error)),
    };
    validate_owner_regular_metadata(&metadata)?;
    if metadata.len() > ATTEMPT_JOURNAL_MAX_BYTES {
        return Err(MetaStoreError::invalid_value(
            "metadata.v28_migration_attempt",
        ));
    }
    let value = fs::read_to_string(path).map_err(MetaStoreError::io_storage)?;
    let mut lines = value.lines();
    if lines.next() != Some(ATTEMPT_JOURNAL_SCHEMA) {
        return Err(MetaStoreError::invalid_value(
            "metadata.v28_migration_attempt",
        ));
    }
    let expected_file = required_value(lines.next(), "expected_file")?;
    let expected_schema = parse_schema(lines.next(), "expected_schema")?;
    let expected_digest = required_value(lines.next(), "expected_digest")?;
    let previous_file = required_value(lines.next(), "previous_file")?;
    let previous_schema = parse_schema(lines.next(), "previous_schema")?;
    let previous_digest = required_value(lines.next(), "previous_digest")?;
    let attempt = MigrationAttempt {
        expected_manifest: parse_manifest(expected_file, expected_schema, expected_digest)?,
        previous: parse_previous(previous_file, previous_schema, previous_digest)?,
        target: ActiveStoreManifest {
            file_name: required_value(lines.next(), "target_file")?.to_string(),
            schema_version: parse_schema(lines.next(), "target_schema")?,
            store_id_digest: required_value(lines.next(), "target_digest")?.to_string(),
        },
    };
    if lines.next().is_some() {
        return Err(MetaStoreError::invalid_value(
            "metadata.v28_migration_attempt",
        ));
    }
    validate_attempt(&attempt)?;
    Ok(Some(attempt))
}

fn validate_attempt(attempt: &MigrationAttempt) -> Result<()> {
    validate_target(&attempt.target)?;
    if let Some(expected) = attempt.expected_manifest.as_ref() {
        validate_expected(expected)?;
    }
    if let Some(previous) = attempt.previous.as_ref() {
        validate_previous(previous)?;
        if previous.file_name == attempt.target.file_name {
            return Err(MetaStoreError::storage_invariant());
        }
    }
    match (
        attempt.expected_manifest.as_ref(),
        attempt.previous.as_ref(),
    ) {
        (Some(expected), Some(previous))
            if expected.file_name == previous.file_name
                && expected.schema_version == previous.schema_version
                && Some(expected.store_id_digest.as_str())
                    == previous.store_id_digest.as_deref() =>
        {
            Ok(())
        }
        (None, _) => Ok(()),
        _ => Err(MetaStoreError::storage_invariant()),
    }
}

fn validate_previous(previous: &PreviousStore) -> Result<()> {
    validate_store_file_name(&previous.file_name)?;
    if let Some(digest) = previous.store_id_digest.as_deref() {
        validate_store_id_digest(digest)?;
    }
    let valid = match previous.schema_version {
        0 | 26 => {
            previous.file_name == crate::METADATA_STORE_FILE && previous.store_id_digest.is_none()
        }
        schema_v27::VERSION => {
            (previous.file_name == crate::METADATA_STORE_FILE
                || previous.file_name.starts_with("metadata-v27-"))
                && previous.store_id_digest.is_some()
        }
        _ => false,
    };
    if !valid {
        return Err(MetaStoreError::invalid_value(
            "metadata.v28_migration_attempt",
        ));
    }
    Ok(())
}

fn validate_expected(expected: &ActiveStoreManifest) -> Result<()> {
    validate_store_file_name(&expected.file_name)?;
    validate_store_id_digest(&expected.store_id_digest)?;
    if expected.schema_version != schema_v27::VERSION {
        return Err(MetaStoreError::invalid_value(
            "metadata.v28_migration_attempt",
        ));
    }
    Ok(())
}

fn validate_target(target: &ActiveStoreManifest) -> Result<()> {
    validate_store_file_name(&target.file_name)?;
    validate_store_id_digest(&target.store_id_digest)?;
    if target.schema_version != schema_v28::VERSION
        || !target.file_name.starts_with("metadata-v28-")
    {
        return Err(MetaStoreError::invalid_value(
            "metadata.v28_migration_attempt",
        ));
    }
    Ok(())
}

fn parse_manifest(
    file_name: &str,
    schema_version: u32,
    digest: &str,
) -> Result<Option<ActiveStoreManifest>> {
    if (file_name, schema_version, digest) == ("none", 0, "none") {
        return Ok(None);
    }
    Ok(Some(ActiveStoreManifest {
        file_name: file_name.to_string(),
        schema_version,
        store_id_digest: digest.to_string(),
    }))
}

fn parse_previous(
    file_name: &str,
    schema_version: u32,
    digest: &str,
) -> Result<Option<PreviousStore>> {
    if (file_name, schema_version, digest) == ("none", 0, "none") {
        return Ok(None);
    }
    Ok(Some(PreviousStore {
        file_name: file_name.to_string(),
        schema_version,
        store_id_digest: (digest != "none").then(|| digest.to_string()),
    }))
}

fn manifest_fields(manifest: Option<&ActiveStoreManifest>) -> (&str, u32, &str) {
    manifest.map_or(("none", 0, "none"), |manifest| {
        (
            manifest.file_name.as_str(),
            manifest.schema_version,
            manifest.store_id_digest.as_str(),
        )
    })
}

fn previous_fields(previous: Option<&PreviousStore>) -> (&str, u32, &str) {
    previous.map_or(("none", 0, "none"), |previous| {
        (
            previous.file_name.as_str(),
            previous.schema_version,
            previous.store_id_digest.as_deref().unwrap_or("none"),
        )
    })
}

fn parse_schema(line: Option<&str>, key: &str) -> Result<u32> {
    required_value(line, key)?
        .parse::<u32>()
        .map_err(|_| MetaStoreError::invalid_value("metadata.v28_migration_attempt"))
}

fn required_value<'a>(line: Option<&'a str>, key: &str) -> Result<&'a str> {
    line.and_then(|line| line.strip_prefix(key))
        .and_then(|value| value.strip_prefix('='))
        .filter(|value| !value.is_empty())
        .ok_or_else(|| MetaStoreError::invalid_value("metadata.v28_migration_attempt"))
}

fn remove_attempt_temporaries(data_dir: &Path) -> Result<()> {
    let mut removed = false;
    for entry in fs::read_dir(data_dir).map_err(MetaStoreError::io_storage)? {
        let entry = entry.map_err(MetaStoreError::io_storage)?;
        let Some(file_name) = entry.file_name().to_str().map(str::to_string) else {
            continue;
        };
        if !file_name.starts_with(ATTEMPT_TEMP_PREFIX) {
            continue;
        }
        remove_owner_file_if_exists(&entry.path())?;
        removed = true;
    }
    if removed {
        sync_parent_directory(data_dir)?;
    }
    Ok(())
}

fn remove_store_artifacts(data_dir: &Path, file_name: &str) -> Result<()> {
    validate_store_file_name(file_name)?;
    let main = data_dir.join(file_name);
    for path in [
        main.clone(),
        sidecar_path(&main, "-journal"),
        sidecar_path(&main, "-wal"),
        sidecar_path(&main, "-shm"),
    ] {
        remove_owner_file_if_exists(&path)?;
    }
    Ok(())
}

fn sidecar_path(path: &Path, suffix: &str) -> PathBuf {
    let mut value = path.as_os_str().to_owned();
    value.push(suffix);
    PathBuf::from(value)
}

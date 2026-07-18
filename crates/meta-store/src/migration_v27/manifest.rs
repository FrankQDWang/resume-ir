use std::{fs, path::Path};

use super::{
    owner_regular_file_exists, sync_parent_directory, validate_owner_regular_metadata,
    MigrationFailpoint,
};
use crate::{
    encode_hex, restrict_private_file_permissions, schema_v27, MetaStoreError, Result,
    METADATA_STORE_FILE,
};

pub(super) const MANIFEST_FILE: &str = "metadata-active.v1";
const MANIFEST_SCHEMA: &str = "resume-ir.metadata-active.v1";
pub(super) const MANIFEST_MAX_BYTES: u64 = 512;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct ActiveStoreManifest {
    pub(super) file_name: String,
    pub(super) store_id_digest: String,
}

/// Atomically publishes the only active-store pointer. The rename is the
/// commit point; failures after it are recovered by validating this manifest
/// on the next locked opener rather than rolling back a visible generation.
pub(super) fn publish_manifest(
    data_dir: &Path,
    file_name: &str,
    store_id_digest: &str,
    failpoint: MigrationFailpoint,
) -> Result<()> {
    validate_store_file_name(file_name)?;
    validate_store_id_digest(store_id_digest)?;
    let manifest_path = data_dir.join(MANIFEST_FILE);
    if owner_regular_file_exists(&manifest_path)? {
        let existing = read_manifest(&manifest_path)?;
        if existing.file_name == file_name && existing.store_id_digest == store_id_digest {
            return Ok(());
        }
        return Err(MetaStoreError::storage_invariant());
    }
    let mut suffix = [0_u8; 8];
    getrandom::getrandom(&mut suffix).map_err(|_| MetaStoreError::random())?;
    let temp_path = data_dir.join(format!(".{MANIFEST_FILE}.tmp-{}", encode_hex(&suffix)));
    let bytes = format!(
        "{MANIFEST_SCHEMA}\nfile={file_name}\nschema={}\ndigest={store_id_digest}",
        schema_v27::VERSION
    );
    crate::write_new_private_file(&temp_path, bytes.as_bytes())
        .map_err(MetaStoreError::io_storage)?;
    if let Err(error) = fs::rename(&temp_path, &manifest_path) {
        let _ = fs::remove_file(&temp_path);
        return Err(MetaStoreError::io_storage(error));
    }
    if failpoint == MigrationFailpoint::AfterManifestRename {
        return Err(MetaStoreError::storage_invariant());
    }
    restrict_private_file_permissions(&manifest_path)?;
    sync_parent_directory(data_dir)
}

pub(super) fn read_manifest(path: &Path) -> Result<ActiveStoreManifest> {
    let metadata = fs::symlink_metadata(path).map_err(MetaStoreError::io_storage)?;
    validate_owner_regular_metadata(&metadata)?;
    if metadata.len() > MANIFEST_MAX_BYTES {
        return Err(MetaStoreError::invalid_value("metadata.active_manifest"));
    }
    let value = fs::read_to_string(path).map_err(MetaStoreError::io_storage)?;
    let mut lines = value.lines();
    if lines.next() != Some(MANIFEST_SCHEMA) {
        return Err(MetaStoreError::invalid_value("metadata.active_manifest"));
    }
    let file_name = required_manifest_value(lines.next(), "file")?.to_string();
    let schema = required_manifest_value(lines.next(), "schema")?;
    let store_id_digest = required_manifest_value(lines.next(), "digest")?.to_string();
    if schema != schema_v27::VERSION.to_string() || lines.next().is_some() {
        return Err(MetaStoreError::invalid_value("metadata.active_manifest"));
    }
    validate_store_file_name(&file_name)?;
    validate_store_id_digest(&store_id_digest)?;
    Ok(ActiveStoreManifest {
        file_name,
        store_id_digest,
    })
}

fn required_manifest_value<'a>(line: Option<&'a str>, key: &str) -> Result<&'a str> {
    line.and_then(|line| line.strip_prefix(key))
        .and_then(|value| value.strip_prefix('='))
        .filter(|value| !value.is_empty())
        .ok_or_else(|| MetaStoreError::invalid_value("metadata.active_manifest"))
}

pub(super) fn validate_store_file_name(file_name: &str) -> Result<()> {
    let valid = file_name == METADATA_STORE_FILE
        || file_name
            .strip_prefix("metadata-v27-")
            .and_then(|value| value.strip_suffix(".sqlite3"))
            .is_some_and(|token| {
                token.len() == 16
                    && token
                        .bytes()
                        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            });
    if !valid {
        return Err(MetaStoreError::invalid_value("metadata.active_store_file"));
    }
    Ok(())
}

pub(super) fn validate_store_id_digest(value: &str) -> Result<()> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(MetaStoreError::invalid_value(
            "metadata.active_store_digest",
        ));
    }
    Ok(())
}

pub(super) fn random_store_id_digest() -> Result<String> {
    let mut bytes = [0_u8; 32];
    getrandom::getrandom(&mut bytes).map_err(|_| MetaStoreError::random())?;
    Ok(encode_hex(&bytes))
}

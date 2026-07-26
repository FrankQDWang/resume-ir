use std::{fs, io::Write, path::Path};

#[cfg(windows)]
use std::fs::OpenOptions;

use tempfile::Builder;

use crate::{
    encode_hex, restrict_private_file_permissions, schema_v29, MetaStoreError, Result,
    METADATA_STORE_FILE,
};
#[cfg(any(test, feature = "migration-test-support"))]
use crate::{schema_v27, schema_v28};

pub(crate) const MANIFEST_FILE: &str = "metadata-active.v1";
const MANIFEST_SCHEMA: &str = "resume-ir.metadata-active.v1";
pub(crate) const MANIFEST_MAX_BYTES: u64 = 512;

#[cfg(windows)]
const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
#[cfg(windows)]
const FILE_FLAG_WRITE_THROUGH: u32 = 0x8000_0000;
#[cfg(windows)]
const FILE_SHARE_READ_WRITE_DELETE: u32 = 0x0000_0007;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ActiveStoreManifest {
    pub(crate) file_name: String,
    pub(crate) schema_version: u32,
    pub(crate) store_id_digest: String,
}

pub(crate) fn publish_new_active_store(
    data_dir: &Path,
    desired: &ActiveStoreManifest,
    after_rename: impl FnOnce() -> Result<()>,
) -> Result<()> {
    validate_manifest(desired)?;
    let path = data_dir.join(MANIFEST_FILE);
    if owner_regular_file_exists(&path)? {
        if read_manifest(&path)? == *desired {
            return Ok(());
        }
        return Err(MetaStoreError::storage_invariant());
    }
    persist_manifest(data_dir, &path, desired, ManifestPersistMode::NoClobber)?;
    after_rename()?;
    finish_manifest_commit(data_dir, &path)
}

#[cfg(any(test, feature = "migration-test-support"))]
pub(crate) fn replace_active_store(
    data_dir: &Path,
    expected: &ActiveStoreManifest,
    desired: &ActiveStoreManifest,
    after_rename: impl FnOnce() -> Result<()>,
) -> Result<()> {
    validate_manifest(expected)?;
    validate_manifest(desired)?;
    if expected == desired {
        return Ok(());
    }
    let path = data_dir.join(MANIFEST_FILE);
    if read_manifest(&path)? != *expected {
        return Err(MetaStoreError::storage_invariant());
    }
    persist_manifest(data_dir, &path, desired, ManifestPersistMode::Replace)?;
    after_rename()?;
    finish_manifest_commit(data_dir, &path)
}

pub(crate) fn read_manifest(path: &Path) -> Result<ActiveStoreManifest> {
    let manifest = parse_manifest(path)?;
    validate_manifest(&manifest)?;
    Ok(manifest)
}

pub(crate) fn read_manifest_schema_version(path: &Path) -> Result<u32> {
    Ok(parse_manifest(path)?.schema_version)
}

fn parse_manifest(path: &Path) -> Result<ActiveStoreManifest> {
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
    let manifest = ActiveStoreManifest {
        file_name: required_value(lines.next(), "file")?.to_string(),
        schema_version: required_value(lines.next(), "schema")?
            .parse::<u32>()
            .map_err(|_| MetaStoreError::invalid_value("metadata.active_manifest"))?,
        store_id_digest: required_value(lines.next(), "digest")?.to_string(),
    };
    if lines.next().is_some() {
        return Err(MetaStoreError::invalid_value("metadata.active_manifest"));
    }
    Ok(manifest)
}

pub(crate) fn validate_store_file_name(file_name: &str) -> Result<()> {
    let versioned = versioned_store_token(file_name, schema_v29::VERSION).is_some();
    #[cfg(any(test, feature = "migration-test-support"))]
    let versioned = versioned
        || [schema_v27::VERSION, schema_v28::VERSION]
            .into_iter()
            .any(|version| versioned_store_token(file_name, version).is_some());
    #[cfg(any(test, feature = "migration-test-support"))]
    let legacy_file_name = file_name == METADATA_STORE_FILE;
    #[cfg(not(any(test, feature = "migration-test-support")))]
    let legacy_file_name = false;
    if !legacy_file_name && !versioned {
        return Err(MetaStoreError::invalid_value("metadata.active_store_file"));
    }
    Ok(())
}

pub(crate) fn validate_store_id_digest(value: &str) -> Result<()> {
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

pub(crate) fn random_store_id_digest() -> Result<String> {
    let mut bytes = [0_u8; 32];
    getrandom::getrandom(&mut bytes).map_err(|_| MetaStoreError::random())?;
    Ok(encode_hex(&bytes))
}

pub(crate) fn owner_regular_file_exists(path: &Path) -> Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            validate_owner_regular_metadata(&metadata)?;
            Ok(true)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(MetaStoreError::io_storage(error)),
    }
}

pub(crate) fn validate_owner_regular_metadata(metadata: &fs::Metadata) -> Result<()> {
    if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
        return Err(MetaStoreError::invalid_value("metadata.owner_file"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o077 != 0 {
            return Err(MetaStoreError::invalid_value(
                "metadata.owner_file_permissions",
            ));
        }
    }
    Ok(())
}

pub(crate) fn validate_owner_directory_metadata(metadata: &fs::Metadata) -> Result<()> {
    if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
        return Err(MetaStoreError::invalid_value("metadata.owner_directory"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o077 != 0 {
            return Err(MetaStoreError::invalid_value(
                "metadata.owner_directory_permissions",
            ));
        }
    }
    Ok(())
}

#[cfg(any(test, feature = "migration-test-support"))]
pub(crate) fn remove_owner_file_if_exists(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            validate_owner_regular_metadata(&metadata)?;
            fs::remove_file(path).map_err(MetaStoreError::io_storage)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(MetaStoreError::io_storage(error)),
    }
}

#[cfg(unix)]
pub(crate) fn sync_parent_directory(data_dir: &Path) -> Result<()> {
    let directory = fs::File::open(data_dir).map_err(MetaStoreError::io_storage)?;
    directory.sync_all().map_err(MetaStoreError::io_storage)
}

#[cfg(windows)]
pub(crate) fn sync_parent_directory(data_dir: &Path) -> Result<()> {
    use std::os::windows::fs::OpenOptionsExt;

    let directory = OpenOptions::new()
        .read(true)
        .write(true)
        .share_mode(FILE_SHARE_READ_WRITE_DELETE)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_WRITE_THROUGH)
        .open(data_dir)
        .map_err(MetaStoreError::io_storage)?;
    directory.sync_all().map_err(MetaStoreError::io_storage)
}

#[cfg(not(any(unix, windows)))]
pub(crate) fn sync_parent_directory(_data_dir: &Path) -> Result<()> {
    Err(MetaStoreError::io_storage(std::io::Error::other(
        "metadata manifest durability is unsupported on this platform",
    )))
}

fn validate_manifest(manifest: &ActiveStoreManifest) -> Result<()> {
    let supported = manifest.schema_version == schema_v29::VERSION;
    #[cfg(any(test, feature = "migration-test-support"))]
    let supported = supported
        || matches!(
            manifest.schema_version,
            schema_v27::VERSION | schema_v28::VERSION
        );
    if !supported {
        return Err(MetaStoreError::invalid_value("metadata.active_manifest"));
    }
    validate_store_file_name(&manifest.file_name)?;
    validate_store_id_digest(&manifest.store_id_digest)?;
    #[cfg(any(test, feature = "migration-test-support"))]
    let legacy_file_matches =
        manifest.file_name == METADATA_STORE_FILE && manifest.schema_version == schema_v27::VERSION;
    #[cfg(not(any(test, feature = "migration-test-support")))]
    let legacy_file_matches = false;
    if (!legacy_file_matches && manifest.file_name == METADATA_STORE_FILE)
        || (manifest.file_name != METADATA_STORE_FILE
            && versioned_store_token(&manifest.file_name, manifest.schema_version).is_none())
    {
        return Err(MetaStoreError::invalid_value("metadata.active_manifest"));
    }
    Ok(())
}

fn versioned_store_token(file_name: &str, version: u32) -> Option<&str> {
    file_name
        .strip_prefix(&format!("metadata-v{version}-"))
        .and_then(|value| value.strip_suffix(".sqlite3"))
        .filter(|token| {
            token.len() == 16
                && token
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        })
}

fn required_value<'a>(line: Option<&'a str>, key: &str) -> Result<&'a str> {
    line.and_then(|line| line.strip_prefix(key))
        .and_then(|value| value.strip_prefix('='))
        .filter(|value| !value.is_empty())
        .ok_or_else(|| MetaStoreError::invalid_value("metadata.active_manifest"))
}

#[derive(Clone, Copy)]
enum ManifestPersistMode {
    NoClobber,
    #[cfg(any(test, feature = "migration-test-support"))]
    Replace,
}

fn persist_manifest(
    data_dir: &Path,
    path: &Path,
    manifest: &ActiveStoreManifest,
    mode: ManifestPersistMode,
) -> Result<()> {
    let bytes = format!(
        "{MANIFEST_SCHEMA}\nfile={}\nschema={}\ndigest={}",
        manifest.file_name, manifest.schema_version, manifest.store_id_digest
    );
    let mut temporary = Builder::new()
        .prefix(&format!(".{MANIFEST_FILE}.tmp-"))
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
    restrict_private_file_permissions(temporary.path())?;
    let result = match mode {
        ManifestPersistMode::NoClobber => temporary.persist_noclobber(path),
        #[cfg(any(test, feature = "migration-test-support"))]
        ManifestPersistMode::Replace => temporary.persist(path),
    };
    result
        .map(|_| ())
        .map_err(|error| MetaStoreError::io_storage(error.error))
}

fn finish_manifest_commit(data_dir: &Path, path: &Path) -> Result<()> {
    restrict_private_file_permissions(path)?;
    sync_parent_directory(data_dir)
}

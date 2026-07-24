use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Read;
use std::path::{Component, Path, PathBuf};

use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::ipc::OptionalRuntimeReason;

const MAX_PATH_BYTES: usize = 4096;
const MAX_MANIFEST_BYTES: u64 = 64 * 1024;
const MAX_PACK_FILES: usize = 128;
const MAX_ASSET_BYTES: u64 = 512 * 1024 * 1024;
const MAX_PACK_BYTES: u64 = 1024 * 1024 * 1024;

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PackFile {
    pub(super) role: String,
    pub(super) file: String,
    pub(super) bytes: u64,
    pub(super) sha256: String,
}

pub(super) struct ValidatedFile {
    pub(super) role: String,
    pub(super) file: String,
    pub(super) path: PathBuf,
    pub(super) bytes: u64,
    pub(super) sha256: String,
}

pub(super) fn validate_pack_files_with_cancel(
    root: &Path,
    entries: &[PackFile],
    cancelled: &dyn Fn() -> bool,
) -> Result<BTreeMap<String, ValidatedFile>, OptionalRuntimeReason> {
    let validated = validate_pack_file_entries_with_cancel(root, entries, cancelled)?;
    let mut files = BTreeMap::new();
    for entry in validated {
        ensure_not_cancelled(cancelled)?;
        if files.insert(entry.role.clone(), entry).is_some() {
            return Err(OptionalRuntimeReason::Invalid);
        }
    }
    ensure_not_cancelled(cancelled)?;
    Ok(files)
}

#[cfg(test)]
pub(super) fn validate_pack_file_entries(
    root: &Path,
    entries: &[PackFile],
) -> Result<Vec<ValidatedFile>, OptionalRuntimeReason> {
    validate_pack_file_entries_with_cancel(root, entries, &|| false)
}

pub(super) fn validate_pack_file_entries_with_cancel(
    root: &Path,
    entries: &[PackFile],
    cancelled: &dyn Fn() -> bool,
) -> Result<Vec<ValidatedFile>, OptionalRuntimeReason> {
    ensure_not_cancelled(cancelled)?;
    if entries.is_empty() || entries.len() > MAX_PACK_FILES {
        return Err(OptionalRuntimeReason::Invalid);
    }
    entries
        .iter()
        .try_fold(0_u64, |total, entry| total.checked_add(entry.bytes))
        .filter(|total| *total <= MAX_PACK_BYTES)
        .ok_or(OptionalRuntimeReason::Invalid)?;
    let mut files = Vec::with_capacity(entries.len());
    let mut names = BTreeSet::new();
    for entry in entries {
        ensure_not_cancelled(cancelled)?;
        if entry.role.is_empty()
            || entry.role.len() > 64
            || entry.bytes == 0
            || entry.bytes > MAX_ASSET_BYTES
            || !valid_digest(&entry.sha256)
            || !names.insert(entry.file.clone())
        {
            return Err(OptionalRuntimeReason::Invalid);
        }
        let path = direct_pack_file(root, &entry.file)?;
        let (bytes, sha256) = file_identity_with_cancel(&path, MAX_ASSET_BYTES, cancelled)?;
        if bytes != entry.bytes || sha256 != entry.sha256 {
            return Err(OptionalRuntimeReason::Invalid);
        }
        ensure_not_cancelled(cancelled)?;
        files.push(ValidatedFile {
            role: entry.role.clone(),
            file: entry.file.clone(),
            path,
            bytes: entry.bytes,
            sha256: entry.sha256.clone(),
        });
    }
    ensure_not_cancelled(cancelled)?;
    Ok(files)
}

pub(super) fn read_manifest_with_cancel<T: for<'de> Deserialize<'de>>(
    root: &Path,
    cancelled: &dyn Fn() -> bool,
) -> Result<T, OptionalRuntimeReason> {
    ensure_not_cancelled(cancelled)?;
    let path = direct_pack_file(root, "runtime-pack.json")?;
    let bytes = read_file_bounded_with_cancel(&path, MAX_MANIFEST_BYTES, cancelled)?;
    let manifest = serde_json::from_slice(&bytes).map_err(|_| OptionalRuntimeReason::Invalid)?;
    ensure_not_cancelled(cancelled)?;
    Ok(manifest)
}

pub(super) fn read_manifest_pinned_with_cancel<T: for<'de> Deserialize<'de>>(
    root: &Path,
    expected_sha256: &str,
    cancelled: &dyn Fn() -> bool,
) -> Result<T, OptionalRuntimeReason> {
    ensure_not_cancelled(cancelled)?;
    let path = direct_pack_file(root, "runtime-pack.json")?;
    let bytes = read_file_bounded_with_cancel(&path, MAX_MANIFEST_BYTES, cancelled)?;
    if sha256_bytes(&bytes) != expected_sha256 {
        return Err(OptionalRuntimeReason::Invalid);
    }
    ensure_not_cancelled(cancelled)?;
    let manifest = serde_json::from_slice(&bytes).map_err(|_| OptionalRuntimeReason::Invalid)?;
    ensure_not_cancelled(cancelled)?;
    Ok(manifest)
}

pub(super) fn ensure_not_cancelled(
    cancelled: &dyn Fn() -> bool,
) -> Result<(), OptionalRuntimeReason> {
    if cancelled() {
        Err(OptionalRuntimeReason::StartFailed)
    } else {
        Ok(())
    }
}

pub(super) fn canonical_directory(path: &Path) -> Result<PathBuf, OptionalRuntimeReason> {
    bounded_absolute(path)?;
    let metadata = fs::symlink_metadata(path).map_err(classify_io)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(OptionalRuntimeReason::Invalid);
    }
    path.canonicalize().map_err(classify_io)
}

pub(super) fn canonical_input_directory(path: &Path) -> Result<PathBuf, OptionalRuntimeReason> {
    let canonical = canonical_directory(path)?;
    if canonical != path {
        return Err(OptionalRuntimeReason::Invalid);
    }
    Ok(canonical)
}

pub(super) fn validate_regular_file(path: &Path) -> Result<PathBuf, OptionalRuntimeReason> {
    bounded_absolute(path)?;
    let metadata = fs::symlink_metadata(path).map_err(classify_io)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() || metadata.len() == 0 {
        return Err(OptionalRuntimeReason::Invalid);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o022 != 0 {
            return Err(OptionalRuntimeReason::Invalid);
        }
    }
    path.canonicalize().map_err(classify_io)
}

pub(super) fn validate_canonical_executable(path: &Path) -> Result<PathBuf, OptionalRuntimeReason> {
    let canonical = validate_executable(path)?;
    if canonical != path {
        return Err(OptionalRuntimeReason::Invalid);
    }
    Ok(canonical)
}

pub(super) fn validate_executable(path: &Path) -> Result<PathBuf, OptionalRuntimeReason> {
    let path = validate_regular_file(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if path.metadata().map_err(classify_io)?.permissions().mode() & 0o111 == 0 {
            return Err(OptionalRuntimeReason::Invalid);
        }
    }
    Ok(path)
}

pub(super) fn matches_declared_executable(path: &Path, declared: bool) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        path.metadata()
            .map(|metadata| (metadata.permissions().mode() & 0o111 != 0) == declared)
            .unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        let _ = declared;
        true
    }
}

#[cfg(test)]
pub(super) fn sha256_file(path: &Path) -> Result<String, OptionalRuntimeReason> {
    file_identity(path, MAX_ASSET_BYTES).map(|(_, sha256)| sha256)
}

#[cfg(test)]
fn file_identity(path: &Path, max_bytes: u64) -> Result<(u64, String), OptionalRuntimeReason> {
    file_identity_with_cancel(path, max_bytes, &|| false)
}

fn file_identity_with_cancel(
    path: &Path,
    max_bytes: u64,
    cancelled: &dyn Fn() -> bool,
) -> Result<(u64, String), OptionalRuntimeReason> {
    ensure_not_cancelled(cancelled)?;
    let mut file = fs::File::open(path).map_err(classify_io)?;
    let metadata = file.metadata().map_err(classify_io)?;
    if metadata.len() == 0 || metadata.len() > max_bytes {
        return Err(OptionalRuntimeReason::Invalid);
    }
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    let mut observed = 0_u64;
    loop {
        ensure_not_cancelled(cancelled)?;
        let read = file
            .read(&mut buffer)
            .map_err(|_| OptionalRuntimeReason::Invalid)?;
        if read == 0 {
            break;
        }
        observed = observed
            .checked_add(read as u64)
            .filter(|observed| *observed <= max_bytes)
            .ok_or(OptionalRuntimeReason::Invalid)?;
        digest.update(&buffer[..read]);
    }
    ensure_not_cancelled(cancelled)?;
    if observed != metadata.len() {
        return Err(OptionalRuntimeReason::Invalid);
    }
    Ok((observed, format!("{:x}", digest.finalize())))
}

fn read_file_bounded_with_cancel(
    path: &Path,
    max_bytes: u64,
    cancelled: &dyn Fn() -> bool,
) -> Result<Vec<u8>, OptionalRuntimeReason> {
    ensure_not_cancelled(cancelled)?;
    let mut file = fs::File::open(path).map_err(classify_io)?;
    let metadata = file.metadata().map_err(classify_io)?;
    if metadata.len() == 0 || metadata.len() > max_bytes {
        return Err(OptionalRuntimeReason::Invalid);
    }
    let mut bytes = Vec::with_capacity(
        usize::try_from(metadata.len()).map_err(|_| OptionalRuntimeReason::Invalid)?,
    );
    let mut buffer = [0_u8; 8 * 1024];
    loop {
        ensure_not_cancelled(cancelled)?;
        let read = file
            .read(&mut buffer)
            .map_err(|_| OptionalRuntimeReason::Invalid)?;
        if read == 0 {
            break;
        }
        bytes
            .len()
            .checked_add(read)
            .filter(|observed| *observed as u64 <= max_bytes)
            .ok_or(OptionalRuntimeReason::Invalid)?;
        bytes.extend_from_slice(&buffer[..read]);
    }
    ensure_not_cancelled(cancelled)?;
    if bytes.len() as u64 != metadata.len() {
        return Err(OptionalRuntimeReason::Invalid);
    }
    Ok(bytes)
}

pub(super) fn read_verified_file_with_cancel(
    path: &Path,
    expected_bytes: u64,
    expected_sha256: &str,
    max_bytes: u64,
    cancelled: &dyn Fn() -> bool,
) -> Result<Vec<u8>, OptionalRuntimeReason> {
    if expected_bytes == 0 || expected_bytes > max_bytes || !valid_digest(expected_sha256) {
        return Err(OptionalRuntimeReason::Invalid);
    }
    let bytes = read_file_bounded_with_cancel(path, max_bytes, cancelled)?;
    if bytes.len() as u64 != expected_bytes || sha256_bytes(&bytes) != expected_sha256 {
        return Err(OptionalRuntimeReason::Invalid);
    }
    ensure_not_cancelled(cancelled)?;
    Ok(bytes)
}

fn sha256_bytes(bytes: &[u8]) -> String {
    let mut digest = Sha256::new();
    digest.update(bytes);
    format!("{:x}", digest.finalize())
}

pub(super) fn valid_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}

fn direct_pack_file(root: &Path, relative: &str) -> Result<PathBuf, OptionalRuntimeReason> {
    let relative = Path::new(relative);
    if relative.as_os_str().is_empty()
        || relative
            .to_str()
            .is_none_or(|value| value.len() > MAX_PATH_BYTES)
        || relative
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(OptionalRuntimeReason::Invalid);
    }
    let mut current = root.to_path_buf();
    let components = relative.components().collect::<Vec<_>>();
    for (index, component) in components.iter().enumerate() {
        let Component::Normal(component) = component else {
            return Err(OptionalRuntimeReason::Invalid);
        };
        current.push(component);
        let metadata = fs::symlink_metadata(&current).map_err(classify_io)?;
        if metadata.file_type().is_symlink()
            || (index + 1 == components.len() && !metadata.is_file())
            || (index + 1 < components.len() && !metadata.is_dir())
        {
            return Err(OptionalRuntimeReason::Invalid);
        }
    }
    current.canonicalize().map_err(classify_io)
}

fn bounded_absolute(path: &Path) -> Result<(), OptionalRuntimeReason> {
    if !path.is_absolute()
        || path
            .to_str()
            .is_none_or(|value| value.is_empty() || value.len() > MAX_PATH_BYTES)
    {
        return Err(OptionalRuntimeReason::Invalid);
    }
    Ok(())
}

fn classify_io(error: std::io::Error) -> OptionalRuntimeReason {
    if error.kind() == std::io::ErrorKind::NotFound {
        OptionalRuntimeReason::Missing
    } else {
        OptionalRuntimeReason::Invalid
    }
}

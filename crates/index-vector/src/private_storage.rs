use std::fs::{self, File, OpenOptions};
use std::io::{ErrorKind, Read, Write};
use std::path::Path;

#[cfg(unix)]
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};

#[cfg(windows)]
use std::os::windows::fs::OpenOptionsExt;

use crate::model::VectorIndexError;

pub(crate) const KEY_LEN: usize = 32;
const ENCODED_KEY_LEN: usize = KEY_LEN * 2;
const MAX_KEY_FILE_BYTES: usize = ENCODED_KEY_LEN + 1;

#[cfg(windows)]
const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
#[cfg(windows)]
const FILE_FLAG_WRITE_THROUGH: u32 = 0x8000_0000;
#[cfg(windows)]
const FILE_SHARE_READ_WRITE_DELETE: u32 = 0x0000_0007;

pub(crate) struct PinnedPrivateDirectory {
    path: std::path::PathBuf,
    identity: same_file::Handle,
}

impl PinnedPrivateDirectory {
    pub(crate) fn acquire(path: &Path) -> Result<Self, VectorIndexError> {
        validate_private_directory_path(path)?;
        let pinned = Self {
            path: path.to_path_buf(),
            identity: same_file::Handle::from_path(path).map_err(|_| VectorIndexError::Storage)?,
        };
        pinned.validate_current()?;
        Ok(pinned)
    }

    pub(crate) fn validate_current(&self) -> Result<(), VectorIndexError> {
        self.validate_identity_at(&self.path)
    }

    pub(crate) fn validate_identity_at(&self, path: &Path) -> Result<(), VectorIndexError> {
        validate_private_directory_path(path)?;
        let current = same_file::Handle::from_path(path).map_err(|_| VectorIndexError::Storage)?;
        validate_private_directory_path(path)?;
        if self.identity == current {
            Ok(())
        } else {
            Err(VectorIndexError::StorageLayoutInvalid)
        }
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    pub(crate) fn same_identity(&self, other: &Self) -> bool {
        self.identity == other.identity
    }
}

fn validate_private_directory_path(path: &Path) -> Result<(), VectorIndexError> {
    let metadata = fs::symlink_metadata(path).map_err(|_| VectorIndexError::Storage)?;
    if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
        return Err(VectorIndexError::StorageLayoutInvalid);
    }
    #[cfg(unix)]
    if metadata.permissions().mode() & 0o777 != 0o700 {
        return Err(VectorIndexError::StorageLayoutInvalid);
    }
    Ok(())
}

pub(crate) fn load_or_create_key(path: &Path) -> Result<[u8; KEY_LEN], VectorIndexError> {
    match fs::symlink_metadata(path) {
        Ok(_) => read_key(path),
        Err(error) if error.kind() == ErrorKind::NotFound => {
            let key = random_bytes::<KEY_LEN>()?;
            write_private_bytes(path, encode_hex(&key).as_bytes())?;
            let parent = path.parent().ok_or(VectorIndexError::Storage)?;
            sync_directory(parent)?;
            Ok(key)
        }
        Err(_) => Err(VectorIndexError::Storage),
    }
}

pub(crate) fn read_key(path: &Path) -> Result<[u8; KEY_LEN], VectorIndexError> {
    let bytes = read_private_bytes_bounded(path, MAX_KEY_FILE_BYTES)?;
    let encoded = match bytes.as_slice() {
        value if value.len() == ENCODED_KEY_LEN => value,
        value if value.len() == MAX_KEY_FILE_BYTES && value.last() == Some(&b'\n') => {
            &value[..ENCODED_KEY_LEN]
        }
        _ => return Err(VectorIndexError::CorruptSnapshot),
    };
    let value = std::str::from_utf8(encoded).map_err(|_| VectorIndexError::CorruptSnapshot)?;
    decode_fixed_hex::<KEY_LEN>(value)
}

pub(crate) fn read_private_bytes(path: &Path) -> Result<Vec<u8>, VectorIndexError> {
    read_private_bytes_with_limit(path, None)
}

pub(crate) fn read_private_bytes_bounded(
    path: &Path,
    max_bytes: usize,
) -> Result<Vec<u8>, VectorIndexError> {
    read_private_bytes_with_limit(path, Some(max_bytes))
}

fn read_private_bytes_with_limit(
    path: &Path,
    max_bytes: Option<usize>,
) -> Result<Vec<u8>, VectorIndexError> {
    let before = fs::symlink_metadata(path).map_err(map_open_snapshot_error)?;
    validate_private_regular_file(&before)?;
    validate_file_size(&before, max_bytes)?;
    let mut file = File::open(path).map_err(map_open_snapshot_error)?;
    let opened = file.metadata().map_err(|_| VectorIndexError::Storage)?;
    validate_private_regular_file(&opened)?;
    validate_file_size(&opened, max_bytes)?;
    let current = fs::symlink_metadata(path).map_err(map_open_snapshot_error)?;
    validate_private_regular_file(&current)?;
    validate_file_size(&current, max_bytes)?;
    if !same_open_file_identity(&file, path, &opened, &current)? {
        return Err(VectorIndexError::StorageLayoutInvalid);
    }
    let mut bytes = Vec::with_capacity(
        usize::try_from(opened.len())
            .unwrap_or(usize::MAX)
            .min(max_bytes.unwrap_or(8 * 1024)),
    );
    if let Some(max_bytes) = max_bytes {
        file.take(max_bytes.saturating_add(1) as u64)
            .read_to_end(&mut bytes)
            .map_err(|_| VectorIndexError::Storage)?;
        if bytes.len() > max_bytes {
            return Err(VectorIndexError::CorruptSnapshot);
        }
    } else {
        file.read_to_end(&mut bytes)
            .map_err(|_| VectorIndexError::Storage)?;
    }
    Ok(bytes)
}

pub(crate) fn random_bytes<const N: usize>() -> Result<[u8; N], VectorIndexError> {
    let mut bytes = [0_u8; N];
    getrandom::getrandom(&mut bytes).map_err(|_| VectorIndexError::Storage)?;
    Ok(bytes)
}

pub(crate) fn random_suffix() -> Result<String, VectorIndexError> {
    random_bytes::<12>().map(|bytes| encode_hex(&bytes))
}

pub(crate) fn write_private_bytes(path: &Path, bytes: &[u8]) -> Result<(), VectorIndexError> {
    let mut file = create_private_file(path)?;
    file.write_all(bytes)
        .map_err(|_| VectorIndexError::Storage)?;
    file.write_all(b"\n")
        .map_err(|_| VectorIndexError::Storage)?;
    file.sync_all().map_err(|_| VectorIndexError::Storage)
}

pub(crate) fn create_private_file(path: &Path) -> Result<File, VectorIndexError> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    options.mode(0o600);
    #[cfg(windows)]
    options.custom_flags(FILE_FLAG_WRITE_THROUGH);
    let file = options.open(path).map_err(|_| VectorIndexError::Storage)?;
    restrict_private_permissions(path)?;
    Ok(file)
}

pub(crate) fn create_private_directory(path: &Path) -> Result<(), VectorIndexError> {
    fs::create_dir(path).map_err(|_| VectorIndexError::Storage)?;
    #[cfg(unix)]
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
        .map_err(|_| VectorIndexError::Storage)?;
    Ok(())
}

#[cfg(unix)]
fn restrict_private_permissions(path: &Path) -> Result<(), VectorIndexError> {
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
        .map_err(|_| VectorIndexError::Storage)
}

#[cfg(not(unix))]
fn restrict_private_permissions(_path: &Path) -> Result<(), VectorIndexError> {
    Ok(())
}

#[cfg(unix)]
pub(crate) fn sync_directory(path: &Path) -> Result<(), VectorIndexError> {
    File::open(path)
        .and_then(|directory| directory.sync_all())
        .map_err(|_| VectorIndexError::Storage)
}

#[cfg(windows)]
pub(crate) fn sync_directory(path: &Path) -> Result<(), VectorIndexError> {
    let directory = OpenOptions::new()
        .read(true)
        .write(true)
        .share_mode(FILE_SHARE_READ_WRITE_DELETE)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_WRITE_THROUGH)
        .open(path)
        .map_err(|_| VectorIndexError::Storage)?;
    directory.sync_all().map_err(|_| VectorIndexError::Storage)
}

#[cfg(not(any(unix, windows)))]
pub(crate) fn sync_directory(_path: &Path) -> Result<(), VectorIndexError> {
    Err(VectorIndexError::Storage)
}

pub(crate) fn encode_hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(&mut encoded, "{byte:02x}");
    }
    encoded
}

pub(crate) fn decode_fixed_hex<const N: usize>(value: &str) -> Result<[u8; N], VectorIndexError> {
    decode_hex(value)?
        .try_into()
        .map_err(|_| VectorIndexError::CorruptSnapshot)
}

pub(crate) fn decode_hex(value: &str) -> Result<Vec<u8>, VectorIndexError> {
    if !value.len().is_multiple_of(2) {
        return Err(VectorIndexError::CorruptSnapshot);
    }
    value
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let pair = std::str::from_utf8(pair).map_err(|_| VectorIndexError::CorruptSnapshot)?;
            u8::from_str_radix(pair, 16).map_err(|_| VectorIndexError::CorruptSnapshot)
        })
        .collect()
}

fn map_open_snapshot_error(error: std::io::Error) -> VectorIndexError {
    if error.kind() == ErrorKind::NotFound {
        VectorIndexError::GenerationNotFound
    } else {
        VectorIndexError::Storage
    }
}

fn validate_private_regular_file(metadata: &fs::Metadata) -> Result<(), VectorIndexError> {
    if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
        return Err(VectorIndexError::StorageLayoutInvalid);
    }
    #[cfg(unix)]
    if metadata.permissions().mode() & 0o777 != 0o600 {
        return Err(VectorIndexError::StorageLayoutInvalid);
    }
    Ok(())
}

fn validate_file_size(
    metadata: &fs::Metadata,
    max_bytes: Option<usize>,
) -> Result<(), VectorIndexError> {
    if max_bytes.is_some_and(|max_bytes| metadata.len() > max_bytes as u64) {
        Err(VectorIndexError::CorruptSnapshot)
    } else {
        Ok(())
    }
}

#[cfg(unix)]
pub(crate) fn same_open_file_identity(
    _file: &File,
    _path: &Path,
    opened: &fs::Metadata,
    current: &fs::Metadata,
) -> Result<bool, VectorIndexError> {
    Ok(opened.dev() == current.dev() && opened.ino() == current.ino())
}

#[cfg(windows)]
pub(crate) fn same_open_file_identity(
    file: &File,
    path: &Path,
    _opened: &fs::Metadata,
    _current: &fs::Metadata,
) -> Result<bool, VectorIndexError> {
    let opened =
        same_file::Handle::from_file(file.try_clone().map_err(|_| VectorIndexError::Storage)?)
            .map_err(|_| VectorIndexError::Storage)?;
    let current = same_file::Handle::from_path(path).map_err(|_| VectorIndexError::Storage)?;
    let final_metadata = fs::symlink_metadata(path).map_err(map_open_snapshot_error)?;
    validate_private_regular_file(&final_metadata)?;
    Ok(opened == current)
}

#[cfg(not(any(unix, windows)))]
pub(crate) fn same_open_file_identity(
    _file: &File,
    _path: &Path,
    _opened: &fs::Metadata,
    _current: &fs::Metadata,
) -> Result<bool, VectorIndexError> {
    Ok(false)
}

#[cfg(all(test, windows))]
mod windows_tests {
    use super::*;

    #[test]
    fn directory_sync_uses_a_flushable_write_through_handle() {
        let root = std::env::temp_dir().join(format!(
            "resume-ir-vector-directory-sync-{}",
            random_suffix().unwrap()
        ));
        fs::create_dir(&root).unwrap();
        sync_directory(&root).unwrap();
        fs::remove_dir(&root).unwrap();
    }

    #[test]
    fn file_identity_uses_volume_and_file_index() {
        let root = std::env::temp_dir().join(format!(
            "resume-ir-vector-file-identity-{}",
            random_suffix().unwrap()
        ));
        fs::create_dir(&root).unwrap();
        let first = root.join("first");
        let second = root.join("second");
        write_private_bytes(&first, b"first").unwrap();
        write_private_bytes(&second, b"second").unwrap();
        let first_metadata = fs::metadata(&first).unwrap();
        let first_file = File::open(&first).unwrap();
        assert!(same_open_file_identity(
            &first_file,
            &first,
            &first_metadata,
            &fs::metadata(&first).unwrap()
        )
        .unwrap());
        assert!(!same_open_file_identity(
            &first_file,
            &second,
            &first_metadata,
            &fs::metadata(&second).unwrap()
        )
        .unwrap());
        fs::remove_dir_all(&root).unwrap();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_read_is_bounded_and_rejects_oversized_input() {
        let root = std::env::temp_dir().join(format!(
            "resume-ir-vector-key-bound-{}",
            random_suffix().unwrap()
        ));
        fs::create_dir(&root).unwrap();
        let key_path = root.join("key");
        let mut key = create_private_file(&key_path).unwrap();
        key.write_all(&[b'a'; MAX_KEY_FILE_BYTES + 1]).unwrap();
        key.sync_all().unwrap();

        assert_eq!(
            read_key(&key_path).unwrap_err(),
            VectorIndexError::CorruptSnapshot
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn key_read_rejects_noncanonical_trailing_whitespace() {
        let root = std::env::temp_dir().join(format!(
            "resume-ir-vector-key-whitespace-{}",
            random_suffix().unwrap()
        ));
        fs::create_dir(&root).unwrap();
        let key_path = root.join("key");
        let mut key = create_private_file(&key_path).unwrap();
        key.write_all(&[b'a'; ENCODED_KEY_LEN]).unwrap();
        key.write_all(b" ").unwrap();
        key.sync_all().unwrap();

        assert_eq!(
            read_key(&key_path).unwrap_err(),
            VectorIndexError::CorruptSnapshot
        );
        fs::remove_dir_all(root).unwrap();
    }
}

use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use core_domain::ContactHash;
use hmac::{Hmac, Mac};
use sha2::Sha256;

const CONTACT_HASH_KEY_LEN: usize = 32;
const CONTACT_HASH_KEY_HEX_LEN: usize = CONTACT_HASH_KEY_LEN * 2;
const CONTACT_HASH_KEY_PATH: &[&str] = &["secrets", "contact-hash-key-v1"];
const CONTACT_HASH_KEY_BACKUP_SCHEMA_VERSION: &str = "resume-ir-contact-hash-key-v1";

type HmacSha256 = Hmac<Sha256>;

pub fn crate_name() -> &'static str {
    "privacy"
}

#[derive(Clone)]
pub struct ContactHasher {
    key: [u8; CONTACT_HASH_KEY_LEN],
}

impl ContactHasher {
    pub fn from_key_bytes(key: [u8; CONTACT_HASH_KEY_LEN]) -> Self {
        Self { key }
    }

    pub fn load_or_create(data_dir: &Path) -> Result<Self> {
        let key_path = contact_hash_key_path(data_dir);
        if key_path.exists() {
            restrict_key_permissions(&key_path)?;
            let key_hex = fs::read_to_string(&key_path).map_err(PrivacyError::storage)?;
            let key = decode_key_hex(key_hex.trim())?;
            return Ok(Self { key });
        }

        let parent = key_path
            .parent()
            .ok_or_else(|| PrivacyError::invalid_key("contact hash key path"))?;
        fs::create_dir_all(parent).map_err(PrivacyError::storage)?;

        let mut key = [0_u8; CONTACT_HASH_KEY_LEN];
        getrandom::getrandom(&mut key).map_err(|_| PrivacyError::random())?;
        let key_hex = encode_hex(&key);
        write_new_key_file(&key_path, key_hex.as_bytes())?;
        restrict_key_permissions(&key_path)?;

        Ok(Self { key })
    }

    pub fn hash_contact(&self, kind: ContactKind, normalized_value: &str) -> Result<ContactHash> {
        let mut mac = HmacSha256::new_from_slice(&self.key)
            .map_err(|_| PrivacyError::invalid_key("hmac key"))?;
        mac.update(kind.domain_separator().as_bytes());
        mac.update(&[0]);
        mac.update(normalized_value.as_bytes());
        let digest = mac.finalize().into_bytes();
        ContactHash::from_keyed_digest(encode_hex(&digest))
            .map_err(|_| PrivacyError::invalid_key("contact hash digest"))
    }
}

impl fmt::Debug for ContactHasher {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ContactHasher")
            .field("key", &"<redacted>")
            .finish()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContactKind {
    Email,
    Phone,
}

impl ContactKind {
    fn domain_separator(self) -> &'static str {
        match self {
            Self::Email => "resume-ir:contact:v1:email",
            Self::Phone => "resume-ir:contact:v1:phone",
        }
    }
}

pub fn contact_hash_key_path(data_dir: &Path) -> PathBuf {
    CONTACT_HASH_KEY_PATH
        .iter()
        .fold(data_dir.to_path_buf(), |path, component| {
            path.join(component)
        })
}

pub fn inspect_contact_hash_key(data_dir: &Path) -> Result<ContactHashKeyInspection> {
    let key_path = contact_hash_key_path(data_dir);
    match key_path.try_exists() {
        Ok(true) => {}
        Ok(false) => {
            return Ok(ContactHashKeyInspection {
                state: ContactHashKeyState::Missing,
            });
        }
        Err(_) => {
            return Ok(ContactHashKeyInspection {
                state: ContactHashKeyState::Unreadable,
            });
        }
    }

    let key_hex = match fs::read_to_string(&key_path) {
        Ok(key_hex) => key_hex,
        Err(_) => {
            return Ok(ContactHashKeyInspection {
                state: ContactHashKeyState::Unreadable,
            });
        }
    };
    if decode_key_hex(key_hex.trim()).is_err() {
        return Ok(ContactHashKeyInspection {
            state: ContactHashKeyState::Invalid,
        });
    }

    if key_permissions_are_weak(&key_path)? {
        return Ok(ContactHashKeyInspection {
            state: ContactHashKeyState::WeakPermissions,
        });
    }

    Ok(ContactHashKeyInspection {
        state: ContactHashKeyState::Ready,
    })
}

pub fn backup_contact_hash_key(
    data_dir: &Path,
    backup_path: &Path,
) -> Result<ContactHashKeyBackup> {
    let key = read_ready_contact_hash_key(data_dir)?;
    create_private_file_parent(backup_path)?;

    let backup = format!(
        "{CONTACT_HASH_KEY_BACKUP_SCHEMA_VERSION}\nkey={}\n",
        encode_hex(&key)
    );
    write_new_key_file(backup_path, backup.as_bytes())?;
    restrict_key_permissions(backup_path)?;

    Ok(ContactHashKeyBackup { _private: () })
}

pub fn restore_contact_hash_key(
    data_dir: &Path,
    backup_path: &Path,
) -> Result<ContactHashKeyRestore> {
    let key_path = contact_hash_key_path(data_dir);
    if key_path.try_exists().map_err(PrivacyError::storage)? {
        return Err(PrivacyError::already_exists());
    }

    let key = read_backup_contact_hash_key(backup_path)?;
    let parent = key_path
        .parent()
        .ok_or_else(|| PrivacyError::invalid_key("contact hash key path"))?;
    fs::create_dir_all(parent).map_err(PrivacyError::storage)?;
    write_new_key_file(&key_path, encode_hex(&key).as_bytes())?;
    restrict_key_permissions(&key_path)?;

    Ok(ContactHashKeyRestore { _private: () })
}

#[derive(Clone, PartialEq, Eq)]
pub struct ContactHashKeyInspection {
    state: ContactHashKeyState,
}

impl ContactHashKeyInspection {
    pub fn state(&self) -> ContactHashKeyState {
        self.state
    }
}

impl fmt::Debug for ContactHashKeyInspection {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ContactHashKeyInspection")
            .field("state", &self.state)
            .finish()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContactHashKeyState {
    Missing,
    Ready,
    Invalid,
    WeakPermissions,
    Unreadable,
}

#[derive(Clone, PartialEq, Eq)]
pub struct ContactHashKeyBackup {
    _private: (),
}

impl fmt::Debug for ContactHashKeyBackup {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ContactHashKeyBackup")
            .field("key", &"<redacted>")
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct ContactHashKeyRestore {
    _private: (),
}

impl fmt::Debug for ContactHashKeyRestore {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ContactHashKeyRestore")
            .field("key", &"<redacted>")
            .finish()
    }
}

fn read_ready_contact_hash_key(data_dir: &Path) -> Result<[u8; CONTACT_HASH_KEY_LEN]> {
    let inspection = inspect_contact_hash_key(data_dir)?;
    if inspection.state() != ContactHashKeyState::Ready {
        return Err(PrivacyError::invalid_key("contact hash key state"));
    }

    let key_path = contact_hash_key_path(data_dir);
    let key_hex = fs::read_to_string(&key_path).map_err(PrivacyError::storage)?;
    decode_key_hex(key_hex.trim())
}

fn read_backup_contact_hash_key(backup_path: &Path) -> Result<[u8; CONTACT_HASH_KEY_LEN]> {
    let backup = fs::read_to_string(backup_path).map_err(PrivacyError::storage)?;
    let mut lines = backup.lines();
    if lines.next() != Some(CONTACT_HASH_KEY_BACKUP_SCHEMA_VERSION) {
        return Err(PrivacyError::invalid_key("contact hash key backup schema"));
    }
    let Some(key_hex) = lines.next().and_then(|line| line.strip_prefix("key=")) else {
        return Err(PrivacyError::invalid_key("contact hash key backup payload"));
    };

    decode_key_hex(key_hex)
}

fn create_private_file_parent(path: &Path) -> Result<()> {
    let Some(parent) = path.parent() else {
        return Ok(());
    };
    if parent.as_os_str().is_empty() {
        return Ok(());
    }

    fs::create_dir_all(parent).map_err(PrivacyError::storage)
}

impl ContactHashKeyState {
    pub fn label(self) -> &'static str {
        match self {
            Self::Missing => "missing",
            Self::Ready => "ready",
            Self::Invalid => "invalid",
            Self::WeakPermissions => "weak_permissions",
            Self::Unreadable => "unreadable",
        }
    }
}

fn write_new_key_file(path: &Path, bytes: &[u8]) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;

        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true).mode(0o600);
        use std::io::Write;
        let mut file = options.open(path).map_err(PrivacyError::storage)?;
        file.write_all(bytes).map_err(PrivacyError::storage)?;
        file.write_all(b"\n").map_err(PrivacyError::storage)?;
        file.sync_all().map_err(PrivacyError::storage)?;
        Ok(())
    }
    #[cfg(not(unix))]
    {
        use std::io::Write;

        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .map_err(PrivacyError::storage)?;
        file.write_all(bytes).map_err(PrivacyError::storage)?;
        file.write_all(b"\n").map_err(PrivacyError::storage)?;
        file.sync_all().map_err(PrivacyError::storage)?;
        Ok(())
    }
}

fn key_permissions_are_weak(path: &Path) -> Result<bool> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let mode = fs::metadata(path)
            .map_err(PrivacyError::storage)?
            .permissions()
            .mode()
            & 0o777;
        Ok(mode != 0o600)
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        Ok(false)
    }
}

fn restrict_key_permissions(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        fs::set_permissions(path, fs::Permissions::from_mode(0o600))
            .map_err(PrivacyError::storage)?;
    }

    Ok(())
}

fn decode_key_hex(value: &str) -> Result<[u8; CONTACT_HASH_KEY_LEN]> {
    if value.len() != CONTACT_HASH_KEY_HEX_LEN {
        return Err(PrivacyError::invalid_key("contact hash key length"));
    }
    let mut key = [0_u8; CONTACT_HASH_KEY_LEN];
    for (index, slot) in key.iter_mut().enumerate() {
        let start = index * 2;
        *slot = u8::from_str_radix(&value[start..start + 2], 16)
            .map_err(|_| PrivacyError::invalid_key("contact hash key hex"))?;
    }
    Ok(key)
}

fn encode_hex(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write;
        let _ = write!(&mut output, "{byte:02x}");
    }
    output
}

pub type Result<T> = std::result::Result<T, PrivacyError>;

#[derive(Clone, PartialEq, Eq)]
pub struct PrivacyError {
    kind: PrivacyErrorKind,
}

impl PrivacyError {
    fn storage(_error: io::Error) -> Self {
        Self {
            kind: PrivacyErrorKind::Storage,
        }
    }

    fn random() -> Self {
        Self {
            kind: PrivacyErrorKind::Random,
        }
    }

    fn invalid_key(_field: &'static str) -> Self {
        Self {
            kind: PrivacyErrorKind::InvalidKey,
        }
    }

    fn already_exists() -> Self {
        Self {
            kind: PrivacyErrorKind::AlreadyExists,
        }
    }
}

impl fmt::Debug for PrivacyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PrivacyError")
            .field("kind", &self.kind)
            .finish()
    }
}

impl fmt::Display for PrivacyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.kind {
            PrivacyErrorKind::Storage => formatter.write_str("privacy storage operation failed"),
            PrivacyErrorKind::Random => formatter.write_str("privacy key generation failed"),
            PrivacyErrorKind::InvalidKey => formatter.write_str("privacy key material is invalid"),
            PrivacyErrorKind::AlreadyExists => formatter.write_str("privacy key already exists"),
        }
    }
}

impl std::error::Error for PrivacyError {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PrivacyErrorKind {
    Storage,
    Random,
    InvalidKey,
    AlreadyExists,
}

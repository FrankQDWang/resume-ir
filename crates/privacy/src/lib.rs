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
        }
    }
}

impl std::error::Error for PrivacyError {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PrivacyErrorKind {
    Storage,
    Random,
    InvalidKey,
}

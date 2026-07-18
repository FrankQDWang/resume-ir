use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use fs4::fs_std::FileExt;

const OWNER_LOCK_FILE: &str = "daemon.owner.lock";
const AUTH_FILE: &str = "ipc.auth";
const ENDPOINT_FILE: &str = "ipc.endpoints.json";
const AUTH_SCHEMA_VERSION: &str = "resume-ir.daemon-auth.v2";
pub(crate) const IPC_PROTOCOL_VERSION: &str = "resume-ir.daemon-ipc.v2";
const GENERATION_ID_BYTES: usize = 32;
const OWNER_FILE_MAX_BYTES: u64 = 16 * 1024;

#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum OwnerMode {
    Standalone,
    DesktopSupervised,
}

impl OwnerMode {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Standalone => "standalone",
            Self::DesktopSupervised => "desktop_supervised",
        }
    }
}

/// Owns one daemon generation and the exclusive data-directory lease.
///
/// The lock is held until drop. Discovery artifacts are generation-bound, so a
/// stale process can never remove files published by a later owner.
pub(crate) struct DaemonGenerationOwner {
    _lock: File,
    data_dir: PathBuf,
    instance_id: String,
    auth_token: String,
    owner_mode: OwnerMode,
}

impl DaemonGenerationOwner {
    pub(crate) fn acquire(data_dir: &Path, owner_mode: OwnerMode) -> Result<Self, GenerationError> {
        fs::create_dir_all(data_dir).map_err(|_| GenerationError::Storage)?;
        let lock_path = data_dir.join(OWNER_LOCK_FILE);
        reject_unsafe_regular_file_or_missing(&lock_path)?;
        let mut options = OpenOptions::new();
        options.create(true).read(true).write(true);
        #[cfg(unix)]
        options.mode(0o600);
        let lock = options
            .open(&lock_path)
            .map_err(|_| GenerationError::Storage)?;
        match FileExt::try_lock_exclusive(&lock) {
            Ok(true) => {}
            Ok(false) => return Err(GenerationError::OwnershipConflict),
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                return Err(GenerationError::OwnershipConflict);
            }
            Err(_) => return Err(GenerationError::Storage),
        }
        secure_owner_file(&lock_path)?;
        clear_stale_owner_file(&data_dir.join(ENDPOINT_FILE))?;
        clear_stale_owner_file(&data_dir.join(AUTH_FILE))?;
        Ok(Self {
            _lock: lock,
            data_dir: data_dir.to_path_buf(),
            instance_id: random_hex(GENERATION_ID_BYTES)?,
            auth_token: random_hex(GENERATION_ID_BYTES)?,
            owner_mode,
        })
    }

    pub(crate) fn auth_token(&self) -> &str {
        &self.auth_token
    }

    pub(crate) fn publish(&self, addr: SocketAddr) -> Result<(), GenerationError> {
        let auth = serde_json::json!({
            "schema_version": AUTH_SCHEMA_VERSION,
            "instance_id": self.instance_id,
            "token": self.auth_token,
        })
        .to_string();
        let endpoints = serde_json::json!({
            "schema_version": IPC_PROTOCOL_VERSION,
            "instance_id": self.instance_id,
            "owner_mode": self.owner_mode.label(),
            "status": format!("http://{addr}/status"),
            "diagnostics": format!("http://{addr}/diagnostics"),
            "imports": format!("http://{addr}/imports"),
            "import_cancel": format!("http://{addr}/imports/cancel"),
            "import_control": format!("http://{addr}/imports/control"),
            "import_progress": format!("http://{addr}/imports/progress"),
            "search": format!("http://{addr}/search"),
            "search_batch": format!("http://{addr}/search/batch"),
            "details": format!("http://{addr}/details"),
            "delete": format!("http://{addr}/delete"),
        })
        .to_string();

        atomic_write_private(
            &self.data_dir,
            AUTH_FILE,
            &self.instance_id,
            auth.as_bytes(),
        )?;
        if let Err(error) = atomic_write_private(
            &self.data_dir,
            ENDPOINT_FILE,
            &self.instance_id,
            endpoints.as_bytes(),
        ) {
            remove_if_owned(&self.data_dir.join(AUTH_FILE), &self.instance_id);
            return Err(error);
        }
        Ok(())
    }
}

impl Drop for DaemonGenerationOwner {
    fn drop(&mut self) {
        remove_if_owned(&self.data_dir.join(ENDPOINT_FILE), &self.instance_id);
        remove_if_owned(&self.data_dir.join(AUTH_FILE), &self.instance_id);
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum GenerationError {
    OwnershipConflict,
    RuntimeIntegrity,
    Storage,
}

fn atomic_write_private(
    data_dir: &Path,
    file_name: &str,
    instance_id: &str,
    bytes: &[u8],
) -> Result<(), GenerationError> {
    let path = data_dir.join(file_name);
    reject_unsafe_regular_file_or_missing(&path)?;
    let temp_path = data_dir.join(format!(".{file_name}.{instance_id}.tmp"));
    let mut options = OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    options.mode(0o600);
    let mut file = options
        .open(&temp_path)
        .map_err(|_| GenerationError::Storage)?;
    if file
        .write_all(bytes)
        .and_then(|_| file.flush())
        .and_then(|_| file.sync_all())
        .is_err()
    {
        let _ = fs::remove_file(&temp_path);
        return Err(GenerationError::Storage);
    }
    #[cfg(unix)]
    if fs::set_permissions(&temp_path, fs::Permissions::from_mode(0o600)).is_err() {
        let _ = fs::remove_file(&temp_path);
        return Err(GenerationError::Storage);
    }
    if fs::rename(&temp_path, &path).is_err() {
        let _ = fs::remove_file(&temp_path);
        return Err(GenerationError::Storage);
    }
    Ok(())
}

fn remove_if_owned(path: &Path, instance_id: &str) {
    if fs::symlink_metadata(path)
        .ok()
        .is_none_or(|metadata| !metadata.file_type().is_file())
    {
        return;
    }
    let Ok(file) = File::open(path) else {
        return;
    };
    let mut bytes = Vec::new();
    if file
        .take(OWNER_FILE_MAX_BYTES.saturating_add(1))
        .read_to_end(&mut bytes)
        .is_err()
        || bytes.len() as u64 > OWNER_FILE_MAX_BYTES
    {
        return;
    }
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
        return;
    };
    if value.get("instance_id").and_then(serde_json::Value::as_str) == Some(instance_id) {
        let _ = fs::remove_file(path);
    }
}

fn clear_stale_owner_file(path: &Path) -> Result<(), GenerationError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_file() => {
            fs::remove_file(path).map_err(|_| GenerationError::Storage)
        }
        Ok(_) => Err(GenerationError::RuntimeIntegrity),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err(GenerationError::Storage),
    }
}

fn reject_unsafe_regular_file_or_missing(path: &Path) -> Result<(), GenerationError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_file() => Ok(()),
        Ok(_) => Err(GenerationError::RuntimeIntegrity),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err(GenerationError::Storage),
    }
}

#[cfg(unix)]
fn secure_owner_file(path: &Path) -> Result<(), GenerationError> {
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
        .map_err(|_| GenerationError::Storage)
}

#[cfg(not(unix))]
fn secure_owner_file(_path: &Path) -> Result<(), GenerationError> {
    Ok(())
}

fn random_hex(byte_count: usize) -> Result<String, GenerationError> {
    let mut bytes = vec![0_u8; byte_count];
    getrandom::getrandom(&mut bytes).map_err(|_| GenerationError::Storage)?;
    let mut token = String::with_capacity(byte_count.saturating_mul(2));
    for byte in bytes {
        use std::fmt::Write as _;
        write!(&mut token, "{byte:02x}").map_err(|_| GenerationError::Storage)?;
    }
    Ok(token)
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::net::SocketAddr;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::{DaemonGenerationOwner, GenerationError, OwnerMode};

    #[test]
    fn owner_is_exclusive_and_generation_credentials_rotate() {
        let data_dir = temp_dir("exclusive");
        let first = DaemonGenerationOwner::acquire(&data_dir, OwnerMode::Standalone).unwrap();
        let first_instance = first.instance_id.clone();
        let first_token = first.auth_token.clone();
        assert_eq!(
            DaemonGenerationOwner::acquire(&data_dir, OwnerMode::Standalone)
                .err()
                .unwrap(),
            GenerationError::OwnershipConflict
        );
        drop(first);

        let second = DaemonGenerationOwner::acquire(&data_dir, OwnerMode::Standalone).unwrap();
        assert_ne!(second.instance_id, first_instance);
        assert_ne!(second.auth_token, first_token);
        drop(second);
        let _ = fs::remove_dir_all(data_dir);
    }

    #[test]
    fn published_auth_and_manifest_share_generation_and_cleanup_is_owned() {
        let data_dir = temp_dir("publication");
        let owner =
            DaemonGenerationOwner::acquire(&data_dir, OwnerMode::DesktopSupervised).unwrap();
        owner
            .publish("127.0.0.1:43111".parse::<SocketAddr>().unwrap())
            .unwrap();
        let auth = read_json(data_dir.join("ipc.auth"));
        let manifest = read_json(data_dir.join("ipc.endpoints.json"));
        assert_eq!(auth["instance_id"], manifest["instance_id"]);
        assert_eq!(manifest["owner_mode"], "desktop_supervised");
        assert_eq!(auth["token"], owner.auth_token);

        let replacement_instance = "f".repeat(64);
        fs::write(
            data_dir.join("ipc.endpoints.json"),
            serde_json::json!({"instance_id": replacement_instance.clone()}).to_string(),
        )
        .unwrap();
        drop(owner);
        assert!(!data_dir.join("ipc.auth").exists());
        assert_eq!(
            read_json(data_dir.join("ipc.endpoints.json"))["instance_id"],
            replacement_instance
        );
        let _ = fs::remove_dir_all(data_dir);
    }

    #[test]
    fn unsafe_discovery_artifact_is_an_integrity_failure() {
        let data_dir = temp_dir("unsafe-artifact");
        fs::create_dir(data_dir.join("ipc.auth")).unwrap();

        assert_eq!(
            DaemonGenerationOwner::acquire(&data_dir, OwnerMode::Standalone)
                .err()
                .unwrap(),
            GenerationError::RuntimeIntegrity
        );

        let _ = fs::remove_dir_all(data_dir);
    }

    fn read_json(path: PathBuf) -> serde_json::Value {
        serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
    }

    fn temp_dir(label: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("resume-ir-daemon-generation-{label}-{unique}"));
        fs::create_dir_all(&path).unwrap();
        path
    }
}

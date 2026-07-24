use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;

use meta_store::DataDirectoryOwnerLease;

const AUTH_FILE: &str = "ipc.auth";
const ENDPOINT_FILE: &str = "ipc.endpoints.json";
const AUTH_SCHEMA_VERSION: &str = "resume-ir.daemon-auth.v3";
pub(crate) const IPC_PROTOCOL_VERSION: &str = "resume-ir.daemon-ipc.v3";
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

/// Owns generation-bound discovery artifacts under the process-wide metadata
/// owner. The shared owner capability keeps the daemon namespace locked until
/// every artifact from this generation has been removed.
pub(crate) struct DaemonGenerationOwner {
    data_directory_owner: Arc<DataDirectoryOwnerLease>,
    launch_id: String,
    instance_id: String,
    auth_token: String,
    owner_mode: OwnerMode,
}

#[derive(Clone)]
pub(crate) struct GenerationPublicationRevoker {
    data_directory_owner: Arc<DataDirectoryOwnerLease>,
    instance_id: String,
}

impl DaemonGenerationOwner {
    pub(crate) fn acquire(
        data_directory_owner: Arc<DataDirectoryOwnerLease>,
        owner_mode: OwnerMode,
        launch_id: String,
    ) -> Result<Self, GenerationError> {
        let data_dir = data_directory_owner.canonical_data_dir();
        clear_stale_owner_file(&data_dir.join(ENDPOINT_FILE))?;
        clear_stale_owner_file(&data_dir.join(AUTH_FILE))?;
        Ok(Self {
            data_directory_owner,
            launch_id,
            instance_id: random_hex(GENERATION_ID_BYTES)?,
            auth_token: random_hex(GENERATION_ID_BYTES)?,
            owner_mode,
        })
    }

    pub(crate) fn auth_token(&self) -> &str {
        &self.auth_token
    }

    pub(crate) fn generate_launch_id() -> Result<String, GenerationError> {
        random_hex(GENERATION_ID_BYTES)
    }

    pub(crate) fn publication_revoker(&self) -> GenerationPublicationRevoker {
        GenerationPublicationRevoker {
            data_directory_owner: Arc::clone(&self.data_directory_owner),
            instance_id: self.instance_id.clone(),
        }
    }

    pub(crate) fn publish(&self, addr: SocketAddr) -> Result<(), GenerationError> {
        let data_dir = self.data_directory_owner.canonical_data_dir();
        let auth = serde_json::json!({
            "schema_version": AUTH_SCHEMA_VERSION,
            "launch_id": self.launch_id,
            "instance_id": self.instance_id,
            "token": self.auth_token,
        })
        .to_string();
        let endpoints = serde_json::json!({
            "schema_version": IPC_PROTOCOL_VERSION,
            "launch_id": self.launch_id,
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

        atomic_write_private(data_dir, AUTH_FILE, &self.instance_id, auth.as_bytes())?;
        if let Err(error) = atomic_write_private(
            data_dir,
            ENDPOINT_FILE,
            &self.instance_id,
            endpoints.as_bytes(),
        ) {
            remove_if_owned(&data_dir.join(AUTH_FILE), &self.instance_id);
            return Err(error);
        }
        Ok(())
    }
}

impl GenerationPublicationRevoker {
    pub(crate) fn withdraw(&self) {
        let data_dir = self.data_directory_owner.canonical_data_dir();
        remove_if_owned(&data_dir.join(ENDPOINT_FILE), &self.instance_id);
        remove_if_owned(&data_dir.join(AUTH_FILE), &self.instance_id);
    }
}

impl Drop for DaemonGenerationOwner {
    fn drop(&mut self) {
        self.publication_revoker().withdraw();
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum GenerationError {
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

    use meta_store::{DataDirectoryOwnerAcquisition, DataDirectoryOwnerLease};

    use super::{
        DaemonGenerationOwner, GenerationError, OwnerMode, AUTH_FILE, ENDPOINT_FILE,
        OWNER_FILE_MAX_BYTES,
    };

    #[test]
    fn owner_is_exclusive_and_generation_credentials_rotate() {
        let data_dir = temp_dir("exclusive");
        let first_data_directory_owner = std::sync::Arc::new(data_directory_owner(&data_dir));
        let first = DaemonGenerationOwner::acquire(
            std::sync::Arc::clone(&first_data_directory_owner),
            OwnerMode::Standalone,
            "1".repeat(64),
        )
        .unwrap();
        let first_instance = first.instance_id.clone();
        let first_token = first.auth_token.clone();
        assert!(matches!(
            DataDirectoryOwnerLease::try_acquire(&data_dir).unwrap(),
            DataDirectoryOwnerAcquisition::Contended
        ));
        drop(first_data_directory_owner);
        drop(first);
        assert!(matches!(
            DataDirectoryOwnerLease::try_acquire(&data_dir).unwrap(),
            DataDirectoryOwnerAcquisition::Acquired(_)
        ));
        let second_data_directory_owner = data_directory_owner(&data_dir);
        let second = DaemonGenerationOwner::acquire(
            std::sync::Arc::new(second_data_directory_owner),
            OwnerMode::Standalone,
            "2".repeat(64),
        )
        .unwrap();
        assert_ne!(second.instance_id, first_instance);
        assert_ne!(second.auth_token, first_token);
        drop(second);
        let _ = fs::remove_dir_all(data_dir);
    }

    #[test]
    fn published_auth_and_manifest_share_generation_and_cleanup_is_owned() {
        let data_dir = temp_dir("publication");
        let data_directory_owner = data_directory_owner(&data_dir);
        let owner = DaemonGenerationOwner::acquire(
            std::sync::Arc::new(data_directory_owner),
            OwnerMode::DesktopSupervised,
            "a".repeat(64),
        )
        .unwrap();
        owner
            .publish("127.0.0.1:43111".parse::<SocketAddr>().unwrap())
            .unwrap();
        let auth = read_json(data_dir.join("ipc.auth"));
        let manifest = read_json(data_dir.join("ipc.endpoints.json"));
        assert_eq!(auth["instance_id"], manifest["instance_id"]);
        assert_eq!(auth["schema_version"], "resume-ir.daemon-auth.v3");
        assert_eq!(manifest["schema_version"], "resume-ir.daemon-ipc.v3");
        assert_eq!(auth["launch_id"], "a".repeat(64));
        assert_eq!(manifest["launch_id"], auth["launch_id"]);
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
        let data_directory_owner = data_directory_owner(&data_dir);

        assert_eq!(
            DaemonGenerationOwner::acquire(
                std::sync::Arc::new(data_directory_owner),
                OwnerMode::Standalone,
                "b".repeat(64),
            )
            .err()
            .unwrap(),
            GenerationError::RuntimeIntegrity
        );
        let _ = fs::remove_dir_all(data_dir);
    }

    #[test]
    fn stale_regular_control_files_are_removed_without_parsing() {
        for (label, bytes) in [
            ("half-written", b"{\"schema_version\":".to_vec()),
            (
                "oversize",
                vec![b'x'; OWNER_FILE_MAX_BYTES.saturating_add(1) as usize],
            ),
        ] {
            for file_name in [AUTH_FILE, ENDPOINT_FILE] {
                let data_dir = temp_dir(&format!("{label}-{file_name}"));
                let path = data_dir.join(file_name);
                fs::write(&path, &bytes).unwrap();
                let owner = DaemonGenerationOwner::acquire(
                    std::sync::Arc::new(data_directory_owner(&data_dir)),
                    OwnerMode::Standalone,
                    "c".repeat(64),
                )
                .unwrap();

                assert!(!path.exists());
                drop(owner);
                let _ = fs::remove_dir_all(data_dir);
            }
        }
    }

    #[test]
    fn control_file_directories_fail_closed_without_modification() {
        for file_name in [AUTH_FILE, ENDPOINT_FILE] {
            let data_dir = temp_dir(&format!("directory-{file_name}"));
            let path = data_dir.join(file_name);
            fs::create_dir(&path).unwrap();
            assert_integrity_failure(&data_dir);
            assert!(path.is_dir());
            let _ = fs::remove_dir_all(data_dir);
        }
    }

    #[cfg(unix)]
    #[test]
    fn symlink_fifo_and_socket_control_files_fail_closed_without_modification() {
        use std::os::unix::fs::{symlink, FileTypeExt};
        use std::os::unix::net::UnixListener;

        use nix::sys::stat::Mode;
        use nix::unistd::mkfifo;

        for file_name in [AUTH_FILE, ENDPOINT_FILE] {
            let symlink_dir = temp_dir(&format!("symlink-{file_name}"));
            let target = symlink_dir.join("target");
            fs::write(&target, b"foreign").unwrap();
            let symlink_path = symlink_dir.join(file_name);
            symlink(&target, &symlink_path).unwrap();
            assert_integrity_failure(&symlink_dir);
            assert!(fs::symlink_metadata(&symlink_path)
                .unwrap()
                .file_type()
                .is_symlink());
            assert_eq!(fs::read(&target).unwrap(), b"foreign");
            let _ = fs::remove_dir_all(symlink_dir);

            let fifo_dir = temp_dir(&format!("fifo-{file_name}"));
            let fifo_path = fifo_dir.join(file_name);
            mkfifo(&fifo_path, Mode::S_IRUSR | Mode::S_IWUSR).unwrap();
            assert_integrity_failure(&fifo_dir);
            assert!(fs::symlink_metadata(&fifo_path)
                .unwrap()
                .file_type()
                .is_fifo());
            let _ = fs::remove_dir_all(fifo_dir);

            let socket_dir = short_socket_temp_dir();
            let socket_path = socket_dir.join(file_name);
            let listener = UnixListener::bind(&socket_path).unwrap();
            assert_integrity_failure(&socket_dir);
            assert!(fs::symlink_metadata(&socket_path)
                .unwrap()
                .file_type()
                .is_socket());
            drop(listener);
            let _ = fs::remove_dir_all(socket_dir);
        }
    }

    fn assert_integrity_failure(data_dir: &std::path::Path) {
        let owner = data_directory_owner(data_dir);
        assert_eq!(
            DaemonGenerationOwner::acquire(
                std::sync::Arc::new(owner),
                OwnerMode::Standalone,
                "d".repeat(64),
            )
            .err()
            .unwrap(),
            GenerationError::RuntimeIntegrity
        );
    }

    fn data_directory_owner(data_dir: &std::path::Path) -> DataDirectoryOwnerLease {
        match DataDirectoryOwnerLease::try_acquire(data_dir).unwrap() {
            DataDirectoryOwnerAcquisition::Acquired(owner) => owner,
            DataDirectoryOwnerAcquisition::Contended => {
                panic!("data-directory owner unexpectedly contended")
            }
        }
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

    #[cfg(unix)]
    fn short_socket_temp_dir() -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = PathBuf::from("/tmp").join(format!("ri-gs-{}-{unique}", std::process::id()));
        fs::create_dir_all(&path).unwrap();
        path
    }
}

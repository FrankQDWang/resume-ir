use std::fs::{self, File};
use std::io::Read;
use std::net::SocketAddr;
use std::path::Path;

use serde::Deserialize;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

use crate::daemon_client::DesktopError;

const ENDPOINT_MANIFEST_FILE: &str = "ipc.endpoints.json";
const AUTH_MANIFEST_FILE: &str = "ipc.auth";
const ENDPOINT_SCHEMA: &str = "resume-ir.daemon-ipc.v2";
const AUTH_SCHEMA: &str = "resume-ir.daemon-auth.v2";
const MAX_ENDPOINT_MANIFEST_BYTES: u64 = 16 * 1024;
const MAX_AUTH_MANIFEST_BYTES: u64 = 1024;
const GENERATION_VALUE_LENGTH: usize = 64;

#[derive(Clone, Copy)]
pub(crate) enum DaemonRoute {
    Status,
    Diagnostics,
    Imports,
    ImportControl,
    Search,
    Details,
    Hydrate,
    Cancel,
}

impl DaemonRoute {
    pub(crate) const fn path(self) -> &'static str {
        match self {
            Self::Status => "/status",
            Self::Diagnostics => "/diagnostics",
            Self::Imports => "/imports",
            Self::ImportControl => "/imports/control",
            Self::Search => "/search",
            Self::Details => "/details",
            Self::Hydrate => "/details/hydrate",
            Self::Cancel => "/search/cancel",
        }
    }
}

/// A fully validated discovery/auth pair for exactly one daemon generation.
pub(crate) struct DaemonConnection {
    instance_id: String,
    token: String,
    addr: SocketAddr,
}

impl DaemonConnection {
    pub(crate) fn addr(&self) -> SocketAddr {
        self.addr
    }

    pub(crate) fn token(&self) -> &str {
        &self.token
    }

    fn is_same_generation(&self, other: &Self) -> bool {
        self.instance_id == other.instance_id
            && self.token == other.token
            && self.addr == other.addr
    }
}

/// Supplies a copied supervisor generation without retaining a lifecycle lock.
/// Implementations return `Some` only for `ready`, and must release any
/// internal lock before returning so owner-file and socket I/O stays lock-free.
pub(crate) trait ConnectionGenerationSource {
    fn ready_generation(&self) -> Option<u64>;
}

/// Native proof that a business request belongs to one supervisor/daemon pair.
pub(crate) struct ConnectionLease {
    supervisor_generation: u64,
    instance_id: String,
}

impl ConnectionLease {
    fn acquire(
        data_dir: &Path,
        generation_source: &impl ConnectionGenerationSource,
    ) -> Result<(Self, DaemonConnection), DesktopError> {
        let supervisor_generation = generation_source
            .ready_generation()
            .ok_or_else(generation_changed)?;
        let connection = load_connection(data_dir).map_err(|error| {
            if error.is_daemon_unavailable() {
                generation_changed()
            } else {
                error
            }
        })?;
        ensure_generation(generation_source, supervisor_generation)?;
        let lease = Self {
            supervisor_generation,
            instance_id: connection.instance_id.clone(),
        };
        Ok((lease, connection))
    }

    fn supervisor_is_current(&self, generation_source: &impl ConnectionGenerationSource) -> bool {
        generation_source.ready_generation() == Some(self.supervisor_generation)
    }

    fn instance_is_current(&self, connection: &DaemonConnection) -> bool {
        self.instance_id == connection.instance_id
    }
}

/// Runs one business request against one supervisor and daemon generation.
///
/// The request closure is invoked at most once. A generation transition before,
/// during, or immediately after I/O invalidates the result instead of replaying
/// an operation whose side effects may already have happened.
pub(crate) fn with_connection_lease<T>(
    data_dir: &Path,
    generation_source: &impl ConnectionGenerationSource,
    request: impl FnOnce(&DaemonConnection) -> Result<T, DesktopError>,
) -> Result<T, DesktopError> {
    let (lease, connection) = ConnectionLease::acquire(data_dir, generation_source)?;

    let result = request(&connection);
    let supervisor_was_stable = lease.supervisor_is_current(generation_source);
    let connection_is_stable = load_connection(data_dir).is_ok_and(|current| {
        lease.instance_is_current(&current) && connection.is_same_generation(&current)
    });
    let supervisor_is_still_stable = lease.supervisor_is_current(generation_source);
    if !supervisor_was_stable || !connection_is_stable || !supervisor_is_still_stable {
        return Err(generation_changed());
    }
    result
}

/// Loads a generation-bound pair for supervisor startup and heartbeat probes.
/// Probes do not use a business lease because the supervisor is still in the
/// `starting` state before it publishes its first ready generation.
pub(crate) fn load_probe_connection(data_dir: &Path) -> Result<DaemonConnection, DesktopError> {
    load_connection(data_dir)
}

fn ensure_generation(
    generation_source: &impl ConnectionGenerationSource,
    expected: u64,
) -> Result<(), DesktopError> {
    if generation_source.ready_generation() == Some(expected) {
        Ok(())
    } else {
        Err(generation_changed())
    }
}

fn load_connection(data_dir: &Path) -> Result<DaemonConnection, DesktopError> {
    let manifest_before = read_owner_file(
        &data_dir.join(ENDPOINT_MANIFEST_FILE),
        MAX_ENDPOINT_MANIFEST_BYTES,
        "本地 daemon 未运行或尚未就绪",
    )?;
    let auth = read_owner_file(
        &data_dir.join(AUTH_MANIFEST_FILE),
        MAX_AUTH_MANIFEST_BYTES,
        "本地 daemon 凭据不可用",
    )?;
    let manifest_after = read_owner_file(
        &data_dir.join(ENDPOINT_MANIFEST_FILE),
        MAX_ENDPOINT_MANIFEST_BYTES,
        "本地 daemon 未运行或尚未就绪",
    )?;
    decode_connection(&manifest_before, &auth, &manifest_after)
}

fn decode_connection(
    manifest_before: &[u8],
    auth: &[u8],
    manifest_after: &[u8],
) -> Result<DaemonConnection, DesktopError> {
    if manifest_before != manifest_after {
        return Err(generation_changed());
    }
    let manifest: EndpointManifest = serde_json::from_slice(manifest_before)
        .map_err(|_| protocol_error("daemon endpoint 合同无效"))?;
    let auth: AuthManifest =
        serde_json::from_slice(auth).map_err(|_| protocol_error("daemon 凭据合同无效"))?;
    if manifest.schema_version != ENDPOINT_SCHEMA || auth.schema_version != AUTH_SCHEMA {
        return Err(protocol_error("daemon endpoint 版本不兼容"));
    }
    if !valid_generation_value(&manifest.instance_id)
        || !valid_generation_value(&auth.instance_id)
        || !valid_generation_value(&auth.token)
    {
        return Err(protocol_error("daemon generation 合同无效"));
    }
    if manifest.instance_id != auth.instance_id {
        return Err(generation_changed());
    }
    if manifest.owner_mode != OwnerMode::DesktopSupervised {
        return Err(protocol_error("daemon ownership 合同不匹配"));
    }

    let endpoints = [
        (&manifest.status, "/status"),
        (&manifest.diagnostics, "/diagnostics"),
        (&manifest.imports, "/imports"),
        (&manifest.import_cancel, "/imports/cancel"),
        (&manifest.import_control, "/imports/control"),
        (&manifest.import_progress, "/imports/progress"),
        (&manifest.search, "/search"),
        (&manifest.search_batch, "/search/batch"),
        (&manifest.details, "/details"),
        (&manifest.delete, "/delete"),
    ];
    let mut addr = None;
    for (endpoint, expected_path) in endpoints {
        let endpoint_addr = parse_exact_endpoint(endpoint, expected_path)?;
        if addr.is_some_and(|expected| expected != endpoint_addr) {
            return Err(protocol_error("daemon endpoints 未绑定同一地址"));
        }
        addr = Some(endpoint_addr);
    }

    Ok(DaemonConnection {
        instance_id: manifest.instance_id,
        token: auth.token,
        addr: addr.ok_or_else(|| protocol_error("daemon endpoint 合同为空"))?,
    })
}

fn parse_exact_endpoint(endpoint: &str, expected_path: &str) -> Result<SocketAddr, DesktopError> {
    let rest = endpoint
        .strip_prefix("http://")
        .ok_or_else(|| protocol_error("daemon endpoint 必须使用 loopback HTTP"))?;
    let authority = rest
        .strip_suffix(expected_path)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| protocol_error("daemon endpoint 路径不匹配"))?;
    let addr = authority
        .parse::<SocketAddr>()
        .map_err(|_| protocol_error("daemon endpoint 地址无效"))?;
    if !addr.ip().is_loopback() || endpoint != format!("http://{addr}{expected_path}") {
        return Err(protocol_error("daemon endpoint 不是规范本机地址"));
    }
    Ok(addr)
}

fn read_owner_file(
    path: &Path,
    max_bytes: u64,
    unavailable_message: &'static str,
) -> Result<Vec<u8>, DesktopError> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err(DesktopError::new("daemon_unavailable", unavailable_message));
        }
        Err(_) => return Err(DesktopError::new("daemon_unavailable", unavailable_message)),
    };
    if metadata.file_type().is_symlink()
        || !metadata.is_file()
        || metadata.len() > max_bytes
        || !owner_only_permissions(&metadata)
    {
        return Err(protocol_error("daemon owner 文件不安全或超过上限"));
    }
    let file = File::open(path)
        .map_err(|_| DesktopError::new("daemon_unavailable", unavailable_message))?;
    let opened_metadata = file
        .metadata()
        .map_err(|_| DesktopError::new("daemon_unavailable", unavailable_message))?;
    if !opened_metadata.is_file()
        || opened_metadata.len() > max_bytes
        || !owner_only_permissions(&opened_metadata)
        || !same_file_identity(&metadata, &opened_metadata)
    {
        return Err(protocol_error("daemon owner 文件在读取期间发生变化"));
    }
    let mut bytes = Vec::new();
    file.take(max_bytes + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| DesktopError::new("daemon_unavailable", unavailable_message))?;
    if bytes.len() as u64 > max_bytes {
        return Err(protocol_error("daemon owner 文件超过桌面上限"));
    }
    Ok(bytes)
}

#[cfg(unix)]
fn owner_only_permissions(metadata: &fs::Metadata) -> bool {
    metadata.permissions().mode() & 0o077 == 0
}

#[cfg(unix)]
fn same_file_identity(expected: &fs::Metadata, opened: &fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;

    expected.dev() == opened.dev() && expected.ino() == opened.ino()
}

#[cfg(not(unix))]
fn owner_only_permissions(_metadata: &fs::Metadata) -> bool {
    true
}

#[cfg(not(unix))]
fn same_file_identity(_expected: &fs::Metadata, _opened: &fs::Metadata) -> bool {
    true
}

fn valid_generation_value(value: &str) -> bool {
    value.len() == GENERATION_VALUE_LENGTH
        && value
            .as_bytes()
            .iter()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(byte))
}

fn protocol_error(message: &'static str) -> DesktopError {
    DesktopError::new("daemon_protocol", message)
}

fn generation_changed() -> DesktopError {
    DesktopError::new(
        "daemon_generation_changed",
        "daemon 已换代，请显式重试当前操作",
    )
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EndpointManifest {
    schema_version: String,
    instance_id: String,
    owner_mode: OwnerMode,
    status: String,
    diagnostics: String,
    imports: String,
    import_cancel: String,
    import_control: String,
    import_progress: String,
    search: String,
    search_batch: String,
    details: String,
    delete: String,
}

#[derive(Clone, Copy, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
enum OwnerMode {
    Standalone,
    DesktopSupervised,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AuthManifest {
    schema_version: String,
    instance_id: String,
    token: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    const INSTANCE: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const TOKEN: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

    #[test]
    fn strict_v2_pair_accepts_one_canonical_loopback_generation() {
        let manifest = manifest(INSTANCE, "desktop_supervised", "127.0.0.1:4312");
        let auth = auth(INSTANCE, TOKEN);
        let connection =
            decode_connection(manifest.as_bytes(), auth.as_bytes(), manifest.as_bytes()).unwrap();
        assert_eq!(connection.addr(), "127.0.0.1:4312".parse().unwrap());
        assert_eq!(connection.token(), TOKEN);
    }

    #[test]
    fn legacy_or_unstructured_auth_contracts_are_rejected() {
        let legacy = manifest(INSTANCE, "desktop_supervised", "127.0.0.1:4312")
            .replace(ENDPOINT_SCHEMA, "resume-ir.daemon-ipc.v1");
        let current = manifest(INSTANCE, "desktop_supervised", "127.0.0.1:4312");
        assert!(decode_connection(
            legacy.as_bytes(),
            auth(INSTANCE, TOKEN).as_bytes(),
            legacy.as_bytes()
        )
        .is_err());
        assert!(
            decode_connection(current.as_bytes(), TOKEN.as_bytes(), current.as_bytes()).is_err()
        );
        let unknown_field = current.replacen('}', ",\"extra\":true}", 1);
        assert!(decode_connection(
            unknown_field.as_bytes(),
            auth(INSTANCE, TOKEN).as_bytes(),
            unknown_field.as_bytes()
        )
        .is_err());
        let uppercase_id = current.replace(INSTANCE, &INSTANCE.to_ascii_uppercase());
        assert!(decode_connection(
            uppercase_id.as_bytes(),
            auth(INSTANCE, TOKEN).as_bytes(),
            uppercase_id.as_bytes()
        )
        .is_err());
    }

    #[cfg(unix)]
    #[test]
    fn discovery_files_must_be_owner_only() {
        use std::os::unix::fs::PermissionsExt;

        let data_dir = tempfile::tempdir().unwrap();
        let manifest_path = data_dir.path().join(ENDPOINT_MANIFEST_FILE);
        let auth_path = data_dir.path().join(AUTH_MANIFEST_FILE);
        std::fs::write(
            &manifest_path,
            manifest(INSTANCE, "desktop_supervised", "127.0.0.1:4312"),
        )
        .unwrap();
        std::fs::write(&auth_path, auth(INSTANCE, TOKEN)).unwrap();
        std::fs::set_permissions(&manifest_path, std::fs::Permissions::from_mode(0o644)).unwrap();
        std::fs::set_permissions(&auth_path, std::fs::Permissions::from_mode(0o600)).unwrap();

        let error = match load_probe_connection(data_dir.path()) {
            Err(error) => error,
            Ok(_) => panic!("group-readable discovery manifest must be rejected"),
        };
        assert_eq!(error.code(), "daemon_protocol");
    }

    #[test]
    fn torn_or_mismatched_generation_returns_generation_error() {
        let first = manifest(INSTANCE, "desktop_supervised", "127.0.0.1:4312");
        let second_id = "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";
        let second = manifest(second_id, "desktop_supervised", "127.0.0.1:4312");
        let torn_error = match decode_connection(
            first.as_bytes(),
            auth(INSTANCE, TOKEN).as_bytes(),
            second.as_bytes(),
        ) {
            Err(error) => error,
            Ok(_) => panic!("torn manifest must be rejected"),
        };
        assert_eq!(torn_error.code(), "daemon_generation_changed");
        let mismatch_error = match decode_connection(
            first.as_bytes(),
            auth(second_id, TOKEN).as_bytes(),
            first.as_bytes(),
        ) {
            Err(error) => error,
            Ok(_) => panic!("mismatched auth generation must be rejected"),
        };
        assert_eq!(mismatch_error.code(), "daemon_generation_changed");
    }

    #[test]
    fn ownership_and_endpoint_contracts_are_exact() {
        let standalone = manifest(INSTANCE, "standalone", "127.0.0.1:4312");
        assert!(decode_connection(
            standalone.as_bytes(),
            auth(INSTANCE, TOKEN).as_bytes(),
            standalone.as_bytes()
        )
        .is_err());

        let wrong_path = manifest(INSTANCE, "desktop_supervised", "127.0.0.1:4312")
            .replace("/search/batch", "/search/wrong");
        assert!(decode_connection(
            wrong_path.as_bytes(),
            auth(INSTANCE, TOKEN).as_bytes(),
            wrong_path.as_bytes()
        )
        .is_err());

        let split_address = manifest(INSTANCE, "desktop_supervised", "127.0.0.1:4312").replace(
            "http://127.0.0.1:4312/delete",
            "http://127.0.0.1:4313/delete",
        );
        assert!(decode_connection(
            split_address.as_bytes(),
            auth(INSTANCE, TOKEN).as_bytes(),
            split_address.as_bytes()
        )
        .is_err());
    }

    fn manifest(instance_id: &str, owner_mode: &str, addr: &str) -> String {
        serde_json::to_string(&serde_json::json!({
            "schema_version": ENDPOINT_SCHEMA,
            "instance_id": instance_id,
            "owner_mode": owner_mode,
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
        }))
        .unwrap()
    }

    fn auth(instance_id: &str, token: &str) -> String {
        serde_json::to_string(&serde_json::json!({
            "schema_version": AUTH_SCHEMA,
            "instance_id": instance_id,
            "token": token,
        }))
        .unwrap()
    }
}

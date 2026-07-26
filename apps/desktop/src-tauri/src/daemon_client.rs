use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::Path;
#[cfg(test)]
use std::path::PathBuf;
use std::time::Duration;

use serde::Serialize;

use crate::daemon_connection::{self, ConnectionGenerationSource, DaemonConnection, DaemonRoute};
#[cfg(test)]
use crate::daemon_exchange::MAX_REQUEST_BYTES;
use crate::daemon_exchange::{ExpectedResponse, PreparedDaemonRequest};
use crate::daemon_request::{
    prepare_legacy_source_root_migration_request, prepare_source_root_control_request,
    prepare_source_root_delete_request, prepare_source_root_register_request,
    prepare_source_root_scan_request, Operation, RootControlAction,
};
use crate::daemon_response::project_response;

pub(crate) use crate::daemon_request::DesktopRequest;
pub(crate) use crate::daemon_response::DesktopResponse;

const MAX_RESPONSE_BYTES: u64 = 2 * 1024 * 1024;
const MAX_RESPONSE_HEADER_BYTES: usize = 16 * 1024;
const DEFAULT_RESPONSE_TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Debug, Serialize)]
pub(crate) struct DesktopError {
    code: &'static str,
    message: &'static str,
}

impl DesktopError {
    pub(crate) fn internal() -> Self {
        Self {
            code: "bridge_internal",
            message: "桌面桥接暂时不可用",
        }
    }

    pub(crate) fn new(code: &'static str, message: &'static str) -> Self {
        Self { code, message }
    }

    pub(crate) fn is_daemon_unavailable(&self) -> bool {
        self.code == "daemon_unavailable"
    }

    pub(crate) fn is_stale_generation(&self) -> bool {
        self.code == "daemon_generation_changed"
    }

    #[cfg(test)]
    pub(crate) fn code(&self) -> &'static str {
        self.code
    }
}

impl std::fmt::Display for DesktopError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.message)
    }
}

impl std::error::Error for DesktopError {}

pub(crate) fn execute_status_probe_from_with_timeout(
    data_dir: &Path,
    expected_launch_id: &str,
    response_timeout: Duration,
) -> Result<DesktopResponse, DesktopError> {
    let connection = daemon_connection::load_probe_connection(data_dir, expected_launch_id)?;
    let prepared = PreparedDaemonRequest::empty(ExpectedResponse::Status, response_timeout);
    send("GET", DaemonRoute::Status, &connection, &prepared)
}

pub(crate) fn execute_diagnostics_from(
    data_dir: &Path,
    generation_source: &impl ConnectionGenerationSource,
) -> Result<DesktopResponse, DesktopError> {
    let prepared =
        PreparedDaemonRequest::empty(ExpectedResponse::Diagnostics, DEFAULT_RESPONSE_TIMEOUT);
    daemon_connection::with_connection_lease(data_dir, generation_source, |connection| {
        send("GET", DaemonRoute::Diagnostics, connection, &prepared)
    })
}

pub(crate) fn execute_from(
    data_dir: &Path,
    generation_source: &impl ConnectionGenerationSource,
    request: DesktopRequest,
) -> Result<DesktopResponse, DesktopError> {
    let prepared = request.prepare()?;
    let operation = prepared.expected().operation();
    let (method, route) = match operation {
        Operation::Status => ("GET", DaemonRoute::Status),
        Operation::Diagnostics => ("GET", DaemonRoute::Diagnostics),
        Operation::Search => ("POST", DaemonRoute::Search),
        Operation::Detail => ("POST", DaemonRoute::Details),
        Operation::Hydrate => ("POST", DaemonRoute::Hydrate),
        Operation::PreviewCreate => ("POST", DaemonRoute::PreviewCreate),
        Operation::PreviewRange => ("POST", DaemonRoute::PreviewRange),
        Operation::PreviewClose => ("POST", DaemonRoute::PreviewClose),
        Operation::Cancel => ("POST", DaemonRoute::Cancel),
        Operation::RootControl => unreachable!("root control uses the native root path"),
        Operation::SourceRoots => unreachable!("source roots use native bridge commands"),
        Operation::RootDeletion => unreachable!("root deletion uses a native bridge command"),
    };
    daemon_connection::with_connection_lease(data_dir, generation_source, |connection| {
        send(method, route, connection, &prepared)
    })
}

pub(crate) fn execute_source_roots_list(
    data_dir: &Path,
    generation_source: &impl ConnectionGenerationSource,
) -> Result<DesktopResponse, DesktopError> {
    let prepared =
        PreparedDaemonRequest::empty(ExpectedResponse::SourceRoots, DEFAULT_RESPONSE_TIMEOUT);
    daemon_connection::with_connection_lease(data_dir, generation_source, |connection| {
        send("GET", DaemonRoute::SourceRoots, connection, &prepared)
    })
}

pub(crate) fn execute_source_root_register(
    data_dir: &Path,
    generation_source: &impl ConnectionGenerationSource,
    root: &Path,
    display_label: &str,
) -> Result<DesktopResponse, DesktopError> {
    let prepared = prepare_source_root_register_request(root, display_label)?;
    daemon_connection::with_connection_lease(data_dir, generation_source, |connection| {
        send(
            "POST",
            DaemonRoute::SourceRootRegister,
            connection,
            &prepared,
        )
    })
}

pub(crate) fn execute_legacy_source_root_migration(
    data_dir: &Path,
    generation_source: &impl ConnectionGenerationSource,
    roots: &[(&Path, &str)],
) -> Result<DesktopResponse, DesktopError> {
    let prepared = prepare_legacy_source_root_migration_request(roots)?;
    daemon_connection::with_connection_lease(data_dir, generation_source, |connection| {
        send(
            "POST",
            DaemonRoute::SourceRootLegacyMigration,
            connection,
            &prepared,
        )
    })
}

pub(crate) fn execute_source_root_scan(
    data_dir: &Path,
    generation_source: &impl ConnectionGenerationSource,
    root_id: &str,
) -> Result<DesktopResponse, DesktopError> {
    let prepared = prepare_source_root_scan_request(root_id)?;
    daemon_connection::with_connection_lease(data_dir, generation_source, |connection| {
        send("POST", DaemonRoute::SourceRootScan, connection, &prepared)
    })
}

pub(crate) fn execute_source_root_control(
    data_dir: &Path,
    generation_source: &impl ConnectionGenerationSource,
    root_id: &str,
    action: RootControlAction,
) -> Result<DesktopResponse, DesktopError> {
    let prepared = prepare_source_root_control_request(root_id, action)?;
    daemon_connection::with_connection_lease(data_dir, generation_source, |connection| {
        send(
            "POST",
            DaemonRoute::SourceRootControl,
            connection,
            &prepared,
        )
    })
}

pub(crate) fn execute_source_root_delete(
    data_dir: &Path,
    generation_source: &impl ConnectionGenerationSource,
    root_id: &str,
) -> Result<DesktopResponse, DesktopError> {
    let prepared = prepare_source_root_delete_request(root_id)?;
    daemon_connection::with_connection_lease(data_dir, generation_source, |connection| {
        send("POST", DaemonRoute::SourceRootDelete, connection, &prepared)
    })
}

fn send(
    method: &str,
    route: DaemonRoute,
    connection: &DaemonConnection,
    prepared: &PreparedDaemonRequest,
) -> Result<DesktopResponse, DesktopError> {
    let body = prepared.body();
    let mut stream = TcpStream::connect_timeout(&connection.addr(), Duration::from_millis(500))
        .map_err(|_| DesktopError::new("daemon_unavailable", "无法连接本地 daemon"))?;
    stream
        .set_read_timeout(Some(prepared.response_timeout()))
        .map_err(|_| DesktopError::new("daemon_unavailable", "无法配置本地 daemon 响应时限"))?;
    stream
        .set_write_timeout(Some(Duration::from_secs(1)))
        .map_err(|_| DesktopError::new("daemon_unavailable", "无法配置本地 daemon 请求时限"))?;
    write!(
        stream,
        "{method} {} HTTP/1.1\r\nHost: {}\r\nAuthorization: Bearer {token}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        route.path(),
        connection.addr(),
        body.len(),
        token = connection.token()
    )
    .and_then(|_| stream.write_all(body))
    .map_err(|_| DesktopError::new("daemon_unavailable", "无法发送本地 daemon 请求"))?;

    let response = read_bounded_http_response(&mut stream, MAX_RESPONSE_BYTES)?;
    parse_response(&response, prepared.expected())
}

pub(crate) fn read_bounded_http_response(
    stream: &mut TcpStream,
    max_response_bytes: u64,
) -> Result<Vec<u8>, DesktopError> {
    let max_response_bytes = usize::try_from(max_response_bytes)
        .map_err(|_| DesktopError::new("response_too_large", "daemon 响应超过桌面上限"))?;
    let mut response = Vec::new();
    let header_end = loop {
        if let Some(index) = response.windows(4).position(|window| window == b"\r\n\r\n") {
            break index;
        }
        if response.len() >= MAX_RESPONSE_HEADER_BYTES {
            return Err(DesktopError::new(
                "daemon_protocol",
                "daemon HTTP header 超过上限",
            ));
        }
        let mut chunk = [0_u8; 4096];
        let allowed = (MAX_RESPONSE_HEADER_BYTES - response.len()).min(chunk.len());
        let read = stream
            .read(&mut chunk[..allowed])
            .map_err(|_| DesktopError::new("daemon_unavailable", "本地 daemon 响应中断"))?;
        if read == 0 {
            return Err(DesktopError::new(
                "daemon_protocol",
                "daemon HTTP 响应不完整",
            ));
        }
        response.extend_from_slice(&chunk[..read]);
    };
    let body_start = header_end + 4;
    let header = std::str::from_utf8(&response[..header_end])
        .map_err(|_| DesktopError::new("daemon_protocol", "daemon HTTP header 无效"))?;
    let content_lengths = header
        .lines()
        .filter_map(|line| line.strip_prefix("Content-Length: "))
        .collect::<Vec<_>>();
    if content_lengths.len() != 1 {
        return Err(DesktopError::new(
            "daemon_protocol",
            "daemon HTTP Content-Length 无效",
        ));
    }
    let content_length = content_lengths[0]
        .parse::<usize>()
        .map_err(|_| DesktopError::new("daemon_protocol", "daemon HTTP Content-Length 无效"))?;
    if body_start
        .checked_add(content_length)
        .is_none_or(|total| total > max_response_bytes)
    {
        return Err(DesktopError::new(
            "response_too_large",
            "daemon 响应超过桌面上限",
        ));
    }
    let expected = body_start
        .checked_add(content_length)
        .ok_or_else(|| DesktopError::new("response_too_large", "daemon 响应超过桌面上限"))?;
    if response.len() > expected {
        return Err(DesktopError::new(
            "daemon_protocol",
            "daemon HTTP 响应包含多余数据",
        ));
    }
    let observed = response.len();
    response.resize(expected, 0);
    stream
        .read_exact(&mut response[observed..])
        .map_err(|_| DesktopError::new("daemon_unavailable", "本地 daemon 响应中断"))?;
    Ok(response)
}

fn parse_response(
    response: &[u8],
    expected: &ExpectedResponse,
) -> Result<DesktopResponse, DesktopError> {
    let header_end = response
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .ok_or_else(|| DesktopError::new("daemon_protocol", "daemon HTTP 响应不完整"))?;
    let header = std::str::from_utf8(&response[..header_end])
        .map_err(|_| DesktopError::new("daemon_protocol", "daemon HTTP header 无效"))?;
    let status = header
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|value| value.parse::<u16>().ok())
        .ok_or_else(|| DesktopError::new("daemon_protocol", "daemon HTTP status 无效"))?;
    project_response(status, &response[header_end + 4..], expected)
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Arc;
    use std::thread;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    const INSTANCE: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const NEXT_INSTANCE: &str = "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";
    const TOKEN: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    const LAUNCH: &str = "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd";

    struct TestGeneration(AtomicU64);

    impl TestGeneration {
        fn ready() -> Self {
            Self(AtomicU64::new(1))
        }
    }

    impl ConnectionGenerationSource for TestGeneration {
        fn ready_identity(&self) -> Option<crate::daemon_lifecycle::ReadyDaemonIdentity> {
            match self.0.load(Ordering::SeqCst) {
                0 => None,
                supervisor_generation => Some(crate::daemon_lifecycle::ReadyDaemonIdentity {
                    supervisor_generation,
                    launch_id: LAUNCH.to_string(),
                }),
            }
        }
    }

    #[test]
    fn webview_error_projection_accepts_only_the_exact_v3_shape() {
        let body = r#"{"schema_version":"resume-ir.error.v3","request_id":"synthetic-request","status":"error","error":{"code":"OVERLOADED","action":"retry","retry_after_ms":250,"capability":null,"reason":null}}"#;
        let expected = ExpectedResponse::Search {
            request_id: "synthetic-request".to_string(),
            max_results: 10,
        };
        let response = parse_response(&http_response(503, body), &expected).unwrap();
        let exposed = serde_json::to_string(&response).unwrap();

        assert!(exposed.contains("OVERLOADED"));
        assert!(exposed.contains("retry_after_ms"));
        let extra = r#"{"schema_version":"resume-ir.error.v3","request_id":"synthetic-request","status":"error","error":{"code":"OVERLOADED","action":"retry","retry_after_ms":250,"capability":null,"reason":null,"private_debug":true}}"#;
        assert!(parse_response(&http_response(503, extra), &expected).is_err());
    }

    #[test]
    fn response_projection_rejects_schema_state_confusion() {
        let body = r#"{"schema_version":"resume-ir.error.v1","status":"error","error":{"code":"BAD_REQUEST","action":"correct_request","capability":null,"reason":null}}"#;
        assert!(parse_response(&http_response(400, body), &ExpectedResponse::Status).is_err());
    }

    #[test]
    fn startup_probe_reads_strict_v5_discovery_v3_auth_and_status_v5() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let data_dir = std::env::temp_dir().join(format!(
            "resume-ir-desktop-test-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&data_dir).unwrap();
        write_connection_files(&data_dir, addr, INSTANCE, TOKEN);

        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = Vec::new();
            while !request.windows(4).any(|window| window == b"\r\n\r\n") {
                let mut chunk = [0_u8; 1024];
                let count = stream.read(&mut chunk).unwrap();
                assert!(count > 0 && request.len() + count <= 4096);
                request.extend_from_slice(&chunk[..count]);
            }
            let request = std::str::from_utf8(&request).unwrap();
            assert!(request.starts_with("GET /status HTTP/1.1"), "{request:?}");
            assert!(request.contains(&format!("Authorization: Bearer {TOKEN}")));
            let body = include_str!("../tests/fixtures/daemon-status-v5-ready.json");
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .unwrap();
        });

        let response =
            execute_status_probe_from_with_timeout(&data_dir, LAUNCH, Duration::from_secs(1))
                .unwrap();
        server.join().unwrap();
        assert_eq!(response.http_status, 200);
        let response = serde_json::to_string(&response).unwrap();
        assert!(response.contains("\"status\":\"ok\""));
        fs::remove_dir_all(data_dir).unwrap();
    }

    #[test]
    fn supervisor_generation_change_interrupts_once_without_replay() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let data_dir = temp_dir("supervisor-generation-change");
        write_connection_files(&data_dir, addr, INSTANCE, TOKEN);
        let generation = Arc::new(TestGeneration::ready());
        let server_generation = Arc::clone(&generation);
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let request = read_request_with_body(&mut stream);
            assert!(request.starts_with("GET /status HTTP/1.1"));
            server_generation.0.store(2, Ordering::SeqCst);
            write_status_response(&mut stream);
            assert_no_replay(&listener);
        });

        let error = match execute_from(&data_dir, generation.as_ref(), DesktopRequest::Status) {
            Err(error) => error,
            Ok(_) => panic!("changed supervisor generation must interrupt the request"),
        };
        assert_eq!(error.code(), "daemon_generation_changed");
        server.join().unwrap();
        fs::remove_dir_all(data_dir).unwrap();
    }

    #[test]
    fn manifest_instance_change_interrupts_once_without_replay() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let data_dir = temp_dir("manifest-generation-change");
        write_connection_files(&data_dir, addr, INSTANCE, TOKEN);
        let server_data_dir = data_dir.clone();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let request = read_request_with_body(&mut stream);
            assert!(request.starts_with("GET /status HTTP/1.1"));
            write_connection_files(&server_data_dir, addr, NEXT_INSTANCE, TOKEN);
            write_status_response(&mut stream);
            assert_no_replay(&listener);
        });

        let error = match execute_from(&data_dir, &TestGeneration::ready(), DesktopRequest::Status)
        {
            Err(error) => error,
            Ok(_) => panic!("changed manifest generation must interrupt the request"),
        };
        assert_eq!(error.code(), "daemon_generation_changed");
        server.join().unwrap();
        fs::remove_dir_all(data_dir).unwrap();
    }

    fn write_connection_files(
        data_dir: &Path,
        addr: std::net::SocketAddr,
        instance_id: &str,
        token: &str,
    ) {
        let auth_path = data_dir.join("ipc.auth");
        fs::write(
            &auth_path,
            serde_json::to_vec(&serde_json::json!({
                "schema_version": "resume-ir.daemon-auth.v3",
                "launch_id": LAUNCH,
                "instance_id": instance_id,
                "token": token,
            }))
            .unwrap(),
        )
        .unwrap();
        make_owner_only(&auth_path);

        let manifest_path = data_dir.join("ipc.endpoints.json");
        fs::write(
            &manifest_path,
            serde_json::to_vec(&serde_json::json!({
                "schema_version": "resume-ir.daemon-ipc.v5",
                "launch_id": LAUNCH,
                "instance_id": instance_id,
                "owner_mode": "desktop_supervised",
                "status": format!("http://{addr}/status"),
                "diagnostics": format!("http://{addr}/diagnostics"),
                "imports": format!("http://{addr}/imports"),
                "import_cancel": format!("http://{addr}/imports/cancel"),
                "import_control": format!("http://{addr}/imports/control"),
                "import_progress": format!("http://{addr}/imports/progress"),
                "search": format!("http://{addr}/search"),
                "search_batch": format!("http://{addr}/search/batch"),
                "details": format!("http://{addr}/details"),
                "hydrate": format!("http://{addr}/details/hydrate"),
                "delete": format!("http://{addr}/delete"),
                "source_roots": format!("http://{addr}/source-roots"),
                "source_root_register": format!("http://{addr}/source-roots/register"),
                "source_root_legacy_migration": format!("http://{addr}/source-roots/migrate-legacy"),
                "source_root_scan": format!("http://{addr}/source-roots/scan"),
                "source_root_control": format!("http://{addr}/source-roots/control"),
                "source_root_delete": format!("http://{addr}/source-roots/delete"),
                "preview_create": format!("http://{addr}/source-preview/create"),
                "preview_range": format!("http://{addr}/source-preview/read-range"),
                "preview_close": format!("http://{addr}/source-preview/close"),
                "source_reveal": format!("http://{addr}/source-reveal/resolve"),
            }))
            .unwrap(),
        )
        .unwrap();
        make_owner_only(&manifest_path);
    }

    #[cfg(unix)]
    fn make_owner_only(path: &Path) {
        use std::os::unix::fs::PermissionsExt;

        fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
    }

    #[cfg(not(unix))]
    fn make_owner_only(_path: &Path) {}

    fn write_status_response(stream: &mut std::net::TcpStream) {
        let body = include_str!("../tests/fixtures/daemon-status-v5-ready.json");
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
        .unwrap();
    }

    fn assert_no_replay(listener: &TcpListener) {
        listener.set_nonblocking(true).unwrap();
        thread::sleep(Duration::from_millis(25));
        match listener.accept() {
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(error) => panic!("unexpected replay-check failure: {error}"),
            Ok(_) => panic!("business request was replayed"),
        }
    }

    fn read_request_with_body(stream: &mut std::net::TcpStream) -> String {
        let mut request = Vec::new();
        let mut expected_length = None;
        loop {
            let mut chunk = [0_u8; 1024];
            let count = stream.read(&mut chunk).unwrap();
            assert!(count > 0 && request.len() + count <= MAX_REQUEST_BYTES);
            request.extend_from_slice(&chunk[..count]);
            if expected_length.is_none() {
                if let Some(header_end) =
                    request.windows(4).position(|window| window == b"\r\n\r\n")
                {
                    let header = std::str::from_utf8(&request[..header_end]).unwrap();
                    let body_length = header
                        .lines()
                        .find_map(|line| line.strip_prefix("Content-Length: "))
                        .unwrap()
                        .parse::<usize>()
                        .unwrap();
                    expected_length = Some(header_end + 4 + body_length);
                }
            }
            if expected_length.is_some_and(|length| request.len() >= length) {
                return String::from_utf8(request).unwrap();
            }
        }
    }

    fn http_response(status: u16, body: &str) -> Vec<u8> {
        format!(
            "HTTP/1.1 {status} synthetic\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
        .into_bytes()
    }

    fn temp_dir(label: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "resume-ir-desktop-{label}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&path).unwrap();
        path
    }
}

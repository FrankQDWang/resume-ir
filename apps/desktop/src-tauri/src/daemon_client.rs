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
    prepare_import_request, prepare_root_control_request, Operation, RootControlAction,
};
use crate::daemon_response::project_response;

pub(crate) use crate::daemon_request::DesktopRequest;
pub(crate) use crate::daemon_response::DesktopResponse;

const MAX_RESPONSE_BYTES: u64 = 2 * 1024 * 1024;
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
    response_timeout: Duration,
) -> Result<DesktopResponse, DesktopError> {
    let connection = daemon_connection::load_probe_connection(data_dir)?;
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
        Operation::Cancel => ("POST", DaemonRoute::Cancel),
        Operation::RootControl => unreachable!("root control uses the native root path"),
        Operation::Import => unreachable!("import uses the native root path"),
    };
    daemon_connection::with_connection_lease(data_dir, generation_source, |connection| {
        send(method, route, connection, &prepared)
    })
}

pub(crate) fn execute_root_control_from(
    data_dir: &Path,
    generation_source: &impl ConnectionGenerationSource,
    root: &Path,
    action: RootControlAction,
) -> Result<DesktopResponse, DesktopError> {
    let prepared = prepare_root_control_request(root, action)?;
    daemon_connection::with_connection_lease(data_dir, generation_source, |connection| {
        send("POST", DaemonRoute::ImportControl, connection, &prepared)
    })
}

pub(crate) fn execute_import_from(
    data_dir: &Path,
    generation_source: &impl ConnectionGenerationSource,
    root: &Path,
) -> Result<DesktopResponse, DesktopError> {
    let root = root
        .to_str()
        .ok_or_else(|| DesktopError::new("import_root_invalid", "所选目录无法用于本地导入"))?;
    let prepared = prepare_import_request(root)?;
    daemon_connection::with_connection_lease(data_dir, generation_source, |connection| {
        send("POST", DaemonRoute::Imports, connection, &prepared)
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

    let mut response = Vec::new();
    stream
        .take(MAX_RESPONSE_BYTES + 1)
        .read_to_end(&mut response)
        .map_err(|_| DesktopError::new("daemon_unavailable", "本地 daemon 响应中断"))?;
    if response.len() as u64 > MAX_RESPONSE_BYTES {
        return Err(DesktopError::new(
            "response_too_large",
            "daemon 响应超过桌面上限",
        ));
    }
    parse_response(&response, prepared.expected())
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

    struct TestGeneration(AtomicU64);

    impl TestGeneration {
        fn ready() -> Self {
            Self(AtomicU64::new(1))
        }
    }

    impl ConnectionGenerationSource for TestGeneration {
        fn ready_generation(&self) -> Option<u64> {
            match self.0.load(Ordering::SeqCst) {
                0 => None,
                generation => Some(generation),
            }
        }
    }

    #[test]
    fn webview_error_projection_drops_daemon_messages_and_extra_fields() {
        let body = r#"{"schema_version":"resume-ir.error.v1","request_id":"synthetic-request","status":"error","error":{"code":"OVERLOADED","action":"retry","message":"synthetic-private-message","retry_after_ms":250,"degraded_mode":"interactive_only"},"private_debug":true}"#;
        let expected = ExpectedResponse::Search {
            request_id: "synthetic-request".to_string(),
            max_results: 10,
        };
        let response = parse_response(&http_response(503, body), &expected).unwrap();
        let exposed = serde_json::to_string(&response).unwrap();

        assert!(exposed.contains("OVERLOADED"));
        assert!(exposed.contains("retry_after_ms"));
        assert!(!exposed.contains("message"));
        assert!(!exposed.contains("private_debug"));
        assert!(!exposed.contains("synthetic-private-message"));
        assert!(!exposed.contains("degraded_mode"));
    }

    #[test]
    fn response_projection_rejects_schema_state_confusion() {
        let body = r#"{"schema_version":"daemon.error.v1","status":"not_found","message":"synthetic-private-message"}"#;
        assert!(parse_response(&http_response(400, body), &ExpectedResponse::Status).is_err());
    }

    #[test]
    fn startup_probe_reads_strict_v2_pair_and_authenticates_status() {
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
            let body = r#"{"schema_version":"daemon.status.v2","status":"ok","process_state":"ready","service_state":"ready","services":{"metadata":"ready","query":"ready"},"repair_reason":null,"error":null,"indexed_documents":4,"searchable_documents":3,"partial_documents":1,"visible_epoch":7,"failed_retryable":0,"failed_permanent":0,"recovery_queue_depth":0,"ocr_queue_depth":0,"embedding_queue_depth":0,"entity_mentions":8,"import_tasks_queued":0,"index_health":"ready","latest_import_scan":null,"ipc":{"accepted":1,"completed":1,"client_disconnect":0,"request_failure":0,"response_failure":0},"private_debug":"synthetic-private-value"}"#;
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .unwrap();
        });

        let response =
            execute_status_probe_from_with_timeout(&data_dir, Duration::from_secs(1)).unwrap();
        server.join().unwrap();
        assert_eq!(response.http_status, 200);
        let response = serde_json::to_string(&response).unwrap();
        assert!(response.contains("\"status\":\"ok\""));
        assert!(!response.contains("private_debug"));
        fs::remove_dir_all(data_dir).unwrap();
    }

    #[test]
    fn native_import_sends_the_path_only_to_the_authenticated_daemon() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let data_dir = temp_dir("import-data");
        let root = temp_dir("private-import-root");
        write_connection_files(&data_dir, addr, INSTANCE, TOKEN);
        let expected_root = root.to_str().unwrap().to_string();

        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let request = read_request_with_body(&mut stream);
            assert!(request.starts_with("POST /imports HTTP/1.1"));
            assert!(request.contains(&format!("Authorization: Bearer {TOKEN}")));
            let body = request.split("\r\n\r\n").nth(1).unwrap();
            let body: serde_json::Value = serde_json::from_str(body).unwrap();
            assert_eq!(
                body,
                serde_json::json!({
                    "roots": [expected_root],
                    "profile": "explicit",
                })
            );
            let body = r#"{"schema_version":"daemon.import.v1","status":"accepted","accepted_roots":1,"new_tasks":1,"task_ids":["imp_00000000000000000000000000000000"],"scan_profile":"explicit","scan_file_limit":null}"#;
            write!(
                stream,
                "HTTP/1.1 202 Accepted\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .unwrap();
        });

        let response = execute_import_from(&data_dir, &TestGeneration::ready(), &root).unwrap();
        server.join().unwrap();
        assert_eq!(response.http_status, 202);
        let exposed = serde_json::to_string(&response).unwrap();
        assert!(exposed.contains("accepted"));
        assert!(!exposed.contains("task_ids"));
        assert!(!exposed.contains("imp_00000000000000000000000000000000"));
        assert!(!exposed.contains(root.to_str().unwrap()));
        fs::remove_dir_all(data_dir).unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn native_root_control_sends_path_only_to_daemon_and_projects_a_bounded_response() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let data_dir = temp_dir("root-control-data");
        let root = temp_dir("root-control-private-root");
        write_connection_files(&data_dir, addr, INSTANCE, TOKEN);
        let expected_root = root.to_str().unwrap().to_owned();

        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let request = read_request_with_body(&mut stream);
            assert!(request.starts_with("POST /imports/control HTTP/1.1"));
            assert!(request.contains(&format!("Authorization: Bearer {TOKEN}")));
            let body: serde_json::Value =
                serde_json::from_str(request.split("\r\n\r\n").nth(1).unwrap()).unwrap();
            assert_eq!(
                body,
                serde_json::json!({
                    "schema_version": "daemon.import_root_control_request.v1",
                    "root_path": expected_root,
                    "action": "pause",
                })
            );
            let body = format!(
                r#"{{"schema_version":"daemon.import_root_control.v1","status":"paused","changed":true,"task_cancel_requested":true,"catch_up_queued":false,"root_path":{expected_root:?},"private_debug":true}}"#
            );
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .unwrap();
        });

        let response = execute_root_control_from(
            &data_dir,
            &TestGeneration::ready(),
            &root,
            RootControlAction::Pause,
        )
        .unwrap();
        server.join().unwrap();
        let exposed = serde_json::to_string(&response).unwrap();
        assert_eq!(response.http_status, 200);
        assert!(exposed.contains("\"status\":\"paused\""));
        assert!(!exposed.contains(root.to_str().unwrap()));
        assert!(!exposed.contains("root_path"));
        assert!(!exposed.contains("private_debug"));
        fs::remove_dir_all(data_dir).unwrap();
        fs::remove_dir_all(root).unwrap();
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
                "schema_version": "resume-ir.daemon-auth.v2",
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
                "schema_version": "resume-ir.daemon-ipc.v2",
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
                "delete": format!("http://{addr}/delete"),
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
        let body = r#"{"schema_version":"daemon.status.v2","status":"ok","process_state":"ready","service_state":"ready","services":{"metadata":"ready","query":"ready"},"repair_reason":null,"error":null,"indexed_documents":4,"searchable_documents":3,"partial_documents":1,"visible_epoch":7,"failed_retryable":0,"failed_permanent":0,"recovery_queue_depth":0,"ocr_queue_depth":0,"embedding_queue_depth":0,"entity_mentions":8,"import_tasks_queued":0,"index_health":"ready","latest_import_scan":null,"ipc":{"accepted":1,"completed":1,"client_disconnect":0,"request_failure":0,"response_failure":0}}"#;
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

use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use meta_store::{DocumentId, ResumeVersionId};

mod support;

use support::{ready_daemon_status_body, write_daemon_auth, write_daemon_discovery};

const DETAIL_FIELD_LIMIT: usize = 256;
const VISIBLE_EPOCH: u64 = 17;

#[test]
fn detail_ipc_uses_complete_selection_and_correlates_the_v3_response() {
    let data_dir = temp_path("detail-ipc-data");
    let selection = test_selection("direct");
    let token_file = temp_file("detail-ipc-token");
    let token = "7878787878787878787878787878787878787878787878787878787878787878";
    write_daemon_auth(&token_file, token);
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind fake daemon");
    let addr = listener.local_addr().unwrap();
    let expected_selection = selection.clone();
    let server = thread::spawn(move || {
        let (mut stream, _) = accept_with_timeout(&listener);
        let payload = read_detail_request(&mut stream, token, &expected_selection);
        let request_id = payload["request_id"].as_str().unwrap();
        let fields = vec![
            serde_json::json!({
                "type": "degree",
                "value": "master",
                "confidence": 0.96,
                "evidence": "Master",
                "extractor": "rules-v1",
            }),
            serde_json::json!({
                "type": "email",
                "value": "candidate@example.test",
                "confidence": 0.99,
                "evidence": "candidate@example.test",
                "extractor": "rules-v1",
            }),
        ];
        let response = detail_response(request_id, &expected_selection, fields, 2, false);
        write_json_response(&mut stream, "200 OK", &response);
    });

    let output = run_detail(
        &data_dir,
        &selection,
        &format!("http://{addr}/details"),
        Some(&token_file),
    );

    server.join().expect("fake daemon joined");
    assert_success(&output);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("resume detail"));
    assert!(stdout.contains(&format!("doc_id: {}", selection.0)));
    assert!(stdout.contains(&format!("version_id: {}", selection.1)));
    assert!(stdout.contains(&format!("visible_epoch: {VISIBLE_EPOCH}")));
    assert!(stdout.contains("field: degree"));
    assert!(stdout.contains("field: email"));
    assert!(!stdout.contains("candidate@example.test"));
    assert!(!stdout.contains(token));
    assert!(!stdout.contains(path_str(&token_file)));
    assert!(!data_dir.exists());

    remove_path(&token_file);
}

#[test]
fn detail_ipc_rejects_oversized_v3_field_lists_without_printing_values() {
    assert_invalid_detail_fields(
        "oversized",
        DETAIL_FIELD_LIMIT + 1,
        DETAIL_FIELD_LIMIT + 1,
        false,
    );
}

#[test]
fn detail_ipc_rejects_inconsistent_v3_field_counts_without_printing_values() {
    assert_invalid_detail_fields("inconsistent", 1, 0, false);
}

#[test]
fn detail_ipc_auto_discovery_binds_v2_manifest_auth_and_status_generation() {
    let data_dir = temp_path("detail-ipc-auto-data");
    let selection = test_selection("auto");
    let token = "2323232323232323232323232323232323232323232323232323232323232323";
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind fake daemon");
    let addr = listener.local_addr().unwrap();
    write_daemon_discovery(&data_dir, addr, token);
    let expected_selection = selection.clone();
    let server = thread::spawn(move || {
        let (mut status_stream, _) = accept_with_timeout(&listener);
        let status_request = read_http_request(&mut status_stream);
        assert!(status_request.starts_with("GET /status HTTP/1.1"));
        assert!(!status_request.contains("Authorization:"));
        write_json_response(&mut status_stream, "200 OK", ready_daemon_status_body());
        drop(status_stream);

        let (mut detail_stream, _) = accept_with_timeout(&listener);
        let payload = read_detail_request(&mut detail_stream, token, &expected_selection);
        let request_id = payload["request_id"].as_str().unwrap();
        let response = detail_response(request_id, &expected_selection, Vec::new(), 0, false);
        write_json_response(&mut detail_stream, "200 OK", &response);
    });

    let output = run_detail(&data_dir, &selection, "auto", None);

    assert_success(&output);
    server.join().expect("fake daemon joined");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains(&format!("doc_id: {}", selection.0)));
    assert!(!stdout.contains(path_str(&data_dir)));
    assert!(!stdout.contains("ipc.auth"));
    assert!(!stdout.contains(token));
    assert!(!data_dir.join("metadata.sqlite3").exists());

    remove_path(&data_dir);
}

#[test]
fn detail_ipc_rejects_mismatched_response_context_without_printing_payload() {
    let data_dir = temp_path("detail-ipc-context-data");
    let selection = test_selection("context");
    let token_file = temp_file("detail-ipc-context-token");
    let token = "3434343434343434343434343434343434343434343434343434343434343434";
    write_daemon_auth(&token_file, token);
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind fake daemon");
    let addr = listener.local_addr().unwrap();
    let expected_selection = selection.clone();
    let server = thread::spawn(move || {
        let (mut stream, _) = accept_with_timeout(&listener);
        let _ = read_detail_request(&mut stream, token, &expected_selection);
        let fields = vec![serde_json::json!({
            "type": "skill",
            "value": "UNTRUSTED_RESPONSE_VALUE",
            "confidence": 0.9,
            "evidence": "UNTRUSTED_RESPONSE_EVIDENCE",
            "extractor": "synthetic-test",
        })];
        let response = detail_response(
            "cli-detail-wrong-generation",
            &expected_selection,
            fields,
            1,
            false,
        );
        write_json_response(&mut stream, "200 OK", &response);
    });

    let output = run_detail(
        &data_dir,
        &selection,
        &format!("http://{addr}/details"),
        Some(&token_file),
    );

    server.join().expect("fake daemon joined");
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("daemon detail ipc returned invalid protocol"));
    assert!(!stderr.contains("UNTRUSTED_RESPONSE"));
    assert!(!stderr.contains(path_str(&token_file)));
    assert!(!data_dir.exists());

    remove_path(&token_file);
}

#[test]
fn detail_cli_requires_the_full_search_selection_before_connecting() {
    let doc_id = DocumentId::from_non_secret_parts(&["s49", "missing-version"]);
    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args(["detail", "--doc-id", doc_id.as_str()])
        .output()
        .expect("run incomplete detail request");

    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("--version-id"));
    assert!(stderr.contains("--visible-epoch"));
}

fn assert_invalid_detail_fields(
    case: &'static str,
    field_count: usize,
    field_count_returned: usize,
    fields_truncated: bool,
) {
    let data_dir = temp_path(&format!("detail-ipc-{case}-data"));
    let selection = test_selection(case);
    let token_file = temp_file(&format!("detail-ipc-{case}-token"));
    let token = "5656565656565656565656565656565656565656565656565656565656565656";
    write_daemon_auth(&token_file, token);
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind fake daemon");
    let addr = listener.local_addr().unwrap();
    let expected_selection = selection.clone();
    let server = thread::spawn(move || {
        let (mut stream, _) = accept_with_timeout(&listener);
        let payload = read_detail_request(&mut stream, token, &expected_selection);
        let request_id = payload["request_id"].as_str().unwrap();
        let fields = (0..field_count)
            .map(|index| {
                serde_json::json!({
                    "type": "skill",
                    "value": format!("UNTRUSTED_FIELD_{index:03}"),
                    "confidence": 0.9,
                    "evidence": "synthetic evidence",
                    "extractor": "synthetic-test",
                })
            })
            .collect();
        let response = detail_response(
            request_id,
            &expected_selection,
            fields,
            field_count_returned,
            fields_truncated,
        );
        write_json_response(&mut stream, "200 OK", &response);
    });

    let output = run_detail(
        &data_dir,
        &selection,
        &format!("http://{addr}/details"),
        Some(&token_file),
    );

    server.join().expect("fake daemon joined");
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("daemon detail ipc returned invalid protocol"));
    assert!(!stderr.contains("UNTRUSTED_FIELD"));
    assert!(!stderr.contains(path_str(&token_file)));
    assert!(!data_dir.exists());

    remove_path(&token_file);
}

fn test_selection(label: &str) -> (DocumentId, ResumeVersionId) {
    (
        DocumentId::from_non_secret_parts(&["s49", label, "doc"]),
        ResumeVersionId::from_non_secret_parts(&["s49", label, "version"]),
    )
}

fn run_detail(
    data_dir: &Path,
    selection: &(DocumentId, ResumeVersionId),
    ipc: &str,
    token_file: Option<&Path>,
) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_resume-cli"));
    command.args([
        "--data-dir",
        path_str(data_dir),
        "detail",
        "--doc-id",
        selection.0.as_str(),
        "--version-id",
        selection.1.as_str(),
        "--visible-epoch",
        &VISIBLE_EPOCH.to_string(),
        "--ipc",
        ipc,
    ]);
    if let Some(token_file) = token_file {
        command.args(["--ipc-token-file", path_str(token_file)]);
    }
    command.output().expect("run resume-cli detail --ipc")
}

fn read_detail_request(
    stream: &mut TcpStream,
    token: &str,
    selection: &(DocumentId, ResumeVersionId),
) -> serde_json::Value {
    let request = read_http_request(stream);
    assert!(request.starts_with("POST /details HTTP/1.1"));
    assert!(request.contains(&format!("Authorization: Bearer {token}")));
    assert!(request.contains("Content-Type: application/json"));
    let body = request.split("\r\n\r\n").nth(1).unwrap_or_default();
    let payload: serde_json::Value = serde_json::from_str(body).unwrap();
    assert_eq!(payload["schema_version"], "resume-ir.detail-request.v3");
    assert!(payload["request_id"]
        .as_str()
        .is_some_and(|request_id| request_id.starts_with("cli-detail-")));
    assert_eq!(payload["selection"]["doc_id"], selection.0.as_str());
    assert_eq!(payload["selection"]["version_id"], selection.1.as_str());
    assert_eq!(payload["selection"]["visible_epoch"], VISIBLE_EPOCH);
    payload
}

fn detail_response(
    request_id: &str,
    selection: &(DocumentId, ResumeVersionId),
    fields: Vec<serde_json::Value>,
    field_count_returned: usize,
    fields_truncated: bool,
) -> String {
    serde_json::json!({
        "schema_version": "resume-ir.detail-response.v3",
        "request_id": request_id,
        "selection": {
            "doc_id": selection.0.as_str(),
            "version_id": selection.1.as_str(),
            "visible_epoch": VISIBLE_EPOCH,
        },
        "status": "ok",
        "document": {
            "source_byte_size": 2048,
            "parse_version": "parser-v1",
            "schema_version": "schema-v27",
            "language_set": ["en"],
            "page_count": 1,
            "quality_score": 0.9,
            "field_limit": DETAIL_FIELD_LIMIT,
            "field_count_total": fields.len(),
            "field_count_returned": field_count_returned,
            "fields_truncated": fields_truncated,
            "fields": fields,
            "snippet": "Java engineer candidate@example.test 155-555-0199",
        },
    })
    .to_string()
}

fn read_http_request(stream: &mut impl Read) -> String {
    let mut request = Vec::new();
    let mut buffer = [0_u8; 512];
    loop {
        let read = stream.read(&mut buffer).expect("read detail request");
        if read == 0 {
            break;
        }
        request.extend_from_slice(&buffer[..read]);
        let Some(header_end) = request
            .windows(4)
            .position(|window| window == b"\r\n\r\n")
            .map(|index| index + 4)
        else {
            continue;
        };
        let headers = String::from_utf8_lossy(&request[..header_end]);
        let content_length = headers
            .lines()
            .find_map(|line| line.strip_prefix("Content-Length: "))
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(0);
        if request.len() >= header_end + content_length {
            break;
        }
    }
    String::from_utf8_lossy(&request).into_owned()
}

fn write_json_response(stream: &mut impl Write, status: &str, body: &str) {
    write!(
        stream,
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body,
    )
    .expect("write fake daemon response");
}

fn accept_with_timeout(listener: &TcpListener) -> (TcpStream, std::net::SocketAddr) {
    listener.set_nonblocking(true).unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        match listener.accept() {
            Ok(pair) => return pair,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                assert!(Instant::now() < deadline, "timed out accepting request");
                thread::sleep(Duration::from_millis(10));
            }
            Err(error) => panic!("accept fake daemon request: {error}"),
        }
    }
}

fn assert_success(output: &Output) {
    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    assert!(output.stderr.is_empty());
}

fn temp_file(label: &str) -> PathBuf {
    let path = temp_path(label);
    let _ = fs::remove_file(&path);
    path
}

fn temp_path(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "resume-ir-s49-{label}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos(),
    ))
}

fn path_str(path: &Path) -> &str {
    path.to_str().unwrap()
}

fn remove_path(path: &Path) {
    let _ = fs::remove_file(path);
    let _ = fs::remove_dir_all(path);
}

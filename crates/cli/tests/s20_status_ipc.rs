use std::fs;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

mod support;

use support::{ready_daemon_status_body, write_daemon_auth, write_daemon_discovery};

const DEFAULT_TEST_TOKEN: &str = "abababababababababababababababababababababababababababababababab";

#[test]
fn status_can_read_redacted_daemon_status_over_loopback_ipc() {
    let token_dir = temp_dir_path("direct-status-token");
    fs::create_dir_all(&token_dir).unwrap();
    let token_file = token_dir.join("ipc.auth");
    write_daemon_auth(&token_file, DEFAULT_TEST_TOKEN);
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind fake daemon");
    listener.set_nonblocking(true).unwrap();
    let addr = listener.local_addr().unwrap();
    let server = thread::spawn(move || {
        let (mut stream, _) = accept_with_timeout(&listener);
        let request = read_http_request(&mut stream);
        assert!(request.starts_with("GET /status HTTP/1.1"));
        assert!(request.contains(&format!("Authorization: Bearer {DEFAULT_TEST_TOKEN}")));

        let mut body: serde_json::Value = serde_json::from_str(ready_daemon_status_body()).unwrap();
        body["import_tasks_cancelled"] = serde_json::json!(1);
        body["ocr_page_budget_blocked"] = serde_json::json!(1);
        body["ocr_remediation"] =
            serde_json::json!("raise OCR max pages per document or skip oversized scanned PDFs");
        body["ocr_language_unavailable"] = serde_json::json!(1);
        body["ocr_language_remediation"] = serde_json::json!(
            "install requested OCR language packs or choose an installed OCR language"
        );
        body["latest_import_scan"] = serde_json::json!({
            "scan_profile": "explicit",
            "files_discovered": 9,
            "ignored_entries": 2,
            "scan_errors": 1,
            "searchable_documents": 4,
            "ocr_required_documents": 1,
            "ocr_jobs_queued": 1,
            "failed_documents": 1,
            "deleted_documents": 0,
            "scan_budget_observed": 9,
            "scan_budget_limit": 10,
            "scan_budget_exhausted": false,
        });
        let body = body.to_string();
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        )
        .expect("write fake status response");
    });

    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "status",
            "--ipc",
            &format!("http://{addr}/status"),
            "--ipc-token-file",
            path_str(&token_file),
        ])
        .output()
        .expect("run resume-cli status --ipc");

    server.join().expect("fake daemon joined");
    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("resume-ir status"));
    assert!(stdout.contains("index health: ready"));
    assert!(stdout.contains("import tasks queued: 0"));
    assert!(stdout.contains("import tasks cancelled: 1"));
    assert!(stdout.contains("ocr page budget blocked: 1"));
    assert!(stdout.contains(
        "ocr remediation: raise OCR max pages per document or skip oversized scanned PDFs"
    ));
    assert!(stdout.contains("ocr language unavailable: 1"));
    assert!(stdout.contains(
        "ocr language remediation: install requested OCR language packs or choose an installed OCR language"
    ));
    assert!(stdout.contains("latest import files discovered: 9"));
    assert!(stdout.contains("latest import searchable documents: 4"));
    assert!(stdout.contains("latest import scan errors: 1"));
    assert!(!stdout.contains("raw_resume_text"));
    assert!(!stdout.contains("PRIVATE"));
    remove_dir(&token_dir);
}

#[test]
fn status_ipc_auto_discovers_endpoint_without_path_leak() {
    let data_dir = temp_dir_path("auto-discovery");
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind fake daemon");
    listener.set_nonblocking(true).unwrap();
    let addr = listener.local_addr().unwrap();
    write_daemon_discovery(&data_dir, addr, DEFAULT_TEST_TOKEN);
    let server = thread::spawn(move || {
        let (mut stream, _) = accept_with_timeout(&listener);
        let request = read_http_request(&mut stream);
        assert!(request.starts_with("GET /status HTTP/1.1"));
        assert!(request.contains(&format!("Authorization: Bearer {DEFAULT_TEST_TOKEN}")));

        let body = ready_daemon_status_body();
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        )
        .expect("write fake status response");
    });

    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args(["--data-dir", path_str(&data_dir), "status", "--ipc", "auto"])
        .output()
        .expect("run resume-cli status --ipc auto");

    server.join().expect("fake daemon joined");
    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("resume-ir status"));
    assert!(stdout.contains("index health: ready"));
    assert!(!stdout.contains(path_str(&data_dir)));
    assert!(!stdout.contains("ipc.auth"));
    assert!(!stdout.contains("raw_resume_text"));

    remove_dir(&data_dir);
}

#[test]
fn status_watch_import_ipc_auto_streams_redacted_progress_without_local_store() {
    let data_dir = temp_dir_path("watch-import-auto");
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind fake daemon");
    listener.set_nonblocking(true).unwrap();
    let addr = listener.local_addr().unwrap();
    let token = "cdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcd";
    write_daemon_discovery(&data_dir, addr, token);
    let server = thread::spawn(move || {
        let (mut status_stream, _) = accept_with_timeout(&listener);
        let status_request = read_http_request(&mut status_stream);
        assert!(status_request.starts_with("GET /status HTTP/1.1"));
        assert!(status_request.contains(&format!("Authorization: Bearer {token}")));
        let mut status_body: serde_json::Value =
            serde_json::from_str(ready_daemon_status_body()).unwrap();
        status_body["import_tasks_queued"] = serde_json::json!(1);
        let status_body = status_body.to_string();
        write!(
            status_stream,
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            status_body.len(),
            status_body
        )
        .expect("write fake status response");
        drop(status_stream);

        let (mut progress_stream, _) = accept_with_timeout(&listener);
        let progress_request = read_http_request(&mut progress_stream);
        assert!(progress_request.starts_with("GET /imports/progress HTTP/1.1"));
        assert!(progress_request.contains(&format!("Authorization: Bearer {token}")));
        let first = "{\"schema_version\":\"daemon.import_progress.v1\",\"event\":\"snapshot\",\"latest_import_scan\":{\"scan_profile\":\"explicit\",\"files_discovered\":7,\"ignored_entries\":1,\"scan_errors\":0,\"searchable_documents\":3,\"ocr_required_documents\":2,\"ocr_jobs_queued\":2,\"failed_documents\":0,\"deleted_documents\":0,\"scan_budget_observed\":7,\"scan_budget_limit\":9,\"scan_budget_exhausted\":false}}\n";
        let second = "{\"schema_version\":\"daemon.import_progress.v1\",\"event\":\"snapshot\",\"latest_import_scan\":{\"scan_profile\":\"explicit\",\"files_discovered\":8,\"ignored_entries\":1,\"scan_errors\":0,\"searchable_documents\":4,\"ocr_required_documents\":2,\"ocr_jobs_queued\":2,\"failed_documents\":0,\"deleted_documents\":0,\"scan_budget_observed\":8,\"scan_budget_limit\":9,\"scan_budget_exhausted\":false}}\n";
        let body = format!("{first}{second}");
        write!(
            progress_stream,
            "HTTP/1.1 200 OK\r\nContent-Type: application/x-ndjson\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        )
        .expect("write fake progress stream");
    });

    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "status",
            "--watch-import",
            "--ipc",
            "auto",
        ])
        .output()
        .expect("run resume-cli status --watch-import --ipc auto");

    server.join().expect("fake daemon joined");
    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("resume-ir import progress stream"));
    assert!(stdout.contains("latest import files discovered: 7"));
    assert!(stdout.contains("latest import files discovered: 8"));
    assert!(stdout.contains("latest import searchable documents: 4"));
    assert!(stdout.contains("latest import scan budget: 8/9 exhausted=no"));
    assert!(!stdout.contains(path_str(&data_dir)));
    assert!(!stdout.contains("ipc.auth"));
    assert!(!stdout.contains(token));
    assert!(!data_dir.join("metadata.sqlite3").exists());

    remove_dir(&data_dir);
}

#[test]
fn status_ipc_connect_failure_does_not_fallback_to_sqlite() {
    let data_dir = temp_dir_path("connect-failure");
    let token_dir = temp_dir_path("connect-failure-token");
    fs::create_dir_all(&token_dir).unwrap();
    let token_file = token_dir.join("ipc.auth");
    write_daemon_auth(&token_file, DEFAULT_TEST_TOKEN);
    let _port_reservation =
        TcpListener::bind("127.0.0.1:0").expect("reserve loopback port for connect failure");
    let status_url = reserved_unbound_loopback_status_url(&_port_reservation);
    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "status",
            "--ipc",
            &status_url,
            "--ipc-token-file",
            path_str(&token_file),
        ])
        .output()
        .expect("run resume-cli status --ipc against closed port");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("unable to connect to daemon status ipc"));
    assert!(!data_dir.exists());
    remove_dir(&token_dir);
}

#[test]
fn status_ipc_http_error_does_not_fallback_to_sqlite() {
    let data_dir = temp_dir_path("http-error");
    let token_dir = temp_dir_path("http-error-token");
    fs::create_dir_all(&token_dir).unwrap();
    let token_file = token_dir.join("ipc.auth");
    write_daemon_auth(&token_file, DEFAULT_TEST_TOKEN);
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind fake daemon");
    let addr = listener.local_addr().unwrap();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept status request");
        let request = read_http_request(&mut stream);
        assert!(request.starts_with("GET /status HTTP/1.1"));
        assert!(request.contains(&format!("Authorization: Bearer {DEFAULT_TEST_TOKEN}")));
        let body = "{\"schema_version\":\"daemon.status.v1\",\"status\":\"error\"}";
        write!(
            stream,
            "HTTP/1.1 500 Internal Server Error\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        )
        .expect("write fake status error response");
    });

    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "status",
            "--ipc",
            &format!("http://{addr}/status"),
            "--ipc-token-file",
            path_str(&token_file),
        ])
        .output()
        .expect("run resume-cli status --ipc against erroring daemon");

    server.join().expect("fake daemon joined");
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("daemon status ipc returned an error"));
    assert!(!data_dir.exists());
    remove_dir(&token_dir);
}

#[test]
fn status_ipc_direct_requires_v3_auth_token_file() {
    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args(["status", "--ipc", "http://127.0.0.1:4000/status"])
        .output()
        .expect("run resume-cli status --ipc without auth");

    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("--ipc-token-file"));
}

#[test]
fn status_ipc_rejects_non_loopback_and_wrong_path() {
    let token_dir = temp_dir_path("invalid-endpoint-token");
    fs::create_dir_all(&token_dir).unwrap();
    let token_file = token_dir.join("ipc.auth");
    write_daemon_auth(&token_file, DEFAULT_TEST_TOKEN);
    let non_loopback = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "status",
            "--ipc",
            "http://192.0.2.1:4000/status",
            "--ipc-token-file",
            path_str(&token_file),
        ])
        .output()
        .expect("run resume-cli status --ipc non-loopback");
    assert!(!non_loopback.status.success());
    assert_eq!(non_loopback.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&non_loopback.stderr).contains("loopback"));

    let wrong_path = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "status",
            "--ipc",
            "http://127.0.0.1:4000/private",
            "--ipc-token-file",
            path_str(&token_file),
        ])
        .output()
        .expect("run resume-cli status --ipc wrong path");
    assert!(!wrong_path.status.success());
    assert_eq!(wrong_path.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&wrong_path.stderr).contains("resume-cli status"));

    let wrong_progress_path = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "status",
            "--watch-import",
            "--ipc",
            "http://127.0.0.1:4000/status",
            "--ipc-token-file",
            "/tmp/resume-ir-token",
        ])
        .output()
        .expect("run resume-cli status --watch-import wrong path");
    assert!(!wrong_progress_path.status.success());
    assert_eq!(wrong_progress_path.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&wrong_progress_path.stderr).contains("resume-cli status"));
    remove_dir(&token_dir);
}

fn read_http_request(stream: &mut impl Read) -> String {
    let mut request = Vec::new();
    let mut buffer = [0_u8; 512];
    loop {
        let read = stream.read(&mut buffer).expect("read status request");
        if read == 0 {
            break;
        }
        request.extend_from_slice(&buffer[..read]);
        if request.windows(4).any(|window| window == b"\r\n\r\n") {
            break;
        }
    }
    String::from_utf8_lossy(&request).into_owned()
}

fn accept_with_timeout(listener: &TcpListener) -> (std::net::TcpStream, std::net::SocketAddr) {
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        match listener.accept() {
            Ok((stream, addr)) => {
                stream.set_nonblocking(false).unwrap();
                return (stream, addr);
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                if Instant::now() >= deadline {
                    panic!("resume-cli did not connect to fake daemon");
                }
                thread::sleep(Duration::from_millis(25));
            }
            Err(error) => panic!("accept status request: {error}"),
        }
    }
}

fn reserved_unbound_loopback_status_url(reservation: &TcpListener) -> String {
    let port = reservation.local_addr().unwrap().port();
    format!("http://127.0.0.2:{port}/status")
}

fn temp_dir_path(label: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!("resume-ir-s20-cli-{label}-{unique}"))
}

fn path_str(path: &Path) -> &str {
    path.to_str().unwrap()
}

fn remove_dir(path: &PathBuf) {
    let _ = fs::remove_dir_all(path);
}

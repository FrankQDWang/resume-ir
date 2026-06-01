use std::fs;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use meta_store::{DocumentId, ResumeVersionId};

#[test]
fn detail_ipc_submits_authenticated_request_and_renders_redacted_detail_without_local_store() {
    let data_dir = temp_path("detail-ipc-data");
    let doc_id = DocumentId::from_non_secret_parts(&["s49", "detail-ipc-doc"]);
    let version_id = ResumeVersionId::from_non_secret_parts(&["s49", "detail-ipc-version"]);
    let token_file = temp_file("detail-ipc-token");
    fs::write(
        &token_file,
        "7878787878787878787878787878787878787878787878787878787878787878\n",
    )
    .unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind fake daemon");
    let addr = listener.local_addr().unwrap();
    let expected_doc_id = doc_id.to_string();
    let expected_version_id = version_id.to_string();
    let server = thread::spawn(move || {
        let (mut stream, _) = accept_with_timeout(&listener);
        let request = read_http_request(&mut stream);
        assert!(request.starts_with("POST /details HTTP/1.1"));
        assert!(request.contains(
            "Authorization: Bearer 7878787878787878787878787878787878787878787878787878787878787878"
        ));
        assert!(request.contains("Content-Type: application/json"));
        let body = request.split("\r\n\r\n").nth(1).unwrap_or_default();
        let payload: serde_json::Value = serde_json::from_str(body).unwrap();
        assert_eq!(payload["doc_id"], expected_doc_id);

        let response = serde_json::json!({
            "schema_version": "daemon.detail.v1",
            "status": "ok",
            "document": {
                "doc_id": expected_doc_id,
                "version_id": expected_version_id,
                "file_name": "candidate@example.test-java.pdf",
                "extension": "pdf",
                "document_status": "searchable",
                "visibility": "searchable",
                "byte_size": 2048,
                "fields": [{
                    "type": "degree",
                    "value": "master",
                    "confidence": 0.96,
                    "evidence": "Master",
                    "extractor": "rules-v1"
                }, {
                    "type": "email",
                    "value": "candidate@example.test",
                    "confidence": 0.99,
                    "evidence": "candidate@example.test",
                    "extractor": "rules-v1"
                }],
                "snippet": "Java engineer candidate@example.test 155-555-0199"
            }
        })
        .to_string();
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            response.len(),
            response
        )
        .expect("write fake detail response");
    });

    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "detail",
            "--doc-id",
            doc_id.as_str(),
            "--ipc",
            &format!("http://{addr}/status"),
            "--ipc-token-file",
            path_str(&token_file),
        ])
        .output()
        .expect("run resume-cli detail --ipc");

    server.join().expect("fake daemon joined");
    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("resume detail"));
    assert!(stdout.contains(&format!("doc_id: {doc_id}")));
    assert!(stdout.contains(&format!("version_id: {version_id}")));
    assert!(stdout.contains("field: degree"));
    assert!(stdout.contains("field: email"));
    assert!(stdout.contains("snippet:"));
    assert!(!stdout.contains("candidate@example.test"));
    assert!(!stdout.contains("155-555-0199"));
    assert!(!stdout.contains(path_str(&token_file)));
    assert!(!stdout.contains("78787878"));
    assert!(!data_dir.exists());

    remove_path(&data_dir);
    remove_path(&token_file);
}

#[test]
fn detail_ipc_errors_reject_unsafe_inputs_without_local_store_or_secret_leaks() {
    let data_dir = temp_path("detail-ipc-error-data");
    let token_file = temp_valid_token("detail-ipc-error-token");
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind fake daemon");
    let addr = listener.local_addr().unwrap();
    let server = thread::spawn(move || {
        let (mut stream, _) = accept_with_timeout(&listener);
        let request = read_http_request(&mut stream);
        assert!(request.starts_with("POST /details HTTP/1.1"));
        let response = "{\"schema_version\":\"daemon.error.v1\",\"status\":\"not_found\"}";
        write!(
            stream,
            "HTTP/1.1 404 Not Found\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            response.len(),
            response
        )
        .expect("write fake detail error response");
    });

    let missing = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "detail",
            "--doc-id",
            "doc_secret",
            "--ipc",
            &format!("http://{addr}/details"),
            "--ipc-token-file",
            path_str(&token_file),
        ])
        .output()
        .expect("run resume-cli detail --ipc against erroring daemon");

    server.join().expect("fake daemon joined");
    assert!(!missing.status.success());
    assert!(missing.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&missing.stderr);
    assert!(stderr.contains("daemon detail ipc returned an error"));
    assert!(!stderr.contains("doc_secret"));
    assert!(!stderr.contains(path_str(&token_file)));
    assert!(!data_dir.exists());

    let invalid_token = temp_file("detail-ipc-invalid-token");
    fs::write(
        &invalid_token,
        "abcd\r\nX-Injected-Header: private-private-private-private-private\n",
    )
    .unwrap();
    let invalid = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "detail",
            "--doc-id",
            "doc_secret",
            "--ipc",
            "http://127.0.0.1:4000/details",
            "--ipc-token-file",
            path_str(&invalid_token),
        ])
        .output()
        .expect("run resume-cli detail --ipc invalid token");
    assert!(!invalid.status.success());
    let stderr = String::from_utf8_lossy(&invalid.stderr);
    assert!(stderr.contains("daemon detail ipc token is invalid"));
    assert!(!stderr.contains("doc_secret"));
    assert!(!stderr.contains(path_str(&invalid_token)));
    assert!(!stderr.contains("Injected"));

    let non_loopback = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "detail",
            "--doc-id",
            "doc_secret",
            "--ipc",
            "http://192.0.2.1:4000/details",
            "--ipc-token-file",
            path_str(&token_file),
        ])
        .output()
        .expect("run resume-cli detail --ipc non-loopback");
    assert!(!non_loopback.status.success());
    assert_eq!(non_loopback.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&non_loopback.stderr);
    assert!(stderr.contains("loopback"));
    assert!(!stderr.contains("doc_secret"));
    assert!(!stderr.contains(path_str(&token_file)));

    remove_path(&data_dir);
    remove_path(&token_file);
    remove_path(&invalid_token);
}

#[test]
fn detail_ipc_rejects_malformed_success_protocol_without_printing_untrusted_fields() {
    let data_dir = temp_path("detail-ipc-invalid-protocol-data");
    let doc_id = DocumentId::from_non_secret_parts(&["s49", "detail-invalid-doc"]);
    let wrong_doc_id = DocumentId::from_non_secret_parts(&["s49", "detail-invalid-other-doc"]);
    let version_id = ResumeVersionId::from_non_secret_parts(&["s49", "detail-invalid-version"]);
    let token_file = temp_valid_token("detail-ipc-invalid-protocol-token");
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind fake daemon");
    let addr = listener.local_addr().unwrap();
    let wrong_doc_id_text = wrong_doc_id.to_string();
    let version_id_text = version_id.to_string();
    let server = thread::spawn(move || {
        let (mut stream, _) = accept_with_timeout(&listener);
        let request = read_http_request(&mut stream);
        assert!(request.starts_with("POST /details HTTP/1.1"));
        let response = serde_json::json!({
            "schema_version": "daemon.detail.v1",
            "status": "ok",
            "document": {
                "doc_id": wrong_doc_id_text,
                "version_id": version_id_text,
                "file_name": "safe.pdf",
                "extension": "/Users/frank/private/extension",
                "document_status": "searchable",
                "visibility": "searchable",
                "byte_size": 1,
                "fields": [{
                    "type": "candidate@example.test",
                    "value": "master",
                    "confidence": 0.9,
                    "evidence": "Master",
                    "extractor": "rules-v1"
                }],
                "snippet": "Java"
            }
        })
        .to_string();
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            response.len(),
            response
        )
        .expect("write invalid detail response");
    });

    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "detail",
            "--doc-id",
            doc_id.as_str(),
            "--ipc",
            &format!("http://{addr}/details"),
            "--ipc-token-file",
            path_str(&token_file),
        ])
        .output()
        .expect("run resume-cli detail --ipc invalid protocol");

    server.join().expect("fake daemon joined");
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("daemon detail ipc returned invalid protocol"));
    assert!(!stderr.contains("candidate@example.test"));
    assert!(!stderr.contains("/Users/frank/private/extension"));
    assert!(!stderr.contains(wrong_doc_id.as_str()));
    assert!(!data_dir.exists());

    remove_path(&data_dir);
    remove_path(&token_file);
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
        if request.windows(4).any(|window| window == b"\r\n\r\n") {
            break;
        }
    }
    if let Some(header_end) = request
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|index| index + 4)
    {
        let headers = String::from_utf8_lossy(&request[..header_end]);
        let content_length = headers
            .lines()
            .find_map(|line| line.strip_prefix("Content-Length: "))
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(0);
        while request.len() < header_end + content_length {
            let read = stream.read(&mut buffer).expect("read detail body");
            if read == 0 {
                break;
            }
            request.extend_from_slice(&buffer[..read]);
        }
    }
    String::from_utf8_lossy(&request).into_owned()
}

fn accept_with_timeout(listener: &TcpListener) -> (std::net::TcpStream, std::net::SocketAddr) {
    listener.set_nonblocking(true).unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        match listener.accept() {
            Ok(connection) => return connection,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                if Instant::now() >= deadline {
                    panic!("resume-cli did not connect to fake daemon");
                }
                thread::sleep(Duration::from_millis(25));
            }
            Err(error) => panic!("accept detail request: {error}"),
        }
    }
}

fn temp_valid_token(label: &str) -> PathBuf {
    let token = temp_file(label);
    fs::write(
        &token,
        "9090909090909090909090909090909090909090909090909090909090909090\n",
    )
    .unwrap();
    token
}

fn temp_path(label: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!("resume-ir-s49-cli-ipc-{label}-{unique}"))
}

fn temp_file(label: &str) -> PathBuf {
    let path = temp_path(label);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    path
}

fn path_str(path: &Path) -> &str {
    path.to_str().unwrap()
}

fn remove_path(path: &PathBuf) {
    let _ = fs::remove_file(path);
    let _ = fs::remove_dir_all(path);
}

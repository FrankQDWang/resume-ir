use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use meta_store::DocumentId;

mod support;

use support::write_daemon_auth;

#[test]
fn delete_ipc_submits_authenticated_request_without_touching_local_store() {
    let data_dir = temp_path("delete-ipc-data");
    let doc_id = DocumentId::from_non_secret_parts(&["s50", "delete-ipc-doc"]);
    let token_file = temp_file("delete-ipc-token");
    write_daemon_auth(
        &token_file,
        "9090909090909090909090909090909090909090909090909090909090909090\n",
    );
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind fake daemon");
    let addr = listener.local_addr().unwrap();
    let expected_doc_id = doc_id.to_string();
    let server = thread::spawn(move || {
        let (mut stream, _) = accept_with_timeout(&listener);
        let request = read_http_request(&mut stream);
        assert!(request.starts_with("POST /delete HTTP/1.1"));
        assert!(request.contains(
            "Authorization: Bearer 9090909090909090909090909090909090909090909090909090909090909090"
        ));
        assert!(request.contains("Content-Type: application/json"));
        let body = request.split("\r\n\r\n").nth(1).unwrap_or_default();
        let payload: serde_json::Value = serde_json::from_str(body).unwrap();
        assert_eq!(payload["doc_id"], expected_doc_id);

        let response = serde_json::json!({
            "schema_version": "resume-ir.delete-response.v2",
            "status": "ok",
            "doc_id": expected_doc_id,
            "publication_committed": true,
            "indexed_documents": 2
        })
        .to_string();
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            response.len(),
            response
        )
        .expect("write fake delete response");
    });

    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "delete",
            "--doc-id",
            doc_id.as_str(),
            "--ipc",
            &format!("http://{addr}/delete"),
            "--ipc-token-file",
            path_str(&token_file),
        ])
        .output()
        .expect("run resume-cli delete --ipc");

    server.join().expect("fake daemon joined");
    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("delete completed"));
    assert!(stdout.contains(&format!("doc_id: {doc_id}")));
    assert!(stdout.contains("status: deleted"));
    assert!(stdout.contains("publication committed: true"));
    assert!(stdout.contains("indexed documents: 2"));
    assert!(!stdout.contains(path_str(&token_file)));
    assert!(!stdout.contains("90909090"));
    assert!(!data_dir.exists());

    remove_path(&data_dir);
    remove_path(&token_file);
}

#[test]
fn delete_ipc_errors_reject_unsafe_inputs_without_local_store_or_secret_leaks() {
    let data_dir = temp_path("delete-ipc-error-data");
    let token_file = temp_valid_token("delete-ipc-error-token");
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind fake daemon");
    let addr = listener.local_addr().unwrap();
    let server = thread::spawn(move || {
        let (mut stream, _) = accept_with_timeout(&listener);
        let request = read_http_request(&mut stream);
        assert!(request.starts_with("POST /delete HTTP/1.1"));
        let response = "{\"schema_version\":\"daemon.error.v1\",\"status\":\"not_found\"}";
        write!(
            stream,
            "HTTP/1.1 404 Not Found\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            response.len(),
            response
        )
        .expect("write fake delete error response");
    });

    let missing = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "delete",
            "--doc-id",
            "doc_secret",
            "--ipc",
            &format!("http://{addr}/delete"),
            "--ipc-token-file",
            path_str(&token_file),
        ])
        .output()
        .expect("run resume-cli delete --ipc against erroring daemon");

    server.join().expect("fake daemon joined");
    assert!(!missing.status.success());
    assert!(missing.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&missing.stderr);
    assert!(stderr.contains("daemon delete ipc returned an error"));
    assert!(!stderr.contains("doc_secret"));
    assert!(!stderr.contains(path_str(&token_file)));
    assert!(!data_dir.exists());

    let non_loopback = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "delete",
            "--doc-id",
            "doc_secret",
            "--ipc",
            "http://192.0.2.1:4000/delete",
            "--ipc-token-file",
            path_str(&token_file),
        ])
        .output()
        .expect("run resume-cli delete --ipc non-loopback");
    assert!(!non_loopback.status.success());
    assert_eq!(non_loopback.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&non_loopback.stderr);
    assert!(stderr.contains("loopback"));
    assert!(!stderr.contains("doc_secret"));
    assert!(!stderr.contains(path_str(&token_file)));

    remove_path(&data_dir);
    remove_path(&token_file);
}

fn read_http_request(stream: &mut impl Read) -> String {
    let mut request = Vec::new();
    let mut buffer = [0_u8; 512];
    loop {
        let read = stream.read(&mut buffer).expect("read delete request");
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
            let read = stream.read(&mut buffer).expect("read delete body");
            if read == 0 {
                break;
            }
            request.extend_from_slice(&buffer[..read]);
        }
    }
    String::from_utf8_lossy(&request).into_owned()
}

fn accept_with_timeout(listener: &TcpListener) -> (TcpStream, std::net::SocketAddr) {
    listener.set_nonblocking(true).unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        match listener.accept() {
            Ok(pair) => return pair,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                assert!(
                    Instant::now() < deadline,
                    "timed out accepting fake daemon request"
                );
                thread::sleep(Duration::from_millis(10));
            }
            Err(error) => panic!("accept fake daemon request: {error}"),
        }
    }
}

fn temp_file(label: &str) -> PathBuf {
    let path = temp_path(label);
    let _ = fs::remove_file(&path);
    path
}

fn temp_valid_token(label: &str) -> PathBuf {
    let path = temp_file(label);
    write_daemon_auth(
        &path,
        "1212121212121212121212121212121212121212121212121212121212121212\n",
    );
    path
}

fn temp_path(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "resume-ir-{label}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ))
}

fn path_str(path: &Path) -> &str {
    path.to_str().unwrap()
}

fn remove_path(path: &Path) {
    let _ = fs::remove_file(path);
    let _ = fs::remove_dir_all(path);
}

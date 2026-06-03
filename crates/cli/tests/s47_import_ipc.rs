use std::fs;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use meta_store::ImportTaskId;

#[test]
fn import_ipc_submits_authenticated_request_without_touching_local_store() {
    let data_dir = temp_path("import-ipc-data");
    let root_dir = temp_dir("import-ipc-private-root");
    let token_file = temp_file("import-ipc-token");
    fs::write(
        &token_file,
        "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc\n",
    )
    .unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind fake daemon");
    let addr = listener.local_addr().unwrap();
    let expected_root = path_str(&root_dir).to_string();
    let server = thread::spawn(move || {
        let (mut stream, _) = accept_with_timeout(&listener);
        let request = read_http_request(&mut stream);
        assert!(request.starts_with("POST /imports HTTP/1.1"));
        assert!(request.contains(
            "Authorization: Bearer cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"
        ));
        assert!(request.contains("Content-Type: application/json"));
        let body = request.split("\r\n\r\n").nth(1).unwrap_or_default();
        let payload: serde_json::Value = serde_json::from_str(body).unwrap();
        assert_eq!(
            payload
                .get("roots")
                .and_then(serde_json::Value::as_array)
                .unwrap()[0],
            expected_root
        );
        assert!(payload["root_preset"].is_null());
        assert_eq!(payload["profile"], "explicit");
        assert_eq!(payload["max_files"], 1);

        let response = "{\"schema_version\":\"daemon.import.v1\",\"status\":\"accepted\",\"accepted_roots\":1,\"new_tasks\":1,\"task_ids\":[\"imp_private\"],\"scan_profile\":\"explicit\",\"scan_file_limit\":1}";
        write!(
            stream,
            "HTTP/1.1 202 Accepted\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            response.len(),
            response
        )
        .expect("write fake import response");
    });

    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "import",
            "--ipc",
            &format!("http://{addr}/status"),
            "--ipc-token-file",
            path_str(&token_file),
            "--root",
            path_str(&root_dir),
            "--max-files",
            "1",
        ])
        .output()
        .expect("run resume-cli import --ipc");

    server.join().expect("fake daemon joined");
    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("import task submitted"));
    assert!(stdout.contains("status: queued"));
    assert!(stdout.contains("roots queued: 1"));
    assert!(stdout.contains("scan profile: explicit"));
    assert!(stdout.contains("scan file limit: 1"));
    assert!(stdout.contains("task id: imp_private"));
    assert!(!stdout.contains(path_str(&root_dir)));
    assert!(!stdout.contains(path_str(&token_file)));
    assert!(!stdout.contains("cccccccc"));
    assert!(!data_dir.exists());

    remove_path(&data_dir);
    remove_path(&root_dir);
    remove_path(&token_file);
}

#[test]
fn import_ipc_auto_discovers_endpoint_and_token_file() {
    let data_dir = temp_dir("import-ipc-auto-data");
    let root_dir = temp_dir("import-ipc-auto-private-root");
    let token = "bcbcbcbcbcbcbcbcbcbcbcbcbcbcbcbcbcbcbcbcbcbcbcbcbcbcbcbcbcbcbcbc";
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind fake daemon");
    let addr = listener.local_addr().unwrap();
    write_auto_ipc_files(&data_dir, addr, token);
    let expected_root = path_str(&root_dir).to_string();
    let server = thread::spawn(move || {
        let (mut status_stream, _) = accept_with_timeout(&listener);
        let status_request = read_http_request(&mut status_stream);
        assert!(status_request.starts_with("GET /status HTTP/1.1"));
        assert!(!status_request.contains("Authorization:"));
        write_auto_status_response(&mut status_stream);
        drop(status_stream);

        let (mut stream, _) = accept_with_timeout(&listener);
        let request = read_http_request(&mut stream);
        assert!(request.starts_with("POST /imports HTTP/1.1"));
        assert!(request.contains(&format!("Authorization: Bearer {token}")));
        assert!(request.contains("Content-Type: application/json"));
        let body = request.split("\r\n\r\n").nth(1).unwrap_or_default();
        let payload: serde_json::Value = serde_json::from_str(body).unwrap();
        assert_eq!(
            payload
                .get("roots")
                .and_then(serde_json::Value::as_array)
                .unwrap()[0],
            expected_root
        );
        assert!(payload["root_preset"].is_null());
        assert_eq!(payload["profile"], "explicit");
        assert_eq!(payload["max_files"], 1);

        let response = "{\"schema_version\":\"daemon.import.v1\",\"status\":\"accepted\",\"accepted_roots\":1,\"new_tasks\":1,\"task_ids\":[\"imp_auto\"],\"scan_profile\":\"explicit\",\"scan_file_limit\":1}";
        write!(
            stream,
            "HTTP/1.1 202 Accepted\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            response.len(),
            response
        )
        .expect("write fake import response");
    });

    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "import",
            "--ipc",
            "auto",
            "--root",
            path_str(&root_dir),
            "--max-files",
            "1",
        ])
        .output()
        .expect("run resume-cli import --ipc auto");

    server.join().expect("fake daemon joined");
    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("import task submitted"));
    assert!(stdout.contains("status: queued"));
    assert!(stdout.contains("roots queued: 1"));
    assert!(stdout.contains("task id: imp_auto"));
    assert!(!stdout.contains(path_str(&data_dir)));
    assert!(!stdout.contains(path_str(&root_dir)));
    assert!(!stdout.contains("ipc.auth"));
    assert!(!stdout.contains("bcbcbcbc"));
    assert!(!data_dir.join("metadata.sqlite3").exists());

    remove_path(&data_dir);
    remove_path(&root_dir);
}

#[test]
fn cancel_import_ipc_submits_authenticated_request_without_touching_local_store() {
    let data_dir = temp_path("cancel-import-ipc-data");
    let token_file = temp_file("cancel-import-ipc-token");
    fs::write(
        &token_file,
        "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd\n",
    )
    .unwrap();
    let task_id = ImportTaskId::from_non_secret_parts(&["s62", "cancel-import-ipc"]);
    let task_id_string = task_id.to_string();
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind fake daemon");
    let addr = listener.local_addr().unwrap();
    let expected_task_id = task_id_string.clone();
    let server = thread::spawn(move || {
        let (mut stream, _) = accept_with_timeout(&listener);
        let request = read_http_request(&mut stream);
        assert!(request.starts_with("POST /imports/cancel HTTP/1.1"));
        assert!(request.contains(
            "Authorization: Bearer dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd"
        ));
        assert!(request.contains("Content-Type: application/json"));
        let body = request.split("\r\n\r\n").nth(1).unwrap_or_default();
        let payload: serde_json::Value = serde_json::from_str(body).unwrap();
        assert_eq!(payload["task_id"], expected_task_id);

        let response = format!(
            "{{\"schema_version\":\"daemon.import_cancel.v1\",\"status\":\"cancel_requested\",\"task_id\":\"{expected_task_id}\",\"already_cancelled\":false}}"
        );
        write!(
            stream,
            "HTTP/1.1 202 Accepted\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            response.len(),
            response
        )
        .expect("write fake import cancel response");
    });

    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "cancel",
            "import",
            "--ipc",
            &format!("http://{addr}/status"),
            "--ipc-token-file",
            path_str(&token_file),
            "--task-id",
            &task_id_string,
        ])
        .output()
        .expect("run resume-cli cancel import --ipc");

    server.join().expect("fake daemon joined");
    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("import task cancelled"));
    assert!(stdout.contains(&format!("task id: {task_id_string}")));
    assert!(stdout.contains("status: cancelled"));
    assert!(!stdout.contains(path_str(&data_dir)));
    assert!(!stdout.contains(path_str(&token_file)));
    assert!(!stdout.contains("dddddddd"));
    assert!(!data_dir.exists());

    remove_path(&data_dir);
    remove_path(&token_file);
}

#[test]
fn cancel_import_ipc_auto_discovers_endpoint_and_token_file() {
    let data_dir = temp_dir("cancel-import-ipc-auto-data");
    let token = "abababababababababababababababababababababababababababababababab";
    let task_id = ImportTaskId::from_non_secret_parts(&["s62", "cancel-import-ipc-auto"]);
    let task_id_string = task_id.to_string();
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind fake daemon");
    let addr = listener.local_addr().unwrap();
    write_auto_ipc_files(&data_dir, addr, token);
    let expected_task_id = task_id_string.clone();
    let server = thread::spawn(move || {
        let (mut status_stream, _) = accept_with_timeout(&listener);
        let status_request = read_http_request(&mut status_stream);
        assert!(status_request.starts_with("GET /status HTTP/1.1"));
        assert!(!status_request.contains("Authorization:"));
        write_auto_status_response(&mut status_stream);
        drop(status_stream);

        let (mut stream, _) = accept_with_timeout(&listener);
        let request = read_http_request(&mut stream);
        assert!(request.starts_with("POST /imports/cancel HTTP/1.1"));
        assert!(request.contains(&format!("Authorization: Bearer {token}")));
        let body = request.split("\r\n\r\n").nth(1).unwrap_or_default();
        let payload: serde_json::Value = serde_json::from_str(body).unwrap();
        assert_eq!(payload["task_id"], expected_task_id);

        let response = format!(
            "{{\"schema_version\":\"daemon.import_cancel.v1\",\"status\":\"cancel_requested\",\"task_id\":\"{expected_task_id}\",\"already_cancelled\":false}}"
        );
        write!(
            stream,
            "HTTP/1.1 202 Accepted\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            response.len(),
            response
        )
        .expect("write fake import cancel response");
    });

    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "cancel",
            "import",
            "--ipc",
            "auto",
            "--task-id",
            &task_id_string,
        ])
        .output()
        .expect("run resume-cli cancel import --ipc auto");

    server.join().expect("fake daemon joined");
    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("import task cancelled"));
    assert!(stdout.contains(&format!("task id: {task_id_string}")));
    assert!(stdout.contains("status: cancelled"));
    assert!(!stdout.contains(path_str(&data_dir)));
    assert!(!stdout.contains(token));

    remove_path(&data_dir);
}

#[test]
fn import_ipc_preserves_local_discovery_preset_in_request() {
    let data_dir = temp_path("import-ipc-preset-data");
    let root_dir = temp_dir("import-ipc-preset-private-root");
    let token_file = temp_file("import-ipc-preset-token");
    fs::write(
        &token_file,
        "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee\n",
    )
    .unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind fake daemon");
    let addr = listener.local_addr().unwrap();
    let expected_root = path_str(&root_dir).to_string();
    let server = thread::spawn(move || {
        let (mut stream, _) = accept_with_timeout(&listener);
        let request = read_http_request(&mut stream);
        assert!(request.starts_with("POST /imports HTTP/1.1"));
        let body = request.split("\r\n\r\n").nth(1).unwrap_or_default();
        let payload: serde_json::Value = serde_json::from_str(body).unwrap();
        assert_eq!(
            payload
                .get("roots")
                .and_then(serde_json::Value::as_array)
                .unwrap()[0],
            expected_root
        );
        assert_eq!(payload["root_preset"], "local-discovery");
        assert_eq!(payload["profile"], "discovery");
        assert_eq!(payload["max_files"], 10_000);

        let response = "{\"schema_version\":\"daemon.import.v1\",\"status\":\"accepted\",\"accepted_roots\":1,\"new_tasks\":1,\"task_ids\":[\"imp_preset\"],\"scan_profile\":\"discovery\",\"scan_file_limit\":10000}";
        write!(
            stream,
            "HTTP/1.1 202 Accepted\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            response.len(),
            response
        )
        .expect("write fake import response");
    });

    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .env("RESUME_IR_LOCAL_DISCOVERY_ROOTS", path_str(&root_dir))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "import",
            "--ipc",
            &format!("http://{addr}/imports"),
            "--ipc-token-file",
            path_str(&token_file),
            "--root-preset",
            "local-discovery",
        ])
        .output()
        .expect("run resume-cli import --ipc local-discovery");

    server.join().expect("fake daemon joined");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("scan profile: discovery"));
    assert!(stdout.contains("scan file limit: 10000"));
    assert!(!stdout.contains(path_str(&root_dir)));
    assert!(!data_dir.exists());

    remove_path(&data_dir);
    remove_path(&root_dir);
    remove_path(&token_file);
}

#[test]
fn import_ipc_http_error_does_not_fallback_to_local_store() {
    let data_dir = temp_path("import-ipc-error-data");
    let root_dir = temp_dir("import-ipc-error-private-root");
    let token_file = temp_file("import-ipc-error-token");
    fs::write(
        &token_file,
        "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd\n",
    )
    .unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind fake daemon");
    let addr = listener.local_addr().unwrap();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept import request");
        let request = read_http_request(&mut stream);
        assert!(request.starts_with("POST /imports HTTP/1.1"));
        let response = "{\"schema_version\":\"daemon.error.v1\",\"status\":\"unauthorized\"}";
        write!(
            stream,
            "HTTP/1.1 401 Unauthorized\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            response.len(),
            response
        )
        .expect("write fake import error response");
    });

    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "import",
            "--ipc",
            &format!("http://{addr}/imports"),
            "--ipc-token-file",
            path_str(&token_file),
            "--root",
            path_str(&root_dir),
        ])
        .output()
        .expect("run resume-cli import --ipc against erroring daemon");

    server.join().expect("fake daemon joined");
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("daemon import ipc returned an error"));
    assert!(!stderr.contains(path_str(&root_dir)));
    assert!(!stderr.contains(path_str(&token_file)));
    assert!(!stderr.contains("dddddddd"));
    assert!(!data_dir.exists());

    remove_path(&data_dir);
    remove_path(&root_dir);
    remove_path(&token_file);
}

#[test]
fn import_ipc_invalid_json_response_does_not_fallback_to_local_store() {
    let data_dir = temp_path("import-ipc-invalid-json-data");
    let root_dir = temp_dir("import-ipc-invalid-json-private-root");
    let token_file = temp_file("import-ipc-invalid-json-token");
    fs::write(
        &token_file,
        "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff\n",
    )
    .unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind fake daemon");
    let addr = listener.local_addr().unwrap();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept import request");
        let request = read_http_request(&mut stream);
        assert!(request.starts_with("POST /imports HTTP/1.1"));
        let response = "not json";
        write!(
            stream,
            "HTTP/1.1 202 Accepted\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            response.len(),
            response
        )
        .expect("write fake import invalid json response");
    });

    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "import",
            "--ipc",
            &format!("http://{addr}/imports"),
            "--ipc-token-file",
            path_str(&token_file),
            "--root",
            path_str(&root_dir),
        ])
        .output()
        .expect("run resume-cli import --ipc against invalid json daemon");

    server.join().expect("fake daemon joined");
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("daemon import ipc returned invalid json"));
    assert!(!stderr.contains(path_str(&root_dir)));
    assert!(!data_dir.exists());

    remove_path(&data_dir);
    remove_path(&root_dir);
    remove_path(&token_file);
}

#[test]
fn import_ipc_transport_failure_does_not_fallback_to_local_store() {
    let data_dir = temp_path("import-ipc-transport-failure-data");
    let root_dir = temp_dir("import-ipc-transport-failure-private-root");
    let token_file = temp_file("import-ipc-transport-failure-token");
    fs::write(
        &token_file,
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\n",
    )
    .unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind fake daemon");
    let addr = listener.local_addr().unwrap();
    let server = thread::spawn(move || {
        let (mut stream, _) = accept_with_timeout(&listener);
        let request = read_http_request(&mut stream);
        assert!(request.starts_with("POST /imports HTTP/1.1"));
    });

    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "import",
            "--ipc",
            &format!("http://{addr}/imports"),
            "--ipc-token-file",
            path_str(&token_file),
            "--root",
            path_str(&root_dir),
        ])
        .output()
        .expect("run resume-cli import --ipc against dropped response");

    server.join().expect("fake daemon joined");
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("daemon import ipc response is invalid"));
    assert!(!stderr.contains(path_str(&root_dir)));
    assert!(!data_dir.exists());

    remove_path(&data_dir);
    remove_path(&root_dir);
    remove_path(&token_file);
}

#[test]
fn import_ipc_malformed_response_does_not_fallback_to_local_store() {
    let data_dir = temp_path("import-ipc-malformed-response-data");
    let root_dir = temp_dir("import-ipc-malformed-response-private-root");
    let token_file = temp_file("import-ipc-malformed-response-token");
    fs::write(
        &token_file,
        "abababababababababababababababababababababababababababababababab\n",
    )
    .unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind fake daemon");
    let addr = listener.local_addr().unwrap();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept import request");
        let request = read_http_request(&mut stream);
        assert!(request.starts_with("POST /imports HTTP/1.1"));
        write!(stream, "not an http response").expect("write malformed response");
    });

    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "import",
            "--ipc",
            &format!("http://{addr}/imports"),
            "--ipc-token-file",
            path_str(&token_file),
            "--root",
            path_str(&root_dir),
        ])
        .output()
        .expect("run resume-cli import --ipc against malformed daemon");

    server.join().expect("fake daemon joined");
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("daemon import ipc response is invalid"));
    assert!(!stderr.contains(path_str(&root_dir)));
    assert!(!data_dir.exists());

    remove_path(&data_dir);
    remove_path(&root_dir);
    remove_path(&token_file);
}

#[test]
fn import_ipc_rejects_non_loopback_and_missing_token_file_without_path_leak() {
    let root_dir = temp_dir("import-ipc-invalid-private-root");
    let missing_token = temp_path("missing-import-ipc-token");
    let invalid_token = temp_file("invalid-import-ipc-token");
    fs::write(
        &invalid_token,
        "abcd\r\nX-Injected-Header: private-private-private-private-private\n",
    )
    .unwrap();

    let non_loopback = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "import",
            "--ipc",
            "http://192.0.2.1:4000/imports",
            "--ipc-token-file",
            path_str(&missing_token),
            "--root",
            path_str(&root_dir),
        ])
        .output()
        .expect("run resume-cli import --ipc non-loopback");
    assert!(!non_loopback.status.success());
    assert_eq!(non_loopback.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&non_loopback.stderr).contains("loopback"));
    assert!(!String::from_utf8_lossy(&non_loopback.stderr).contains(path_str(&root_dir)));

    let invalid = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "import",
            "--ipc",
            "http://127.0.0.1:4000/imports",
            "--ipc-token-file",
            path_str(&invalid_token),
            "--root",
            path_str(&root_dir),
        ])
        .output()
        .expect("run resume-cli import --ipc invalid token file");
    assert!(!invalid.status.success());
    let stderr = String::from_utf8_lossy(&invalid.stderr);
    assert!(stderr.contains("daemon import ipc token is invalid"));
    assert!(!stderr.contains(path_str(&invalid_token)));
    assert!(!stderr.contains("Injected"));

    let missing = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "import",
            "--ipc",
            "http://127.0.0.1:4000/imports",
            "--ipc-token-file",
            path_str(&missing_token),
            "--root",
            path_str(&root_dir),
        ])
        .output()
        .expect("run resume-cli import --ipc missing token file");
    assert!(!missing.status.success());
    let stderr = String::from_utf8_lossy(&missing.stderr);
    assert!(stderr.contains("unable to read daemon import ipc token"));
    assert!(!stderr.contains(path_str(&missing_token)));
    assert!(!stderr.contains(path_str(&root_dir)));

    remove_path(&root_dir);
    remove_path(&invalid_token);
}

fn read_http_request(stream: &mut impl Read) -> String {
    let mut request = Vec::new();
    let mut buffer = [0_u8; 512];
    loop {
        let read = stream.read(&mut buffer).expect("read import request");
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
            let read = stream.read(&mut buffer).expect("read import body");
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
            Err(error) => panic!("accept import request: {error}"),
        }
    }
}

fn write_auto_ipc_files(data_dir: &Path, addr: SocketAddr, token: &str) {
    fs::create_dir_all(data_dir).unwrap();
    fs::write(
        data_dir.join("ipc.endpoints.json"),
        format!(
            "{{\"schema_version\":\"resume-ir.daemon-ipc.v1\",\"status\":\"http://{addr}/status\",\"imports\":\"http://{addr}/imports\",\"import_cancel\":\"http://{addr}/imports/cancel\",\"search\":\"http://{addr}/search\",\"details\":\"http://{addr}/details\"}}"
        ),
    )
    .unwrap();
    fs::write(data_dir.join("ipc.auth"), format!("{token}\n")).unwrap();
}

fn write_auto_status_response(stream: &mut impl Write) {
    let response = "{\"schema_version\":\"daemon.status.v1\",\"status\":\"ok\",\"index_health\":\"ready\",\"import_tasks_queued\":0,\"import_tasks_cancelled\":0}";
    write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        response.len(),
        response
    )
    .expect("write fake status response");
}

fn temp_path(label: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!("resume-ir-s47-cli-{label}-{unique}"))
}

fn temp_dir(label: &str) -> PathBuf {
    let path = temp_path(label);
    fs::create_dir_all(&path).unwrap();
    path
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

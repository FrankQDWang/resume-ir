use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[test]
fn status_can_read_redacted_daemon_status_over_loopback_ipc() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind fake daemon");
    listener.set_nonblocking(true).unwrap();
    let addr = listener.local_addr().unwrap();
    let server = thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(3);
        let (mut stream, _) = loop {
            match listener.accept() {
                Ok(connection) => break connection,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    if Instant::now() >= deadline {
                        panic!("resume-cli did not connect to fake daemon");
                    }
                    thread::sleep(Duration::from_millis(25));
                }
                Err(error) => panic!("accept status request: {error}"),
            }
        };
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
        let request = String::from_utf8_lossy(&request);
        assert!(request.starts_with("GET /status HTTP/1.1"));

        let body = "{\"schema_version\":\"daemon.status.v1\",\"status\":\"ok\",\"index_health\":\"ready\",\"import_tasks_queued\":0,\"import_tasks_cancelled\":1}";
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        )
        .expect("write fake status response");
    });

    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args(["status", "--ipc", &format!("http://{addr}/status")])
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
    assert!(!stdout.contains("raw_resume_text"));
    assert!(!stdout.contains("PRIVATE"));
}

#[test]
fn status_ipc_connect_failure_does_not_fallback_to_sqlite() {
    let data_dir = temp_dir_path("connect-failure");
    let status_url = unused_loopback_status_url();
    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "status",
            "--ipc",
            &status_url,
        ])
        .output()
        .expect("run resume-cli status --ipc against closed port");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("unable to connect to daemon status ipc"));
    assert!(!data_dir.exists());
}

#[test]
fn status_ipc_http_error_does_not_fallback_to_sqlite() {
    let data_dir = temp_dir_path("http-error");
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind fake daemon");
    let addr = listener.local_addr().unwrap();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept status request");
        let request = read_http_request(&mut stream);
        assert!(request.starts_with("GET /status HTTP/1.1"));
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
        ])
        .output()
        .expect("run resume-cli status --ipc against erroring daemon");

    server.join().expect("fake daemon joined");
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("daemon status ipc returned an error"));
    assert!(!data_dir.exists());
}

#[test]
fn status_ipc_rejects_non_loopback_and_wrong_path() {
    let non_loopback = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args(["status", "--ipc", "http://192.0.2.1:4000/status"])
        .output()
        .expect("run resume-cli status --ipc non-loopback");
    assert!(!non_loopback.status.success());
    assert_eq!(non_loopback.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&non_loopback.stderr).contains("loopback"));

    let wrong_path = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args(["status", "--ipc", "http://127.0.0.1:4000/private"])
        .output()
        .expect("run resume-cli status --ipc wrong path");
    assert!(!wrong_path.status.success());
    assert_eq!(wrong_path.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&wrong_path.stderr).contains("resume-cli status [--ipc"));
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

fn unused_loopback_status_url() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind unused loopback port");
    let addr = listener.local_addr().unwrap();
    drop(listener);
    format!("http://{addr}/status")
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

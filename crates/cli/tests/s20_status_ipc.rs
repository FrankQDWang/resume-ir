use std::fs;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener};
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
fn status_ipc_auto_discovers_endpoint_without_path_leak() {
    let data_dir = temp_dir_path("auto-discovery");
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind fake daemon");
    listener.set_nonblocking(true).unwrap();
    let addr = listener.local_addr().unwrap();
    write_ipc_endpoint_file(&data_dir, addr);
    let server = thread::spawn(move || {
        let (mut stream, _) = accept_with_timeout(&listener);
        let request = read_http_request(&mut stream);
        assert!(request.starts_with("GET /status HTTP/1.1"));

        let body = "{\"schema_version\":\"daemon.status.v1\",\"status\":\"ok\",\"index_health\":\"ready\",\"import_tasks_queued\":0,\"import_tasks_cancelled\":0}";
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

fn write_ipc_endpoint_file(data_dir: &Path, addr: SocketAddr) {
    fs::create_dir_all(data_dir).unwrap();
    fs::write(
        data_dir.join("ipc.endpoints.json"),
        format!(
            "{{\"schema_version\":\"resume-ir.daemon-ipc.v1\",\"status\":\"http://{addr}/status\",\"imports\":\"http://{addr}/imports\",\"search\":\"http://{addr}/search\",\"details\":\"http://{addr}/details\"}}"
        ),
    )
    .unwrap();
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

fn remove_dir(path: &PathBuf) {
    let _ = fs::remove_dir_all(path);
}

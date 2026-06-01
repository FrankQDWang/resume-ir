use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use meta_store::{IndexState, IndexStateStatus, MetaStore, UnixTimestamp};

#[test]
fn daemon_serves_redacted_status_over_loopback_ipc() {
    let data_dir = temp_dir("ipc-status-data");
    seed_snapshot_state(&data_dir);
    let mut child = Command::new(env!("CARGO_BIN_EXE_resume-daemon"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "run",
            "--foreground",
            "--ipc-listen",
            "127.0.0.1:0",
            "--max-requests",
            "1",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start resume-daemon ipc");

    let stdout = child.stdout.take().expect("daemon stdout");
    let mut stdout = BufReader::new(stdout);
    let endpoint = read_ipc_endpoint(&mut stdout);
    let response = http_get(&endpoint);

    assert!(response.contains("HTTP/1.1 200 OK"));
    assert!(response.contains("\"schema_version\":\"daemon.status.v1\""));
    assert!(response.contains("\"status\":\"ok\""));
    assert!(response.contains("\"index_health\":\"ready\""));
    assert!(response.contains("\"import_tasks_queued\":0"));
    assert!(response.contains("\"snapshot_present\":true"));
    assert!(!response.contains(path_str(&data_dir)));
    assert!(!response.contains("PRIVATE_SNAPSHOT_TOKEN"));
    assert!(!response.contains("PRIVATE_MANIFEST"));
    assert!(!response.contains("last_snapshot_id"));
    assert!(!response.contains("raw_resume_text"));

    let output = wait_child(child);
    assert!(output.success, "stderr:\n{}", output.stderr);
    assert!(output.stderr.is_empty());

    remove_dir(&data_dir);
}

#[test]
fn daemon_rejects_non_loopback_ipc_bind() {
    let data_dir = temp_dir("ipc-non-loopback-data");
    let output = Command::new(env!("CARGO_BIN_EXE_resume-daemon"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "run",
            "--foreground",
            "--ipc-listen",
            "0.0.0.0:0",
            "--max-requests",
            "1",
        ])
        .output()
        .expect("run resume-daemon with non-loopback ipc");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("ipc listener must bind to loopback"));

    remove_dir(&data_dir);
}

#[test]
fn daemon_returns_404_for_non_status_ipc_path() {
    let data_dir = temp_dir("ipc-wrong-path-data");
    let mut child = Command::new(env!("CARGO_BIN_EXE_resume-daemon"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "run",
            "--foreground",
            "--ipc-listen",
            "127.0.0.1:0",
            "--max-requests",
            "1",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start resume-daemon ipc");

    let stdout = child.stdout.take().expect("daemon stdout");
    let mut stdout = BufReader::new(stdout);
    let endpoint = read_ipc_endpoint(&mut stdout);
    let response = http_get_path(&endpoint, "/not-status");

    assert!(response.contains("HTTP/1.1 404 Not Found"));
    assert!(!response.contains(path_str(&data_dir)));
    assert!(!response.contains("raw_resume_text"));

    let output = wait_child(child);
    assert!(output.success, "stderr:\n{}", output.stderr);
    assert!(output.stderr.is_empty());

    remove_dir(&data_dir);
}

fn read_ipc_endpoint(stdout: &mut BufReader<impl Read>) -> String {
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut line = String::new();

    while Instant::now() < deadline {
        line.clear();
        let bytes = stdout.read_line(&mut line).expect("read daemon stdout");
        if bytes == 0 {
            continue;
        }
        if let Some(endpoint) = line.trim().strip_prefix("ipc status endpoint: ") {
            return endpoint.to_string();
        }
    }

    panic!("daemon did not print ipc status endpoint");
}

fn http_get(endpoint: &str) -> String {
    let rest = endpoint
        .strip_prefix("http://")
        .expect("endpoint has http scheme");
    let (_addr, path) = rest.split_once('/').expect("endpoint has path");
    http_get_path(endpoint, &format!("/{path}"))
}

fn http_get_path(endpoint: &str, request_path: &str) -> String {
    let rest = endpoint
        .strip_prefix("http://")
        .expect("endpoint has http scheme");
    let (addr, _path) = rest.split_once('/').expect("endpoint has path");
    let mut stream = TcpStream::connect(addr).expect("connect daemon ipc");
    write!(
        stream,
        "GET {request_path} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n"
    )
    .expect("write request");
    let mut response = String::new();
    stream.read_to_string(&mut response).expect("read response");
    response
}

fn seed_snapshot_state(data_dir: &Path) {
    let store = MetaStore::open(data_dir.join("metadata.sqlite3")).unwrap();
    store.run_migrations().unwrap();
    store
        .upsert_index_state(&IndexState {
            manifest_version: "PRIVATE_MANIFEST".to_string(),
            snapshot_token: Some("PRIVATE_SNAPSHOT_TOKEN".to_string()),
            status: IndexStateStatus::Ready,
            updated_at: UnixTimestamp::from_unix_seconds(1_800_000_000),
        })
        .unwrap();
}

struct ChildOutput {
    success: bool,
    stderr: String,
}

fn wait_child(mut child: Child) -> ChildOutput {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(status) = child.try_wait().expect("poll child") {
            let mut stderr = String::new();
            child
                .stderr
                .take()
                .expect("daemon stderr")
                .read_to_string(&mut stderr)
                .expect("read daemon stderr");
            return ChildOutput {
                success: status.success(),
                stderr,
            };
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            panic!("daemon did not exit after max requests");
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

fn temp_dir(label: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!("resume-ir-s20-daemon-{label}-{unique}"));
    fs::create_dir_all(&path).unwrap();
    path
}

fn path_str(path: &Path) -> &str {
    path.to_str().unwrap()
}

fn remove_dir(path: &PathBuf) {
    let _ = fs::remove_dir_all(path);
}

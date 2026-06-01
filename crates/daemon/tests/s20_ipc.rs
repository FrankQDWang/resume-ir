use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use meta_store::{
    ImportRootKind, ImportScanProfile, ImportScanScope, ImportTask, ImportTaskId, ImportTaskStatus,
    IndexState, IndexStateStatus, MetaStore, UnixTimestamp,
};

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
    let endpoint = read_ipc_endpoint(&mut child, &mut stdout);
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
    let endpoint = read_ipc_endpoint(&mut child, &mut stdout);
    let response = http_get_path(&endpoint, "/not-status");

    assert!(response.contains("HTTP/1.1 404 Not Found"));
    assert!(!response.contains(path_str(&data_dir)));
    assert!(!response.contains("raw_resume_text"));

    let output = wait_child(child);
    assert!(output.success, "stderr:\n{}", output.stderr);
    assert!(output.stderr.is_empty());

    remove_dir(&data_dir);
}

#[test]
fn daemon_serves_status_while_import_worker_processes_late_queued_task() {
    let data_dir = temp_dir("ipc-import-worker-data");
    let fixture_root = fixture_root();
    let canonical_fixture_root = fs::canonicalize(&fixture_root).unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_resume-daemon"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "run",
            "--foreground",
            "--work-imports",
            "--worker-interval-ms",
            "25",
            "--ipc-listen",
            "127.0.0.1:0",
            "--max-requests",
            "40",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start resume-daemon ipc plus import worker");

    let stdout = child.stdout.take().expect("daemon stdout");
    let mut stdout = BufReader::new(stdout);
    let endpoint = read_ipc_endpoint(&mut child, &mut stdout);
    let initial_response = http_get(&endpoint);
    assert!(initial_response.contains("HTTP/1.1 200 OK"));
    assert!(initial_response.contains("\"searchable_documents\":0"));

    let task_id = seed_queued_import_task(
        &data_dir,
        "ipc-import-worker",
        &canonical_fixture_root,
        1_700_000_000,
    );
    let (worker_requests, completed_response) = wait_for_searchable_documents(&endpoint, 2, 39);
    let used_requests = 1 + worker_requests;
    drain_status_requests(&endpoint, 40 - used_requests);

    let output = wait_child(child);
    assert!(output.success, "stderr:\n{}", output.stderr);
    assert!(output.stderr.is_empty());

    let store = MetaStore::open(data_dir.join("metadata.sqlite3")).unwrap();
    store.run_migrations().unwrap();
    let task = store.import_task_by_id(&task_id).unwrap().unwrap();
    assert_eq!(task.status, ImportTaskStatus::Completed);
    assert_eq!(store.status_summary().unwrap().searchable_documents, 2);
    assert!(!initial_response.contains(path_str(&data_dir)));
    assert!(!initial_response.contains(path_str(&fixture_root)));
    assert!(!initial_response.contains(path_str(&canonical_fixture_root)));
    assert!(!completed_response.contains(path_str(&data_dir)));
    assert!(!completed_response.contains(path_str(&fixture_root)));
    assert!(!completed_response.contains(path_str(&canonical_fixture_root)));

    remove_dir(&data_dir);
}

#[test]
fn daemon_does_not_start_import_worker_when_ipc_bind_fails() {
    let data_dir = temp_dir("ipc-bind-failure-worker-data");
    let fixture_root = fixture_root();
    let canonical_fixture_root = fs::canonicalize(&fixture_root).unwrap();
    let task_id = seed_queued_import_task(
        &data_dir,
        "ipc-bind-failure-worker",
        &canonical_fixture_root,
        1_700_000_000,
    );
    let blocker = TcpListener::bind("127.0.0.1:0").expect("bind blocker listener");
    let blocked_addr = blocker.local_addr().unwrap().to_string();

    let output = Command::new(env!("CARGO_BIN_EXE_resume-daemon"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "run",
            "--foreground",
            "--work-imports",
            "--worker-interval-ms",
            "25",
            "--ipc-listen",
            &blocked_addr,
        ])
        .output()
        .expect("run resume-daemon combined mode with occupied ipc port");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("unable to bind daemon ipc listener"));
    let store = MetaStore::open(data_dir.join("metadata.sqlite3")).unwrap();
    store.run_migrations().unwrap();
    let task = store.import_task_by_id(&task_id).unwrap().unwrap();
    assert_eq!(task.status, ImportTaskStatus::Queued);
    assert_eq!(store.status_summary().unwrap().searchable_documents, 0);

    remove_dir(&data_dir);
}

#[test]
fn daemon_rejects_worker_tick_limit_in_combined_ipc_worker_mode() {
    let data_dir = temp_dir("ipc-worker-tick-limit-data");
    let output = Command::new(env!("CARGO_BIN_EXE_resume-daemon"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "run",
            "--foreground",
            "--work-imports",
            "--max-worker-ticks",
            "1",
            "--ipc-listen",
            "127.0.0.1:0",
        ])
        .output()
        .expect("run resume-daemon combined mode with worker tick limit");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("--max-worker-ticks cannot be combined with --ipc-listen"));

    remove_dir(&data_dir);
}

fn read_ipc_endpoint(child: &mut Child, stdout: &mut BufReader<impl Read>) -> String {
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

    let _ = child.kill();
    let _ = child.wait();
    panic!("daemon did not print ipc status endpoint");
}

fn wait_for_searchable_documents(
    endpoint: &str,
    expected: usize,
    max_requests: usize,
) -> (usize, String) {
    for request_count in 1..=max_requests {
        let response = http_get(endpoint);
        assert!(response.contains("HTTP/1.1 200 OK"));
        if response.contains(&format!("\"searchable_documents\":{expected}")) {
            assert!(response.contains("\"import_tasks_queued\":0"));
            assert!(!response.contains("raw_resume_text"));
            return (request_count, response);
        }
        std::thread::sleep(Duration::from_millis(25));
    }

    panic!("daemon status did not report searchable document count {expected}");
}

fn drain_status_requests(endpoint: &str, count: usize) {
    for _ in 0..count {
        let _ = http_get(endpoint);
    }
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

fn seed_queued_import_task(
    data_dir: &Path,
    label: &str,
    canonical_root: &Path,
    queued_at_seconds: i64,
) -> ImportTaskId {
    let store = MetaStore::open(data_dir.join("metadata.sqlite3")).unwrap();
    store.run_migrations().unwrap();
    let now = UnixTimestamp::from_unix_seconds(queued_at_seconds);
    let task_id = ImportTaskId::from_non_secret_parts(&["s45", label]);
    store
        .insert_import_task(&ImportTask {
            id: task_id.clone(),
            root_path: path_str(canonical_root).to_string(),
            status: ImportTaskStatus::Queued,
            queued_at: now,
            started_at: None,
            finished_at: None,
            updated_at: now,
        })
        .unwrap();
    store
        .upsert_import_scan_scope(&ImportScanScope {
            import_task_id: task_id.clone(),
            root_kind: ImportRootKind::Explicit,
            root_preset: None,
            scan_profile: ImportScanProfile::Explicit,
            requested_root_path: path_str(canonical_root).to_string(),
            canonical_root_path: path_str(canonical_root).to_string(),
            files_discovered: 0,
            ignored_entries: 0,
            scan_errors: 0,
            searchable_documents: 0,
            ocr_required_documents: 0,
            ocr_jobs_queued: 0,
            failed_documents: 0,
            deleted_documents: 0,
            scan_budget_kind: None,
            scan_budget_limit: None,
            scan_budget_observed: None,
            scan_budget_exhausted: false,
            updated_at: now,
        })
        .unwrap();
    task_id
}

fn fixture_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("tests/fixtures/resumes")
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
            let _ = child.wait();
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

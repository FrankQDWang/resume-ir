use std::fs;
use std::io::{self, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use meta_store::{
    ImportRootKind, ImportRootPreset, ImportScanProfile, ImportScanScope, ImportTask, ImportTaskId,
    ImportTaskStatus, IndexState, IndexStateStatus, MetaStore, UnixTimestamp,
};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

#[cfg(windows)]
const IPC_ENDPOINT_TIMEOUT: Duration = Duration::from_secs(30);
#[cfg(not(windows))]
const IPC_ENDPOINT_TIMEOUT: Duration = Duration::from_secs(10);
const IPC_ENDPOINT_POLL_DELAY: Duration = Duration::from_millis(25);
const IMPORT_WORKER_STATUS_REQUEST_LIMIT: usize = 320;
const IMPORT_WORKER_SEARCHABLE_MAX_REQUESTS: usize = 260;
const IMPORT_WORKER_SEARCHABLE_TIMEOUT: Duration = Duration::from_secs(20);
const IMPORT_WORKER_STATUS_POLL_DELAY: Duration = Duration::from_millis(50);

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
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start resume-daemon ipc");

    let endpoint = read_ipc_endpoint(&mut child, &data_dir);
    let token = read_ipc_auth_token(&data_dir);
    let endpoint_manifest_path = data_dir.join("ipc.endpoints.json");
    let endpoint_manifest =
        fs::read_to_string(&endpoint_manifest_path).expect("read daemon ipc endpoint manifest");
    let endpoint_manifest_json: serde_json::Value =
        serde_json::from_str(&endpoint_manifest).expect("parse daemon ipc endpoint manifest");
    let base_endpoint = endpoint.strip_suffix("/status").unwrap();
    assert_eq!(
        endpoint_manifest_json["schema_version"],
        "resume-ir.daemon-ipc.v1"
    );
    assert_eq!(endpoint_manifest_json["status"], endpoint);
    assert_eq!(
        endpoint_manifest_json["imports"],
        format!("{base_endpoint}/imports")
    );
    assert_eq!(
        endpoint_manifest_json["import_cancel"],
        format!("{base_endpoint}/imports/cancel")
    );
    assert_eq!(
        endpoint_manifest_json["import_progress"],
        format!("{base_endpoint}/imports/progress")
    );
    assert_eq!(
        endpoint_manifest_json["search"],
        format!("{base_endpoint}/search")
    );
    assert_eq!(
        endpoint_manifest_json["details"],
        format!("{base_endpoint}/details")
    );
    assert!(!endpoint_manifest.contains(path_str(&data_dir)));
    assert!(!endpoint_manifest.contains("ipc.auth"));
    assert!(!endpoint_manifest.contains(&token));
    assert!(!endpoint_manifest.contains("raw_resume_text"));
    let response = http_get(&endpoint);

    assert!(response.contains("HTTP/1.1 200 OK"));
    assert!(response.contains("\"schema_version\":\"daemon.status.v1\""));
    assert!(response.contains("\"status\":\"ok\""));
    assert!(response.contains("\"index_health\":\"ready\""));
    assert!(response.contains("\"import_tasks_queued\":0"));
    assert!(response.contains("\"import_tasks_cancelled\":0"));
    assert!(response.contains("\"ocr_language_unavailable\":0"));
    assert!(response.contains("\"snapshot_present\":true"));
    assert!(!response.contains(path_str(&data_dir)));
    assert!(!response.contains("PRIVATE_SNAPSHOT_TOKEN"));
    assert!(!response.contains("PRIVATE_MANIFEST"));
    assert!(!response.contains("last_snapshot_id"));
    assert!(!response.contains("raw_resume_text"));

    let output = wait_child(child);
    assert!(output.success, "stderr:\n{}", output.stderr);
    assert!(output.stderr.is_empty());
    assert!(!endpoint_manifest_path.exists());

    remove_dir(&data_dir);
}

#[test]
fn daemon_streams_redacted_import_progress_over_loopback_ipc() {
    let data_dir = temp_dir("ipc-import-progress-data");
    let fixture_root = fixture_root();
    let canonical_fixture_root = fs::canonicalize(&fixture_root).unwrap();
    let task_id = seed_running_import_task(
        &data_dir,
        "progress-stream",
        &canonical_fixture_root,
        1_800_040_000,
    );
    seed_import_progress_scope(&data_dir, &task_id, &canonical_fixture_root);
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
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start resume-daemon ipc");

    let endpoint = read_ipc_endpoint(&mut child, &data_dir);
    let token = read_ipc_auth_token(&data_dir);
    let response = http_get_import_progress(&endpoint, Some(&token));

    assert!(response.contains("HTTP/1.1 200 OK"));
    assert!(response.contains("Content-Type: application/x-ndjson"));
    assert!(response.contains("\"schema_version\":\"daemon.import_progress.v1\""));
    assert!(response.contains("\"event\":\"snapshot\""));
    assert!(response.contains("\"files_discovered\":42"));
    assert!(response.contains("\"searchable_documents\":13"));
    assert!(response.contains("\"scan_budget_observed\":42"));
    assert!(response.contains("\"scan_budget_limit\":100"));
    assert!(!response.contains(path_str(&data_dir)));
    assert!(!response.contains(path_str(&fixture_root)));
    assert!(!response.contains(path_str(&canonical_fixture_root)));
    assert!(!response.contains(&token));

    let output = wait_child(child);
    assert!(output.success, "stderr:\n{}", output.stderr);
    assert!(output.stderr.is_empty());

    remove_dir(&data_dir);
}

#[cfg(unix)]
#[test]
fn daemon_rejects_symlinked_ipc_endpoint_manifest_without_clobbering_target() {
    let data_dir = temp_dir("ipc-endpoint-symlink-data");
    let target = data_dir.join("private-target.txt");
    fs::write(&target, "PRIVATE_TARGET_CONTENT").unwrap();
    std::os::unix::fs::symlink(&target, data_dir.join("ipc.endpoints.json")).unwrap();

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
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("start resume-daemon ipc with symlinked endpoint manifest");

    let deadline = Instant::now() + Duration::from_secs(2);
    let mut exited = false;
    while Instant::now() < deadline {
        if child.try_wait().expect("poll daemon child").is_some() {
            exited = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    if !exited {
        let _ = child.kill();
        let _ = child.wait();
    }
    assert!(exited, "daemon should reject symlinked endpoint manifest");
    let status = child.wait().expect("wait daemon child");
    assert!(!status.success());
    assert_eq!(
        fs::read_to_string(&target).unwrap(),
        "PRIVATE_TARGET_CONTENT"
    );

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
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start resume-daemon ipc");

    let endpoint = read_ipc_endpoint(&mut child, &data_dir);
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
fn daemon_requires_bearer_token_for_import_command_ipc() {
    let data_dir = temp_dir("ipc-import-auth-required-data");
    let fixture_root = fixture_root();
    let canonical_fixture_root = fs::canonicalize(&fixture_root).unwrap();
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
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start resume-daemon ipc");

    let endpoint = read_ipc_endpoint(&mut child, &data_dir);
    let response = http_post_import_command(&endpoint, None, &fixture_root, Some(1));

    assert!(response.contains("HTTP/1.1 401 Unauthorized"));
    assert!(response.contains("\"status\":\"unauthorized\""));
    assert!(!response.contains(path_str(&data_dir)));
    assert!(!response.contains(path_str(&fixture_root)));
    assert!(!response.contains(path_str(&canonical_fixture_root)));
    assert!(!response.contains("raw_resume_text"));

    let output = wait_child(child);
    assert!(output.success, "stderr:\n{}", output.stderr);
    assert!(output.stderr.is_empty());

    let store = MetaStore::open_data_dir(&data_dir).unwrap();
    store.run_migrations().unwrap();
    assert_eq!(store.status_summary().unwrap().import_tasks_queued, 0);

    remove_dir(&data_dir);
}

#[test]
fn daemon_authenticates_and_queues_import_command_over_ipc() {
    let data_dir = temp_dir("ipc-import-command-data");
    let fixture_root = fixture_root();
    let canonical_fixture_root = fs::canonicalize(&fixture_root).unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_resume-daemon"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "run",
            "--foreground",
            "--ipc-listen",
            "127.0.0.1:0",
            "--max-requests",
            "2",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start resume-daemon ipc");

    let endpoint = read_ipc_endpoint(&mut child, &data_dir);
    let token = read_ipc_auth_token(&data_dir);
    let response = http_post_import_command(&endpoint, Some(&token), &fixture_root, Some(1));
    let status_response = http_get(&endpoint);

    assert!(response.contains("HTTP/1.1 202 Accepted"));
    assert!(response.contains("\"schema_version\":\"daemon.import.v1\""));
    assert!(response.contains("\"status\":\"accepted\""));
    assert!(response.contains("\"accepted_roots\":1"));
    assert!(response.contains("\"new_tasks\":1"));
    assert!(response.contains("\"scan_profile\":\"explicit\""));
    assert!(response.contains("\"scan_file_limit\":1"));
    assert!(!response.contains(path_str(&data_dir)));
    assert!(!response.contains(path_str(&fixture_root)));
    assert!(!response.contains(path_str(&canonical_fixture_root)));
    assert!(!response.contains(&token));
    assert!(!response.contains("PRIVATE"));
    assert!(status_response.contains("\"import_tasks_queued\":1"));
    assert!(status_response.contains("\"latest_import_scan\""));
    assert!(status_response.contains("\"files_discovered\":0"));
    assert!(status_response.contains("\"scan_profile\":\"explicit\""));
    assert!(!status_response.contains(path_str(&data_dir)));
    assert!(!status_response.contains(path_str(&fixture_root)));
    assert!(!status_response.contains(path_str(&canonical_fixture_root)));

    let output = wait_child(child);
    assert!(output.success, "stderr:\n{}", output.stderr);
    assert!(output.stderr.is_empty());

    let store = MetaStore::open_data_dir(&data_dir).unwrap();
    store.run_migrations().unwrap();
    let summary = store.status_summary().unwrap();
    assert_eq!(summary.import_tasks_queued, 1);
    let scope = store.latest_import_scan_scope().unwrap().unwrap();
    assert_eq!(scope.root_kind, ImportRootKind::Explicit);
    assert_eq!(scope.scan_profile, ImportScanProfile::Explicit);
    assert_eq!(scope.requested_root_path, path_str(&fixture_root));
    assert_eq!(scope.canonical_root_path, path_str(&canonical_fixture_root));
    assert_eq!(scope.scan_budget_limit, Some(1));
    let task = store
        .import_task_by_id(&scope.import_task_id)
        .unwrap()
        .unwrap();
    assert_eq!(task.status, ImportTaskStatus::Queued);

    remove_dir(&data_dir);
}

#[test]
fn daemon_import_command_can_requeue_root_after_prior_task_cancelled() {
    let data_dir = temp_dir("ipc-import-command-cancel-requeue-data");
    let fixture_root = fixture_root();
    let canonical_fixture_root = fs::canonicalize(&fixture_root).unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_resume-daemon"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "run",
            "--foreground",
            "--ipc-listen",
            "127.0.0.1:0",
            "--max-requests",
            "2",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start resume-daemon ipc");

    let endpoint = read_ipc_endpoint(&mut child, &data_dir);
    let token = read_ipc_auth_token(&data_dir);
    let first_response = http_post_import_command(&endpoint, Some(&token), &fixture_root, Some(1));
    assert!(first_response.contains("HTTP/1.1 202 Accepted"));
    assert!(first_response.contains("\"new_tasks\":1"));

    let store = MetaStore::open_data_dir(&data_dir).unwrap();
    store.run_migrations().unwrap();
    let first_scope = store.latest_import_scan_scope().unwrap().unwrap();
    store
        .cancel_import_task(
            &first_scope.import_task_id,
            UnixTimestamp::from_unix_seconds(1_800_020_000),
        )
        .unwrap();

    let second_response = http_post_import_command(&endpoint, Some(&token), &fixture_root, Some(1));
    assert!(second_response.contains("HTTP/1.1 202 Accepted"));
    assert!(second_response.contains("\"new_tasks\":1"));
    assert!(!second_response.contains(path_str(&data_dir)));
    assert!(!second_response.contains(path_str(&fixture_root)));
    assert!(!second_response.contains(path_str(&canonical_fixture_root)));
    assert!(!second_response.contains(&token));

    let output = wait_child(child);
    assert!(output.success, "stderr:\n{}", output.stderr);
    assert!(output.stderr.is_empty());

    let summary = store.status_summary().unwrap();
    assert_eq!(summary.import_tasks_queued, 1);
    assert_eq!(summary.import_tasks_cancelled, 1);
    let latest_scope = store.latest_import_scan_scope().unwrap().unwrap();
    assert_ne!(latest_scope.import_task_id, first_scope.import_task_id);
    assert_eq!(
        latest_scope.canonical_root_path,
        path_str(&canonical_fixture_root)
    );

    remove_dir(&data_dir);
}

#[test]
fn daemon_import_command_preserves_local_discovery_preset_scope() {
    let data_dir = temp_dir("ipc-import-preset-command-data");
    let fixture_root = fixture_root();
    let canonical_fixture_root = fs::canonicalize(&fixture_root).unwrap();
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
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start resume-daemon ipc");

    let endpoint = read_ipc_endpoint(&mut child, &data_dir);
    let token = read_ipc_auth_token(&data_dir);
    let response = http_post_import_command_with_root_preset(
        &endpoint,
        &token,
        &fixture_root,
        Some("local-discovery"),
        Some(1),
    );

    assert!(response.contains("HTTP/1.1 202 Accepted"));
    assert!(!response.contains(path_str(&data_dir)));
    assert!(!response.contains(path_str(&fixture_root)));
    assert!(!response.contains(path_str(&canonical_fixture_root)));
    assert!(!response.contains(&token));

    let output = wait_child(child);
    assert!(output.success, "stderr:\n{}", output.stderr);
    assert!(output.stderr.is_empty());

    let store = MetaStore::open_data_dir(&data_dir).unwrap();
    store.run_migrations().unwrap();
    let scope = store.latest_import_scan_scope().unwrap().unwrap();
    assert_eq!(scope.root_kind, ImportRootKind::Preset);
    assert_eq!(scope.root_preset, Some(ImportRootPreset::LocalDiscovery));
    assert_eq!(scope.scan_profile, ImportScanProfile::Explicit);
    assert_eq!(scope.requested_root_path, path_str(&fixture_root));
    assert_eq!(scope.canonical_root_path, path_str(&canonical_fixture_root));

    remove_dir(&data_dir);
}

#[test]
fn daemon_import_cancel_command_records_cancellation_without_path_leak() {
    let data_dir = temp_dir("ipc-import-cancel-command-data");
    let fixture_root = fixture_root();
    let canonical_fixture_root = fs::canonicalize(&fixture_root).unwrap();
    let started_at_seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
        .saturating_sub(60) as i64;
    let task_id = seed_running_import_task(
        &data_dir,
        "cancel-running-over-ipc",
        &canonical_fixture_root,
        started_at_seconds,
    );
    let mut child = Command::new(env!("CARGO_BIN_EXE_resume-daemon"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "run",
            "--foreground",
            "--ipc-listen",
            "127.0.0.1:0",
            "--max-requests",
            "2",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start resume-daemon ipc");

    let endpoint = read_ipc_endpoint(&mut child, &data_dir);
    let token = read_ipc_auth_token(&data_dir);
    let response = http_post_import_cancel_command(&endpoint, Some(&token), &task_id);
    assert!(response.contains("HTTP/1.1 202 Accepted"));
    assert!(response.contains("\"schema_version\":\"daemon.import_cancel.v1\""));
    assert!(response.contains("\"status\":\"cancel_requested\""));
    assert!(response.contains(&format!("\"task_id\":\"{task_id}\"")));
    assert!(!response.contains(path_str(&data_dir)));
    assert!(!response.contains(path_str(&fixture_root)));
    assert!(!response.contains(path_str(&canonical_fixture_root)));
    assert!(!response.contains(&token));

    let status_response = http_get(&endpoint);
    assert!(status_response.contains("HTTP/1.1 200 OK"));
    assert!(status_response.contains("\"import_tasks_cancelled\":1"));
    assert!(!status_response.contains(path_str(&data_dir)));
    assert!(!status_response.contains(path_str(&fixture_root)));
    assert!(!status_response.contains(path_str(&canonical_fixture_root)));

    let output = wait_child(child);
    assert!(output.success, "stderr:\n{}", output.stderr);
    assert!(output.stderr.is_empty());

    let store = MetaStore::open_data_dir(&data_dir).unwrap();
    store.run_migrations().unwrap();
    assert!(store.is_import_task_cancelled(&task_id).unwrap());
    let summary = store.status_summary().unwrap();
    assert_eq!(summary.import_tasks_cancelled, 1);

    remove_dir(&data_dir);
}

#[test]
fn daemon_rejects_wrong_bearer_token_for_import_command_ipc() {
    let data_dir = temp_dir("ipc-import-wrong-token-data");
    let fixture_root = fixture_root();
    let canonical_fixture_root = fs::canonicalize(&fixture_root).unwrap();
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
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start resume-daemon ipc");

    let endpoint = read_ipc_endpoint(&mut child, &data_dir);
    let response = http_post_import_command(
        &endpoint,
        Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"),
        &fixture_root,
        Some(1),
    );

    assert!(response.contains("HTTP/1.1 401 Unauthorized"));
    assert!(!response.contains(path_str(&data_dir)));
    assert!(!response.contains(path_str(&fixture_root)));
    assert!(!response.contains(path_str(&canonical_fixture_root)));

    let output = wait_child(child);
    assert!(output.success, "stderr:\n{}", output.stderr);
    assert!(output.stderr.is_empty());
    let store = MetaStore::open_data_dir(&data_dir).unwrap();
    store.run_migrations().unwrap();
    assert_eq!(store.status_summary().unwrap().import_tasks_queued, 0);

    remove_dir(&data_dir);
}

#[test]
fn daemon_rejects_malformed_ipc_request_without_stopping() {
    let data_dir = temp_dir("ipc-malformed-request-data");
    let mut child = Command::new(env!("CARGO_BIN_EXE_resume-daemon"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "run",
            "--foreground",
            "--ipc-listen",
            "127.0.0.1:0",
            "--max-requests",
            "2",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start resume-daemon ipc");

    let endpoint = read_ipc_endpoint(&mut child, &data_dir);
    let malformed = raw_ipc_request(
        &endpoint,
        b"POST /imports HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Length: nope\r\n\r\n",
    );
    let status_response = http_get(&endpoint);

    assert!(malformed.contains("HTTP/1.1 400 Bad Request"));
    assert!(status_response.contains("HTTP/1.1 200 OK"));
    assert!(status_response.contains("\"status\":\"ok\""));

    let output = wait_child(child);
    assert!(output.success, "stderr:\n{}", output.stderr);
    assert!(output.stderr.is_empty());

    remove_dir(&data_dir);
}

#[test]
fn daemon_rejects_import_command_for_running_root_without_rewriting_scope() {
    let data_dir = temp_dir("ipc-import-running-conflict-data");
    let fixture_root = fixture_root();
    let canonical_fixture_root = fs::canonicalize(&fixture_root).unwrap();
    let task_id = seed_running_import_task(
        &data_dir,
        "ipc-running-conflict",
        &canonical_fixture_root,
        1_700_000_000,
    );
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
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start resume-daemon ipc");

    let endpoint = read_ipc_endpoint(&mut child, &data_dir);
    let token = read_ipc_auth_token(&data_dir);
    let response = http_post_import_command(&endpoint, Some(&token), &fixture_root, Some(1));

    assert!(response.contains("HTTP/1.1 409 Conflict"));
    assert!(response.contains("\"status\":\"conflict\""));
    assert!(!response.contains(path_str(&data_dir)));
    assert!(!response.contains(path_str(&fixture_root)));
    assert!(!response.contains(path_str(&canonical_fixture_root)));
    assert!(!response.contains(&token));

    let output = wait_child(child);
    assert!(output.success, "stderr:\n{}", output.stderr);
    assert!(output.stderr.is_empty());

    let store = MetaStore::open_data_dir(&data_dir).unwrap();
    store.run_migrations().unwrap();
    let task = store.import_task_by_id(&task_id).unwrap().unwrap();
    assert_eq!(task.status, ImportTaskStatus::Running);
    let scope = store
        .import_scan_scope_by_task_id(&task_id)
        .unwrap()
        .unwrap();
    assert_eq!(scope.scan_budget_limit, None);

    remove_dir(&data_dir);
}

#[test]
fn daemon_import_command_ipc_feeds_running_import_worker_loop() {
    let data_dir = temp_dir("ipc-import-command-worker-data");
    let fixture_root = fixture_root();
    let canonical_fixture_root = fs::canonicalize(&fixture_root).unwrap();
    let request_limit = IMPORT_WORKER_STATUS_REQUEST_LIMIT;
    let request_limit_arg = request_limit.to_string();
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
            request_limit_arg.as_str(),
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start resume-daemon ipc plus import worker");

    let endpoint = read_ipc_endpoint(&mut child, &data_dir);
    let token = read_ipc_auth_token(&data_dir);
    let response = http_post_import_command(&endpoint, Some(&token), &fixture_root, None);
    assert!(response.contains("HTTP/1.1 202 Accepted"));

    let (worker_requests, completed_response) = wait_for_searchable_documents(
        &mut child,
        &data_dir,
        &endpoint,
        2,
        IMPORT_WORKER_SEARCHABLE_MAX_REQUESTS,
    );
    let used_requests = 1 + worker_requests;
    drain_status_requests(&endpoint, request_limit - used_requests);

    let output = wait_child(child);
    assert!(output.success, "stderr:\n{}", output.stderr);
    assert!(output.stderr.is_empty());

    let store = MetaStore::open_data_dir(&data_dir).unwrap();
    store.run_migrations().unwrap();
    let scope = store.latest_import_scan_scope().unwrap().unwrap();
    let task = store
        .import_task_by_id(&scope.import_task_id)
        .unwrap()
        .unwrap();
    assert_eq!(task.status, ImportTaskStatus::Completed);
    assert_eq!(store.status_summary().unwrap().searchable_documents, 2);
    assert!(!response.contains(path_str(&data_dir)));
    assert!(!response.contains(path_str(&fixture_root)));
    assert!(!response.contains(path_str(&canonical_fixture_root)));
    assert!(!response.contains(&token));
    assert!(!completed_response.contains(path_str(&data_dir)));
    assert!(!completed_response.contains(path_str(&fixture_root)));
    assert!(!completed_response.contains(path_str(&canonical_fixture_root)));

    remove_dir(&data_dir);
}

#[cfg(unix)]
#[test]
fn daemon_repairs_existing_weak_ipc_token_permissions() {
    let data_dir = temp_dir("ipc-token-permissions-data");
    let token = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\n";
    fs::write(data_dir.join("ipc.auth"), token).unwrap();
    fs::set_permissions(data_dir.join("ipc.auth"), fs::Permissions::from_mode(0o644)).unwrap();
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
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start resume-daemon ipc");

    let endpoint = read_ipc_endpoint(&mut child, &data_dir);
    let response = http_get(&endpoint);

    assert!(response.contains("HTTP/1.1 200 OK"));
    let output = wait_child(child);
    assert!(output.success, "stderr:\n{}", output.stderr);
    assert!(output.stderr.is_empty());
    let permissions = fs::metadata(data_dir.join("ipc.auth"))
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(permissions & 0o777, 0o600);

    remove_dir(&data_dir);
}

#[test]
fn daemon_serves_status_while_import_worker_processes_late_queued_task() {
    let data_dir = temp_dir("ipc-import-worker-data");
    let fixture_root = fixture_root();
    let canonical_fixture_root = fs::canonicalize(&fixture_root).unwrap();
    let request_limit = IMPORT_WORKER_STATUS_REQUEST_LIMIT;
    let request_limit_arg = request_limit.to_string();
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
            request_limit_arg.as_str(),
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start resume-daemon ipc plus import worker");

    let endpoint = read_ipc_endpoint(&mut child, &data_dir);
    let initial_response = http_get(&endpoint);
    assert!(initial_response.contains("HTTP/1.1 200 OK"));
    assert!(initial_response.contains("\"searchable_documents\":0"));

    let task_id = seed_queued_import_task(
        &data_dir,
        "ipc-import-worker",
        &canonical_fixture_root,
        1_700_000_000,
    );
    let (worker_requests, completed_response) = wait_for_searchable_documents(
        &mut child,
        &data_dir,
        &endpoint,
        2,
        IMPORT_WORKER_SEARCHABLE_MAX_REQUESTS,
    );
    let used_requests = 1 + worker_requests;
    drain_status_requests(&endpoint, request_limit - used_requests);

    let output = wait_child(child);
    assert!(output.success, "stderr:\n{}", output.stderr);
    assert!(output.stderr.is_empty());

    let store = MetaStore::open_data_dir(&data_dir).unwrap();
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
    let store = MetaStore::open_data_dir(&data_dir).unwrap();
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

fn read_ipc_endpoint(child: &mut Child, data_dir: &Path) -> String {
    let deadline = Instant::now() + IPC_ENDPOINT_TIMEOUT;
    let manifest_path = data_dir.join("ipc.endpoints.json");
    let mut last_manifest_state = "<not observed>".to_string();

    while Instant::now() < deadline {
        if let Some(status) = child.try_wait().expect("poll daemon child") {
            let stderr = read_child_stderr(child);
            panic!(
                "daemon exited before ipc status endpoint was discoverable: {status}\nmanifest: {last_manifest_state}\nstderr:\n{stderr}"
            );
        }

        match fs::read_to_string(&manifest_path) {
            Ok(body) => {
                let manifest: serde_json::Value = match serde_json::from_str(&body) {
                    Ok(manifest) => manifest,
                    Err(error) => {
                        last_manifest_state = format!("invalid json: {error}: {body}");
                        std::thread::sleep(IPC_ENDPOINT_POLL_DELAY);
                        continue;
                    }
                };
                if let Some(endpoint) = manifest["status"].as_str() {
                    return endpoint.to_string();
                }
                last_manifest_state = format!("missing status endpoint field: {body}");
            }
            Err(error) => {
                last_manifest_state = format!("unavailable: {error}");
            }
        }

        std::thread::sleep(IPC_ENDPOINT_POLL_DELAY);
    }

    if let Some(status) = child.try_wait().expect("poll daemon child") {
        let stderr = read_child_stderr(child);
        panic!(
            "daemon exited before ipc status endpoint was discoverable: {status}\nmanifest: {last_manifest_state}\nstderr:\n{stderr}"
        );
    }

    let _ = child.kill();
    let _ = child.wait();
    let stderr = read_child_stderr(child);
    panic!(
        "daemon did not make ipc status endpoint discoverable within {:?}\nmanifest: {last_manifest_state}\nstderr:\n{stderr}",
        IPC_ENDPOINT_TIMEOUT
    );
}

fn wait_for_searchable_documents(
    child: &mut Child,
    data_dir: &Path,
    endpoint: &str,
    expected: usize,
    max_requests: usize,
) -> (usize, String) {
    let deadline = Instant::now() + IMPORT_WORKER_SEARCHABLE_TIMEOUT;
    let mut last_response = String::new();
    for request_count in 1..=max_requests {
        if let Some(status) = child.try_wait().expect("poll daemon child") {
            let stderr = read_child_stderr(child);
            let store_state = describe_store_state(data_dir);
            panic!(
                "daemon exited before searchable document count {expected}: {status}\n{stderr}\n{store_state}"
            );
        }
        let response = match try_http_get(endpoint) {
            Ok(response) if response.is_empty() => {
                last_response = "<empty status response>".to_string();
                std::thread::sleep(IMPORT_WORKER_STATUS_POLL_DELAY);
                continue;
            }
            Ok(response) => response,
            Err(_) => {
                last_response = "<status request unavailable>".to_string();
                std::thread::sleep(IMPORT_WORKER_STATUS_POLL_DELAY);
                continue;
            }
        };
        last_response = response.clone();
        if !response.contains("HTTP/1.1 200 OK") {
            let _ = child.kill();
            let _ = child.wait();
            let stderr = read_child_stderr(child);
            panic!("unexpected status response: {response}\nstderr:\n{stderr}");
        }
        if response.contains(&format!("\"searchable_documents\":{expected}")) {
            assert!(response.contains("\"import_tasks_queued\":0"));
            assert!(!response.contains("raw_resume_text"));
            return (request_count, response);
        }
        if Instant::now() >= deadline {
            break;
        }
        std::thread::sleep(IMPORT_WORKER_STATUS_POLL_DELAY);
    }

    let _ = child.kill();
    let _ = child.wait();
    let stderr = read_child_stderr(child);
    let store_state = describe_store_state(data_dir);
    panic!(
        "daemon status did not report searchable document count {expected} within {max_requests} requests or {:?}\nlast response:\n{last_response}\nstderr:\n{stderr}\n{store_state}",
        IMPORT_WORKER_SEARCHABLE_TIMEOUT
    );
}

fn describe_store_state(data_dir: &Path) -> String {
    let store = match MetaStore::open_data_dir(data_dir) {
        Ok(store) => store,
        Err(error) => return format!("store open failed: {error}"),
    };
    let schema_version = store.schema_version();
    let summary = store.status_summary();
    let latest_scope = store.latest_import_scan_scope();
    format!(
        "store state: encryption={}, schema={schema_version:?}, summary={summary:?}, latest_scope={latest_scope:?}",
        store.metadata_encryption_state().label()
    )
}

fn read_child_stderr(child: &mut Child) -> String {
    let mut stderr = String::new();
    if let Some(mut pipe) = child.stderr.take() {
        pipe.read_to_string(&mut stderr)
            .expect("read daemon stderr");
    }
    stderr
}

fn drain_status_requests(endpoint: &str, count: usize) {
    for _ in 0..count {
        if try_http_get(endpoint).is_err() {
            return;
        }
    }
}

fn http_get(endpoint: &str) -> String {
    try_http_get(endpoint).expect("read response")
}

fn http_get_import_progress(endpoint: &str, token: Option<&str>) -> String {
    let rest = endpoint
        .strip_prefix("http://")
        .expect("endpoint has http scheme");
    let (addr, _path) = rest.split_once('/').expect("endpoint has path");
    let authorization = token
        .map(|token| format!("Authorization: Bearer {token}\r\n"))
        .unwrap_or_default();
    let mut stream = TcpStream::connect(addr).expect("connect daemon ipc");
    write!(
        stream,
        "GET /imports/progress HTTP/1.1\r\nHost: {addr}\r\n{authorization}Connection: close\r\n\r\n"
    )
    .expect("write import progress request");
    let mut response = String::new();
    stream.read_to_string(&mut response).expect("read response");
    response
}

fn try_http_get(endpoint: &str) -> io::Result<String> {
    let rest = endpoint
        .strip_prefix("http://")
        .expect("endpoint has http scheme");
    let (_addr, path) = rest.split_once('/').expect("endpoint has path");
    try_http_get_path(endpoint, &format!("/{path}"))
}

fn http_get_path(endpoint: &str, request_path: &str) -> String {
    try_http_get_path(endpoint, request_path).expect("read response")
}

fn try_http_get_path(endpoint: &str, request_path: &str) -> io::Result<String> {
    let rest = endpoint
        .strip_prefix("http://")
        .expect("endpoint has http scheme");
    let (addr, _path) = rest.split_once('/').expect("endpoint has path");
    let mut stream = TcpStream::connect(addr)?;
    write!(
        stream,
        "GET {request_path} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n"
    )?;
    let mut response = String::new();
    stream.read_to_string(&mut response)?;
    Ok(response)
}

fn http_post_import_command(
    endpoint: &str,
    token: Option<&str>,
    root: &Path,
    max_files: Option<usize>,
) -> String {
    http_post_import_command_value(
        endpoint,
        token,
        serde_json::json!({
            "roots": [path_str(root)],
            "profile": "explicit",
            "max_files": max_files,
        }),
    )
}

fn http_post_import_command_with_root_preset(
    endpoint: &str,
    token: &str,
    root: &Path,
    root_preset: Option<&str>,
    max_files: Option<usize>,
) -> String {
    http_post_import_command_value(
        endpoint,
        Some(token),
        serde_json::json!({
            "roots": [path_str(root)],
            "root_preset": root_preset,
            "profile": "explicit",
            "max_files": max_files,
        }),
    )
}

fn http_post_import_command_value(
    endpoint: &str,
    token: Option<&str>,
    payload: serde_json::Value,
) -> String {
    let rest = endpoint
        .strip_prefix("http://")
        .expect("endpoint has http scheme");
    let (addr, _path) = rest.split_once('/').expect("endpoint has path");
    let body = payload.to_string();
    let authorization = token
        .map(|token| format!("Authorization: Bearer {token}\r\n"))
        .unwrap_or_default();
    let mut stream = TcpStream::connect(addr).expect("connect daemon ipc");
    write!(
        stream,
        "POST /imports HTTP/1.1\r\nHost: {addr}\r\nContent-Type: application/json\r\n{authorization}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
    .expect("write request");
    let mut response = String::new();
    stream.read_to_string(&mut response).expect("read response");
    response
}

fn http_post_import_cancel_command(
    endpoint: &str,
    token: Option<&str>,
    task_id: &ImportTaskId,
) -> String {
    let rest = endpoint
        .strip_prefix("http://")
        .expect("endpoint has http scheme");
    let (addr, _path) = rest.split_once('/').expect("endpoint has path");
    let body = serde_json::json!({
        "task_id": task_id.to_string(),
    })
    .to_string();
    let authorization = token
        .map(|token| format!("Authorization: Bearer {token}\r\n"))
        .unwrap_or_default();
    let mut stream = TcpStream::connect(addr).expect("connect daemon ipc");
    write!(
        stream,
        "POST /imports/cancel HTTP/1.1\r\nHost: {addr}\r\nContent-Type: application/json\r\n{authorization}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
    .expect("write request");
    let mut response = String::new();
    stream.read_to_string(&mut response).expect("read response");
    response
}

fn raw_ipc_request(endpoint: &str, request: &[u8]) -> String {
    let rest = endpoint
        .strip_prefix("http://")
        .expect("endpoint has http scheme");
    let (addr, _path) = rest.split_once('/').expect("endpoint has path");
    let mut stream = TcpStream::connect(addr).expect("connect daemon ipc");
    stream.write_all(request).expect("write raw request");
    let mut response = String::new();
    stream.read_to_string(&mut response).expect("read response");
    response
}

fn read_ipc_auth_token(data_dir: &Path) -> String {
    let token = fs::read_to_string(data_dir.join("ipc.auth")).expect("read daemon ipc auth token");
    let token = token.trim().to_string();
    assert!(token.len() >= 64);
    token
}

fn seed_snapshot_state(data_dir: &Path) {
    let store = MetaStore::open_data_dir(data_dir).unwrap();
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
    let store = MetaStore::open_data_dir(data_dir).unwrap();
    store.run_migrations().unwrap();
    let now = UnixTimestamp::from_unix_seconds(queued_at_seconds);
    let task_id = ImportTaskId::from_non_secret_parts(&["s45", label]);
    let root_path = path_str(canonical_root).to_string();
    let task = ImportTask {
        id: task_id.clone(),
        root_path: root_path.clone(),
        status: ImportTaskStatus::Queued,
        queued_at: now,
        started_at: None,
        finished_at: None,
        updated_at: now,
    };
    let scope = ImportScanScope {
        import_task_id: task_id.clone(),
        root_kind: ImportRootKind::Explicit,
        root_preset: None,
        scan_profile: ImportScanProfile::Explicit,
        requested_root_path: root_path.clone(),
        canonical_root_path: root_path,
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
    };
    store
        .insert_import_task_with_scan_scope(&task, &scope)
        .unwrap();
    task_id
}

fn seed_running_import_task(
    data_dir: &Path,
    label: &str,
    canonical_root: &Path,
    started_at_seconds: i64,
) -> ImportTaskId {
    let store = MetaStore::open_data_dir(data_dir).unwrap();
    store.run_migrations().unwrap();
    let now = UnixTimestamp::from_unix_seconds(started_at_seconds);
    let task_id = ImportTaskId::from_non_secret_parts(&["s46", label]);
    store
        .insert_import_task(&ImportTask {
            id: task_id.clone(),
            root_path: path_str(canonical_root).to_string(),
            status: ImportTaskStatus::Running,
            queued_at: now,
            started_at: Some(now),
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

fn seed_import_progress_scope(data_dir: &Path, task_id: &ImportTaskId, canonical_root: &Path) {
    let store = MetaStore::open_data_dir(data_dir).unwrap();
    store.run_migrations().unwrap();
    store
        .upsert_import_scan_scope(&ImportScanScope {
            import_task_id: task_id.clone(),
            root_kind: ImportRootKind::Explicit,
            root_preset: None,
            scan_profile: ImportScanProfile::Explicit,
            requested_root_path: path_str(canonical_root).to_string(),
            canonical_root_path: path_str(canonical_root).to_string(),
            files_discovered: 42,
            ignored_entries: 3,
            scan_errors: 2,
            searchable_documents: 13,
            ocr_required_documents: 5,
            ocr_jobs_queued: 4,
            failed_documents: 1,
            deleted_documents: 0,
            scan_budget_kind: Some(meta_store::ImportScanBudgetKind::Files),
            scan_budget_limit: Some(100),
            scan_budget_observed: Some(42),
            scan_budget_exhausted: false,
            updated_at: UnixTimestamp::from_unix_seconds(1_800_040_100),
        })
        .unwrap();
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

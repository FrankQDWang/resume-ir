use std::fs;
use std::io::{self, Read, Write};
#[cfg(not(unix))]
use std::net::Shutdown;
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use import_pipeline::{import_root_with_options, ImportOptions};
use meta_store::{
    ImportRootControlStatus, ImportRootKind, ImportRootPreset, ImportScanProfile, ImportScanScope,
    ImportTask, ImportTaskId, ImportTaskStatus, MetaStore, UnixTimestamp,
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
        "resume-ir.daemon-ipc.v2"
    );
    assert_eq!(endpoint_manifest_json["owner_mode"], "standalone");
    assert!(endpoint_manifest_json["instance_id"]
        .as_str()
        .is_some_and(|value| value.len() == 64));
    assert_eq!(endpoint_manifest_json["status"], endpoint);
    assert_eq!(
        endpoint_manifest_json["diagnostics"],
        format!("{base_endpoint}/diagnostics")
    );
    assert_eq!(
        endpoint_manifest_json["imports"],
        format!("{base_endpoint}/imports")
    );
    assert_eq!(
        endpoint_manifest_json["import_cancel"],
        format!("{base_endpoint}/imports/cancel")
    );
    assert_eq!(
        endpoint_manifest_json["import_control"],
        format!("{base_endpoint}/imports/control")
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
        endpoint_manifest_json["search_batch"],
        format!("{base_endpoint}/search/batch")
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
    assert!(response.contains("\"schema_version\":\"daemon.status.v2\""));
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
fn daemon_serves_only_authenticated_redacted_v2_diagnostics() {
    let data_dir = temp_dir("ipc-diagnostics-data");
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
            "2",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start resume-daemon diagnostics ipc");

    let endpoint = read_ipc_endpoint(&mut child, &data_dir);
    let token = read_ipc_auth_token(&data_dir);
    let unauthorized = http_get_diagnostics(&endpoint, None);
    let response = http_get_diagnostics(&endpoint, Some(&token));

    assert!(unauthorized.contains("HTTP/1.1 401 Unauthorized"));
    assert!(response.contains("HTTP/1.1 200 OK"));
    let body = response.split("\r\n\r\n").nth(1).unwrap();
    let payload: serde_json::Value = serde_json::from_str(body).unwrap();
    assert_eq!(payload["schema_version"], "resume-ir.diagnostics.v3");
    assert_eq!(payload["privacy_boundary"], "redacted_local_aggregate");
    assert_eq!(payload["evidence_lane"], "gui_manual");
    assert_eq!(payload["evidence_status"], "unaccepted");
    for flag in [
        "contains_raw_resume_text",
        "contains_queries",
        "contains_resume_paths",
        "contains_candidate_results",
        "contains_snippet_text",
    ] {
        assert_eq!(payload[flag], false);
    }
    assert!(payload["metrics"].is_object());
    assert!(payload["error_counts"].is_object());
    assert_eq!(payload["benchmark_refs"], serde_json::json!([]));
    assert!(!body.contains(path_str(&data_dir)));
    assert!(!body.contains(&token));
    assert!(!body.contains("PRIVATE_SNAPSHOT_TOKEN"));
    assert!(!body.contains("PRIVATE_MANIFEST"));

    let output = wait_child(child);
    assert!(output.success, "stderr:\n{}", output.stderr);
    assert!(output.stderr.is_empty());
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
    assert_fatal_event(&stderr, "configuration_invalid", "blocked");

    remove_dir(&data_dir);
}

#[test]
fn daemon_blocks_an_attested_ipc_protocol_mismatch() {
    let data_dir = temp_dir("ipc-protocol-mismatch-data");
    let output = Command::new(env!("CARGO_BIN_EXE_resume-daemon"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "run",
            "--foreground",
            "--ipc-listen",
            "127.0.0.1:0",
            "--expected-ipc-protocol",
            "resume-ir.daemon-ipc.v1",
            "--max-requests",
            "1",
        ])
        .output()
        .expect("run resume-daemon with mismatched ipc protocol");

    assert!(!output.status.success());
    assert_fatal_event(
        &String::from_utf8_lossy(&output.stderr),
        "protocol_mismatch",
        "blocked",
    );
    assert!(!data_dir.join("ipc.endpoints.json").exists());
    assert!(!data_dir.join("ipc.auth").exists());

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
fn daemon_controls_managed_root_durably_with_bounded_path_free_contract() {
    let data_dir = temp_dir("ipc-root-control-data");
    let root_a = fs::canonicalize(temp_dir("ipc-root-control-a")).unwrap();
    let root_b = fs::canonicalize(temp_dir("ipc-root-control-b")).unwrap();
    let unknown = fs::canonicalize(temp_dir("ipc-root-control-unknown")).unwrap();
    seed_completed_import_scope(&data_dir, "root-a", &root_a);
    seed_completed_import_scope(&data_dir, "root-b", &root_b);

    let mut child = start_ipc_daemon(&data_dir, 8);
    let endpoint = read_ipc_endpoint(&mut child, &data_dir);
    let token = read_ipc_auth_token(&data_dir);
    let responses = [
        http_post_root_control(&endpoint, None, path_str(&root_a), "pause", None),
        http_post_root_control(&endpoint, Some(&token), "relative/root", "pause", None),
        http_post_root_control(&endpoint, Some(&token), path_str(&unknown), "pause", None),
        http_post_root_control(
            &endpoint,
            Some(&token),
            path_str(&root_a),
            "pause",
            Some(("unexpected", serde_json::Value::Bool(true))),
        ),
        http_post_root_control(
            &endpoint,
            Some(&token),
            &format!("/{}", "x".repeat(32 * 1024)),
            "pause",
            None,
        ),
        http_post_root_control(&endpoint, Some(&token), path_str(&root_a), "pause", None),
        http_post_root_control(&endpoint, Some(&token), path_str(&root_b), "inspect", None),
        http_post_import_command(&endpoint, Some(&token), &root_a, None),
    ];
    let output = wait_child(child);
    assert!(output.success, "stderr:\n{}", output.stderr);
    assert!(responses[0].contains("401 Unauthorized"));
    assert!(responses[1].contains("400 Bad Request"));
    assert!(responses[2].contains("404 Not Found"));
    assert!(responses[3].contains("400 Bad Request"));
    assert!(responses[4].contains("400 Bad Request"));
    assert_root_control_response(&responses[5], "paused", true, false, false);
    assert_root_control_response(&responses[6], "active", false, false, false);
    assert!(responses[7].contains("409 Conflict"));
    assert_responses_are_path_free(&responses, &[&root_a, &root_b, &unknown]);

    let store = MetaStore::open_data_dir(&data_dir).unwrap();
    store.run_migrations().unwrap();
    assert_eq!(
        store.import_root_control_status(path_str(&root_a)).unwrap(),
        Some(ImportRootControlStatus::Paused)
    );
    drop(store);

    let mut child = start_ipc_daemon(&data_dir, 3);
    let endpoint = read_ipc_endpoint(&mut child, &data_dir);
    let token = read_ipc_auth_token(&data_dir);
    let inspect =
        http_post_root_control(&endpoint, Some(&token), path_str(&root_a), "inspect", None);
    let resume = http_post_root_control(&endpoint, Some(&token), path_str(&root_a), "resume", None);
    let duplicate =
        http_post_root_control(&endpoint, Some(&token), path_str(&root_a), "resume", None);
    let output = wait_child(child);
    assert!(output.success, "stderr:\n{}", output.stderr);
    assert_root_control_response(&inspect, "paused", false, false, false);
    assert_root_control_response(&resume, "active", true, false, true);
    assert_root_control_response(&duplicate, "active", false, false, false);
    assert_responses_are_path_free(&[inspect, resume, duplicate], &[&root_a, &root_b, &unknown]);
    let store = MetaStore::open_data_dir(&data_dir).unwrap();
    store.run_migrations().unwrap();
    assert_eq!(store.status_summary().unwrap().import_tasks_queued, 1);
    assert_eq!(
        store.import_root_control_status(path_str(&root_a)).unwrap(),
        Some(ImportRootControlStatus::Active)
    );

    remove_dir(&data_dir);
    remove_dir(&root_a);
    remove_dir(&root_b);
    remove_dir(&unknown);
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
    assert!(status_response.contains("\"process_state\":\"ready\""));
    assert!(status_response.contains("\"status\":\"repairing\""));

    let output = wait_child(child);
    assert!(output.success, "stderr:\n{}", output.stderr);
    assert!(output.stderr.is_empty());

    remove_dir(&data_dir);
}

#[test]
fn daemon_survives_client_disconnect_while_writing_ipc_response() {
    let data_dir = temp_dir("ipc-response-disconnect-data");
    let mut child = start_ipc_daemon(&data_dir, 3);
    let endpoint = read_ipc_endpoint(&mut child, &data_dir);
    let token = read_ipc_auth_token(&data_dir);

    disconnect_during_partial_request(&endpoint);
    disconnect_during_import_progress(&endpoint, &token);
    let status_response = http_get(&endpoint);

    assert!(status_response.contains("HTTP/1.1 200 OK"));
    assert!(status_response.contains("\"process_state\":\"ready\""));
    assert!(status_response.contains("\"status\":\"repairing\""));
    let output = wait_child(child);
    assert!(output.success, "stderr:\n{}", output.stderr);
    assert!(output.stderr.is_empty());

    remove_dir(&data_dir);
}

#[test]
fn daemon_data_directory_has_one_fail_closed_owner() {
    let data_dir = temp_dir("ipc-exclusive-owner-data");
    let mut owner = start_ipc_daemon(&data_dir, 1);
    let endpoint = read_ipc_endpoint(&mut owner, &data_dir);
    let owner_manifest = fs::read_to_string(data_dir.join("ipc.endpoints.json")).unwrap();

    let contender = Command::new(env!("CARGO_BIN_EXE_resume-daemon"))
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
        .output()
        .expect("run duplicate daemon owner");

    assert!(!contender.status.success());
    let contender_stderr = String::from_utf8_lossy(&contender.stderr);
    assert_fatal_event(&contender_stderr, "ownership_conflict", "blocked");
    assert_eq!(
        fs::read_to_string(data_dir.join("ipc.endpoints.json")).unwrap(),
        owner_manifest
    );
    assert!(http_get(&endpoint).contains("HTTP/1.1 200 OK"));
    let output = wait_child(owner);
    assert!(output.success, "stderr:\n{}", output.stderr);

    remove_dir(&data_dir);
}

#[test]
fn daemon_rotates_generation_identity_and_authentication_token() {
    let data_dir = temp_dir("ipc-generation-rotation-data");
    let mut first = start_ipc_daemon(&data_dir, 1);
    let first_endpoint = read_ipc_endpoint(&mut first, &data_dir);
    let first_manifest = read_ipc_owner_file(&data_dir, "ipc.endpoints.json");
    let first_auth = read_ipc_owner_file(&data_dir, "ipc.auth");
    assert_eq!(first_manifest["instance_id"], first_auth["instance_id"]);
    assert!(http_get(&first_endpoint).contains("HTTP/1.1 200 OK"));
    assert!(wait_child(first).success);
    assert!(!data_dir.join("ipc.endpoints.json").exists());
    assert!(!data_dir.join("ipc.auth").exists());

    let mut second = start_ipc_daemon(&data_dir, 2);
    let second_endpoint = read_ipc_endpoint(&mut second, &data_dir);
    let second_manifest = read_ipc_owner_file(&data_dir, "ipc.endpoints.json");
    let second_auth = read_ipc_owner_file(&data_dir, "ipc.auth");
    assert_eq!(second_manifest["instance_id"], second_auth["instance_id"]);
    assert_ne!(
        first_manifest["instance_id"],
        second_manifest["instance_id"]
    );
    assert_ne!(first_auth["token"], second_auth["token"]);
    let stale_token = first_auth["token"].as_str().unwrap();
    let active_token = second_auth["token"].as_str().unwrap();
    assert!(http_get_diagnostics(&second_endpoint, Some(stale_token))
        .contains("HTTP/1.1 401 Unauthorized"));
    assert!(http_get_diagnostics(&second_endpoint, Some(active_token)).contains("HTTP/1.1 200 OK"));
    assert!(wait_child(second).success);

    remove_dir(&data_dir);
}

#[test]
fn stale_generation_cannot_delete_replaced_owner_files_and_auth_is_in_memory() {
    let data_dir = temp_dir("ipc-stale-cleanup-data");
    let mut child = start_ipc_daemon(&data_dir, 1);
    let endpoint = read_ipc_endpoint(&mut child, &data_dir);
    let token = read_ipc_auth_token(&data_dir);
    let fake_instance_id = "f".repeat(64);
    let fake_auth = serde_json::json!({
        "schema_version": "resume-ir.daemon-auth.v2",
        "instance_id": fake_instance_id.clone(),
        "token": "e".repeat(64),
    });
    let fake_manifest = serde_json::json!({
        "schema_version": "resume-ir.daemon-ipc.v2",
        "instance_id": fake_instance_id.clone(),
        "owner_mode": "standalone",
        "status": endpoint,
    });
    fs::write(data_dir.join("ipc.auth"), fake_auth.to_string()).unwrap();
    fs::write(
        data_dir.join("ipc.endpoints.json"),
        fake_manifest.to_string(),
    )
    .unwrap();

    let response = http_get_diagnostics(&endpoint, Some(&token));
    assert!(response.contains("HTTP/1.1 200 OK"));
    assert!(wait_child(child).success);
    assert_eq!(
        read_ipc_owner_file(&data_dir, "ipc.auth")["instance_id"],
        fake_instance_id
    );
    assert_eq!(
        read_ipc_owner_file(&data_dir, "ipc.endpoints.json")["instance_id"],
        fake_instance_id
    );

    remove_dir(&data_dir);
}

#[test]
fn daemon_survives_one_hundred_mixed_connection_faults_and_reports_counts() {
    let data_dir = temp_dir("ipc-mixed-fault-metrics-data");
    let mut child = start_ipc_daemon(&data_dir, 101);
    let endpoint = read_ipc_endpoint(&mut child, &data_dir);
    let token = read_ipc_auth_token(&data_dir);

    for _ in 0..10 {
        disconnect_during_partial_request(&endpoint);
        disconnect_during_import_progress(&endpoint, &token);
    }
    for _ in 0..40 {
        let response = raw_ipc_request(
            &endpoint,
            b"POST /imports HTTP/1.1\r\nHost: local\r\nContent-Length: invalid\r\n\r\n",
        );
        assert!(response.contains("HTTP/1.1 400 Bad Request"));
    }
    for _ in 0..40 {
        let response = http_get_diagnostics(&endpoint, Some("invalid-token"));
        assert!(response.contains("HTTP/1.1 401 Unauthorized"));
    }

    let response = http_get_diagnostics(&endpoint, Some(&token));
    let body = response.split("\r\n\r\n").nth(1).unwrap();
    let payload: serde_json::Value = serde_json::from_str(body).unwrap();
    let ipc = &payload["metrics"]["ipc"];
    assert_eq!(ipc["accepted"], 101);
    let completed = ipc["completed"].as_u64().unwrap();
    let request_failure = ipc["request_failure"].as_u64().unwrap();
    let response_failure = ipc["response_failure"].as_u64().unwrap();
    assert_eq!(completed + request_failure + response_failure, 100);
    assert!(completed > 0);
    assert!(ipc["client_disconnect"].as_u64().unwrap() <= response_failure);
    assert!(!body.contains(path_str(&data_dir)));
    assert!(!body.contains(&token));
    let output = wait_child(child);
    assert!(output.success, "stderr:\n{}", output.stderr);

    remove_dir(&data_dir);
}

#[test]
fn mixed_connection_fault_soak_harness_keeps_one_resident_daemon_healthy() {
    run_mixed_connection_fault_soak(2, Duration::from_millis(10), Duration::from_millis(20));
}

#[test]
#[ignore = "two-hour synthetic resident-daemon fault soak"]
fn daemon_survives_two_hour_mixed_connection_fault_soak() {
    run_mixed_connection_fault_soak(7_200, Duration::from_secs(1), Duration::from_secs(7_200));
}

fn run_mixed_connection_fault_soak(
    cycle_count: usize,
    cycle_interval: Duration,
    minimum_duration: Duration,
) {
    const FAULTS_PER_CYCLE: usize = 10;
    const REQUESTS_PER_CYCLE: usize = FAULTS_PER_CYCLE + 1;

    assert!(cycle_count > 0);
    let data_dir = temp_dir("ipc-mixed-fault-soak-data");
    let max_requests = cycle_count
        .checked_mul(REQUESTS_PER_CYCLE)
        .and_then(|count| count.checked_add(1))
        .expect("bounded soak request count");
    let mut child = start_ipc_daemon(&data_dir, max_requests);
    let endpoint = read_ipc_endpoint(&mut child, &data_dir);
    let token = read_ipc_auth_token(&data_dir);
    let started_at = Instant::now();

    for cycle in 0..cycle_count {
        let scheduled_at = started_at
            + cycle_interval
                .checked_mul(cycle.try_into().expect("bounded soak cycle count"))
                .expect("bounded soak schedule");
        if let Some(delay) = scheduled_at.checked_duration_since(Instant::now()) {
            std::thread::sleep(delay);
        }

        disconnect_during_partial_request(&endpoint);
        disconnect_during_import_progress(&endpoint, &token);
        for _ in 0..4 {
            let response = raw_ipc_request(
                &endpoint,
                b"POST /imports HTTP/1.1\r\nHost: local\r\nContent-Length: invalid\r\n\r\n",
            );
            assert!(response.contains("HTTP/1.1 400 Bad Request"));
        }
        for _ in 0..4 {
            let response = http_get_diagnostics(&endpoint, Some("invalid-token"));
            assert!(response.contains("HTTP/1.1 401 Unauthorized"));
        }

        let expected_accepted = (cycle + 1) * REQUESTS_PER_CYCLE;
        assert_soak_diagnostics(
            &http_get_diagnostics(&endpoint, Some(&token)),
            expected_accepted,
            &data_dir,
            &token,
        );
    }

    if let Some(delay) = (started_at + minimum_duration).checked_duration_since(Instant::now()) {
        std::thread::sleep(delay);
    }
    let final_accepted = cycle_count * REQUESTS_PER_CYCLE + 1;
    assert_soak_diagnostics(
        &http_get_diagnostics(&endpoint, Some(&token)),
        final_accepted,
        &data_dir,
        &token,
    );
    assert!(started_at.elapsed() >= minimum_duration);

    let output = wait_child(child);
    assert!(output.success, "stderr:\n{}", output.stderr);
    assert!(output.stderr.is_empty());
    remove_dir(&data_dir);
}

fn assert_soak_diagnostics(response: &str, expected_accepted: usize, data_dir: &Path, token: &str) {
    let body = response.split("\r\n\r\n").nth(1).unwrap();
    let payload: serde_json::Value = serde_json::from_str(body).unwrap();
    let ipc = &payload["metrics"]["ipc"];
    assert_eq!(ipc["accepted"], expected_accepted);
    let completed = ipc["completed"].as_u64().unwrap();
    let request_failure = ipc["request_failure"].as_u64().unwrap();
    let response_failure = ipc["response_failure"].as_u64().unwrap();
    assert_eq!(
        completed + request_failure + response_failure,
        expected_accepted.saturating_sub(1) as u64
    );
    assert!(completed > 0);
    assert!(ipc["client_disconnect"].as_u64().unwrap() <= response_failure);
    assert!(!body.contains(path_str(data_dir)));
    assert!(!body.contains(token));
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
fn daemon_replaces_legacy_auth_with_private_generation_credentials() {
    let data_dir = temp_dir("ipc-token-permissions-data");
    let legacy_token = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\n";
    fs::write(data_dir.join("ipc.auth"), legacy_token).unwrap();
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
    let auth = read_ipc_owner_file(&data_dir, "ipc.auth");
    let permissions = fs::metadata(data_dir.join("ipc.auth"))
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(permissions & 0o777, 0o600);
    assert_eq!(auth["schema_version"], "resume-ir.daemon-auth.v2");
    assert_ne!(auth["token"], legacy_token.trim());
    let response = http_get(&endpoint);

    assert!(response.contains("HTTP/1.1 200 OK"));
    let output = wait_child(child);
    assert!(output.success, "stderr:\n{}", output.stderr);
    assert!(output.stderr.is_empty());
    assert!(!data_dir.join("ipc.auth").exists());

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
    assert_fatal_event(&stderr, "control_plane_failure", "restartable");
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
    assert_fatal_event(&stderr, "configuration_invalid", "blocked");

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

fn http_get_diagnostics(endpoint: &str, token: Option<&str>) -> String {
    let rest = endpoint
        .strip_prefix("http://")
        .expect("endpoint has http scheme");
    let (addr, _path) = rest.split_once('/').expect("endpoint has path");
    let authorization = token
        .map(|token| format!("Authorization: Bearer {token}\r\n"))
        .unwrap_or_default();
    let mut stream = TcpStream::connect(addr).expect("connect daemon diagnostics ipc");
    write!(
        stream,
        "GET /diagnostics HTTP/1.1\r\nHost: {addr}\r\n{authorization}Connection: close\r\n\r\n"
    )
    .expect("write diagnostics request");
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

fn http_post_root_control(
    endpoint: &str,
    token: Option<&str>,
    root_path: &str,
    action: &str,
    extra: Option<(&str, serde_json::Value)>,
) -> String {
    let mut payload = serde_json::json!({
        "schema_version": "daemon.import_root_control_request.v1",
        "root_path": root_path,
        "action": action,
    });
    if let Some((name, value)) = extra {
        payload
            .as_object_mut()
            .unwrap()
            .insert(name.to_string(), value);
    }
    let rest = endpoint.strip_prefix("http://").unwrap();
    let (addr, _) = rest.split_once('/').unwrap();
    let body = payload.to_string();
    let authorization = token
        .map(|token| format!("Authorization: Bearer {token}\r\n"))
        .unwrap_or_default();
    let mut stream = TcpStream::connect(addr).unwrap();
    write!(
        stream,
        "POST /imports/control HTTP/1.1\r\nHost: {addr}\r\nContent-Type: application/json\r\n{authorization}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
    .unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    response
}

fn assert_root_control_response(
    response: &str,
    status: &str,
    changed: bool,
    task_cancel_requested: bool,
    catch_up_queued: bool,
) {
    let payload: serde_json::Value =
        serde_json::from_str(response.split("\r\n\r\n").nth(1).unwrap()).unwrap();
    assert_eq!(
        payload,
        serde_json::json!({
            "schema_version": "daemon.import_root_control.v1",
            "status": status,
            "changed": changed,
            "task_cancel_requested": task_cancel_requested,
            "catch_up_queued": catch_up_queued,
        })
    );
}

fn assert_responses_are_path_free(responses: &[String], roots: &[&Path]) {
    for response in responses {
        for root in roots {
            assert!(!response.contains(path_str(root)));
        }
        assert!(!response.contains("task_id"));
    }
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

fn disconnect_during_partial_request(endpoint: &str) {
    let rest = endpoint
        .strip_prefix("http://")
        .expect("endpoint has http scheme");
    let (addr, _path) = rest.split_once('/').expect("endpoint has path");
    let mut stream = TcpStream::connect(addr).expect("connect daemon ipc");
    prepare_abortive_close(&stream);
    stream
        .write_all(b"GET /status HTTP/1.1\r\nHost:")
        .expect("write partial request");
    disconnect_client(stream);
}

fn disconnect_during_import_progress(endpoint: &str, token: &str) {
    let rest = endpoint
        .strip_prefix("http://")
        .expect("endpoint has http scheme");
    let (addr, _path) = rest.split_once('/').expect("endpoint has path");
    let mut stream = TcpStream::connect(addr).expect("connect daemon ipc");
    prepare_abortive_close(&stream);
    write!(
        stream,
        "GET /imports/progress HTTP/1.1\r\nHost: {addr}\r\nAuthorization: Bearer {token}\r\nConnection: close\r\n\r\n"
    )
    .expect("write import progress request");
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .expect("bound progress response synchronization");
    let mut first_response_byte = [0_u8; 1];
    stream
        .read_exact(&mut first_response_byte)
        .expect("observe progress response before abortive close");
    disconnect_client(stream);
}

#[cfg(unix)]
fn prepare_abortive_close(stream: &TcpStream) {
    let linger = nix::libc::linger {
        l_onoff: 1,
        l_linger: 0,
    };
    nix::sys::socket::setsockopt(stream, nix::sys::socket::sockopt::Linger, &linger)
        .expect("configure abortive client close");
}

#[cfg(not(unix))]
fn prepare_abortive_close(_stream: &TcpStream) {}

fn disconnect_client(stream: TcpStream) {
    #[cfg(not(unix))]
    stream
        .shutdown(Shutdown::Both)
        .expect("disconnect ipc client");
    drop(stream);
}

fn read_ipc_auth_token(data_dir: &Path) -> String {
    let auth = read_ipc_owner_file(data_dir, "ipc.auth");
    assert_eq!(auth["schema_version"], "resume-ir.daemon-auth.v2");
    assert_eq!(auth["instance_id"].as_str().map(str::len), Some(64));
    let token = auth["token"].as_str().expect("auth token").to_string();
    assert_eq!(token.len(), 64);
    token
}

fn read_ipc_owner_file(data_dir: &Path, file_name: &str) -> serde_json::Value {
    let body = fs::read_to_string(data_dir.join(file_name)).expect("read daemon owner file");
    serde_json::from_str(&body).expect("parse daemon owner file")
}

fn assert_fatal_event(stderr: &str, class: &str, disposition: &str) {
    let body = stderr.trim();
    assert!(body.len() <= 1024, "fatal event exceeded bound: {body}");
    let event: serde_json::Value = serde_json::from_str(body).expect("parse daemon fatal event");
    assert_eq!(event.as_object().unwrap().len(), 4);
    assert_eq!(event["schema_version"], "resume-ir.daemon-fatal.v1");
    assert_eq!(event["event"], "fatal");
    assert_eq!(event["class"], class);
    assert_eq!(event["disposition"], disposition);
}

fn seed_snapshot_state(data_dir: &Path) {
    let root = data_dir.join("synthetic-status-corpus");
    fs::create_dir_all(&root).unwrap();
    fs::write(
        root.join("candidate.txt"),
        "SUMMARY\nSynthetic candidate\nEXPERIENCE\nBuilt Rust systems\nSKILLS\nRust",
    )
    .unwrap();
    let store = MetaStore::open_data_dir(data_dir).unwrap();
    store.run_migrations().unwrap();
    let now = UnixTimestamp::from_unix_seconds(1_800_000_000);
    let task = ImportTask {
        id: ImportTaskId::from_non_secret_parts(&["s20", "status-publication"]),
        root_path: path_str(&root).to_string(),
        status: ImportTaskStatus::Running,
        queued_at: now,
        started_at: Some(now),
        finished_at: None,
        updated_at: now,
    };
    store.insert_import_task(&task).unwrap();
    import_root_with_options(
        data_dir,
        &store,
        &task,
        &root,
        now,
        ImportOptions::default(),
    )
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

fn seed_completed_import_scope(data_dir: &Path, label: &str, root: &Path) {
    let task_id = seed_queued_import_task(data_dir, label, root, 1_800_200_000);
    let store = MetaStore::open_data_dir(data_dir).unwrap();
    store.run_migrations().unwrap();
    let running_at = UnixTimestamp::from_unix_seconds(1_800_200_001);
    store
        .update_import_task_status(&task_id, ImportTaskStatus::Running, running_at)
        .unwrap();
    store
        .update_import_task_status(
            &task_id,
            ImportTaskStatus::Completed,
            UnixTimestamp::from_unix_seconds(1_800_200_002),
        )
        .unwrap();
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

fn start_ipc_daemon(data_dir: &Path, max_requests: usize) -> Child {
    let max_requests = max_requests.to_string();
    Command::new(env!("CARGO_BIN_EXE_resume-daemon"))
        .args([
            "--data-dir",
            path_str(data_dir),
            "run",
            "--foreground",
            "--ipc-listen",
            "127.0.0.1:0",
            "--max-requests",
            max_requests.as_str(),
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap()
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

use std::fs;
use std::io::{self, Read, Write};
#[cfg(not(unix))]
use std::net::Shutdown;
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::str::FromStr;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use import_pipeline::{
    import_root_with_options, DataDirectoryOwnerAcquisition, DataDirectoryOwnerLease,
    ImportOptions, ImportTaskOwnerLock,
};
use meta_store::{
    ClassificationStatus, ContentDigest, Document, DocumentId, DocumentStatus, FileExtension,
    ImportRootControlStatus, ImportRootKind, ImportRootPreset, ImportScanProfile, ImportScanScope,
    ImportTask, ImportTaskId, ImportTaskStatus, IngestJobId, OwnedMetaStore, ReadMetaStore,
    ReasonCode, SourceRevision, SourceRevisionTriage, SourceRootDeletionPhase, UnixTimestamp,
};

mod support;

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

#[cfg(windows)]
const IPC_ENDPOINT_TIMEOUT: Duration = Duration::from_secs(30);
#[cfg(not(windows))]
const IPC_ENDPOINT_TIMEOUT: Duration = Duration::from_secs(10);
const IPC_CORE_INITIALIZATION_TIMEOUT: Duration = Duration::from_secs(120);
const IPC_ENDPOINT_POLL_DELAY: Duration = Duration::from_millis(25);
const IMPORT_WORKER_SEARCHABLE_MAX_REQUESTS: usize = 260;
const IMPORT_WORKER_SEARCHABLE_TIMEOUT: Duration = Duration::from_secs(20);
const IMPORT_WORKER_STATUS_POLL_DELAY: Duration = Duration::from_millis(50);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct TestIpcMetrics {
    accepted: u64,
    completed: u64,
    client_disconnect: u64,
    request_failure: u64,
    response_failure: u64,
}

impl TestIpcMetrics {
    fn from_status(payload: &serde_json::Value) -> Self {
        Self::from_value(&payload["ipc"])
    }

    fn from_diagnostics(payload: &serde_json::Value) -> Self {
        Self::from_value(&payload["metrics"]["ipc"])
    }

    fn from_value(value: &serde_json::Value) -> Self {
        Self {
            accepted: value["accepted"].as_u64().expect("accepted counter"),
            completed: value["completed"].as_u64().expect("completed counter"),
            client_disconnect: value["client_disconnect"]
                .as_u64()
                .expect("client disconnect counter"),
            request_failure: value["request_failure"]
                .as_u64()
                .expect("request failure counter"),
            response_failure: value["response_failure"]
                .as_u64()
                .expect("response failure counter"),
        }
    }

    fn delta_from(self, baseline: Self) -> Self {
        Self {
            accepted: checked_counter_delta(self.accepted, baseline.accepted),
            completed: checked_counter_delta(self.completed, baseline.completed),
            client_disconnect: checked_counter_delta(
                self.client_disconnect,
                baseline.client_disconnect,
            ),
            request_failure: checked_counter_delta(self.request_failure, baseline.request_failure),
            response_failure: checked_counter_delta(
                self.response_failure,
                baseline.response_failure,
            ),
        }
    }

    fn terminal(self) -> u64 {
        self.completed + self.request_failure + self.response_failure
    }
}

fn checked_counter_delta(current: u64, baseline: u64) -> u64 {
    current
        .checked_sub(baseline)
        .expect("daemon IPC counter must be monotonic")
}

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
            "2",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start resume-daemon ipc");

    let endpoint = read_ipc_endpoint(&mut child, &data_dir);
    let token = read_ipc_auth_token(&data_dir);
    wait_for_serving_control_plane(&mut child, &endpoint, &token);
    let endpoint_manifest_path = data_dir.join("ipc.endpoints.json");
    let endpoint_manifest =
        fs::read_to_string(&endpoint_manifest_path).expect("read daemon ipc endpoint manifest");
    let endpoint_manifest_json: serde_json::Value =
        serde_json::from_str(&endpoint_manifest).expect("parse daemon ipc endpoint manifest");
    let base_endpoint = endpoint.strip_suffix("/status").unwrap();
    assert_eq!(
        endpoint_manifest_json["schema_version"],
        "resume-ir.daemon-ipc.v5"
    );
    assert_eq!(
        endpoint_manifest_json["launch_id"].as_str().map(str::len),
        Some(64)
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
    let response = http_get(&endpoint, &token);

    assert!(response.contains("HTTP/1.1 200 OK"));
    assert!(response.contains("\"schema_version\":\"daemon.status.v6\""));
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
fn daemon_serves_only_authenticated_redacted_v4_diagnostics() {
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
            "3",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start resume-daemon diagnostics ipc");

    let endpoint = read_ipc_endpoint(&mut child, &data_dir);
    let token = read_ipc_auth_token(&data_dir);
    wait_for_serving_control_plane(&mut child, &endpoint, &token);
    let unauthorized = http_get_diagnostics(&endpoint, None);
    let response = http_get_diagnostics(&endpoint, Some(&token));

    assert!(unauthorized.contains("HTTP/1.1 401 Unauthorized"));
    assert!(response.contains("HTTP/1.1 200 OK"));
    let body = response.split("\r\n\r\n").nth(1).unwrap();
    let payload: serde_json::Value = serde_json::from_str(body).unwrap();
    assert_eq!(payload["schema_version"], "resume-ir.diagnostics.v10");
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
#[cfg_attr(not(feature = "native-runtime-tests"), ignore)]
fn source_root_delete_returns_accepted_without_restarting_or_blocking_a_second_root() {
    const DELETION_POLL_REQUEST_BUDGET: usize = 80;
    const TEXT_IMPORT_READINESS_REQUEST_BUDGET: usize = 120;
    const DELETE_REQUEST_SCHEMA: &str = "resume-ir.source-root-delete-request.v1";
    const CONTROL_REQUEST_SCHEMA: &str = "resume-ir.source-root-control-request.v1";

    let runtime_capacity = support::import_runtime_capacity_lease();
    let data_dir = temp_dir("ipc-source-root-delete-data");
    seed_reviewed_snapshot_state(&data_dir);
    let source_a = data_dir.join("synthetic-source-a");
    let source_b = data_dir.join("synthetic-source-b");
    fs::create_dir_all(&source_a).unwrap();
    fs::create_dir_all(&source_b).unwrap();
    let canonical_source_a = fs::canonicalize(&source_a).unwrap();
    let canonical_source_b = fs::canonicalize(&source_b).unwrap();
    let now = UnixTimestamp::from_unix_seconds(1_800_300_200);
    let [root_a, root_b] = {
        let store = open_owned_store(&data_dir);
        [
            (&canonical_source_a, "Synthetic source A"),
            (&canonical_source_b, "Synthetic source B"),
        ]
        .map(|(path, label)| {
            store
                .register_source_root(path_str(path), path_str(path), label, now)
                .unwrap()
        })
    };
    let root_a_id = root_a.id.as_str();
    let root_b_id = root_b.id.as_str();
    let ocr_job = seed_ocr_backlog_for_root(&data_dir, &root_a, now);
    let active_task = seed_running_import_task(
        &data_dir,
        "source-root-delete-quiescence",
        &canonical_source_a,
        now.as_unix_seconds().saturating_sub(1),
    );
    let task_owner = ImportTaskOwnerLock::acquire(&data_dir, &active_task).unwrap();
    let mut child = start_import_capable_ipc_daemon(
        &runtime_capacity,
        &data_dir,
        DELETION_POLL_REQUEST_BUDGET + TEXT_IMPORT_READINESS_REQUEST_BUDGET + 6,
    );
    let endpoint = read_ipc_endpoint(&mut child, &data_dir);
    let token = read_ipc_auth_token(&data_dir);
    let (child_id, instance_id) = daemon_identity(&child, &data_dir);
    let readiness_deadline = Instant::now() + Duration::from_secs(15);
    let mut remaining_readiness_requests = TEXT_IMPORT_READINESS_REQUEST_BUDGET;
    loop {
        assert!(
            remaining_readiness_requests > 0 && Instant::now() < readiness_deadline,
            "text import did not become available within the time and request budgets"
        );
        remaining_readiness_requests -= 1;
        let status = http_get(&endpoint, &token);
        if response_json(&status)["capabilities"]["text_import"]["state"] == "available" {
            assert_text_import_available_without_ocr(&status);
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    assert_same_daemon_instance(&mut child, &data_dir, child_id, &instance_id);

    let deletion = http_post_json(
        &endpoint,
        "/source-roots/delete",
        &token,
        serde_json::json!({"schema_version": DELETE_REQUEST_SCHEMA, "root_id": root_a_id}),
    );
    assert!(deletion.starts_with("HTTP/1.1 202 Accepted"), "{deletion}");
    assert_eq!(
        response_json(&deletion),
        serde_json::json!({
            "schema_version": "resume-ir.root-deletion-receipt.v1",
            "status": "deleting",
            "root_id": root_a_id,
            "affected_documents": 1,
            "removed_documents": 0,
            "source_files_deleted": false,
        })
    );
    assert_same_daemon_instance(&mut child, &data_dir, child_id, &instance_id);

    let cancellation_deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let cancelled = ReadMetaStore::open_data_dir(&data_dir)
            .unwrap()
            .is_import_task_cancelled(&active_task)
            .unwrap();
        if cancelled {
            break;
        }
        assert!(
            Instant::now() < cancellation_deadline,
            "source-root deletion did not preempt the active import task"
        );
        std::thread::sleep(Duration::from_millis(25));
    }

    let deleting_error = post_source_root_register(
        &endpoint,
        &token,
        root_a.canonical_path.as_str(),
        "Synthetic source A duplicate",
        "HTTP/1.1 409 Conflict",
    );
    assert_eq!(deleting_error["schema_version"], "resume-ir.error.v3");
    assert_eq!(deleting_error["error"]["code"], "CONFLICT");
    assert_eq!(deleting_error["error"]["action"], "retry");
    assert_eq!(deleting_error["error"]["reason"], "source_root_deleting");

    let roots_during_delete = source_roots_json(&endpoint, &token);
    assert_eq!(
        listed_source_root(&roots_during_delete, root_a_id).unwrap()["state"],
        "deleting"
    );
    assert!(listed_source_root(&roots_during_delete, root_b_id).is_some());

    let second_root_control = http_post_json(
        &endpoint,
        "/source-roots/control",
        &token,
        serde_json::json!({
            "schema_version": CONTROL_REQUEST_SCHEMA,
            "root_id": root_b_id,
            "action": "pause"
        }),
    );
    assert!(second_root_control.starts_with("HTTP/1.1 200 OK"));
    assert_eq!(
        response_json(&second_root_control)["root"]["watcher_state"],
        "paused"
    );
    assert_same_daemon_instance(&mut child, &data_dir, child_id, &instance_id);

    drop(task_owner);
    let (remaining_poll_requests, roots_after_delete) =
        wait_for_source_root_absent(&endpoint, &token, root_a_id, DELETION_POLL_REQUEST_BUDGET);
    assert!(listed_source_root(&roots_after_delete, root_b_id).is_some());

    let replacement = post_source_root_register(
        &endpoint,
        &token,
        root_a.canonical_path.as_str(),
        "Synthetic source A replacement",
        "HTTP/1.1 200 OK",
    );
    assert_ne!(replacement["root"]["root_id"].as_str().unwrap(), root_a_id);
    assert_same_daemon_instance(&mut child, &data_dir, child_id, &instance_id);
    assert!([source_a, source_b].iter().all(|source| source.is_dir()));
    assert!(ReadMetaStore::open_data_dir(&data_dir)
        .unwrap()
        .ingest_job_by_id(&ocr_job)
        .unwrap()
        .is_none());

    for _ in 0..=remaining_poll_requests + remaining_readiness_requests {
        assert_ready_status(&http_get(&endpoint, &token));
    }
    let output = wait_child(child);
    assert!(output.success, "stderr:\n{}", output.stderr);
    assert!(output.stderr.is_empty(), "stderr:\n{}", output.stderr);
    remove_dir(&data_dir);
}

#[test]
#[cfg_attr(not(feature = "native-runtime-tests"), ignore)]
fn ready_daemon_registers_and_scans_two_non_overlapping_roots() {
    const SCAN_REQUEST_SCHEMA: &str = "resume-ir.source-root-scan-request.v1";
    let runtime_capacity = support::import_runtime_capacity_lease();
    let data_dir = temp_dir("ipc-two-source-roots-data");
    seed_reviewed_snapshot_state(&data_dir);
    let source_a = temp_dir("ipc-two-source-root-a");
    let source_b = temp_dir("ipc-two-source-root-b");
    let canonical_source_a = fs::canonicalize(&source_a).unwrap();
    let canonical_source_b = fs::canonicalize(&source_b).unwrap();
    let mut child = start_import_capable_ipc_daemon(&runtime_capacity, &data_dir, 5);
    let endpoint = read_ipc_endpoint(&mut child, &data_dir);
    let token = read_ipc_auth_token(&data_dir);
    let (child_id, instance_id) = daemon_identity(&child, &data_dir);
    assert_text_import_available_without_ocr(&http_get(&endpoint, &token));

    let [root_a, root_b] = [
        (&canonical_source_a, "Synthetic source A"),
        (&canonical_source_b, "Synthetic source B"),
    ]
    .map(|(path, label)| {
        let response =
            post_source_root_register(&endpoint, &token, path_str(path), label, "HTTP/1.1 200 OK");
        response["root"]["root_id"].as_str().unwrap().to_string()
    });
    assert_ne!(root_a, root_b);

    for root_id in [&root_a, &root_b] {
        let scan = http_post_json(
            &endpoint,
            "/source-roots/scan",
            &token,
            serde_json::json!({"schema_version": SCAN_REQUEST_SCHEMA, "root_id": root_id}),
        );
        assert!(scan.starts_with("HTTP/1.1 200 OK"), "{scan}");
        assert_eq!(response_json(&scan)["root"]["last_scan"]["phase"], "queued");
    }
    assert_same_daemon_instance(&mut child, &data_dir, child_id, &instance_id);

    let output = wait_child(child);
    assert!(output.success, "stderr:\n{}", output.stderr);
    assert!(output.stderr.is_empty(), "stderr:\n{}", output.stderr);
    let store = open_owned_store(&data_dir);
    assert_eq!(store.source_roots().unwrap().len(), 2);
    assert!(store.status_summary().unwrap().searchable_documents > 0);
    drop(store);
    remove_dir(&data_dir);
    remove_dir(&source_a);
    remove_dir(&source_b);
}

#[test]
fn startup_recovers_source_root_deletion_with_the_same_receipt() {
    for (fixture_name, seeded_phase) in [
        ("requested", None),
        ("publishing", Some(SourceRootDeletionPhase::Publishing)),
    ] {
        let data_dir = temp_dir(&format!("ipc-source-root-recovery-{fixture_name}-data"));
        seed_snapshot_state(&data_dir);
        let source = data_dir.join("synthetic-source");
        fs::create_dir_all(&source).unwrap();
        let started_at = UnixTimestamp::from_unix_seconds(1_700_300_210);
        let receipt = {
            let store = open_owned_store(&data_dir);
            let source_path = path_str(&source);
            let root = store
                .register_source_root(
                    source_path,
                    source_path,
                    "Synthetic recovery source",
                    started_at,
                )
                .unwrap();
            let receipt = store
                .begin_source_root_deletion(&root.id, started_at)
                .unwrap();
            if let Some(terminal_phase) = seeded_phase {
                for phase in [SourceRootDeletionPhase::Quiescing, terminal_phase] {
                    store
                        .set_source_root_deletion_phase(&root.id, phase, started_at)
                        .unwrap();
                }
            }
            receipt
        };

        const REQUEST_BUDGET: usize = 32;
        let mut child = start_ipc_daemon(&data_dir, REQUEST_BUDGET);
        let endpoint = read_ipc_endpoint(&mut child, &data_dir);
        let token = read_ipc_auth_token(&data_dir);
        let (child_id, instance_id) = daemon_identity(&child, &data_dir);
        let (remaining_requests, _) = wait_for_source_root_absent(
            &endpoint,
            &token,
            receipt.root_id.as_str(),
            REQUEST_BUDGET - 1,
        );
        assert_same_daemon_instance(&mut child, &data_dir, child_id, &instance_id);
        drain_status_requests(&endpoint, remaining_requests + 1);

        let output = wait_child(child);
        assert!(output.success, "stderr:\n{}", output.stderr);
        let store = open_owned_store(&data_dir);
        let recovered_receipt = store
            .source_root_deletion(&receipt.root_id)
            .unwrap()
            .unwrap();
        assert!(output.stderr.is_empty(), "stderr:\n{}", output.stderr);
        assert_eq!(recovered_receipt.phase, SourceRootDeletionPhase::Complete);
        assert_eq!(recovered_receipt.started_at, receipt.started_at);
        assert_eq!(recovered_receipt.root_id, receipt.root_id);
        assert!(store.source_root(&receipt.root_id).unwrap().is_none());
        assert!(source.is_dir());
        drop(store);
        remove_dir(&data_dir);
    }
}

#[test]
fn delete_cannot_publish_when_index_publication_capability_is_unavailable() {
    let data_dir = temp_dir("ipc-delete-runtime-gate-data");
    seed_snapshot_state(&data_dir);
    let state_before = ReadMetaStore::open_data_dir(&data_dir)
        .unwrap()
        .search_projection_state()
        .unwrap();
    let mut child = start_degraded_ipc_daemon(&data_dir, 1);
    let endpoint = read_ipc_endpoint(&mut child, &data_dir);
    let token = read_ipc_auth_token(&data_dir);

    let response = http_post_json(&endpoint, "/delete", &token, serde_json::json!({}));
    assert!(response.starts_with("HTTP/1.1 503"), "{response}");
    let body = response_json(&response);
    assert_eq!(body["schema_version"], "resume-ir.error.v3");
    assert_eq!(body["error"]["code"], "CAPABILITY_UNAVAILABLE");
    assert_eq!(body["error"]["capability"], "index_publication");
    assert_eq!(body["error"]["reason"], "embedding_unavailable");

    let output = wait_child(child);
    assert!(output.success, "stderr:\n{}", output.stderr);
    let state_after = ReadMetaStore::open_data_dir(&data_dir)
        .unwrap()
        .search_projection_state()
        .unwrap();
    assert_eq!(state_after, state_before);
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
            "2",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start resume-daemon ipc");

    let endpoint = read_ipc_endpoint(&mut child, &data_dir);
    let token = read_ipc_auth_token(&data_dir);
    wait_for_serving_control_plane(&mut child, &endpoint, &token);
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
    drain_status_requests(&endpoint, 2);

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
            "2",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start resume-daemon ipc");

    let endpoint = read_ipc_endpoint(&mut child, &data_dir);
    let token = read_ipc_auth_token(&data_dir);
    wait_for_serving_control_plane(&mut child, &endpoint, &token);
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
            "2",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start resume-daemon ipc");

    let endpoint = read_ipc_endpoint(&mut child, &data_dir);
    let token = read_ipc_auth_token(&data_dir);
    wait_for_serving_control_plane(&mut child, &endpoint, &token);
    let response = http_post_import_command(&endpoint, None, &fixture_root, Some(1));

    assert!(response.contains("HTTP/1.1 401 Unauthorized"));
    assert!(response.contains("\"status\":\"error\""));
    assert!(response.contains("\"code\":\"UNAUTHORIZED\""));
    assert!(response.contains("\"action\":\"authenticate\""));
    assert!(!response.contains(path_str(&data_dir)));
    assert!(!response.contains(path_str(&fixture_root)));
    assert!(!response.contains(path_str(&canonical_fixture_root)));
    assert!(!response.contains("raw_resume_text"));

    let output = wait_child(child);
    assert!(output.success, "stderr:\n{}", output.stderr);
    assert!(output.stderr.is_empty());

    let store = ReadMetaStore::open_data_dir(&data_dir).unwrap();
    assert_eq!(store.status_summary().unwrap().import_tasks_queued, 0);

    remove_dir(&data_dir);
}

#[test]
#[cfg_attr(
    not(feature = "native-runtime-tests"),
    ignore = "requires reviewed native runtime packs"
)]
fn daemon_authenticates_and_queues_import_command_over_ipc() {
    let runtime_capacity = support::import_runtime_capacity_lease();
    let data_dir = temp_dir("ipc-import-command-data");
    seed_snapshot_state(&data_dir);
    let fixture_root = fixture_root();
    let canonical_fixture_root = fs::canonicalize(&fixture_root).unwrap();
    let mut child = support::import_capable_daemon_command(&runtime_capacity)
        .args([
            "--data-dir",
            path_str(&data_dir),
            "run",
            "--foreground",
            "--ipc-listen",
            "127.0.0.1:0",
            "--max-requests",
            "3",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start resume-daemon ipc");

    let endpoint = read_ipc_endpoint(&mut child, &data_dir);
    let token = read_ipc_auth_token(&data_dir);
    wait_for_ready_control_plane(&mut child, &endpoint, &token);
    let response = http_post_import_command(&endpoint, Some(&token), &fixture_root, Some(1));
    let status_response = status_after_snapshot_refresh(&endpoint, &token);

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
    assert!(
        status_response.contains("\"files_discovered\":0"),
        "{status_response}"
    );
    assert!(status_response.contains("\"scan_profile\":\"explicit\""));
    assert!(!status_response.contains(path_str(&data_dir)));
    assert!(!status_response.contains(path_str(&fixture_root)));
    assert!(!status_response.contains(path_str(&canonical_fixture_root)));

    let output = wait_child(child);
    assert!(output.success, "stderr:\n{}", output.stderr);
    assert!(output.stderr.is_empty());

    let store = ReadMetaStore::open_data_dir(&data_dir).unwrap();
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
#[cfg_attr(
    not(feature = "native-runtime-tests"),
    ignore = "requires reviewed native runtime packs"
)]
fn daemon_rejects_import_before_mutation_when_writer_contract_is_unsupported() {
    #[cfg(unix)]
    use std::os::unix::process::CommandExt;

    let runtime_capacity = support::import_runtime_capacity_lease();
    let data_dir = temp_dir("ipc-import-unsupported-writer-data");
    seed_snapshot_state(&data_dir);
    let fixture_root = fixture_root();
    let canonical_fixture_root = fs::canonicalize(&fixture_root).unwrap();
    let launch_id = "3".repeat(64);
    let mut command = support::import_capable_daemon_command(&runtime_capacity);
    command
        .args([
            "--data-dir",
            path_str(&data_dir),
            "run",
            "--foreground",
            "--parent-lifecycle-stdin",
            "--launch-id",
            launch_id.as_str(),
            "--work-imports",
            "--work-index",
            "--worker-interval-ms",
            "25",
            "--ipc-listen",
            "127.0.0.1:0",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    #[cfg(unix)]
    command.process_group(0);
    let mut child = command
        .spawn()
        .expect("start import daemon with an unsupported writer contract");

    let endpoint = read_ipc_endpoint(&mut child, &data_dir);
    let token = read_ipc_auth_token(&data_dir);
    wait_for_ready_control_plane(&mut child, &endpoint, &token);
    let status_response = wait_for_text_import_blocked_by_writer(&mut child, &endpoint, &token);
    let status = response_json(&status_response);
    assert_eq!(status["writer"]["state"], "unavailable");
    assert_eq!(status["writer"]["reason"], "unsupported_transition");
    assert_eq!(status["capabilities"]["text_import"]["state"], "blocked");
    assert_eq!(
        status["capabilities"]["text_import"]["reason"],
        "writer_unavailable"
    );

    let queued_before = ReadMetaStore::open_data_dir(&data_dir)
        .unwrap()
        .status_summary()
        .unwrap()
        .import_tasks_queued;
    let import_response = http_post_import_command(&endpoint, Some(&token), &fixture_root, Some(1));
    assert!(
        import_response.starts_with("HTTP/1.1 503 Service Unavailable"),
        "{import_response}"
    );
    let import_error = response_json(&import_response);
    assert_eq!(import_error["error"]["code"], "CAPABILITY_UNAVAILABLE");
    assert_eq!(import_error["error"]["capability"], "text_import");
    assert_eq!(import_error["error"]["reason"], "writer_unavailable");

    let registration_error = post_source_root_register(
        &endpoint,
        &token,
        path_str(&canonical_fixture_root),
        "Synthetic unsupported writer root",
        "HTTP/1.1 503 Service Unavailable",
    );
    assert_eq!(
        registration_error["error"]["code"],
        "CAPABILITY_UNAVAILABLE"
    );
    assert_eq!(registration_error["error"]["capability"], "text_import");
    assert_eq!(registration_error["error"]["reason"], "writer_unavailable");
    let roots = source_roots_json(&endpoint, &token);
    assert!(roots["roots"].as_array().unwrap().is_empty());

    let queued_after = ReadMetaStore::open_data_dir(&data_dir)
        .unwrap()
        .status_summary()
        .unwrap()
        .import_tasks_queued;
    assert_eq!(queued_after, queued_before);
    assert!(!status_response.contains(path_str(&data_dir)));
    assert!(!import_response.contains(path_str(&data_dir)));
    assert!(!import_response.contains(path_str(&fixture_root)));
    assert!(!import_response.contains(path_str(&canonical_fixture_root)));
    assert!(!import_response.contains(&token));

    drop(child.stdin.take());
    let output = wait_child(child);
    assert!(output.success, "stderr:\n{}", output.stderr);
    assert!(output.stderr.is_empty(), "stderr:\n{}", output.stderr);
    remove_dir(&data_dir);
}

#[test]
#[cfg_attr(
    not(feature = "native-runtime-tests"),
    ignore = "requires reviewed native runtime packs"
)]
fn daemon_import_response_reflects_the_persisted_replacement_budget() {
    let runtime_capacity = support::import_runtime_capacity_lease();
    let data_dir = temp_dir("ipc-import-budget-replacement-data");
    seed_snapshot_state(&data_dir);
    let root = temp_dir("ipc-import-budget-replacement-root");
    let canonical_root = fs::canonicalize(&root).unwrap();
    let mut child = start_import_capable_ipc_daemon(&runtime_capacity, &data_dir, 2);
    let endpoint = read_ipc_endpoint(&mut child, &data_dir);
    let token = read_ipc_auth_token(&data_dir);

    let finite = http_post_import_command(&endpoint, Some(&token), &root, Some(1));
    let unbounded = http_post_import_command(&endpoint, Some(&token), &root, None);
    let finite_payload: serde_json::Value =
        serde_json::from_str(finite.split("\r\n\r\n").nth(1).unwrap()).unwrap();
    let unbounded_payload: serde_json::Value =
        serde_json::from_str(unbounded.split("\r\n\r\n").nth(1).unwrap()).unwrap();
    assert!(finite.contains("HTTP/1.1 202 Accepted"));
    assert!(unbounded.contains("HTTP/1.1 202 Accepted"));
    assert_eq!(finite_payload["new_tasks"], 1);
    assert_eq!(finite_payload["scan_file_limit"], 1);
    assert_eq!(unbounded_payload["new_tasks"], 1);
    assert_eq!(
        unbounded_payload["scan_file_limit"],
        serde_json::Value::Null
    );
    let finite_id =
        ImportTaskId::from_str(finite_payload["task_ids"][0].as_str().unwrap()).unwrap();
    let unbounded_id =
        ImportTaskId::from_str(unbounded_payload["task_ids"][0].as_str().unwrap()).unwrap();
    assert_ne!(finite_id, unbounded_id);

    let output = wait_child(child);
    assert!(output.success, "stderr:\n{}", output.stderr);
    assert!(output.stderr.is_empty());
    let store = ReadMetaStore::open_data_dir(&data_dir).unwrap();
    assert!(store.is_import_task_cancelled(&finite_id).unwrap());
    assert_eq!(
        store
            .latest_import_task_by_root(path_str(&canonical_root))
            .unwrap()
            .unwrap()
            .id,
        unbounded_id
    );
    let persisted_scope = store
        .import_scan_scope_by_task_id(&unbounded_id)
        .unwrap()
        .unwrap();
    assert_eq!(persisted_scope.import_task_id, unbounded_id);
    assert_eq!(persisted_scope.scan_budget_kind, None);
    assert_eq!(persisted_scope.scan_budget_limit, None);
    assert!(!finite.contains(path_str(&root)));
    assert!(!unbounded.contains(path_str(&root)));

    remove_dir(&data_dir);
    remove_dir(&root);
}

#[test]
#[cfg_attr(
    not(feature = "native-runtime-tests"),
    ignore = "requires reviewed native runtime packs"
)]
fn daemon_multi_root_import_conflict_rolls_back_every_root() {
    let runtime_capacity = support::import_runtime_capacity_lease();
    let data_dir = temp_dir("ipc-import-atomic-conflict-data");
    let roots_parent = temp_dir("ipc-import-atomic-conflict-roots");
    let first_root = roots_parent.join("a-first");
    let running_root = roots_parent.join("z-running");
    fs::create_dir_all(&first_root).unwrap();
    fs::create_dir_all(&running_root).unwrap();
    let canonical_first = fs::canonicalize(&first_root).unwrap();
    let canonical_running = fs::canonicalize(&running_root).unwrap();
    let started_at_seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
        .saturating_sub(60) as i64;
    let running_id = seed_running_import_task(
        &data_dir,
        "atomic-running",
        &canonical_running,
        started_at_seconds,
    );
    let task_owner = ImportTaskOwnerLock::acquire(&data_dir, &running_id).unwrap();

    let mut child = start_import_capable_ipc_daemon(&runtime_capacity, &data_dir, 2);
    let endpoint = read_ipc_endpoint(&mut child, &data_dir);
    let token = read_ipc_auth_token(&data_dir);
    assert_text_import_available_without_ocr(&http_get(&endpoint, &token));

    let response = http_post_import_command_value(
        &endpoint,
        Some(&token),
        serde_json::json!({
            "roots": [path_str(&first_root), path_str(&running_root)],
            "profile": "explicit",
            "max_files": null,
        }),
    );
    assert!(response.contains("HTTP/1.1 409 Conflict"));
    assert_conflict_error_v2(&response);
    assert!(!response.contains("import task is already running"));
    assert!(!response.contains(path_str(&first_root)));
    assert!(!response.contains(path_str(&running_root)));

    let output = wait_child(child);
    assert!(output.success, "stderr:\n{}", output.stderr);
    assert!(output.stderr.is_empty());
    let store = ReadMetaStore::open_data_dir(&data_dir).unwrap();
    assert!(store
        .latest_import_task_by_root(path_str(&canonical_first))
        .unwrap()
        .is_none());
    assert!(!store
        .active_authorized_import_roots()
        .unwrap()
        .iter()
        .any(|root| root == path_str(&canonical_first)));
    assert_eq!(
        store
            .latest_import_task_by_root(path_str(&canonical_running))
            .unwrap()
            .unwrap()
            .id,
        running_id
    );
    assert!(!store.is_import_task_cancelled(&running_id).unwrap());

    drop(task_owner);
    remove_dir(&data_dir);
    remove_dir(&roots_parent);
}

#[test]
#[cfg_attr(
    not(feature = "native-runtime-tests"),
    ignore = "requires reviewed native runtime packs"
)]
fn daemon_controls_managed_root_durably_with_bounded_path_free_contract() {
    let runtime_capacity = support::import_runtime_capacity_lease();
    let data_dir = temp_dir("ipc-root-control-data");
    let root_a = fs::canonicalize(temp_dir("ipc-root-control-a")).unwrap();
    let root_b = fs::canonicalize(temp_dir("ipc-root-control-b")).unwrap();
    let unknown = fs::canonicalize(temp_dir("ipc-root-control-unknown")).unwrap();
    seed_completed_import_scope(&data_dir, "root-a", &root_a);
    seed_completed_import_scope(&data_dir, "root-b", &root_b);

    let mut child = start_import_capable_ipc_daemon(&runtime_capacity, &data_dir, 8);
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

    let store = ReadMetaStore::open_data_dir(&data_dir).unwrap();
    assert_eq!(
        store.import_root_control_status(path_str(&root_a)).unwrap(),
        Some(ImportRootControlStatus::Paused)
    );
    drop(store);

    let mut child = start_import_capable_ipc_daemon(&runtime_capacity, &data_dir, 3);
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
    let store = ReadMetaStore::open_data_dir(&data_dir).unwrap();
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
#[cfg_attr(
    not(feature = "native-runtime-tests"),
    ignore = "requires reviewed native runtime packs"
)]
fn daemon_import_command_can_requeue_root_after_prior_task_cancelled() {
    let runtime_capacity = support::import_runtime_capacity_lease();
    let data_dir = temp_dir("ipc-import-command-cancel-requeue-data");
    seed_snapshot_state(&data_dir);
    let fixture_root = fixture_root();
    let canonical_fixture_root = fs::canonicalize(&fixture_root).unwrap();
    let mut child = support::import_capable_daemon_command(&runtime_capacity)
        .args([
            "--data-dir",
            path_str(&data_dir),
            "run",
            "--foreground",
            "--ipc-listen",
            "127.0.0.1:0",
            "--max-requests",
            "4",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start resume-daemon ipc");

    let endpoint = read_ipc_endpoint(&mut child, &data_dir);
    let token = read_ipc_auth_token(&data_dir);
    wait_for_ready_control_plane(&mut child, &endpoint, &token);
    let first_response = http_post_import_command(&endpoint, Some(&token), &fixture_root, Some(1));
    assert!(first_response.contains("HTTP/1.1 202 Accepted"));
    assert!(first_response.contains("\"new_tasks\":1"));

    let store = ReadMetaStore::open_data_dir(&data_dir).unwrap();
    let first_scope = store.latest_import_scan_scope().unwrap().unwrap();
    let cancelled =
        http_post_import_cancel_command(&endpoint, Some(&token), &first_scope.import_task_id);
    assert!(cancelled.contains("HTTP/1.1 202 Accepted"));

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
#[cfg_attr(
    not(feature = "native-runtime-tests"),
    ignore = "requires reviewed native runtime packs"
)]
fn daemon_import_command_preserves_local_discovery_preset_scope() {
    let runtime_capacity = support::import_runtime_capacity_lease();
    let data_dir = temp_dir("ipc-import-preset-command-data");
    seed_snapshot_state(&data_dir);
    let fixture_root = fixture_root();
    let canonical_fixture_root = fs::canonicalize(&fixture_root).unwrap();
    let mut child = support::import_capable_daemon_command(&runtime_capacity)
        .args([
            "--data-dir",
            path_str(&data_dir),
            "run",
            "--foreground",
            "--ipc-listen",
            "127.0.0.1:0",
            "--max-requests",
            "3",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start resume-daemon ipc");

    let endpoint = read_ipc_endpoint(&mut child, &data_dir);
    let token = read_ipc_auth_token(&data_dir);
    wait_for_ready_control_plane(&mut child, &endpoint, &token);
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
    drain_status_requests(&endpoint, 2);

    let output = wait_child(child);
    assert!(output.success, "stderr:\n{}", output.stderr);
    assert!(output.stderr.is_empty());

    let store = ReadMetaStore::open_data_dir(&data_dir).unwrap();
    let scope = store.latest_import_scan_scope().unwrap().unwrap();
    assert_eq!(scope.root_kind, ImportRootKind::Preset);
    assert_eq!(scope.root_preset, Some(ImportRootPreset::LocalDiscovery));
    assert_eq!(scope.scan_profile, ImportScanProfile::Explicit);
    assert_eq!(scope.requested_root_path, path_str(&fixture_root));
    assert_eq!(scope.canonical_root_path, path_str(&canonical_fixture_root));

    remove_dir(&data_dir);
}

#[test]
#[cfg_attr(
    not(feature = "native-runtime-tests"),
    ignore = "requires reviewed native runtime packs"
)]
fn daemon_import_cancel_command_records_cancellation_without_path_leak() {
    let runtime_capacity = support::import_runtime_capacity_lease();
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
    let mut child = support::import_capable_daemon_command(&runtime_capacity)
        .args([
            "--data-dir",
            path_str(&data_dir),
            "run",
            "--foreground",
            "--ipc-listen",
            "127.0.0.1:0",
            "--max-requests",
            "3",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start resume-daemon ipc");

    let endpoint = read_ipc_endpoint(&mut child, &data_dir);
    let token = read_ipc_auth_token(&data_dir);
    wait_for_ready_control_plane(&mut child, &endpoint, &token);
    let response = http_post_import_cancel_command(&endpoint, Some(&token), &task_id);
    assert!(response.contains("HTTP/1.1 202 Accepted"));
    assert!(response.contains("\"schema_version\":\"daemon.import_cancel.v1\""));
    assert!(response.contains("\"status\":\"cancel_requested\""));
    assert!(response.contains(&format!("\"task_id\":\"{task_id}\"")));
    assert!(!response.contains(path_str(&data_dir)));
    assert!(!response.contains(path_str(&fixture_root)));
    assert!(!response.contains(path_str(&canonical_fixture_root)));
    assert!(!response.contains(&token));

    let status_response = status_after_snapshot_refresh(&endpoint, &token);
    assert!(status_response.contains("HTTP/1.1 200 OK"));
    assert!(status_response.contains("\"import_tasks_cancelled\":1"));
    assert!(!status_response.contains(path_str(&data_dir)));
    assert!(!status_response.contains(path_str(&fixture_root)));
    assert!(!status_response.contains(path_str(&canonical_fixture_root)));

    let output = wait_child(child);
    assert!(output.success, "stderr:\n{}", output.stderr);
    assert!(output.stderr.is_empty());

    let store = ReadMetaStore::open_data_dir(&data_dir).unwrap();
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
            "2",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start resume-daemon ipc");

    let endpoint = read_ipc_endpoint(&mut child, &data_dir);
    let token = read_ipc_auth_token(&data_dir);
    wait_for_serving_control_plane(&mut child, &endpoint, &token);
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
    let store = ReadMetaStore::open_data_dir(&data_dir).unwrap();
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
            "3",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start resume-daemon ipc");

    let endpoint = read_ipc_endpoint(&mut child, &data_dir);
    let token = read_ipc_auth_token(&data_dir);
    wait_for_serving_control_plane(&mut child, &endpoint, &token);
    let malformed = raw_ipc_request(
        &endpoint,
        b"POST /imports HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Length: nope\r\n\r\n",
    );
    let status_response = http_get(&endpoint, &token);

    assert!(malformed.contains("HTTP/1.1 400 Bad Request"));
    assert!(status_response.contains("HTTP/1.1 200 OK"));
    assert!(status_response.contains("\"process_state\":\"ready\""));
    assert!(
        status_response.contains("\"status\":\"repairing\""),
        "{status_response}"
    );
    assert!(status_response
        .contains("\"core\":{\"reason\":\"migration_rebuild\",\"state\":\"repairing\"}"));

    let output = wait_child(child);
    assert!(output.success, "stderr:\n{}", output.stderr);
    assert!(output.stderr.is_empty());

    remove_dir(&data_dir);
}

#[test]
fn daemon_survives_client_disconnect_across_every_product_ipc_route() {
    const EXPECTED_ACCEPTED: u64 = 16;

    let data_dir = temp_dir("ipc-response-disconnect-matrix-data");
    seed_snapshot_state(&data_dir);
    let (mut child, baseline_ipc) =
        start_ipc_daemon_with_baseline(&data_dir, EXPECTED_ACCEPTED as usize);
    let child_id = child.id();
    let endpoint = read_ipc_endpoint(&mut child, &data_dir);
    let token = read_ipc_auth_token(&data_dir);
    let instance_id = read_ipc_owner_file(&data_dir, "ipc.endpoints.json")["instance_id"]
        .as_str()
        .expect("daemon instance id")
        .to_string();

    let setup_search = http_post_json(
        &endpoint,
        "/search",
        &token,
        search_request("disconnect-selection-setup", "codex_validation"),
    );
    assert_search_response(&setup_search, "disconnect-selection-setup");
    let selection = response_json(&setup_search)["results"][0]["selection"].clone();

    disconnect_after_response_started(
        &endpoint,
        authenticated_get_request(&endpoint, "/status", &token),
    );
    let status = http_get(&endpoint, &token);
    assert_ready_status(&status);
    assert_same_daemon_instance(&mut child, &data_dir, child_id, &instance_id);

    disconnect_after_response_started(
        &endpoint,
        authenticated_post_request(
            &endpoint,
            "/search",
            &token,
            search_request("disconnect-search", "codex_validation"),
        ),
    );
    let search = http_post_json(
        &endpoint,
        "/search",
        &token,
        search_request("disconnect-search-probe", "codex_validation"),
    );
    assert_search_response(&search, "disconnect-search-probe");
    assert_same_daemon_instance(&mut child, &data_dir, child_id, &instance_id);

    disconnect_after_response_started(
        &endpoint,
        authenticated_post_request(
            &endpoint,
            "/search/batch",
            &token,
            search_batch_request("disconnect-batch", "disconnect-batch-child"),
        ),
    );
    // A response header only proves that the batch stream started. The server
    // finishes this route's child dispatch before accepting the next request,
    // so advancing the same FIFO worker is a causal terminal barrier. The
    // diagnostics closure below then proves every earlier connection recorded
    // exactly one terminal outcome.
    let batch_terminal_barrier = http_post_json(
        &endpoint,
        "/search",
        &token,
        search_request("disconnect-batch-terminal-barrier", "codex_validation"),
    );
    assert_search_response(&batch_terminal_barrier, "disconnect-batch-terminal-barrier");
    let terminal_diagnostics = http_get_diagnostics(&endpoint, Some(&token));
    let terminal_payload = response_json(&terminal_diagnostics);
    let terminal_ipc = TestIpcMetrics::from_diagnostics(&terminal_payload);
    let terminal_delta = terminal_ipc.delta_from(baseline_ipc);
    assert_eq!(terminal_delta.accepted, 8);
    assert_eq!(
        terminal_delta.terminal(),
        8,
        "the disconnected batch must be terminal before its successor: {terminal_ipc:?}"
    );
    let batch = http_post_json(
        &endpoint,
        "/search/batch",
        &token,
        search_batch_request("disconnect-batch-probe", "disconnect-batch-probe-child"),
    );
    assert_batch_response(
        &batch,
        "disconnect-batch-probe",
        "disconnect-batch-probe-child",
    );
    assert_same_daemon_instance(&mut child, &data_dir, child_id, &instance_id);

    disconnect_after_response_started(
        &endpoint,
        authenticated_post_request(
            &endpoint,
            "/details",
            &token,
            detail_request("disconnect-detail", &selection),
        ),
    );
    let detail = http_post_json(
        &endpoint,
        "/details",
        &token,
        detail_request("disconnect-detail-probe", &selection),
    );
    assert_json_response(
        &detail,
        "resume-ir.detail-response.v3",
        "disconnect-detail-probe",
        &selection,
    );
    assert_same_daemon_instance(&mut child, &data_dir, child_id, &instance_id);

    disconnect_after_response_started(
        &endpoint,
        authenticated_post_request(
            &endpoint,
            "/details/hydrate",
            &token,
            hydrate_request("disconnect-hydrate", &selection),
        ),
    );
    let hydrate = http_post_json(
        &endpoint,
        "/details/hydrate",
        &token,
        hydrate_request("disconnect-hydrate-probe", &selection),
    );
    assert_json_response(
        &hydrate,
        "resume-ir.detail-hydrate-response.v3",
        "disconnect-hydrate-probe",
        &selection,
    );
    assert_same_daemon_instance(&mut child, &data_dir, child_id, &instance_id);

    disconnect_after_response_started(
        &endpoint,
        authenticated_get_request(&endpoint, "/imports/progress", &token),
    );
    let progress = http_get_import_progress(&endpoint, Some(&token));
    assert!(progress.contains("HTTP/1.1 200 OK"));
    assert_eq!(
        progress.matches("\"event\":\"snapshot\"").count(),
        3,
        "{progress}"
    );
    assert_same_daemon_instance(&mut child, &data_dir, child_id, &instance_id);

    let diagnostics = http_get_diagnostics(&endpoint, Some(&token));
    let diagnostics_body = diagnostics.split("\r\n\r\n").nth(1).unwrap();
    let diagnostics_payload: serde_json::Value = serde_json::from_str(diagnostics_body).unwrap();
    let ipc = TestIpcMetrics::from_diagnostics(&diagnostics_payload);
    let delta = ipc.delta_from(baseline_ipc);
    assert_eq!(delta.accepted, EXPECTED_ACCEPTED);
    assert_eq!(
        delta.terminal(),
        EXPECTED_ACCEPTED,
        "every admitted connection since the serving baseline must have one terminal outcome: {ipc:?}"
    );
    assert!(
        delta.response_failure >= 1,
        "response-side client disconnect was not observed: {ipc:?}"
    );
    assert_eq!(delta.client_disconnect, delta.response_failure);
    assert!(!diagnostics_body.contains(path_str(&data_dir)));
    assert!(!diagnostics_body.contains(&token));

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
    let token = read_ipc_auth_token(&data_dir);
    assert!(http_get(&endpoint, &token).contains("HTTP/1.1 200 OK"));
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
    assert!(
        http_get(&first_endpoint, first_auth["token"].as_str().unwrap())
            .contains("HTTP/1.1 200 OK")
    );
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
        "schema_version": "resume-ir.daemon-auth.v3",
        "launch_id": "d".repeat(64),
        "instance_id": fake_instance_id.clone(),
        "token": "e".repeat(64),
    });
    let fake_manifest = serde_json::json!({
        "schema_version": "resume-ir.daemon-ipc.v5",
        "launch_id": "d".repeat(64),
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
    let (mut child, baseline_ipc) = start_ipc_daemon_with_baseline(&data_dir, 101);
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
    let ipc = TestIpcMetrics::from_diagnostics(&payload);
    let delta = ipc.delta_from(baseline_ipc);
    assert_eq!(delta.accepted, 101);
    assert_eq!(delta.terminal(), 101);
    assert!(delta.completed > 0);
    assert!(delta.client_disconnect <= delta.response_failure);
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
    let (mut child, baseline_ipc) = start_ipc_daemon_with_baseline(&data_dir, max_requests);
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
            baseline_ipc,
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
        baseline_ipc,
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

fn assert_soak_diagnostics(
    response: &str,
    baseline: TestIpcMetrics,
    expected_accepted: usize,
    data_dir: &Path,
    token: &str,
) {
    let body = response.split("\r\n\r\n").nth(1).unwrap();
    let payload: serde_json::Value = serde_json::from_str(body).unwrap();
    let ipc = TestIpcMetrics::from_diagnostics(&payload);
    let delta = ipc.delta_from(baseline);
    assert_eq!(delta.accepted, expected_accepted as u64);
    assert_eq!(delta.terminal(), expected_accepted as u64);
    assert!(delta.completed > 0);
    assert!(delta.client_disconnect <= delta.response_failure);
    assert!(!body.contains(path_str(data_dir)));
    assert!(!body.contains(token));
}

#[test]
#[cfg_attr(
    not(feature = "native-runtime-tests"),
    ignore = "requires reviewed native runtime packs"
)]
fn daemon_rejects_import_command_for_running_root_without_rewriting_scope() {
    let runtime_capacity = support::import_runtime_capacity_lease();
    let data_dir = temp_dir("ipc-import-running-conflict-data");
    let fixture_root = temp_dir("ipc-import-running-conflict-root");
    let canonical_fixture_root = fs::canonicalize(&fixture_root).unwrap();
    let started_at_seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
        .saturating_sub(60) as i64;
    let task_id = seed_running_import_task(
        &data_dir,
        "ipc-running-conflict",
        &canonical_fixture_root,
        started_at_seconds,
    );
    let task_owner = ImportTaskOwnerLock::acquire(&data_dir, &task_id).unwrap();
    let mut child = start_import_capable_ipc_daemon(&runtime_capacity, &data_dir, 2);

    let endpoint = read_ipc_endpoint(&mut child, &data_dir);
    let token = read_ipc_auth_token(&data_dir);
    assert_text_import_available_without_ocr(&http_get(&endpoint, &token));
    let response = http_post_import_command(&endpoint, Some(&token), &fixture_root, Some(1));

    assert!(response.contains("HTTP/1.1 409 Conflict"));
    assert_conflict_error_v2(&response);
    assert!(!response.contains(path_str(&data_dir)));
    assert!(!response.contains(path_str(&fixture_root)));
    assert!(!response.contains(path_str(&canonical_fixture_root)));
    assert!(!response.contains(&token));

    let output = wait_child(child);
    assert!(output.success, "stderr:\n{}", output.stderr);
    assert!(output.stderr.is_empty());

    let store = ReadMetaStore::open_data_dir(&data_dir).unwrap();
    assert_eq!(
        store
            .latest_import_task_by_root(path_str(&canonical_fixture_root))
            .unwrap()
            .unwrap()
            .id,
        task_id
    );
    let scope = store
        .import_scan_scope_by_task_id(&task_id)
        .unwrap()
        .unwrap();
    assert_eq!(scope.scan_budget_limit, None);

    drop(task_owner);
    remove_dir(&data_dir);
    remove_dir(&fixture_root);
}

#[test]
#[cfg_attr(
    not(feature = "native-runtime-tests"),
    ignore = "requires reviewed native runtime packs"
)]
fn daemon_import_command_ipc_feeds_running_import_worker_loop() {
    #[cfg(unix)]
    use std::os::unix::process::CommandExt;

    let runtime_capacity = support::import_runtime_capacity_lease();
    let data_dir = temp_dir("ipc-import-command-worker-data");
    let fixture_root = fixture_root();
    let canonical_fixture_root = fs::canonicalize(&fixture_root).unwrap();
    let launch_id = "6".repeat(64);
    let mut command = support::fully_capable_daemon_command(&runtime_capacity);
    command
        .args([
            "--data-dir",
            path_str(&data_dir),
            "run",
            "--foreground",
            "--parent-lifecycle-stdin",
            "--launch-id",
            launch_id.as_str(),
            "--work-imports",
            "--work-index",
            "--worker-interval-ms",
            "25",
            "--ipc-listen",
            "127.0.0.1:0",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    #[cfg(unix)]
    command.process_group(0);
    let mut child = command
        .spawn()
        .expect("start resume-daemon ipc plus import worker");

    let endpoint = read_ipc_endpoint(&mut child, &data_dir);
    let token = read_ipc_auth_token(&data_dir);
    wait_for_ready_control_plane(&mut child, &endpoint, &token);
    let response = http_post_import_command(&endpoint, Some(&token), &fixture_root, None);
    assert!(response.contains("HTTP/1.1 202 Accepted"));
    let task_id = response_json(&response)["task_ids"][0]
        .as_str()
        .unwrap()
        .to_string();

    let (_, completed_response) = wait_for_searchable_documents(
        &mut child,
        &data_dir,
        &endpoint,
        2,
        IMPORT_WORKER_SEARCHABLE_MAX_REQUESTS,
    );
    let progress = http_get_import_progress(&endpoint, Some(&token));
    let event: serde_json::Value = serde_json::from_str(
        progress
            .split("\r\n\r\n")
            .nth(1)
            .unwrap()
            .lines()
            .next()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(event["schema_version"], "daemon.import_progress.v1");
    assert_eq!(
        event["latest_import_attribution"]["schema_version"],
        "daemon.import_attribution.v1"
    );
    assert_eq!(event["latest_import_attribution"]["task_id"], task_id);
    assert!(event["latest_import_attribution"]["ocr"]["jobs_queued"]
        .as_u64()
        .is_some_and(|count| count > 0));
    let attribution = event["latest_import_attribution"].to_string();
    for forbidden in [
        "canonical_root",
        "path",
        "raw_text",
        "content_hash",
        "vectors",
    ] {
        assert!(!attribution.contains(forbidden), "{attribution}");
    }
    drop(child.stdin.take());
    let output = wait_child(child);
    assert!(output.success, "stderr:\n{}", output.stderr);
    assert!(output.stderr.is_empty());

    let store = ReadMetaStore::open_data_dir(&data_dir).unwrap();
    let scope = store.latest_import_scan_scope().unwrap().unwrap();
    let task = store
        .import_task_by_id(&scope.import_task_id)
        .unwrap()
        .unwrap();
    assert_eq!(task.status, ImportTaskStatus::Completed);
    let status = store.status_summary().unwrap();
    assert_eq!(status.searchable_documents, 2);
    assert!(status.ocr_queue_depth > 0, "OCR endpoint waited for drain");
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
            "2",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start resume-daemon ipc");

    let endpoint = read_ipc_endpoint(&mut child, &data_dir);
    let auth = read_ipc_owner_file(&data_dir, "ipc.auth");
    wait_for_serving_control_plane(&mut child, &endpoint, auth["token"].as_str().unwrap());
    let permissions = fs::metadata(data_dir.join("ipc.auth"))
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(permissions & 0o777, 0o600);
    assert_eq!(auth["schema_version"], "resume-ir.daemon-auth.v3");
    assert_ne!(auth["token"], legacy_token.trim());
    let response = http_get(&endpoint, auth["token"].as_str().unwrap());

    assert!(response.contains("HTTP/1.1 200 OK"));
    let output = wait_child(child);
    assert!(output.success, "stderr:\n{}", output.stderr);
    assert!(output.stderr.is_empty());
    assert!(!data_dir.join("ipc.auth").exists());

    remove_dir(&data_dir);
}

#[test]
#[cfg_attr(
    not(feature = "native-runtime-tests"),
    ignore = "requires reviewed native runtime packs"
)]
fn daemon_serves_status_while_import_worker_processes_late_queued_task() {
    #[cfg(unix)]
    use std::os::unix::process::CommandExt;

    let runtime_capacity = support::import_runtime_capacity_lease();
    let data_dir = temp_dir("ipc-import-worker-data");
    let fixture_root = fixture_root();
    let canonical_fixture_root = fs::canonicalize(&fixture_root).unwrap();
    let launch_id = "7".repeat(64);
    let mut command = support::fully_capable_daemon_command(&runtime_capacity);
    command
        .args([
            "--data-dir",
            path_str(&data_dir),
            "run",
            "--foreground",
            "--parent-lifecycle-stdin",
            "--launch-id",
            launch_id.as_str(),
            "--work-imports",
            "--work-index",
            "--worker-interval-ms",
            "25",
            "--ipc-listen",
            "127.0.0.1:0",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    #[cfg(unix)]
    command.process_group(0);
    let mut child = command
        .spawn()
        .expect("start resume-daemon ipc plus import worker");

    let endpoint = read_ipc_endpoint(&mut child, &data_dir);
    let token = read_ipc_auth_token(&data_dir);
    wait_for_ready_control_plane(&mut child, &endpoint, &token);
    let initial_response = http_get(&endpoint, &token);
    assert!(initial_response.contains("HTTP/1.1 200 OK"));
    assert!(initial_response.contains("\"searchable_documents\":0"));

    let queued_response = http_post_import_command(&endpoint, Some(&token), &fixture_root, None);
    assert!(queued_response.contains("HTTP/1.1 202 Accepted"));
    let queued_payload = response_json(&queued_response);
    let task_id = ImportTaskId::from_str(queued_payload["task_ids"][0].as_str().unwrap()).unwrap();
    let (_, completed_response) = wait_for_searchable_documents(
        &mut child,
        &data_dir,
        &endpoint,
        2,
        IMPORT_WORKER_SEARCHABLE_MAX_REQUESTS,
    );

    drop(child.stdin.take());
    let output = wait_child(child);
    assert!(output.success, "stderr:\n{}", output.stderr);
    assert!(output.stderr.is_empty());

    let store = ReadMetaStore::open_data_dir(&data_dir).unwrap();
    let task = store.import_task_by_id(&task_id).unwrap().unwrap();
    assert_eq!(task.status, ImportTaskStatus::Completed);
    assert_eq!(store.status_summary().unwrap().searchable_documents, 2);
    assert!(!initial_response.contains(path_str(&data_dir)));
    assert!(!initial_response.contains(path_str(&fixture_root)));
    assert!(!initial_response.contains(path_str(&canonical_fixture_root)));
    assert!(!completed_response.contains(path_str(&data_dir)));
    assert!(!completed_response.contains(path_str(&fixture_root)));
    assert!(!completed_response.contains(path_str(&canonical_fixture_root)));
    assert!(!queued_response.contains(&token));

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
    let store = ReadMetaStore::open_data_dir(&data_dir).unwrap();
    let task = store.import_task_by_id(&task_id).unwrap().unwrap();
    assert_eq!(task.status, ImportTaskStatus::Queued);
    assert_eq!(store.status_summary().unwrap().searchable_documents, 0);

    remove_dir(&data_dir);
}

#[test]
fn daemon_binds_control_plane_before_starting_search_artifact_recovery() {
    let data_dir = temp_dir("ipc-bind-before-artifact-recovery-data");
    seed_snapshot_state(&data_dir);
    let state_before = ReadMetaStore::open_data_dir(&data_dir)
        .unwrap()
        .search_projection_state()
        .unwrap();
    fs::remove_dir_all(data_dir.join("search-index")).unwrap();
    let blocker = TcpListener::bind("127.0.0.1:0").expect("bind blocker listener");
    let blocked_addr = blocker.local_addr().unwrap().to_string();

    let output = Command::new(env!("CARGO_BIN_EXE_resume-daemon"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "run",
            "--foreground",
            "--work-index",
            "--ipc-listen",
            &blocked_addr,
        ])
        .output()
        .expect("run resume-daemon with occupied ipc port");

    assert!(!output.status.success());
    assert_fatal_event(
        &String::from_utf8_lossy(&output.stderr),
        "control_plane_failure",
        "restartable",
    );
    let state_after = ReadMetaStore::open_data_dir(&data_dir)
        .unwrap()
        .search_projection_state()
        .unwrap();
    assert_eq!(state_after, state_before);
    assert!(!data_dir.join("search-index").exists());

    remove_dir(&data_dir);
}

#[test]
fn ipc_only_mode_serves_status_without_implicitly_repairing_search_artifacts() {
    let data_dir = temp_dir("ipc-only-does-not-repair-artifacts-data");
    seed_snapshot_state(&data_dir);
    let state_before = ReadMetaStore::open_data_dir(&data_dir)
        .unwrap()
        .search_projection_state()
        .unwrap();
    fs::remove_dir_all(data_dir.join("search-index")).unwrap();
    let vector_root_before = fs::read_dir(data_dir.join("vector-index"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect::<Vec<_>>();
    let mut child = start_ipc_daemon(&data_dir, 1);

    let endpoint = read_ipc_endpoint(&mut child, &data_dir);
    let token = read_ipc_auth_token(&data_dir);
    let response = http_get(&endpoint, &token);
    assert!(response.contains("HTTP/1.1 200 OK"));
    assert!(response.contains(r#""process_state":"ready""#));
    let output = wait_child(child);
    assert!(output.success, "stderr:\n{}", output.stderr);
    assert!(output.stderr.is_empty());

    let state_after = ReadMetaStore::open_data_dir(&data_dir)
        .unwrap()
        .search_projection_state()
        .unwrap();
    assert_eq!(state_after, state_before);
    assert!(!data_dir.join("search-index").exists());
    assert_eq!(
        fs::read_dir(data_dir.join("vector-index"))
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect::<Vec<_>>(),
        vector_root_before
    );

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

fn wait_for_ready_control_plane(child: &mut Child, endpoint: &str, token: &str) {
    let deadline = Instant::now() + IPC_CORE_INITIALIZATION_TIMEOUT;
    let mut last_state = "unobserved".to_string();
    while Instant::now() < deadline {
        if let Some(status) = child.try_wait().expect("poll daemon readiness") {
            let stderr = read_child_stderr(child);
            panic!(
                "daemon exited before its control plane became ready: {status}; last state={last_state}; stderr={stderr}"
            );
        }
        match try_http_get_authenticated(endpoint, token) {
            Ok(response) if response.starts_with("HTTP/1.1 200") => {
                let payload = response_json(&response);
                last_state = payload["core"]["state"]
                    .as_str()
                    .unwrap_or("invalid")
                    .to_string();
                match last_state.as_str() {
                    "ready" => return,
                    "blocked" | "degraded" => {
                        panic!("daemon control plane cannot serve capability test: {payload}")
                    }
                    "initializing" | "repairing" => {}
                    _ => panic!("daemon returned an invalid core state: {payload}"),
                }
            }
            Ok(response) => last_state = response,
            Err(error) => last_state = error.to_string(),
        }
        std::thread::sleep(IPC_ENDPOINT_POLL_DELAY);
    }
    panic!("daemon control plane did not become ready: {last_state}");
}

fn wait_for_text_import_blocked_by_writer(
    child: &mut Child,
    endpoint: &str,
    token: &str,
) -> String {
    let deadline = Instant::now() + IPC_CORE_INITIALIZATION_TIMEOUT;
    let mut last_state = "unobserved".to_string();
    while Instant::now() < deadline {
        if let Some(status) = child.try_wait().expect("poll daemon capability state") {
            let stderr = read_child_stderr(child);
            panic!(
                "daemon exited before text import reflected the writer barrier: {status}; last state={last_state}; stderr={stderr}"
            );
        }
        match try_http_get_authenticated(endpoint, token) {
            Ok(response) if response.starts_with("HTTP/1.1 200") => {
                let payload = response_json(&response);
                let state = payload["capabilities"]["text_import"]["state"]
                    .as_str()
                    .unwrap_or("invalid");
                let reason = payload["capabilities"]["text_import"]["reason"]
                    .as_str()
                    .unwrap_or("none");
                last_state = format!("{state}/{reason}");
                if state == "blocked" && reason == "writer_unavailable" {
                    return response;
                }
            }
            Ok(response) => last_state = response,
            Err(error) => last_state = error.to_string(),
        }
        std::thread::sleep(IPC_ENDPOINT_POLL_DELAY);
    }
    panic!("text import did not reflect the writer barrier: {last_state}");
}

fn wait_for_serving_control_plane(
    child: &mut Child,
    endpoint: &str,
    token: &str,
) -> TestIpcMetrics {
    let deadline = Instant::now() + IPC_CORE_INITIALIZATION_TIMEOUT;
    let mut last_state = "unobserved".to_string();
    while Instant::now() < deadline {
        if let Some(status) = child.try_wait().expect("poll daemon handoff") {
            let stderr = read_child_stderr(child);
            panic!(
                "daemon exited before its full control plane took ownership: {status}; last state={last_state}; stderr={stderr}"
            );
        }
        match try_http_get_authenticated(endpoint, token) {
            Ok(response) if response.starts_with("HTTP/1.1 200") => {
                let payload = response_json(&response);
                last_state = payload["core"]["state"]
                    .as_str()
                    .unwrap_or("invalid")
                    .to_string();
                match last_state.as_str() {
                    "ready" | "repairing" | "degraded" => {
                        return TestIpcMetrics::from_status(&payload)
                    }
                    "blocked" => {
                        panic!(
                            "daemon blocked before its serving route owner was proven: {payload}"
                        )
                    }
                    "initializing" => {}
                    _ => panic!("daemon returned an invalid core state: {payload}"),
                }
            }
            Ok(response) => last_state = response,
            Err(error) => last_state = error.to_string(),
        }
        std::thread::sleep(IPC_ENDPOINT_POLL_DELAY);
    }
    panic!("daemon full control plane did not take ownership: {last_state}");
}

fn wait_for_searchable_documents(
    child: &mut Child,
    data_dir: &Path,
    endpoint: &str,
    expected: usize,
    max_requests: usize,
) -> (usize, String) {
    let deadline = Instant::now() + IMPORT_WORKER_SEARCHABLE_TIMEOUT;
    let token = read_ipc_auth_token(data_dir);
    let mut last_response = String::new();
    for request_count in 1..=max_requests {
        if let Some(status) = child.try_wait().expect("poll daemon child") {
            let stderr = read_child_stderr(child);
            let store_state = describe_store_state(data_dir);
            panic!(
                "daemon exited before searchable document count {expected}: {status}\n{stderr}\n{store_state}"
            );
        }
        let response = match try_http_get_authenticated(endpoint, &token) {
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
    let store = match ReadMetaStore::open_data_dir(data_dir) {
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

fn http_get(endpoint: &str, token: &str) -> String {
    try_http_get_authenticated(endpoint, token).expect("read response")
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

fn http_post_json(
    endpoint: &str,
    request_path: &str,
    token: &str,
    payload: serde_json::Value,
) -> String {
    raw_ipc_request(
        endpoint,
        &authenticated_post_request(endpoint, request_path, token, payload),
    )
}

fn post_source_root_register(
    endpoint: &str,
    token: &str,
    requested_path: &str,
    display_label: &str,
    expected_status: &str,
) -> serde_json::Value {
    let response = http_post_json(
        endpoint,
        "/source-roots/register",
        token,
        serde_json::json!({
            "schema_version": "resume-ir.source-root-register-request.v1",
            "requested_path": requested_path,
            "display_label": display_label
        }),
    );
    assert!(response.starts_with(expected_status), "{response}");
    response_json(&response)
}

fn source_roots_json(endpoint: &str, token: &str) -> serde_json::Value {
    let response = raw_ipc_request(
        endpoint,
        &authenticated_get_request(endpoint, "/source-roots", token),
    );
    assert!(response.starts_with("HTTP/1.1 200 OK"), "{response}");
    response_json(&response)
}

fn listed_source_root<'a>(
    response: &'a serde_json::Value,
    root_id: &str,
) -> Option<&'a serde_json::Value> {
    response["roots"]
        .as_array()?
        .iter()
        .find(|root| root["root_id"] == root_id)
}

fn wait_for_source_root_absent(
    endpoint: &str,
    token: &str,
    root_id: &str,
    request_budget: usize,
) -> (usize, serde_json::Value) {
    let deadline = Instant::now() + Duration::from_secs(5);
    for used_requests in 1..=request_budget {
        let roots = source_roots_json(endpoint, token);
        if listed_source_root(&roots, root_id).is_none() {
            return (request_budget - used_requests, roots);
        }
        assert!(
            used_requests < request_budget && Instant::now() < deadline,
            "source root deletion did not complete within the time and request budgets"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    unreachable!("positive request budget loop must return or panic")
}

fn daemon_identity(child: &Child, data_dir: &Path) -> (u32, String) {
    let instance_id = read_ipc_owner_file(data_dir, "ipc.endpoints.json")["instance_id"]
        .as_str()
        .unwrap()
        .to_string();
    (child.id(), instance_id)
}

fn authenticated_get_request(endpoint: &str, request_path: &str, token: &str) -> Vec<u8> {
    let addr = endpoint_address(endpoint);
    format!(
        "GET {request_path} HTTP/1.1\r\nHost: {addr}\r\nAuthorization: Bearer {token}\r\nConnection: close\r\n\r\n"
    )
    .into_bytes()
}

fn authenticated_post_request(
    endpoint: &str,
    request_path: &str,
    token: &str,
    payload: serde_json::Value,
) -> Vec<u8> {
    let addr = endpoint_address(endpoint);
    let body = payload.to_string();
    format!(
        "POST {request_path} HTTP/1.1\r\nHost: {addr}\r\nContent-Type: application/json\r\nAuthorization: Bearer {token}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
    .into_bytes()
}

fn endpoint_address(endpoint: &str) -> &str {
    endpoint
        .strip_prefix("http://")
        .expect("endpoint has http scheme")
        .split_once('/')
        .expect("endpoint has path")
        .0
}

fn search_request(request_id: &str, client_capability: &str) -> serde_json::Value {
    serde_json::json!({
        "schema_version": "resume-ir.ipc-request.v3",
        "request_id": request_id,
        "client_capability": client_capability,
        "deadline_ms": 5_000,
        "payload": {
            "query": "Rust",
            "mode": "fulltext",
            "top_k": 1,
        },
    })
}

fn search_batch_request(batch_id: &str, child_request_id: &str) -> serde_json::Value {
    serde_json::json!({
        "schema_version": "resume-ir.search-batch-request.v1",
        "batch_id": batch_id,
        "requests": [search_request(child_request_id, "benchmark")],
    })
}

fn detail_request(request_id: &str, selection: &serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "schema_version": "resume-ir.detail-request.v3",
        "request_id": request_id,
        "selection": selection,
    })
}

fn hydrate_request(request_id: &str, selection: &serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "schema_version": "resume-ir.detail-hydrate-request.v3",
        "request_id": request_id,
        "selection": selection,
        "body_offset_bytes": 0,
        "body_limit_bytes": 32 * 1024,
    })
}

fn response_json(response: &str) -> serde_json::Value {
    serde_json::from_str(response.split("\r\n\r\n").nth(1).unwrap()).unwrap()
}

fn assert_ready_status(response: &str) {
    assert!(response.contains("HTTP/1.1 200 OK"), "{response}");
    let payload = response_json(response);
    assert_eq!(payload["schema_version"], "daemon.status.v6");
    assert_eq!(payload["process_state"], "ready");
    assert_eq!(payload["core"]["state"], "ready");
    assert_eq!(payload["core"]["reason"], serde_json::Value::Null);
}

fn assert_search_response(response: &str, request_id: &str) {
    assert!(response.contains("HTTP/1.1 200 OK"), "{response}");
    let payload = response_json(response);
    assert_eq!(payload["schema_version"], "resume-ir.search-response.v3");
    assert_eq!(payload["request_id"], request_id);
    assert_eq!(payload["status"], "ok");
    assert_eq!(payload["result_count"], 1);
    assert_eq!(payload["results"].as_array().unwrap().len(), 1);
}

fn assert_batch_response(response: &str, batch_id: &str, child_request_id: &str) {
    assert!(response.contains("HTTP/1.1 200 OK"), "{response}");
    let body = response.split("\r\n\r\n").nth(1).unwrap();
    let lines = body.lines().collect::<Vec<_>>();
    assert_eq!(lines.len(), 1, "{response}");
    let payload: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
    assert_eq!(
        payload["schema_version"],
        "resume-ir.search-batch-child-response.v1"
    );
    assert_eq!(payload["batch_id"], batch_id);
    assert_eq!(payload["sequence"], 0);
    assert_eq!(payload["http_status"], 200);
    assert_eq!(payload["response"]["request_id"], child_request_id);
    assert_eq!(payload["response"]["status"], "ok");
}

fn assert_json_response(
    response: &str,
    schema_version: &str,
    request_id: &str,
    selection: &serde_json::Value,
) {
    assert!(response.contains("HTTP/1.1 200 OK"), "{response}");
    let payload = response_json(response);
    assert_eq!(payload["schema_version"], schema_version);
    assert_eq!(payload["request_id"], request_id);
    assert_eq!(&payload["selection"], selection);
    assert_eq!(payload["status"], "ok");
}

fn disconnect_after_response_started(endpoint: &str, request: Vec<u8>) {
    const MAX_RESPONSE_HEADER_BYTES: usize = 8 * 1024;

    let mut stream = TcpStream::connect(endpoint_address(endpoint)).expect("connect daemon ipc");
    // Unix configures an abortive close (RST). Windows exercises the same
    // request-local contract with an orderly client disconnect.
    prepare_abortive_close(&stream);
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .expect("bound response-start barrier");
    stream
        .write_all(&request)
        .expect("write complete ipc request");

    let mut response_head = Vec::with_capacity(256);
    while !response_head.ends_with(b"\r\n\r\n") {
        assert!(
            response_head.len() < MAX_RESPONSE_HEADER_BYTES,
            "response header exceeded test synchronization bound"
        );
        let mut byte = [0_u8; 1];
        stream
            .read_exact(&mut byte)
            .expect("observe response write before client disconnect");
        response_head.push(byte[0]);
    }
    assert!(response_head.starts_with(b"HTTP/1.1 "));
    disconnect_client(stream);
}

fn assert_same_daemon_instance(
    child: &mut Child,
    data_dir: &Path,
    expected_child_id: u32,
    expected_instance_id: &str,
) {
    assert_eq!(child.id(), expected_child_id);
    assert!(
        child.try_wait().expect("poll daemon process").is_none(),
        "daemon exited after a request-scoped client disconnect"
    );
    assert_eq!(
        read_ipc_owner_file(data_dir, "ipc.endpoints.json")["instance_id"],
        expected_instance_id
    );
}

fn try_http_get(endpoint: &str) -> io::Result<String> {
    let rest = endpoint
        .strip_prefix("http://")
        .expect("endpoint has http scheme");
    let (_addr, path) = rest.split_once('/').expect("endpoint has path");
    try_http_get_path(endpoint, &format!("/{path}"))
}

fn try_http_get_authenticated(endpoint: &str, token: &str) -> io::Result<String> {
    let rest = endpoint
        .strip_prefix("http://")
        .expect("endpoint has http scheme");
    let (addr, path) = rest.split_once('/').expect("endpoint has path");
    let mut stream = TcpStream::connect(addr)?;
    write!(
        stream,
        "GET /{path} HTTP/1.1\r\nHost: {addr}\r\nAuthorization: Bearer {token}\r\nConnection: close\r\n\r\n"
    )?;
    let mut response = String::new();
    stream.read_to_string(&mut response)?;
    Ok(response)
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
    assert_eq!(auth["schema_version"], "resume-ir.daemon-auth.v3");
    assert_eq!(auth["launch_id"].as_str().map(str::len), Some(64));
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
    seed_snapshot_state_with(
        data_dir,
        ImportOptions::default(),
        support::insert_import_task,
    );
}

fn seed_reviewed_snapshot_state(data_dir: &Path) {
    seed_snapshot_state_with(
        data_dir,
        support::reviewed_import_options(),
        support::insert_import_task_with_reviewed_contract,
    );
}

fn seed_snapshot_state_with(
    data_dir: &Path,
    import_options: ImportOptions,
    bind_task: fn(&OwnedMetaStore, &ImportTask) -> meta_store::ImportProcessingContract,
) {
    let root = data_dir.join("synthetic-status-corpus");
    fs::create_dir_all(&root).unwrap();
    fs::write(
        root.join("candidate.txt"),
        "SUMMARY\nSynthetic candidate\nEXPERIENCE\nBuilt Rust systems\nSKILLS\nRust",
    )
    .unwrap();
    let store = open_owned_store(data_dir);
    let now = UnixTimestamp::from_unix_seconds(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
            .saturating_sub(1) as i64,
    );
    let task = ImportTask {
        id: ImportTaskId::from_non_secret_parts(&["s20", "status-publication"]),
        root_path: path_str(&root).to_string(),
        status: ImportTaskStatus::Running,
        queued_at: now,
        started_at: Some(now),
        finished_at: None,
        updated_at: now,
    };
    bind_task(&store, &task);
    import_root_with_options(data_dir, &store, &task, &root, now, import_options).unwrap();
}

fn seed_queued_import_task(
    data_dir: &Path,
    label: &str,
    canonical_root: &Path,
    queued_at_seconds: i64,
) -> ImportTaskId {
    let store = open_owned_store(data_dir);
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
    support::insert_import_task_with_scope(&store, &task, &scope);
    task_id
}

fn seed_ocr_backlog_for_root(
    data_dir: &Path,
    root: &meta_store::SourceRoot,
    now: UnixTimestamp,
) -> IngestJobId {
    let store = open_owned_store(data_dir);
    let source = Path::new(&root.canonical_path).join("synthetic-ocr.pdf");
    let bytes = b"synthetic queued OCR source";
    fs::write(&source, bytes).unwrap();
    let document_id = DocumentId::from_non_secret_parts(&["s20", "source-root-delete-ocr"]);
    let revision = SourceRevision::for_content(
        document_id.clone(),
        ContentDigest::from_bytes(bytes),
        bytes.len() as u64,
    );
    store
        .upsert_document(&Document {
            id: document_id.clone(),
            source_uri: path_str(&source).to_string(),
            normalized_path: path_str(&source).to_string(),
            file_name: "synthetic-ocr.pdf".to_string(),
            extension: FileExtension::Pdf,
            byte_size: bytes.len() as u64,
            mtime: now,
            content_hash: Some(revision.content_hash.as_str().to_string()),
            text_hash: None,
            is_deleted: false,
            created_at: now,
            updated_at: now,
            status: DocumentStatus::OcrRequired,
        })
        .unwrap();
    store.insert_source_revision(&revision).unwrap();
    let contract = support::reviewed_processing_contract();
    let triage_epoch = contract.source_triage_epoch();
    store
        .insert_source_revision_triage(&SourceRevisionTriage {
            source_revision_id: revision.id.clone(),
            status: ClassificationStatus::OcrBacklog,
            triage_epoch: triage_epoch.as_str().to_string(),
            reason_codes: vec![ReasonCode::OcrRequired],
            triaged_at: now,
        })
        .unwrap();
    store
        .begin_scan(
            &root.id,
            "source-root-delete-ocr-scan",
            meta_store::ScanTrigger::Manual,
            now,
        )
        .unwrap();
    store
        .observe_source_occurrence(
            &root.id,
            "synthetic-ocr.pdf",
            &document_id,
            &revision.id,
            "source-root-delete-ocr-scan",
            now,
        )
        .unwrap();
    store
        .enqueue_ocr_job_for_source_triage(&revision.id, &triage_epoch, now)
        .unwrap()
        .job
        .id
}

fn seed_completed_import_scope(data_dir: &Path, label: &str, root: &Path) {
    let task_id = seed_queued_import_task(data_dir, label, root, 1_800_200_000);
    let store = open_owned_store(data_dir);
    let queued = store.import_task_by_id(&task_id).unwrap().unwrap();
    let _owner_lock = ImportTaskOwnerLock::acquire(data_dir, &task_id).unwrap();
    let task = store
        .claim_observed_import_task_for_worker(
            &queued,
            UnixTimestamp::from_unix_seconds(1_800_200_001),
        )
        .unwrap()
        .unwrap();
    import_root_with_options(
        data_dir,
        &store,
        &task,
        root,
        task.updated_at,
        ImportOptions::default(),
    )
    .unwrap();
}

fn seed_running_import_task(
    data_dir: &Path,
    label: &str,
    canonical_root: &Path,
    started_at_seconds: i64,
) -> ImportTaskId {
    let store = open_owned_store(data_dir);
    let now = UnixTimestamp::from_unix_seconds(started_at_seconds);
    let task_id = ImportTaskId::from_non_secret_parts(&["s46", label]);
    let task = ImportTask {
        id: task_id.clone(),
        root_path: path_str(canonical_root).to_string(),
        status: ImportTaskStatus::Running,
        queued_at: now,
        started_at: Some(now),
        finished_at: None,
        updated_at: now,
    };
    let scope = ImportScanScope {
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
    };
    support::insert_import_task_with_scope(&store, &task, &scope);
    task_id
}

fn seed_import_progress_scope(data_dir: &Path, task_id: &ImportTaskId, canonical_root: &Path) {
    let store = open_owned_store(data_dir);
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

fn acquire_test_processing_owner(data_dir: &Path) -> DataDirectoryOwnerLease {
    match DataDirectoryOwnerLease::try_acquire(data_dir).unwrap() {
        DataDirectoryOwnerAcquisition::Acquired(owner) => owner,
        DataDirectoryOwnerAcquisition::Contended => panic!("test processing owner contended"),
    }
}

fn open_owned_store(data_dir: &Path) -> OwnedMetaStore {
    acquire_test_processing_owner(data_dir)
        .open_store()
        .unwrap()
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
    start_ipc_daemon_with_baseline(data_dir, max_requests).0
}

fn start_ipc_daemon_with_baseline(data_dir: &Path, max_requests: usize) -> (Child, TestIpcMetrics) {
    let max_requests = max_requests
        .checked_add(1)
        .expect("test request budget")
        .to_string();
    let mut child = Command::new(env!("CARGO_BIN_EXE_resume-daemon"))
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
        .unwrap();
    let endpoint = read_ipc_endpoint(&mut child, data_dir);
    let token = read_ipc_auth_token(data_dir);
    let baseline = wait_for_serving_control_plane(&mut child, &endpoint, &token);
    (child, baseline)
}

fn start_import_capable_ipc_daemon(
    runtime_capacity: &support::ImportRuntimeCapacityLease,
    data_dir: &Path,
    max_requests: usize,
) -> Child {
    let max_requests = max_requests
        .checked_add(1)
        .expect("test request budget")
        .to_string();
    let mut child = support::import_capable_daemon_command(runtime_capacity)
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
        .unwrap();
    let endpoint = read_ipc_endpoint(&mut child, data_dir);
    let token = read_ipc_auth_token(data_dir);
    wait_for_ready_control_plane(&mut child, &endpoint, &token);
    child
}

fn start_degraded_ipc_daemon(data_dir: &Path, max_requests: usize) -> Child {
    let max_requests = max_requests
        .checked_add(1)
        .expect("test request budget")
        .to_string();
    let mut child = Command::new(env!("CARGO_BIN_EXE_resume-daemon"))
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
        .unwrap();
    let endpoint = read_ipc_endpoint(&mut child, data_dir);
    let token = read_ipc_auth_token(data_dir);
    wait_for_serving_control_plane(&mut child, &endpoint, &token);
    child
}

fn status_after_snapshot_refresh(endpoint: &str, token: &str) -> String {
    std::thread::sleep(Duration::from_secs(1));
    http_get(endpoint, token)
}

fn assert_text_import_available_without_ocr(response: &str) {
    let status = response_json(response);
    assert_eq!(
        status["optional_runtimes"]["embedding"]["state"],
        "available"
    );
    assert_eq!(
        status["optional_runtimes"]["classifier"]["state"],
        "available"
    );
    assert_eq!(status["optional_runtimes"]["ocr"]["state"], "unavailable");
    assert_eq!(status["capabilities"]["text_import"]["state"], "available");
}

fn assert_conflict_error_v2(response: &str) {
    let error = response_json(response);
    assert_eq!(error["schema_version"], "resume-ir.error.v3");
    assert_eq!(error["status"], "error");
    assert_eq!(error["error"]["code"], "CONFLICT");
    assert_eq!(error["error"]["action"], "retry");
    assert_eq!(error["error"]["capability"], serde_json::Value::Null);
    assert_eq!(error["error"]["reason"], serde_json::Value::Null);
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

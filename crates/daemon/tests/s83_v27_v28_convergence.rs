use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use import_pipeline::{current_import_processing_contract, ImportOptions};
use meta_store::{
    migration_test_support::{
        seed_v27_repairing_fixture, seed_v28_blocked_processing_contract_fixture,
    },
    DataDirectoryOwnerAcquisition, DataDirectoryOwnerLease, MetaStoreErrorClass, ReadMetaStore,
};
use process_containment::ContainedChild;
use sha2::{Digest, Sha256};
use tempfile::tempdir;

const STARTUP_TIMEOUT: Duration = Duration::from_secs(20);

#[test]
fn migration_fixture_builder_rejects_non_synthetic_roots() {
    let workspace = tempdir().unwrap();
    let data_dir = workspace.path().join("data");
    let non_synthetic_root = workspace.path().join("private-root");
    fs::create_dir_all(&non_synthetic_root).unwrap();

    assert!(seed_v27_repairing_fixture(&data_dir, &non_synthetic_root, 1).is_err());
}

#[test]
fn standalone_daemon_rejects_v27_without_migrating_or_rewriting_existing_files() {
    let workspace = tempdir().unwrap();
    let data_dir = workspace.path().join("data");
    let source_root = workspace.path().join("resume-ir-synthetic-v27-hard-cut");
    fs::create_dir_all(&source_root).unwrap();
    fs::write(source_root.join("synthetic.txt"), "synthetic v27 fixture").unwrap();
    let canonical_root = fs::canonicalize(&source_root).unwrap();
    seed_v27_repairing_fixture(&data_dir, &canonical_root, 41).unwrap();
    assert_unsupported_store(&data_dir);
    let before = snapshot_existing_files(&data_dir);

    let output = run_once(&data_dir);

    assert!(!output.status.success());
    assert_fatal_runtime_integrity(&output.stderr);
    assert_eq!(snapshot_existing_files(&data_dir), before);
    assert_unsupported_store(&data_dir);
}

#[test]
fn standalone_daemon_rejects_v28_without_migrating_or_rewriting_existing_files() {
    let workspace = tempdir().unwrap();
    let data_dir = workspace.path().join("data");
    let source_root = workspace.path().join("resume-ir-synthetic-v28-hard-cut");
    fs::create_dir_all(&source_root).unwrap();
    fs::write(source_root.join("synthetic.txt"), "synthetic v28 fixture").unwrap();
    let canonical_root = fs::canonicalize(&source_root).unwrap();
    let contract = current_import_processing_contract(&ImportOptions::default()).unwrap();
    seed_v28_blocked_processing_contract_fixture(&data_dir, &canonical_root, 41, &contract)
        .unwrap();
    assert_unsupported_store(&data_dir);
    let before = snapshot_existing_files(&data_dir);

    let output = run_once(&data_dir);

    assert!(!output.status.success());
    assert_fatal_runtime_integrity(&output.stderr);
    assert_eq!(snapshot_existing_files(&data_dir), before);
    assert_unsupported_store(&data_dir);
}

#[test]
fn standalone_daemon_rejects_unknown_manifest_authority_without_rewriting_existing_files() {
    let workspace = tempdir().unwrap();
    let data_dir = workspace.path().join("data");
    let owner = match DataDirectoryOwnerLease::try_acquire(&data_dir).unwrap() {
        DataDirectoryOwnerAcquisition::Acquired(owner) => owner,
        DataDirectoryOwnerAcquisition::Contended => panic!("synthetic data dir is owned"),
    };
    drop(owner.open_store().unwrap());
    drop(owner);
    let manifest_path = data_dir.join("metadata-active.v1");
    let manifest = fs::read_to_string(&manifest_path).unwrap();
    assert!(manifest.contains("\nschema=29\n"));
    fs::write(
        &manifest_path,
        manifest.replace("\nschema=29\n", "\nschema=30\n"),
    )
    .unwrap();
    assert_unsupported_store(&data_dir);
    let before = snapshot_existing_files(&data_dir);

    let output = run_once(&data_dir);

    assert!(!output.status.success());
    assert_fatal_runtime_integrity(&output.stderr);
    assert_eq!(snapshot_existing_files(&data_dir), before);
    assert_unsupported_store(&data_dir);
}

#[test]
fn repeated_v28_control_plane_generations_remain_blocked_and_never_consume_old_bytes() {
    let workspace = tempdir().unwrap();
    let data_dir = workspace.path().join("data");
    let source_root = workspace
        .path()
        .join("resume-ir-synthetic-v28-restart-hard-cut");
    fs::create_dir_all(&source_root).unwrap();
    let canonical_root = fs::canonicalize(&source_root).unwrap();
    let contract = current_import_processing_contract(&ImportOptions::default()).unwrap();
    seed_v28_blocked_processing_contract_fixture(&data_dir, &canonical_root, 41, &contract)
        .unwrap();
    let before = snapshot_existing_files(&data_dir);
    let mut prior_instance_id = None;

    for launch_id in [
        "8383838383838383838383838383838383838383838383838383838383838383",
        "8484848484848484848484848484848484848484848484848484848484848484",
    ] {
        let mut command = Command::new(env!("CARGO_BIN_EXE_resume-daemon"));
        command
            .args([
                "--data-dir",
                path_str(&data_dir),
                "run",
                "--foreground",
                "--parent-lifecycle-stdin",
                "--launch-id",
                launch_id,
                "--ipc-listen",
                "127.0.0.1:0",
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        let mut child = ContainedChild::spawn(&mut command).unwrap();
        let parent_stdin = child.take_stdin().unwrap();
        let mut stderr = child.take_stderr().unwrap();
        let generation = wait_for_generation(&mut child, &data_dir, launch_id);
        assert_ne!(
            prior_instance_id.as_deref(),
            Some(generation.instance_id.as_str())
        );
        assert_eq!(generation.launch_id, launch_id);
        prior_instance_id = Some(generation.instance_id.clone());
        let payload = wait_for_blocked_status(&mut child, &generation);
        assert_eq!(payload["process_state"], "ready");
        assert_eq!(payload["status"], "blocked");
        assert_eq!(payload["core"]["state"], "blocked");
        assert_eq!(payload["core"]["reason"], "unsupported_store_schema");
        assert_eq!(payload["error"]["code"], "SERVICE_BLOCKED");
        assert_eq!(payload["error"]["action"], "repair_required");
        assert_eq!(payload["error"]["capability"], serde_json::Value::Null);
        assert_eq!(payload["error"]["reason"], "unsupported_store_schema");
        drop(parent_stdin);
        let status = child.wait().unwrap();
        let mut stderr_body = Vec::new();
        stderr.read_to_end(&mut stderr_body).unwrap();
        assert!(
            status.success(),
            "daemon shutdown failed: {}",
            String::from_utf8_lossy(&stderr_body)
        );
        assert!(stderr_body.is_empty());
        assert_eq!(snapshot_existing_files(&data_dir), before);
        assert!(!data_dir.join("ipc.endpoints.json").exists());
        assert!(!data_dir.join("ipc.auth").exists());
    }
}

fn run_once(data_dir: &Path) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_resume-daemon"))
        .args([
            "--data-dir",
            path_str(data_dir),
            "run",
            "--foreground",
            "--once",
        ])
        .output()
        .unwrap()
}

fn assert_unsupported_store(data_dir: &Path) {
    assert_eq!(
        ReadMetaStore::open_data_dir(data_dir).unwrap_err().class(),
        MetaStoreErrorClass::UnsupportedStoreSchema
    );
}

fn assert_fatal_runtime_integrity(stderr: &[u8]) {
    let payload: serde_json::Value = serde_json::from_slice(stderr).unwrap();
    assert_eq!(payload["schema_version"], "resume-ir.daemon-fatal.v1");
    assert_eq!(payload["class"], "runtime_integrity");
    assert_eq!(payload["disposition"], "blocked");
}

struct Generation {
    launch_id: String,
    instance_id: String,
    token: String,
    status_endpoint: String,
}

fn wait_for_generation(
    child: &mut ContainedChild,
    data_dir: &Path,
    expected_launch_id: &str,
) -> Generation {
    let deadline = Instant::now() + STARTUP_TIMEOUT;
    loop {
        let endpoints = fs::read(data_dir.join("ipc.endpoints.json"))
            .ok()
            .and_then(|body| serde_json::from_slice::<serde_json::Value>(&body).ok());
        let auth = fs::read(data_dir.join("ipc.auth"))
            .ok()
            .and_then(|body| serde_json::from_slice::<serde_json::Value>(&body).ok());
        if let (Some(endpoints), Some(auth)) = (endpoints, auth) {
            if endpoints["schema_version"] == "resume-ir.daemon-ipc.v3"
                && auth["schema_version"] == "resume-ir.daemon-auth.v3"
                && endpoints["launch_id"] == auth["launch_id"]
                && endpoints["instance_id"] == auth["instance_id"]
            {
                assert_control_file_contract(&endpoints, &auth, expected_launch_id);
                return Generation {
                    launch_id: endpoints["launch_id"].as_str().unwrap().to_string(),
                    instance_id: endpoints["instance_id"].as_str().unwrap().to_string(),
                    token: auth["token"].as_str().unwrap().to_string(),
                    status_endpoint: endpoints["status"].as_str().unwrap().to_string(),
                };
            }
        }
        if let Some(status) = child.try_wait().unwrap() {
            panic!("daemon exited before v3 control publication: {status}");
        }
        assert!(
            Instant::now() < deadline,
            "v3 control publication timed out"
        );
        thread::sleep(Duration::from_millis(20));
    }
}

fn wait_for_blocked_status(
    child: &mut ContainedChild,
    generation: &Generation,
) -> serde_json::Value {
    let deadline = Instant::now() + STARTUP_TIMEOUT;
    loop {
        let status = authenticated_get(&generation.status_endpoint, &generation.token);
        assert!(status.starts_with("HTTP/1.1 200"), "{status}");
        let payload: serde_json::Value =
            serde_json::from_str(status.split_once("\r\n\r\n").unwrap().1).unwrap();
        assert_eq!(payload["schema_version"], "daemon.status.v3");
        assert_eq!(payload["process_state"], "ready");
        if payload["core"]["state"] == "blocked" {
            return payload;
        }
        assert_eq!(payload["core"]["state"], "initializing");
        assert_eq!(payload["core"]["reason"], "metadata_initializing");
        if let Some(status) = child.try_wait().unwrap() {
            panic!("daemon exited before publishing blocked status: {status}");
        }
        assert!(Instant::now() < deadline, "blocked status timed out");
        thread::sleep(Duration::from_millis(20));
    }
}

fn assert_control_file_contract(
    endpoints: &serde_json::Value,
    auth: &serde_json::Value,
    expected_launch_id: &str,
) {
    assert_eq!(
        object_keys(endpoints),
        BTreeSet::from([
            "schema_version",
            "launch_id",
            "instance_id",
            "owner_mode",
            "status",
            "diagnostics",
            "imports",
            "import_cancel",
            "import_control",
            "import_progress",
            "search",
            "search_batch",
            "details",
            "delete",
        ])
    );
    assert_eq!(
        object_keys(auth),
        BTreeSet::from(["schema_version", "launch_id", "instance_id", "token"])
    );
    assert_eq!(endpoints["owner_mode"], "desktop_supervised");
    assert_eq!(endpoints["launch_id"], auth["launch_id"]);
    assert_eq!(endpoints["instance_id"], auth["instance_id"]);
    assert_eq!(endpoints["launch_id"].as_str().unwrap(), expected_launch_id);
    assert_eq!(endpoints["instance_id"].as_str().unwrap().len(), 64);
    assert_eq!(auth["token"].as_str().unwrap().len(), 64);
}

fn object_keys(value: &serde_json::Value) -> BTreeSet<&str> {
    value
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect()
}

fn authenticated_get(endpoint: &str, token: &str) -> String {
    let (address, path) = endpoint
        .strip_prefix("http://")
        .unwrap()
        .split_once('/')
        .unwrap();
    let mut stream = TcpStream::connect(address).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    stream
        .set_write_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    write!(
        stream,
        "GET /{path} HTTP/1.1\r\nHost: {address}\r\nAuthorization: Bearer {token}\r\nConnection: close\r\n\r\n"
    )
    .unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    response
}

fn snapshot_existing_files(root: &Path) -> BTreeMap<PathBuf, (u64, String)> {
    let mut snapshot = BTreeMap::new();
    collect_files(root, root, &mut snapshot);
    snapshot
}

fn collect_files(root: &Path, current: &Path, snapshot: &mut BTreeMap<PathBuf, (u64, String)>) {
    for entry in fs::read_dir(current).unwrap() {
        let entry = entry.unwrap();
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path).unwrap();
        if metadata.file_type().is_dir() {
            collect_files(root, &path, snapshot);
        } else if metadata.file_type().is_file() {
            let bytes = fs::read(&path).unwrap();
            snapshot.insert(
                path.strip_prefix(root).unwrap().to_path_buf(),
                (bytes.len() as u64, format!("{:x}", Sha256::digest(bytes))),
            );
        }
    }
}

fn path_str(path: &Path) -> &str {
    path.to_str().unwrap()
}

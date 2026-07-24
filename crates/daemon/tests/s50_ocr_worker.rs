use std::collections::BTreeMap;
use std::fs;
use std::io::{self, Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStderr, Command, ExitStatus, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use import_pipeline::import_root_with_options;
use meta_store::{
    DataDirectoryOwnerAcquisition, DataDirectoryOwnerLease, ImportTask, ImportTaskId,
    ImportTaskStatus, IngestJobStatus, OwnedMetaStore, ReadMetaStore, UnixTimestamp,
};
use process_containment::ContainedChild;
use sha2::{Digest, Sha256};

mod support;

// OCR execution, page budgeting, retry, cache, and renderer semantics remain
// covered at their owning boundaries in `ocr-client`, `import-pipeline`, and
// CLI integration tests. This suite owns the daemon hard-cut boundary: only a
// reviewed pinned pack may reach those components, and every other candidate
// must remain observable but unexecuted and unable to claim durable work.
const STARTUP_TIMEOUT: Duration = Duration::from_secs(30);

trait PollDaemonChild {
    fn poll(&mut self) -> io::Result<Option<ExitStatus>>;
    fn take_stderr_pipe(&mut self) -> Option<ChildStderr>;
}

impl PollDaemonChild for Child {
    fn poll(&mut self) -> io::Result<Option<ExitStatus>> {
        self.try_wait()
    }

    fn take_stderr_pipe(&mut self) -> Option<ChildStderr> {
        self.stderr.take()
    }
}

impl PollDaemonChild for ContainedChild {
    fn poll(&mut self) -> io::Result<Option<ExitStatus>> {
        self.try_wait()
    }

    fn take_stderr_pipe(&mut self) -> Option<ChildStderr> {
        self.take_stderr()
    }
}

#[test]
fn reviewed_ocr_pack_is_available_with_the_bundled_classifier_and_embedding_packs() {
    let runtime_capacity = support::import_runtime_capacity_lease();
    let data_dir = ready_data_dir("ocr-reviewed-pack");

    let (status, output) = status_from(
        support::fully_capable_daemon_command(&runtime_capacity),
        &data_dir,
        &[],
    );

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        status["optional_runtimes"]["embedding"]["state"],
        "available"
    );
    assert_eq!(
        status["optional_runtimes"]["classifier"]["state"],
        "available"
    );
    assert_eq!(status["optional_runtimes"]["ocr"]["state"], "available");
    assert_eq!(status["capabilities"]["text_import"]["state"], "available");
    assert_eq!(status["capabilities"]["ocr_import"]["state"], "available");
    remove_dir(&data_dir);
}

#[test]
fn unmanifested_ocr_command_is_rejected_before_a_once_worker_opens_or_claims_the_store() {
    let runtime_capacity = support::import_runtime_capacity_lease();
    let data_dir = ready_data_dir("ocr-unmanifested-once");
    let command = marker_command("ocr-unmanifested-once-command");
    let marker = command.with_extension("executed");
    let before = snapshot_existing_files(&data_dir);

    let output = support::import_capable_daemon_command(&runtime_capacity)
        .args([
            "--data-dir",
            path_str(&data_dir),
            "run",
            "--foreground",
            "--once",
            "--work-ocr-once",
            "--ocr-command",
            path_str(&command),
        ])
        .output()
        .unwrap();

    assert_configuration_blocked(&output);
    assert!(!marker.exists(), "unvalidated OCR command executed");
    assert_eq!(snapshot_selected_files(&data_dir, before.keys()), before);
    remove_dir(&data_dir);
    remove_dir(command.parent().unwrap());
}

#[test]
fn invalid_ocr_manifest_is_reported_without_claiming_a_queued_job() {
    let runtime_capacity = support::import_runtime_capacity_lease();
    let data_dir = data_dir_with_queued_ocr("ocr-invalid-manifest-job");
    let command = fs::canonicalize(marker_command("ocr-invalid-manifest-command")).unwrap();
    let renderer = support::attested_pdf_renderer();
    fs::write(
        command.parent().unwrap().join("runtime-pack.json"),
        br#"{"schema_version":"resume-ir.desktop-ocr-runtime-pack.v1"}"#,
    )
    .unwrap();

    let (status, output) = status_from(
        support::import_capable_daemon_command(&runtime_capacity),
        &data_dir,
        &[
            "--work-ocr",
            "--ocr-command",
            path_str(&command),
            "--ocr-render-command",
            path_str(&renderer),
        ],
    );

    assert!(output.status.success());
    assert_runtime_unavailable(&status, "ocr", "invalid");
    assert_capability_unavailable(&status, "ocr_import", "ocr_unavailable");
    assert_eq!(queued_ocr_job_status(&data_dir), IngestJobStatus::Queued);
    assert!(!command.with_extension("executed").exists());
    remove_dir(&data_dir);
    remove_dir(command.parent().unwrap());
}

#[test]
fn missing_ocr_runtime_keeps_text_import_and_index_publication_available() {
    let runtime_capacity = support::import_runtime_capacity_lease();
    let data_dir = ready_data_dir("ocr-missing-independent-capabilities");

    let (status, output) = status_from(
        support::import_capable_daemon_command(&runtime_capacity),
        &data_dir,
        &[],
    );

    assert!(output.status.success());
    assert_runtime_unavailable(&status, "ocr", "not_configured");
    assert_eq!(status["capabilities"]["text_import"]["state"], "available");
    assert_eq!(
        status["capabilities"]["index_publication"]["state"],
        "available"
    );
    assert_capability_unavailable(&status, "ocr_import", "ocr_unavailable");
    remove_dir(&data_dir);
}

#[test]
fn missing_embedding_runtime_blocks_all_mutating_publication_capabilities() {
    let data_dir = ready_data_dir("ocr-missing-embedding");
    let model = support::reviewed_classifier_model();

    let (status, output) = status_from(
        Command::new(env!("CARGO_BIN_EXE_resume-daemon")),
        &data_dir,
        &["--resume-classifier-model", path_str(&model)],
    );

    assert!(output.status.success());
    assert_runtime_unavailable(&status, "embedding", "not_configured");
    assert_eq!(
        status["optional_runtimes"]["classifier"]["state"],
        "available"
    );
    assert_capability_unavailable(&status, "text_import", "embedding_unavailable");
    assert_capability_unavailable(&status, "ocr_import", "embedding_unavailable");
    assert_capability_unavailable(&status, "index_publication", "embedding_unavailable");
    remove_dir(&data_dir);
}

#[test]
fn missing_classifier_keeps_index_publication_available_without_unblocking_imports() {
    let runtime_capacity = support::import_runtime_capacity_lease();
    let data_dir = ready_data_dir("ocr-missing-classifier-index-publication");
    let mut command = support::import_capable_daemon_command(&runtime_capacity);
    command.env_remove("RESUME_IR_TEST_CLASSIFIER_MODEL");

    let (status, output) = status_from(command, &data_dir, &[]);

    assert!(output.status.success());
    assert_eq!(
        status["optional_runtimes"]["embedding"]["state"],
        "available"
    );
    assert_runtime_unavailable(&status, "classifier", "not_configured");
    assert_capability_unavailable(&status, "text_import", "classifier_unavailable");
    assert_capability_unavailable(&status, "ocr_import", "classifier_unavailable");
    assert_eq!(
        status["capabilities"]["index_publication"],
        serde_json::json!({"state": "available", "reason": null})
    );
    remove_dir(&data_dir);
}

#[test]
fn missing_classifier_is_reported_independently_from_other_missing_runtimes() {
    let data_dir = ready_data_dir("ocr-missing-classifier");

    let (status, output) = status_from(
        Command::new(env!("CARGO_BIN_EXE_resume-daemon")),
        &data_dir,
        &[],
    );

    assert!(output.status.success());
    assert_runtime_unavailable(&status, "embedding", "not_configured");
    assert_runtime_unavailable(&status, "ocr", "not_configured");
    assert_runtime_unavailable(&status, "classifier", "not_configured");
    assert_capability_unavailable(&status, "text_import", "classifier_unavailable");
    assert_capability_unavailable(&status, "ocr_import", "classifier_unavailable");
    assert_capability_unavailable(&status, "index_publication", "embedding_unavailable");
    remove_dir(&data_dir);
}

#[test]
fn unvalidated_renderer_cannot_widen_a_missing_ocr_pack() {
    let runtime_capacity = support::import_runtime_capacity_lease();
    let data_dir = ready_data_dir("ocr-unvalidated-renderer");
    let engine = fs::canonicalize(marker_command("ocr-unvalidated-engine")).unwrap();
    let renderer = fs::canonicalize(marker_command("ocr-unvalidated-renderer-command")).unwrap();

    let (status, output) = status_from(
        support::import_capable_daemon_command(&runtime_capacity),
        &data_dir,
        &[
            "--ocr-tesseract-command",
            path_str(&engine),
            "--ocr-pdftoppm-command",
            path_str(&renderer),
        ],
    );

    assert!(output.status.success());
    assert_runtime_unavailable(&status, "ocr", "missing");
    assert!(!engine.with_extension("executed").exists());
    assert!(!renderer.with_extension("executed").exists());
    remove_dir(&data_dir);
    remove_dir(engine.parent().unwrap());
    remove_dir(renderer.parent().unwrap());
}

#[test]
fn invalid_ocr_worker_loop_never_claims_or_reclassifies_existing_work() {
    let runtime_capacity = support::import_runtime_capacity_lease();
    let data_dir = data_dir_with_queued_ocr("ocr-invalid-loop-no-claim");
    let command = marker_command("ocr-invalid-loop-command");

    let (status, output) = status_from(
        support::import_capable_daemon_command(&runtime_capacity),
        &data_dir,
        &["--work-ocr", "--ocr-command", path_str(&command)],
    );

    assert!(output.status.success());
    assert_runtime_unavailable(&status, "ocr", "missing");
    assert_eq!(queued_ocr_job_status(&data_dir), IngestJobStatus::Queued);
    assert!(!command.with_extension("executed").exists());
    remove_dir(&data_dir);
    remove_dir(command.parent().unwrap());
}

#[test]
fn invalid_ocr_command_crash_fixture_is_never_executed() {
    let runtime_capacity = support::import_runtime_capacity_lease();
    let data_dir = ready_data_dir("ocr-crash-never-executed");
    let command = marker_command_with_exit("ocr-crash-command", 19);

    let (status, output) = status_from(
        support::import_capable_daemon_command(&runtime_capacity),
        &data_dir,
        &["--work-ocr", "--ocr-command", path_str(&command)],
    );

    assert!(output.status.success());
    assert_runtime_unavailable(&status, "ocr", "missing");
    assert!(!command.with_extension("executed").exists());
    remove_dir(&data_dir);
    remove_dir(command.parent().unwrap());
}

#[test]
fn unsupported_ocr_language_cannot_be_smuggled_into_the_reviewed_pack_contract() {
    let runtime_capacity = support::import_runtime_capacity_lease();
    let data_dir = ready_data_dir("ocr-language-hard-cut");
    let mut command = support::fully_capable_daemon_command(&runtime_capacity);
    command.env_remove("RESUME_IR_TEST_OCR_COMMAND");
    let reviewed_ocr = support::reviewed_ocr_command();
    let pdf_renderer = support::attested_pdf_renderer();
    command.env("TESSDATA_PREFIX", support::reviewed_ocr_tessdata());

    let (status, output) = status_from(
        command,
        &data_dir,
        &[
            "--ocr-tesseract-command",
            path_str(&reviewed_ocr),
            "--ocr-render-command",
            path_str(&pdf_renderer),
            "--ocr-lang",
            "deu",
        ],
    );

    assert!(output.status.success());
    assert_runtime_unavailable(&status, "ocr", "invalid");
    assert_capability_unavailable(&status, "ocr_import", "ocr_unavailable");
    remove_dir(&data_dir);
}

#[test]
fn runtime_faults_are_reported_as_a_fixed_independent_matrix() {
    let data_dir = ready_data_dir("ocr-runtime-matrix");

    let (status, output) = status_from(
        Command::new(env!("CARGO_BIN_EXE_resume-daemon")),
        &data_dir,
        &[],
    );

    assert!(output.status.success());
    let runtimes = status["optional_runtimes"].as_object().unwrap();
    assert_eq!(runtimes.len(), 3);
    assert!(runtimes.contains_key("embedding"));
    assert!(runtimes.contains_key("ocr"));
    assert!(runtimes.contains_key("classifier"));
    remove_dir(&data_dir);
}

#[test]
fn parent_shutdown_with_invalid_ocr_withdraws_generation_files_without_running_it() {
    let runtime_capacity = support::import_runtime_capacity_lease();
    let data_dir = ready_data_dir("ocr-parent-shutdown");
    let command = marker_command("ocr-parent-shutdown-command");
    let mut daemon = support::import_capable_daemon_command(&runtime_capacity);
    daemon.args([
        "--data-dir",
        path_str(&data_dir),
        "run",
        "--foreground",
        "--parent-lifecycle-stdin",
        "--launch-id",
        "5050505050505050505050505050505050505050505050505050505050505050",
        "--work-ocr",
        "--ocr-command",
        path_str(&command),
        "--ipc-listen",
        "127.0.0.1:0",
    ]);
    daemon
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    let mut child = ContainedChild::spawn(&mut daemon).unwrap();
    let parent_stdin = child.take_stdin().unwrap();
    wait_for_generation(&mut child, &data_dir);

    drop(parent_stdin);
    let (status, stderr) = wait_contained_with_stderr(child);

    assert!(status.success(), "daemon exited with {status:?}: {stderr}");
    assert!(!command.with_extension("executed").exists());
    assert!(!data_dir.join("ipc.endpoints.json").exists());
    assert!(!data_dir.join("ipc.auth").exists());
    remove_dir(&data_dir);
    remove_dir(command.parent().unwrap());
}

#[test]
fn reviewed_ocr_pack_remains_available_across_fresh_daemon_generations() {
    let runtime_capacity = support::import_runtime_capacity_lease();
    let data_dir = ready_data_dir("ocr-reviewed-pack-restart");

    for _ in 0..2 {
        let (status, output) = status_from(
            support::fully_capable_daemon_command(&runtime_capacity),
            &data_dir,
            &[],
        );
        assert!(output.status.success());
        assert_eq!(status["optional_runtimes"]["ocr"]["state"], "available");
        assert_eq!(status["capabilities"]["ocr_import"]["state"], "available");
    }
    remove_dir(&data_dir);
}

#[test]
fn ready_daemon_downgrades_tampered_ocr_runtime_without_losing_core_reads() {
    let runtime_capacity = support::import_runtime_capacity_lease();
    let data_dir = ready_data_dir("ocr-runtime-tamper-degrades");
    let renderer_dir = temp_dir("ocr-runtime-tamper-renderer");
    let renderer = support::copy_attested_pdf_renderer(&renderer_dir);
    let mut daemon = support::fully_capable_daemon_command(&runtime_capacity);
    daemon
        .env("RESUME_IR_TEST_OCR_RENDER_COMMAND", &renderer)
        .args([
            "--data-dir",
            path_str(&data_dir),
            "run",
            "--foreground",
            "--parent-lifecycle-stdin",
            "--launch-id",
            "8585858585858585858585858585858585858585858585858585858585858585",
            "--work-ocr",
            "--worker-interval-ms",
            "50",
            "--ipc-listen",
            "127.0.0.1:0",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    let mut child = ContainedChild::spawn(&mut daemon).unwrap();
    let parent_stdin = child.take_stdin().unwrap();
    let generation = wait_for_generation(&mut child, &data_dir);
    let initial = wait_for_resolved_status(&mut child, &generation);
    assert_eq!(
        initial["optional_runtimes"]["ocr"]["state"], "available",
        "initial status: {initial}"
    );

    fs::OpenOptions::new()
        .append(true)
        .open(&renderer)
        .unwrap()
        .write_all(b"tampered-after-ready")
        .unwrap();

    let degraded = wait_for_status(&mut child, &generation, |status| {
        status["optional_runtimes"]["ocr"]["state"] == "unavailable"
    });
    assert_eq!(degraded["optional_runtimes"]["ocr"]["reason"], "invalid");
    assert_eq!(degraded["core"]["state"], "ready");
    assert_eq!(
        degraded["capabilities"]["keyword_search"]["state"],
        "available"
    );
    assert_eq!(degraded["capabilities"]["detail"]["state"], "available");
    assert_eq!(
        degraded["capabilities"]["ocr_import"]["state"],
        "unavailable"
    );
    assert!(
        child.try_wait().unwrap().is_none(),
        "daemon exited on OCR degradation"
    );

    drop(parent_stdin);
    let (status, stderr) = wait_contained_with_stderr(child);
    assert!(status.success(), "{}", stderr);
    remove_dir(&data_dir);
    remove_dir(&renderer_dir);
}

fn status_from(
    mut command: Command,
    data_dir: &Path,
    extra_args: &[&str],
) -> (serde_json::Value, std::process::Output) {
    command.args(["--data-dir", path_str(data_dir), "run", "--foreground"]);
    command.args(extra_args);
    command.args(["--ipc-listen", "127.0.0.1:0", "--max-requests", "1"]);
    command.stdout(Stdio::null()).stderr(Stdio::piped());
    let mut child = command.spawn().unwrap();
    let generation = wait_for_generation(&mut child, data_dir);
    let status = wait_for_resolved_status(&mut child, &generation);
    let output = child.wait_with_output().unwrap();
    (status, output)
}

struct Generation {
    token: String,
    status_endpoint: String,
}

fn wait_for_generation(child: &mut impl PollDaemonChild, data_dir: &Path) -> Generation {
    let deadline = Instant::now() + STARTUP_TIMEOUT;
    loop {
        let endpoints = read_json(data_dir.join("ipc.endpoints.json"));
        let auth = read_json(data_dir.join("ipc.auth"));
        if let (Some(endpoints), Some(auth)) = (endpoints, auth) {
            if endpoints["schema_version"] == "resume-ir.daemon-ipc.v3"
                && auth["schema_version"] == "resume-ir.daemon-auth.v3"
                && endpoints["launch_id"] == auth["launch_id"]
                && endpoints["instance_id"] == auth["instance_id"]
            {
                return Generation {
                    token: auth["token"].as_str().unwrap().to_string(),
                    status_endpoint: endpoints["status"].as_str().unwrap().to_string(),
                };
            }
        }
        if let Some(status) = child.poll().unwrap() {
            let mut stderr = String::new();
            if let Some(mut pipe) = child.take_stderr_pipe() {
                pipe.read_to_string(&mut stderr).unwrap();
            }
            panic!("daemon exited before publishing v3 control plane: {status}\nstderr:\n{stderr}");
        }
        assert!(
            Instant::now() < deadline,
            "v3 control publication timed out"
        );
        thread::sleep(Duration::from_millis(20));
    }
}

fn wait_for_resolved_status(
    child: &mut impl PollDaemonChild,
    generation: &Generation,
) -> serde_json::Value {
    let deadline = Instant::now() + STARTUP_TIMEOUT;
    loop {
        let response = authenticated_get(&generation.status_endpoint, &generation.token);
        if response.starts_with("HTTP/1.1 200") {
            let payload: serde_json::Value =
                serde_json::from_str(response.split_once("\r\n\r\n").unwrap().1).unwrap();
            if payload["core"]["state"] != "initializing"
                && payload["optional_runtimes"]["embedding"]["state"] != "initializing"
                && payload["optional_runtimes"]["ocr"]["state"] != "initializing"
                && payload["optional_runtimes"]["classifier"]["state"] != "initializing"
            {
                return payload;
            }
        }
        if let Some(status) = child.poll().unwrap() {
            panic!("daemon exited before resolving runtimes: {status}");
        }
        assert!(Instant::now() < deadline, "runtime resolution timed out");
        thread::sleep(Duration::from_millis(20));
    }
}

fn wait_for_status(
    child: &mut impl PollDaemonChild,
    generation: &Generation,
    predicate: impl Fn(&serde_json::Value) -> bool,
) -> serde_json::Value {
    let deadline = Instant::now() + STARTUP_TIMEOUT;
    loop {
        let response = authenticated_get(&generation.status_endpoint, &generation.token);
        if response.starts_with("HTTP/1.1 200") {
            let payload: serde_json::Value =
                serde_json::from_str(response.split_once("\r\n\r\n").unwrap().1).unwrap();
            if predicate(&payload) {
                return payload;
            }
        }
        if let Some(status) = child.poll().unwrap() {
            panic!("daemon exited while waiting for status transition: {status}");
        }
        assert!(Instant::now() < deadline, "status transition timed out");
        thread::sleep(Duration::from_millis(20));
    }
}

fn wait_contained_with_stderr(mut child: ContainedChild) -> (ExitStatus, String) {
    let mut stderr = child.take_stderr().expect("daemon stderr");
    let stderr_reader = thread::spawn(move || {
        let mut output = String::new();
        stderr
            .read_to_string(&mut output)
            .expect("read contained daemon stderr");
        output
    });
    let status = child.wait().expect("wait for contained daemon");
    let stderr = stderr_reader.join().expect("join daemon stderr reader");
    (status, stderr)
}

fn authenticated_get(endpoint: &str, token: &str) -> String {
    let (address, path) = endpoint
        .strip_prefix("http://")
        .unwrap()
        .split_once('/')
        .unwrap();
    let mut stream = TcpStream::connect(address).unwrap();
    write!(
        stream,
        "GET /{path} HTTP/1.1\r\nHost: {address}\r\nAuthorization: Bearer {token}\r\nConnection: close\r\n\r\n"
    )
    .unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    response
}

fn ready_data_dir(label: &str) -> PathBuf {
    let data_dir = temp_dir(label);
    let store = open_owned_store(&data_dir);
    let now = UnixTimestamp::from_unix_seconds(1_800_050_000);
    support::activate_reviewed_processing_contract(&store, now);
    import_pipeline::prepare_migration_rebuild_artifacts(
        &store,
        now,
        &import_pipeline::PipelineRunControl::default(),
    )
    .unwrap();
    import_pipeline::finalize_migration_rebuild(
        &store,
        now,
        &support::reviewed_processing_contract(),
        &import_pipeline::SearchPublicationVectorization::default(),
        &import_pipeline::PipelineRunControl::default(),
    )
    .unwrap();
    drop(store);
    data_dir
}

fn data_dir_with_queued_ocr(label: &str) -> PathBuf {
    let data_dir = temp_dir(label);
    let private_root = data_dir.join("synthetic-scanned-resumes");
    fs::create_dir_all(&private_root).unwrap();
    fs::write(
        private_root.join("synthetic-scanned.pdf"),
        single_page_pdf(),
    )
    .unwrap();
    let canonical_root = fs::canonicalize(&private_root).unwrap();
    let store = open_owned_store(&data_dir);
    let now = UnixTimestamp::from_unix_seconds(1_800_050_000);
    let task = ImportTask {
        id: ImportTaskId::from_non_secret_parts(&["s50", label]),
        root_path: path_str(&canonical_root).to_string(),
        status: ImportTaskStatus::Running,
        queued_at: now,
        started_at: Some(now),
        finished_at: None,
        updated_at: now,
    };
    support::insert_import_task_with_reviewed_contract(&store, &task);
    import_root_with_options(
        &data_dir,
        &store,
        &task,
        &canonical_root,
        now,
        support::reviewed_import_options(),
    )
    .unwrap();
    assert_eq!(store.ingest_jobs().unwrap().len(), 1);
    drop(store);
    data_dir
}

fn queued_ocr_job_status(data_dir: &Path) -> IngestJobStatus {
    ReadMetaStore::open_data_dir(data_dir)
        .unwrap()
        .ingest_jobs()
        .unwrap()[0]
        .status
}

fn open_owned_store(data_dir: &Path) -> OwnedMetaStore {
    let owner = match DataDirectoryOwnerLease::try_acquire(data_dir).unwrap() {
        DataDirectoryOwnerAcquisition::Acquired(owner) => owner,
        DataDirectoryOwnerAcquisition::Contended => panic!("synthetic data dir is owned"),
    };
    owner.open_store().unwrap()
}

fn single_page_pdf() -> Vec<u8> {
    let objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>".as_slice(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".as_slice(),
        b"<< /Type /Page /Parent 2 0 R /Resources << /XObject << /Im1 4 0 R >> >> /MediaBox [0 0 72 72] /Contents 5 0 R >>".as_slice(),
        b"<< /Type /XObject /Subtype /Image /Width 10 /Height 10 /ColorSpace /DeviceGray /BitsPerComponent 8 /Length 4 >>\nstream\n1111\nendstream".as_slice(),
        b"<< /Length 29 >>\nstream\nq 10 0 0 10 0 0 cm /Im1 Do Q\n\nendstream".as_slice(),
    ];
    let mut output = b"%PDF-1.4\n".to_vec();
    let mut offsets = Vec::new();
    for (index, object) in objects.iter().enumerate() {
        offsets.push(output.len());
        output.extend_from_slice(format!("{} 0 obj\n", index + 1).as_bytes());
        output.extend_from_slice(object);
        output.extend_from_slice(b"\nendobj\n");
    }
    let xref = output.len();
    output.extend_from_slice(
        format!("xref\n0 {}\n0000000000 65535 f\r\n", objects.len() + 1).as_bytes(),
    );
    for offset in offsets {
        output.extend_from_slice(format!("{offset:010} 00000 n\r\n").as_bytes());
    }
    output.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            objects.len() + 1
        )
        .as_bytes(),
    );
    output
}

fn marker_command(label: &str) -> PathBuf {
    marker_command_with_exit(label, 0)
}

fn marker_command_with_exit(label: &str, code: i32) -> PathBuf {
    let directory = temp_dir(label);
    let command = directory.join("tesseract");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::write(
            &command,
            format!(
                "#!/bin/sh\ntouch '{}.executed'\nexit {code}\n",
                command.display()
            ),
        )
        .unwrap();
        fs::set_permissions(&command, fs::Permissions::from_mode(0o755)).unwrap();
    }
    #[cfg(windows)]
    fs::write(&command, b"synthetic invalid executable").unwrap();
    command
}

fn assert_configuration_blocked(output: &std::process::Output) {
    assert!(!output.status.success());
    let payload: serde_json::Value = serde_json::from_slice(&output.stderr).unwrap();
    assert_eq!(payload["schema_version"], "resume-ir.daemon-fatal.v1");
    assert_eq!(payload["class"], "configuration_invalid");
    assert_eq!(payload["disposition"], "blocked");
}

fn assert_runtime_unavailable(status: &serde_json::Value, runtime: &str, reason: &str) {
    assert_eq!(status["optional_runtimes"][runtime]["state"], "unavailable");
    assert_eq!(status["optional_runtimes"][runtime]["reason"], reason);
}

fn assert_capability_unavailable(status: &serde_json::Value, capability: &str, reason: &str) {
    assert_eq!(status["capabilities"][capability]["state"], "unavailable");
    assert_eq!(status["capabilities"][capability]["reason"], reason);
}

fn read_json(path: PathBuf) -> Option<serde_json::Value> {
    serde_json::from_slice(&fs::read(path).ok()?).ok()
}

fn snapshot_existing_files(root: &Path) -> BTreeMap<PathBuf, String> {
    let mut snapshot = BTreeMap::new();
    collect_files(root, root, &mut snapshot);
    snapshot
}

fn collect_files(root: &Path, current: &Path, snapshot: &mut BTreeMap<PathBuf, String>) {
    for entry in fs::read_dir(current).unwrap() {
        let path = entry.unwrap().path();
        let metadata = fs::symlink_metadata(&path).unwrap();
        if metadata.file_type().is_dir() {
            collect_files(root, &path, snapshot);
        } else if metadata.file_type().is_file() {
            snapshot.insert(
                path.strip_prefix(root).unwrap().to_path_buf(),
                format!("{:x}", Sha256::digest(fs::read(path).unwrap())),
            );
        }
    }
}

fn snapshot_selected_files<'a>(
    root: &Path,
    files: impl Iterator<Item = &'a PathBuf>,
) -> BTreeMap<PathBuf, String> {
    files
        .map(|relative| {
            (
                relative.clone(),
                format!(
                    "{:x}",
                    Sha256::digest(fs::read(root.join(relative)).unwrap())
                ),
            )
        })
        .collect()
}

fn temp_dir(label: &str) -> PathBuf {
    let directory = std::env::temp_dir().join(format!(
        "resume-ir-{label}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir(&directory).unwrap();
    directory
}

fn path_str(path: &Path) -> &str {
    path.to_str().unwrap()
}

fn remove_dir(path: &Path) {
    if path.exists() {
        fs::remove_dir_all(path).unwrap();
    }
}

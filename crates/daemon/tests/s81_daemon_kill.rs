use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
#[cfg(unix)]
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStderr, ChildStdout, Command, Stdio};
use std::sync::mpsc::{Receiver, TryRecvError};
#[cfg(windows)]
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use import_pipeline::ImportTaskOwnerLock;
use meta_store::{
    DataDirectoryOwnerAcquisition, DataDirectoryOwnerLease, ImportRootKind, ImportScanProfile,
    ImportScanScope, ImportTask, ImportTaskId, ImportTaskStatus, ReadMetaStore, UnixTimestamp,
};
use process_containment::ContainedChild;

mod support;

const ACTIVE_IMPORT_FILE_COUNT: usize = 1_024;

// These tests each own a resident native daemon, and one deliberately drives a
// large active import. Windows process and index-writer startup can otherwise
// be starved by a sibling test in the same libtest process. Production daemon
// ownership remains one daemon per data directory; only this test process has
// a single native lifecycle admission slot on Windows.
macro_rules! serialize_windows_s81_daemon_test {
    () => {
        #[cfg(windows)]
        let _guard = windows_s81_daemon_test_lock();
    };
}

#[cfg(windows)]
fn windows_s81_daemon_test_lock() -> MutexGuard<'static, ()> {
    static WINDOWS_S81_DAEMON_TEST_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    WINDOWS_S81_DAEMON_TEST_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[test]
fn foreground_daemon_can_be_killed_and_restarted_without_path_leak() {
    serialize_windows_s81_daemon_test!();
    let data_dir = temp_dir("daemon-kill-restart-data");

    let mut child = Command::new(env!("CARGO_BIN_EXE_resume-daemon"))
        .args(["--data-dir", path_str(&data_dir), "run", "--foreground"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start resume-daemon foreground");
    let stdout = child.stdout.take().expect("daemon stdout");
    let stdout = spawn_stdout_reader(stdout);

    wait_until_metadata_store_ready(&mut child, &data_dir);
    wait_until_stdout_contains(
        &mut child,
        &stdout,
        "resume-daemon foreground ready",
        Duration::from_secs(5),
    );
    child.kill().expect("kill foreground daemon");
    let killed = wait_child(child, stdout);
    assert!(!killed.success);
    assert!(killed.stderr.is_empty());
    assert!(killed.stdout.contains("resume-daemon foreground ready"));
    assert!(killed.stdout.contains("mode: foreground"));
    assert!(!killed.stdout.contains(path_str(&data_dir)));

    let restart = Command::new(env!("CARGO_BIN_EXE_resume-daemon"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "run",
            "--foreground",
            "--once",
        ])
        .output()
        .expect("restart resume-daemon foreground once");
    assert!(
        restart.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&restart.stdout),
        String::from_utf8_lossy(&restart.stderr)
    );
    assert!(restart.stderr.is_empty());
    let restart_stdout = String::from_utf8_lossy(&restart.stdout);
    assert!(restart_stdout.contains("resume-daemon foreground ready"));
    assert!(restart_stdout.contains("mode: once"));
    assert!(restart_stdout.contains("index health: empty"));
    assert!(!restart_stdout.contains(path_str(&data_dir)));

    remove_dir(&data_dir);
}

#[test]
fn parent_lifecycle_eof_gracefully_stops_foreground_daemon() {
    serialize_windows_s81_daemon_test!();
    let data_dir = temp_dir("parent-lifecycle-eof-data");

    let mut command = Command::new(env!("CARGO_BIN_EXE_resume-daemon"));
    command
        .args([
            "--data-dir",
            path_str(&data_dir),
            "run",
            "--foreground",
            "--parent-lifecycle-stdin",
            "--launch-id",
            "8181818181818181818181818181818181818181818181818181818181818101",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = ContainedChild::spawn(&mut command).expect("start parent-owned resume-daemon");
    let lifecycle_stdin = child.take_stdin().expect("daemon lifecycle stdin");
    let stdout = child.take_stdout().expect("daemon stdout");
    let stderr = child.take_stderr().expect("daemon stderr");
    let stdout = spawn_stdout_reader(stdout);

    wait_until_contained_stdout_contains(
        &mut child,
        &stdout,
        "resume-daemon foreground ready",
        Duration::from_secs(5),
    );
    drop(lifecycle_stdin);

    let stopped = wait_contained_child(child, stdout, stderr, Duration::from_secs(5));
    assert!(stopped.success, "stderr:\n{}", stopped.stderr);
    assert!(stopped.stderr.is_empty());
    assert!(stopped.stdout.contains("resume-daemon foreground ready"));
    assert!(!stopped.stdout.contains(path_str(&data_dir)));

    remove_dir(&data_dir);
}

#[test]
fn parent_lifecycle_eof_interrupts_an_active_import_without_partial_publication() {
    serialize_windows_s81_daemon_test!();
    let runtime_capacity = support::import_runtime_capacity_lease();
    let data_dir = temp_dir("parent-lifecycle-active-import-data");
    let import_root = active_import_root(ACTIVE_IMPORT_FILE_COUNT);
    let canonical_root = fs::canonicalize(&import_root).unwrap();
    let task_id = seed_queued_import_task(&data_dir, &canonical_root);
    let observer = ReadMetaStore::open_data_dir(&data_dir).unwrap();

    let mut command = support::import_capable_daemon_command(&runtime_capacity);
    command
        .args([
            "--data-dir",
            path_str(&data_dir),
            "run",
            "--foreground",
            "--parent-lifecycle-stdin",
            "--launch-id",
            "8181818181818181818181818181818181818181818181818181818181818102",
            "--work-imports",
            "--worker-interval-ms",
            "10",
            "--max-worker-ticks",
            "2400",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = ContainedChild::spawn(&mut command).expect("start active import daemon");
    let lifecycle_stdin = child.take_stdin().expect("daemon lifecycle stdin");
    let stdout = child.take_stdout().expect("daemon stdout");
    let stderr = child.take_stderr().expect("daemon stderr");
    let stdout = spawn_stdout_reader(stdout);
    wait_until_import_task_is_running_and_owned(&mut child, &observer, &data_dir, &task_id);

    let started = Instant::now();
    drop(lifecycle_stdin);
    let stopped = wait_contained_child(child, stdout, stderr, Duration::from_secs(4));
    let elapsed = started.elapsed();
    assert!(stopped.success, "stderr:\n{}", stopped.stderr);
    assert!(stopped.stderr.is_empty());
    assert!(
        elapsed < Duration::from_secs(2),
        "active import did not stop cooperatively: {elapsed:?}"
    );

    assert_eq!(
        observer
            .import_task_by_id(&task_id)
            .unwrap()
            .unwrap()
            .status,
        ImportTaskStatus::Queued
    );
    let interrupted_state = observer.search_projection_state().unwrap();
    assert_eq!(interrupted_state.visible_epoch, 0);
    assert!(interrupted_state.generation.is_none());
    drop(observer);

    let restart = support::import_capable_daemon_command(&runtime_capacity)
        .args([
            "--data-dir",
            path_str(&data_dir),
            "run",
            "--foreground",
            "--work-imports",
            "--worker-interval-ms",
            "1",
            "--max-worker-ticks",
            "2",
        ])
        .output()
        .expect("restart interrupted import daemon");
    assert!(
        restart.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&restart.stdout),
        String::from_utf8_lossy(&restart.stderr)
    );
    assert!(restart.stderr.is_empty());
    let store = ReadMetaStore::open_data_dir(&data_dir).unwrap();
    assert_eq!(
        store.import_task_by_id(&task_id).unwrap().unwrap().status,
        ImportTaskStatus::Completed
    );
    assert!(store
        .search_projection_state()
        .unwrap()
        .generation
        .is_some());

    remove_dir(&data_dir);
    remove_dir(&import_root);
}

#[cfg(unix)]
#[test]
fn parent_lifecycle_stdin_rejects_a_non_group_leader_without_signalling_its_caller() {
    let data_dir = temp_dir("parent-lifecycle-non-leader-data");
    let output = Command::new(env!("CARGO_BIN_EXE_resume-daemon"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "run",
            "--foreground",
            "--parent-lifecycle-stdin",
            "--launch-id",
            "8181818181818181818181818181818181818181818181818181818181818103",
        ])
        .stdin(Stdio::null())
        .output()
        .expect("run non-isolated daemon");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    let fatal: serde_json::Value = serde_json::from_str(stderr.trim()).unwrap();
    assert_eq!(fatal["schema_version"], "resume-ir.daemon-fatal.v1");
    assert_eq!(fatal["class"], "runtime_integrity");
    assert_eq!(fatal["disposition"], "blocked");
    remove_dir(&data_dir);
}

#[cfg(unix)]
#[test]
fn parent_lifecycle_eof_revokes_generation_before_a_slow_client_can_force_kill() {
    let data_dir = temp_dir("parent-lifecycle-stalled-data");
    let mut command = term_ignoring_daemon_command();
    command
        .args([
            "--data-dir",
            path_str(&data_dir),
            "run",
            "--foreground",
            "--parent-lifecycle-stdin",
            "--launch-id",
            "8181818181818181818181818181818181818181818181818181818181818104",
            "--ipc-listen",
            "127.0.0.1:0",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = ContainedChild::spawn(&mut command).expect("start isolated daemon group");
    let lifecycle_stdin = child.take_stdin().expect("daemon lifecycle stdin");
    let stdout = child.take_stdout().expect("daemon stdout");
    let stderr = child.take_stderr().expect("daemon stderr");
    let stdout = spawn_stdout_reader(stdout);
    let endpoint = wait_until_contained_stdout_prefix(
        &mut child,
        &stdout,
        "ipc status endpoint: ",
        Duration::from_secs(5),
    );
    let address = endpoint
        .strip_prefix("http://")
        .and_then(|value| value.split_once('/').map(|(address, _)| address))
        .expect("status endpoint address");
    let mut stalled_stream = TcpStream::connect(address).expect("connect stalled IPC client");
    stalled_stream
        .write_all(b"G")
        .expect("start incomplete IPC request");
    let dripper = std::thread::spawn(move || {
        for _ in 0..20 {
            std::thread::sleep(Duration::from_millis(250));
            if stalled_stream.write_all(b"E").is_err() {
                return;
            }
        }
    });
    std::thread::sleep(Duration::from_millis(500));

    let started = Instant::now();
    drop(lifecycle_stdin);
    let revocation_deadline = started + Duration::from_millis(1_500);
    while (data_dir.join("ipc.endpoints.json").exists() || data_dir.join("ipc.auth").exists())
        && Instant::now() < revocation_deadline
    {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        !data_dir.join("ipc.endpoints.json").exists(),
        "discovery survived into the watchdog kill window"
    );
    assert!(
        !data_dir.join("ipc.auth").exists(),
        "auth survived into the watchdog kill window"
    );
    let stopped = wait_contained_child(child, stdout, stderr, Duration::from_secs(4));
    let elapsed = started.elapsed();
    dripper.join().expect("join stalled IPC client");

    assert!(stopped.success, "stderr:\n{}", stopped.stderr);
    assert!(
        elapsed < Duration::from_secs(2),
        "slow client prevented cooperative shutdown: {elapsed:?}"
    );
    assert!(stopped.stderr.is_empty(), "stderr:\n{}", stopped.stderr);
    remove_dir(&data_dir);
}

#[cfg(unix)]
fn term_ignoring_daemon_command() -> Command {
    let mut command = Command::new("/bin/sh");
    command
        .args([
            "-c",
            "trap '' TERM; exec \"$RESUME_IR_DAEMON_TEST_BINARY\" \"$@\"",
            "resume-daemon",
        ])
        .env(
            "RESUME_IR_DAEMON_TEST_BINARY",
            env!("CARGO_BIN_EXE_resume-daemon"),
        );
    command
}

fn wait_until_metadata_store_ready(child: &mut Child, data_dir: &Path) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if ReadMetaStore::open_data_dir(data_dir)
            .and_then(|store| store.status_summary().map(|_| ()))
            .is_ok()
        {
            return;
        }
        if let Some(status) = child.try_wait().expect("poll daemon child") {
            panic!("daemon exited before metadata store was ready: {status}");
        }
        std::thread::sleep(Duration::from_millis(25));
    }

    let _ = child.kill();
    let _ = child.wait();
    panic!("daemon did not prepare metadata store before timeout");
}

fn wait_until_import_task_is_running_and_owned(
    child: &mut ContainedChild,
    store: &ReadMetaStore,
    data_dir: &Path,
    task_id: &ImportTaskId,
) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline {
        let task_is_running = store
            .import_task_by_id(task_id)
            .ok()
            .flatten()
            .is_some_and(|task| task.status == ImportTaskStatus::Running);
        if task_is_running
            && matches!(
                ImportTaskOwnerLock::try_acquire(data_dir, task_id),
                Ok(None)
            )
        {
            return;
        }
        if let Some(status) = child.try_wait().expect("poll active import daemon") {
            panic!("daemon exited before active import ownership: {status}");
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    child.terminate();
    panic!("daemon did not own an active import before timeout");
}

struct StdoutReader {
    receiver: Receiver<String>,
    join: JoinHandle<String>,
}

fn spawn_stdout_reader(stdout: ChildStdout) -> StdoutReader {
    let (sender, receiver) = std::sync::mpsc::channel();
    let join = std::thread::spawn(move || {
        let mut reader = BufReader::new(stdout);
        let mut output = String::new();
        loop {
            let mut line = String::new();
            match reader.read_line(&mut line) {
                Ok(0) => return output,
                Ok(_) => {
                    output.push_str(&line);
                    let _ = sender.send(line);
                }
                Err(_) => return output,
            }
        }
    });

    StdoutReader { receiver, join }
}

fn wait_until_stdout_contains(
    child: &mut Child,
    stdout: &StdoutReader,
    needle: &str,
    timeout: Duration,
) {
    let deadline = Instant::now() + timeout;
    loop {
        match stdout.receiver.try_recv() {
            Ok(line) if line.contains(needle) => return,
            Ok(_) => {}
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => {
                panic!("daemon stdout closed before expected line");
            }
        }
        if let Some(status) = child.try_wait().expect("poll daemon child") {
            panic!("daemon exited before expected stdout line: {status}");
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("daemon did not print expected stdout line before timeout");
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

fn wait_until_contained_stdout_contains(
    child: &mut ContainedChild,
    stdout: &StdoutReader,
    needle: &str,
    timeout: Duration,
) {
    let _ = wait_until_contained_stdout_match(
        child,
        stdout,
        ContainedStdoutMatch::Contains(needle),
        timeout,
    );
}

fn wait_until_contained_stdout_prefix(
    child: &mut ContainedChild,
    stdout: &StdoutReader,
    prefix: &str,
    timeout: Duration,
) -> String {
    wait_until_contained_stdout_match(child, stdout, ContainedStdoutMatch::Prefix(prefix), timeout)
}

#[derive(Clone, Copy)]
enum ContainedStdoutMatch<'a> {
    Contains(&'a str),
    Prefix(&'a str),
}

fn wait_until_contained_stdout_match(
    child: &mut ContainedChild,
    stdout: &StdoutReader,
    expected: ContainedStdoutMatch<'_>,
    timeout: Duration,
) -> String {
    let deadline = Instant::now() + timeout;
    loop {
        match stdout.receiver.try_recv() {
            Ok(line) => match expected {
                ContainedStdoutMatch::Contains(needle) if line.contains(needle) => return line,
                ContainedStdoutMatch::Prefix(prefix) => {
                    if let Some(value) = line.trim().strip_prefix(prefix) {
                        return value.to_string();
                    }
                }
                ContainedStdoutMatch::Contains(_) => {}
            },
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => {
                panic!("daemon stdout closed before expected line");
            }
        }
        if let Some(status) = child.try_wait().expect("poll contained daemon") {
            panic!("daemon exited before expected stdout line: {status}");
        }
        if Instant::now() >= deadline {
            child.terminate();
            panic!("daemon did not print expected stdout line before timeout");
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

struct ChildOutput {
    success: bool,
    stdout: String,
    stderr: String,
}

fn wait_child(mut child: Child, stdout: StdoutReader) -> ChildOutput {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(status) = child.try_wait().expect("poll daemon child") {
            let stdout = stdout.join.join().unwrap_or_default();
            let mut stderr = String::new();
            child
                .stderr
                .take()
                .expect("daemon stderr")
                .read_to_string(&mut stderr)
                .expect("read daemon stderr");
            return ChildOutput {
                success: status.success(),
                stdout,
                stderr,
            };
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("daemon did not exit after kill");
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

fn wait_contained_child(
    mut child: ContainedChild,
    stdout: StdoutReader,
    mut stderr: ChildStderr,
    timeout: Duration,
) -> ChildOutput {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(status) = child.try_wait().expect("poll contained daemon") {
            let stdout = stdout.join.join().unwrap_or_default();
            let mut stderr_output = String::new();
            stderr
                .read_to_string(&mut stderr_output)
                .expect("read daemon stderr");
            return ChildOutput {
                success: status.success(),
                stdout,
                stderr: stderr_output,
            };
        }
        if Instant::now() >= deadline {
            child.terminate();
            panic!("contained daemon did not exit before timeout");
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

fn temp_dir(label: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!("resume-ir-s81-daemon-{label}-{unique}"));
    remove_dir(&path);
    fs::create_dir_all(&path).unwrap();
    path
}

fn active_import_root(file_count: usize) -> PathBuf {
    let root = temp_dir("active-import-root");
    for index in 0..file_count {
        fs::write(
            root.join(format!("candidate-{index:04}.txt")),
            format!(
                "SUMMARY\nLifecycleCandidate{index} backend engineer.\nEXPERIENCE\nBuilt reliable Rust services. {}\nSKILLS\nRust distributed systems",
                "local-first search publication ".repeat(64)
            ),
        )
        .unwrap();
    }
    root
}

fn seed_queued_import_task(data_dir: &Path, canonical_root: &Path) -> ImportTaskId {
    let owner = match DataDirectoryOwnerLease::try_acquire(data_dir).unwrap() {
        DataDirectoryOwnerAcquisition::Acquired(owner) => owner,
        DataDirectoryOwnerAcquisition::Contended => panic!("synthetic data directory is owned"),
    };
    let store = owner.open_store().unwrap();
    let now = UnixTimestamp::from_unix_seconds(1_700_000_000);
    let task_id = ImportTaskId::from_non_secret_parts(&["s807", "lifecycle-active-import"]);
    let task = ImportTask {
        id: task_id.clone(),
        root_path: path_str(canonical_root).to_string(),
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
    let contract = support::activate_reviewed_processing_contract(&store, now);
    store
        .insert_import_task_with_scan_scope(&task, &scope, &contract)
        .unwrap();
    task_id
}

fn path_str(path: &Path) -> &str {
    path.to_str().unwrap()
}

fn remove_dir(path: &Path) {
    let _ = fs::remove_dir_all(path);
}

use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
#[cfg(unix)]
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStderr, ChildStdout, Command, Stdio};
use std::sync::mpsc::{Receiver, TryRecvError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use meta_store::MetaStore;
use process_containment::ContainedChild;

#[test]
fn foreground_daemon_can_be_killed_and_restarted_without_path_leak() {
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
    let data_dir = temp_dir("parent-lifecycle-eof-data");

    let mut command = Command::new(env!("CARGO_BIN_EXE_resume-daemon"));
    command
        .args([
            "--data-dir",
            path_str(&data_dir),
            "run",
            "--foreground",
            "--parent-lifecycle-stdin",
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
            "--once",
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
fn parent_lifecycle_eof_forces_a_stalled_daemon_group_to_exit() {
    let data_dir = temp_dir("parent-lifecycle-stalled-data");
    let mut command = term_ignoring_daemon_command();
    command
        .args([
            "--data-dir",
            path_str(&data_dir),
            "run",
            "--foreground",
            "--parent-lifecycle-stdin",
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
    let stopped = wait_contained_child(child, stdout, stderr, Duration::from_secs(4));
    let elapsed = started.elapsed();
    dripper.join().expect("join stalled IPC client");

    assert!(!stopped.success, "daemon exited cooperatively");
    assert!(
        elapsed >= Duration::from_millis(1_800),
        "watchdog skipped graceful shutdown window: {elapsed:?}"
    );
    assert!(
        elapsed < Duration::from_secs(3),
        "watchdog exceeded bounded shutdown: {elapsed:?}"
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
        if MetaStore::open_data_dir(data_dir)
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

fn path_str(path: &Path) -> &str {
    path.to_str().unwrap()
}

fn remove_dir(path: &Path) {
    let _ = fs::remove_dir_all(path);
}

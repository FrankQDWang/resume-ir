use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

#[test]
fn fault_simulate_disk_space_low_reproduces_without_writing_or_leaking_paths() {
    let data_dir = temp_path("fault-disk-private-data");
    let scratch_dir = temp_path("fault-disk-private-scratch");

    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "fault-simulate",
            "--case",
            "disk-space-low",
            "--scratch-dir",
            path_str(&scratch_dir),
            "--required-bytes",
            "4096",
            "--available-bytes",
            "1024",
        ])
        .output()
        .expect("run disk-space-low fault simulation");

    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("fault: disk_space_low"));
    assert!(stdout.contains("status: reproduced"));
    assert!(stdout.contains("required bytes: 4096"));
    assert!(stdout.contains("available bytes: 1024"));
    assert!(stdout.contains("probe writes: skipped"));
    assert!(stdout.contains("paths: <redacted>"));
    assert!(!stdout.contains(path_str(&data_dir)));
    assert!(!stdout.contains(path_str(&scratch_dir)));
    assert!(!scratch_dir.exists());
}

#[test]
fn fault_simulate_disk_space_ok_writes_bounded_probe_and_cleans_up() {
    let data_dir = temp_path("fault-disk-ok-private-data");
    let scratch_dir = temp_path("fault-disk-ok-private-scratch");

    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "fault-simulate",
            "--case",
            "disk-space-low",
            "--scratch-dir",
            path_str(&scratch_dir),
            "--required-bytes",
            "64",
            "--available-bytes",
            "4096",
        ])
        .output()
        .expect("run disk-space-ok fault simulation");

    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("fault: disk_space_low"));
    assert!(stdout.contains("status: not reproduced"));
    assert!(stdout.contains("probe writes: completed"));
    assert!(stdout.contains("probe bytes: 64"));
    assert!(stdout.contains("paths: <redacted>"));
    assert!(!stdout.contains(path_str(&data_dir)));
    assert!(!stdout.contains(path_str(&scratch_dir)));
    assert!(scratch_dir.exists());
    assert!(fs::read_dir(&scratch_dir).unwrap().next().is_none());

    remove_dir(&scratch_dir);
}

#[cfg(unix)]
#[test]
fn fault_simulate_permission_denied_reproduces_without_path_leak() {
    use std::os::unix::fs::PermissionsExt;

    let data_dir = temp_path("fault-permission-private-data");
    let scratch_dir = temp_dir("fault-permission-private-scratch");
    fs::set_permissions(&scratch_dir, fs::Permissions::from_mode(0o500)).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "fault-simulate",
            "--case",
            "permission-denied",
            "--scratch-dir",
            path_str(&scratch_dir),
        ])
        .output()
        .expect("run permission-denied fault simulation");

    fs::set_permissions(&scratch_dir, fs::Permissions::from_mode(0o700)).unwrap();

    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("fault: permission_denied"));
    assert!(stdout.contains("status: reproduced"));
    assert!(stdout.contains("probe writes: denied"));
    assert!(stdout.contains("paths: <redacted>"));
    assert!(!stdout.contains(path_str(&data_dir)));
    assert!(!stdout.contains(path_str(&scratch_dir)));

    remove_dir(&scratch_dir);
}

#[test]
fn fault_simulate_file_lock_reproduces_contention_without_path_leak() {
    let data_dir = temp_path("fault-lock-private-data");
    let scratch_dir = temp_path("fault-lock-private-scratch");

    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "fault-simulate",
            "--case",
            "file-lock",
            "--scratch-dir",
            path_str(&scratch_dir),
        ])
        .output()
        .expect("run file-lock fault simulation");

    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("fault: file_lock"));
    assert!(stdout.contains("status: reproduced"));
    assert!(stdout.contains("lock holder: active"));
    assert!(stdout.contains("contended lock: denied"));
    assert!(stdout.contains("paths: <redacted>"));
    assert!(!stdout.contains(path_str(&data_dir)));
    assert!(!stdout.contains(path_str(&scratch_dir)));
    assert!(scratch_dir.exists());
    assert!(fs::read_dir(&scratch_dir).unwrap().next().is_none());

    remove_dir(&scratch_dir);
}

#[cfg(unix)]
#[test]
fn fault_simulate_daemon_kill_restarts_configured_daemon_without_path_leak() {
    let data_dir = temp_path("fault-daemon-private-data");
    let scratch_dir = temp_path("fault-daemon-private-scratch");
    let daemon_binary = daemon_fixture_script("fault-daemon-private-helper");

    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "fault-simulate",
            "--case",
            "daemon-kill",
            "--scratch-dir",
            path_str(&scratch_dir),
            "--daemon-binary",
            path_str(&daemon_binary),
        ])
        .output()
        .expect("run daemon-kill fault simulation");

    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("fault: daemon_kill"));
    assert!(stdout.contains("status: reproduced"));
    assert!(stdout.contains("daemon ready: yes"));
    assert!(stdout.contains("terminated daemon: yes"));
    assert!(stdout.contains("restart check: passed"));
    assert!(stdout.contains("paths: <redacted>"));
    assert!(!stdout.contains(path_str(&data_dir)));
    assert!(!stdout.contains(path_str(&scratch_dir)));
    assert!(!stdout.contains(path_str(&daemon_binary)));
    assert!(scratch_dir.exists());
    assert!(fs::read_dir(&scratch_dir).unwrap().next().is_none());

    remove_dir(&scratch_dir);
    let _ = fs::remove_file(&daemon_binary);
}

#[test]
fn fault_simulate_usage_errors_do_not_leak_private_paths() {
    let data_dir = temp_path("fault-usage-private-data");
    let scratch_dir = temp_path("fault-usage-private-scratch");

    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "fault-simulate",
            "--case",
            "disk-space-low",
            "--scratch-dir",
            path_str(&scratch_dir),
            "--required-bytes",
            "0",
            "--available-bytes",
            "1024",
        ])
        .output()
        .expect("run invalid fault simulation");

    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("resume-cli fault-simulate"));
    assert!(!stderr.contains(path_str(&data_dir)));
    assert!(!stderr.contains(path_str(&scratch_dir)));
}

#[cfg(unix)]
fn daemon_fixture_script(label: &str) -> PathBuf {
    let path = temp_path(label);
    fs::write(
        &path,
        r#"#!/bin/sh
if [ "$1" != "--data-dir" ]; then
  exit 64
fi
shift 2
if [ "$1" != "run" ]; then
  exit 64
fi
shift
once=0
while [ "$#" -gt 0 ]; do
  case "$1" in
    --foreground)
      shift
      ;;
    --once)
      once=1
      shift
      ;;
    *)
      exit 64
      ;;
  esac
done
printf 'resume-daemon foreground ready\n'
printf 'mode: foreground\n'
if [ "$once" = 1 ]; then
  exit 0
fi
while :; do
  sleep 1
done
"#,
    )
    .unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
    path
}

fn temp_path(label: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!("resume-ir-s71-cli-{label}-{unique}"))
}

fn temp_dir(label: &str) -> PathBuf {
    let path = temp_path(label);
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

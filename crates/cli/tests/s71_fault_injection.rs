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
fn fault_simulate_json_outputs_structured_redacted_evidence() {
    let data_dir = temp_path("fault-json-private-data");
    let scratch_dir = temp_path("fault-json-private-scratch");

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
            "--json",
        ])
        .output()
        .expect("run disk-space-low JSON fault simulation");

    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["schema_version"], "fault-simulation.v1");
    assert_eq!(report["redacted"], true);
    assert_eq!(report["fault"], "disk_space_low");
    assert_eq!(report["status"], "reproduced");
    assert_eq!(report["paths"], "<redacted>");
    assert_eq!(report["details"]["required_bytes"], 4096);
    assert_eq!(report["details"]["available_bytes"], 1024);
    assert_eq!(report["details"]["probe_writes"], "skipped");
    assert!(!stdout.contains(path_str(&data_dir)));
    assert!(!stdout.contains(path_str(&scratch_dir)));
    assert!(!scratch_dir.exists());
}

#[test]
fn fault_simulate_local_safe_suite_json_runs_redacted_reproducible_probes() {
    let data_dir = temp_path("fault-suite-private-data");
    let scratch_dir = temp_path("fault-suite-private-scratch");

    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "fault-simulate",
            "--suite",
            "local-safe",
            "--scratch-dir",
            path_str(&scratch_dir),
            "--json",
        ])
        .output()
        .expect("run local-safe fault simulation suite");

    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["schema_version"], "fault-simulation-suite.v1");
    assert_eq!(report["suite"], "local_safe");
    assert_eq!(report["redacted"], true);
    assert_eq!(report["paths"], "<redacted>");
    assert_eq!(report["evidence_level"], "local_synthetic_fault_suite");
    assert_eq!(report["release_hardware_drills"], "blocked");
    assert_eq!(report["summary"]["total_cases"], 10);
    assert_eq!(report["summary"]["failed_cases"], 0);
    assert_eq!(report["summary"]["release_blockers_cleared"], false);
    let cases = report["cases"].as_array().expect("suite cases");
    for expected in [
        "disk_space_low",
        "permission_denied",
        "file_lock",
        "index_snapshot_corrupt",
        "metadata_migration",
        "model_checksum",
        "daemon_kill",
        "ocr_crash",
        "battery_mode",
        "external_drive_disconnect",
    ] {
        let case = cases
            .iter()
            .find(|case| case["fault"] == expected)
            .unwrap_or_else(|| panic!("missing suite case {expected}"));
        assert!(case["status"].is_string());
        assert_ne!(case["status"], "failed");
        assert_eq!(case["paths"], "<redacted>");
        assert_eq!(case["redacted"], true);
    }
    assert!(stdout.contains("\"real_hardware_drill\": \"blocked\""));
    assert!(!stdout.contains(path_str(&data_dir)));
    assert!(!stdout.contains(path_str(&scratch_dir)));
    assert!(!stdout.contains("SYNTHETIC MODEL CHECKSUM PROBE"));
    assert!(!stdout.contains("SYNTHETIC OCR CRASH PROBE BYTES"));

    remove_dir(&scratch_dir);
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

#[test]
fn fault_simulate_index_snapshot_corrupt_recovers_without_payload_or_path_leak() {
    let data_dir = temp_path("fault-index-corrupt-private-data");
    let scratch_dir = temp_path("fault-index-corrupt-private-scratch");

    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "fault-simulate",
            "--case",
            "index-snapshot-corrupt",
            "--scratch-dir",
            path_str(&scratch_dir),
        ])
        .output()
        .expect("run index-snapshot-corrupt fault simulation");

    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("fault: index_snapshot_corrupt"));
    assert!(stdout.contains("status: reproduced"));
    assert!(stdout.contains("active snapshot: corrupt"));
    assert!(stdout.contains("fallback snapshot: recovered"));
    assert!(stdout.contains("query after recovery: passed"));
    assert!(stdout.contains("paths: <redacted>"));
    assert!(!stdout.contains("SYNTHETIC_INDEX_CORRUPT_PRIVATE_TOKEN"));
    assert!(!stdout.contains("synthetic-corrupt-active.pdf"));
    assert!(!stdout.contains("synthetic-recovered.pdf"));
    assert!(!stdout.contains(path_str(&data_dir)));
    assert!(!stdout.contains(path_str(&scratch_dir)));
    assert!(scratch_dir.exists());
    assert!(fs::read_dir(&scratch_dir).unwrap().next().is_none());

    remove_dir(&scratch_dir);
}

#[test]
fn fault_simulate_battery_mode_reproduces_degradation_without_path_leak() {
    let data_dir = temp_path("fault-battery-private-data");
    let scratch_dir = temp_path("fault-battery-private-scratch");

    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "fault-simulate",
            "--case",
            "battery-mode",
            "--scratch-dir",
            path_str(&scratch_dir),
            "--battery-state",
            "battery",
        ])
        .output()
        .expect("run battery-mode fault simulation");

    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("fault: battery_mode"));
    assert!(stdout.contains("status: reproduced"));
    assert!(stdout.contains("power source: battery"));
    assert!(stdout.contains("degradation: pause or lower OCR/vector worker budgets"));
    assert!(stdout.contains("real hardware drill: blocked"));
    assert!(stdout.contains("paths: <redacted>"));
    assert!(!stdout.contains(path_str(&data_dir)));
    assert!(!stdout.contains(path_str(&scratch_dir)));
    assert!(!scratch_dir.exists());
}

#[test]
fn fault_simulate_external_drive_disconnect_reproduces_without_path_leak() {
    let data_dir = temp_path("fault-drive-private-data");
    let scratch_dir = temp_path("fault-drive-private-scratch");

    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "fault-simulate",
            "--case",
            "external-drive-disconnect",
            "--scratch-dir",
            path_str(&scratch_dir),
            "--drive-state",
            "disconnected",
        ])
        .output()
        .expect("run external-drive-disconnect fault simulation");

    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("fault: external_drive_disconnect"));
    assert!(stdout.contains("status: reproduced"));
    assert!(stdout.contains("mount state: disconnected"));
    assert!(stdout.contains("import roots: unavailable"));
    assert!(stdout.contains("recovery: reconnect drive or reselect root before retry"));
    assert!(stdout.contains("real hardware drill: blocked"));
    assert!(stdout.contains("paths: <redacted>"));
    assert!(!stdout.contains(path_str(&data_dir)));
    assert!(!stdout.contains(path_str(&scratch_dir)));
    assert!(!scratch_dir.exists());
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

#[cfg(unix)]
#[test]
fn fault_simulate_ocr_crash_reproduces_engine_failure_without_payload_or_path_leak() {
    let data_dir = temp_path("fault-ocr-crash-private-data");
    let scratch_dir = temp_path("fault-ocr-crash-private-scratch");
    let ocr_command = ocr_crash_fixture_script("fault-ocr-crash-private-helper");

    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "fault-simulate",
            "--case",
            "ocr-crash",
            "--scratch-dir",
            path_str(&scratch_dir),
            "--ocr-command",
            path_str(&ocr_command),
        ])
        .output()
        .expect("run ocr-crash fault simulation");

    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("fault: ocr_crash"));
    assert!(stdout.contains("status: reproduced"));
    assert!(stdout.contains("ocr command: failed"));
    assert!(stdout.contains("probe bytes: 31"));
    assert!(stdout.contains("paths: <redacted>"));
    assert!(!stdout.contains("PRIVATE_OCR_CRASH_STDOUT"));
    assert!(!stdout.contains("PRIVATE_OCR_CRASH_STDERR"));
    assert!(!stdout.contains("SYNTHETIC OCR CRASH PROBE BYTES"));
    assert!(!stdout.contains(path_str(&data_dir)));
    assert!(!stdout.contains(path_str(&scratch_dir)));
    assert!(!stdout.contains(path_str(&ocr_command)));
    assert!(scratch_dir.exists());
    assert!(fs::read_dir(&scratch_dir).unwrap().next().is_none());

    remove_dir(&scratch_dir);
    let _ = fs::remove_file(&ocr_command);
}

#[test]
fn fault_simulate_model_checksum_reproduces_mismatch_without_model_or_path_leak() {
    let data_dir = temp_path("fault-model-checksum-private-data");
    let scratch_dir = temp_path("fault-model-checksum-private-scratch");
    let model_file = temp_path("fault-model-checksum-private-model");
    let model_bytes = b"SYNTHETIC MODEL CHECKSUM PROBE\n";
    fs::write(&model_file, model_bytes).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "fault-simulate",
            "--case",
            "model-checksum",
            "--scratch-dir",
            path_str(&scratch_dir),
            "--model-file",
            path_str(&model_file),
            "--expected-sha256",
            "0000000000000000000000000000000000000000000000000000000000000000",
        ])
        .output()
        .expect("run model-checksum fault simulation");

    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("fault: model_checksum"));
    assert!(stdout.contains("status: reproduced"));
    assert!(stdout.contains("checksum match: no"));
    assert!(stdout.contains("expected sha256 prefix: 00000000"));
    assert!(stdout.contains("actual sha256 prefix: c5ef7975"));
    assert!(stdout.contains("paths: <redacted>"));
    assert!(!stdout.contains("SYNTHETIC MODEL CHECKSUM PROBE"));
    assert!(!stdout.contains(path_str(&data_dir)));
    assert!(!stdout.contains(path_str(&scratch_dir)));
    assert!(!stdout.contains(path_str(&model_file)));

    let _ = fs::remove_file(&model_file);
}

#[test]
fn fault_simulate_model_checksum_reports_match_without_path_leak() {
    let data_dir = temp_path("fault-model-checksum-ok-private-data");
    let scratch_dir = temp_path("fault-model-checksum-ok-private-scratch");
    let model_file = temp_path("fault-model-checksum-ok-private-model");
    fs::write(&model_file, b"SYNTHETIC MODEL CHECKSUM PROBE\n").unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "fault-simulate",
            "--case",
            "model-checksum",
            "--scratch-dir",
            path_str(&scratch_dir),
            "--model-file",
            path_str(&model_file),
            "--expected-sha256",
            "c5ef7975f1916e5f519ffa62ab13dbcbe6f1f3fc7ebe64defc4e592ba743a1b3",
        ])
        .output()
        .expect("run model-checksum match simulation");

    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("fault: model_checksum"));
    assert!(stdout.contains("status: not reproduced"));
    assert!(stdout.contains("checksum match: yes"));
    assert!(stdout.contains("paths: <redacted>"));
    assert!(!stdout.contains(path_str(&data_dir)));
    assert!(!stdout.contains(path_str(&scratch_dir)));
    assert!(!stdout.contains(path_str(&model_file)));

    let _ = fs::remove_file(&model_file);
}

#[test]
fn fault_simulate_metadata_migration_failure_reproduces_without_path_or_schema_leak() {
    let data_dir = temp_path("fault-migration-private-data");
    let scratch_dir = temp_path("fault-migration-private-scratch");

    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "fault-simulate",
            "--case",
            "migration-failure",
            "--scratch-dir",
            path_str(&scratch_dir),
        ])
        .output()
        .expect("run migration-failure fault simulation");

    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("fault: metadata_migration"));
    assert!(stdout.contains("status: reproduced"));
    assert!(stdout.contains("migration check: failed"));
    assert!(stdout.contains("recovery: restore metadata backup before retrying migration"));
    assert!(stdout.contains("paths: <redacted>"));
    assert!(!stdout.contains(path_str(&data_dir)));
    assert!(!stdout.contains(path_str(&scratch_dir)));
    assert!(!stdout.contains("schema_migrations"));
    assert!(!stdout.contains("CREATE TABLE"));
    assert!(!data_dir.exists());
    assert!(scratch_dir.exists());
    assert!(fs::read_dir(&scratch_dir).unwrap().next().is_none());

    remove_dir(&scratch_dir);
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

#[cfg(unix)]
fn ocr_crash_fixture_script(label: &str) -> PathBuf {
    let path = temp_path(label);
    fs::write(
        &path,
        r#"#!/bin/sh
printf 'PRIVATE_OCR_CRASH_STDOUT\n'
printf 'PRIVATE_OCR_CRASH_STDERR\n' >&2
exit 17
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

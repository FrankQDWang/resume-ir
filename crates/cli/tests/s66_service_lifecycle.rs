use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn service_install_writes_launch_agent_plist_without_cli_path_leaks() {
    let data_dir = temp_path("service-private-data");
    let launch_agent_dir = temp_path("service-private-launch-agents");
    let daemon_dir = temp_dir("daemon & private bin");
    let daemon_binary = daemon_dir.join("resume-daemon");
    let ocr_command = daemon_dir.join("ocr-worker");
    fs::write(&daemon_binary, "#!/bin/sh\n").unwrap();
    fs::write(&ocr_command, "#!/bin/sh\n").unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "service",
            "install",
            "--launch-agent-dir",
            path_str(&launch_agent_dir),
            "--daemon-binary",
            path_str(&daemon_binary),
            "--ocr-command",
            path_str(&ocr_command),
            "--ocr-max-pages-per-document",
            "7",
        ])
        .output()
        .expect("run service install");

    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("service: installed"));
    assert!(stdout.contains("label: com.resume-ir.daemon"));
    assert!(stdout.contains("platform: macos-launch-agent"));
    assert!(!stdout.contains(path_str(&data_dir)));
    assert!(!stdout.contains(path_str(&launch_agent_dir)));
    assert!(!stdout.contains(path_str(&daemon_binary)));
    assert!(!stdout.contains(path_str(&ocr_command)));

    let plist_path = launch_agent_dir.join("com.resume-ir.daemon.plist");
    let plist = fs::read_to_string(&plist_path).expect("read launch agent plist");
    assert!(plist.contains("<key>Label</key>"));
    assert!(plist.contains("<string>com.resume-ir.daemon</string>"));
    assert!(plist.contains("<key>ProgramArguments</key>"));
    assert!(plist.contains(path_str(&data_dir)));
    assert!(plist.contains("daemon &amp; private bin"));
    assert!(plist.contains("--work-imports"));
    assert!(plist.contains("--work-index"));
    assert!(plist.contains("--ipc-listen"));
    assert!(plist.contains("127.0.0.1:0"));
    assert!(plist.contains("--work-ocr"));
    assert!(plist.contains("--ocr-command"));
    assert!(plist.contains(&path_str(&ocr_command).replace('&', "&amp;")));
    assert!(plist.contains("--ocr-max-pages-per-document"));
    assert!(plist.contains("<string>7</string>"));
    assert!(plist.contains("<key>RunAtLoad</key>"));
    assert!(plist.contains("<key>KeepAlive</key>"));
    assert!(data_dir.join("logs").exists());

    remove_dir(&data_dir);
    remove_dir(&launch_agent_dir);
    remove_dir(&daemon_dir);
}

#[test]
fn service_status_and_uninstall_are_redacted_and_preserve_user_data() {
    let data_dir = temp_dir("service-status-private-data");
    let launch_agent_dir = temp_path("service-status-private-launch-agents");
    let daemon_dir = temp_dir("service-status-private-bin");
    let daemon_binary = daemon_dir.join("resume-daemon");
    fs::write(&daemon_binary, "#!/bin/sh\n").unwrap();

    let install = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "service",
            "install",
            "--launch-agent-dir",
            path_str(&launch_agent_dir),
            "--daemon-binary",
            path_str(&daemon_binary),
        ])
        .output()
        .expect("run service install");
    assert!(install.status.success());

    let status = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "service",
            "status",
            "--launch-agent-dir",
            path_str(&launch_agent_dir),
        ])
        .output()
        .expect("run service status");
    assert!(status.status.success());
    assert!(status.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&status.stdout);
    assert!(stdout.contains("service: installed"));
    assert!(stdout.contains("label: com.resume-ir.daemon"));
    assert!(stdout.contains("runtime: "));
    assert!(!stdout.contains(path_str(&data_dir)));
    assert!(!stdout.contains(path_str(&launch_agent_dir)));

    let uninstall = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "service",
            "uninstall",
            "--launch-agent-dir",
            path_str(&launch_agent_dir),
        ])
        .output()
        .expect("run service uninstall");
    assert!(uninstall.status.success());
    assert!(uninstall.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&uninstall.stdout);
    assert!(stdout.contains("service: uninstalled"));
    assert!(!stdout.contains(path_str(&data_dir)));
    assert!(!stdout.contains(path_str(&launch_agent_dir)));
    assert!(!launch_agent_dir.join("com.resume-ir.daemon.plist").exists());
    assert!(data_dir.exists(), "uninstall must not delete user data");

    let status_after = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "service",
            "status",
            "--launch-agent-dir",
            path_str(&launch_agent_dir),
        ])
        .output()
        .expect("run service status after uninstall");
    assert!(status_after.status.success());
    let stdout = String::from_utf8_lossy(&status_after.stdout);
    assert!(stdout.contains("service: not installed"));
    assert!(stdout.contains("runtime: not_loaded"));
    assert!(!stdout.contains(path_str(&data_dir)));
    assert!(!stdout.contains(path_str(&launch_agent_dir)));

    remove_dir(&data_dir);
    remove_dir(&launch_agent_dir);
    remove_dir(&daemon_dir);
}

#[test]
fn service_start_and_stop_dry_run_do_not_load_or_leak_local_paths() {
    let data_dir = temp_path("service-dry-run-private-data");
    let launch_agent_dir = temp_dir("service-dry-run-private-launch-agents");
    let plist_path = launch_agent_dir.join("com.resume-ir.daemon.plist");
    fs::write(&plist_path, "<plist version=\"1.0\"></plist>").unwrap();

    let start = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "service",
            "start",
            "--launch-agent-dir",
            path_str(&launch_agent_dir),
            "--dry-run",
        ])
        .output()
        .expect("run service start dry-run");
    assert!(start.status.success());
    assert!(start.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&start.stdout);
    assert!(stdout.contains("service: start dry-run"));
    assert!(stdout.contains("launchctl bootstrap"));
    assert!(stdout.contains("launchctl kickstart"));
    assert!(!stdout.contains(path_str(&data_dir)));
    assert!(!stdout.contains(path_str(&launch_agent_dir)));

    let stop = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "service",
            "stop",
            "--launch-agent-dir",
            path_str(&launch_agent_dir),
            "--dry-run",
        ])
        .output()
        .expect("run service stop dry-run");
    assert!(stop.status.success());
    assert!(stop.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&stop.stdout);
    assert!(stdout.contains("service: stop dry-run"));
    assert!(stdout.contains("launchctl bootout"));
    assert!(!stdout.contains(path_str(&data_dir)));
    assert!(!stdout.contains(path_str(&launch_agent_dir)));

    remove_dir(&data_dir);
    remove_dir(&launch_agent_dir);
}

#[test]
fn service_rejects_invalid_label_without_path_leak() {
    let data_dir = temp_path("service-invalid-label-private-data");
    let launch_agent_dir = temp_path("service-invalid-label-private-launch-agents");
    let daemon_binary = temp_path("service-invalid-label-private-daemon");
    fs::write(&daemon_binary, "#!/bin/sh\n").unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "service",
            "install",
            "--launch-agent-dir",
            path_str(&launch_agent_dir),
            "--daemon-binary",
            path_str(&daemon_binary),
            "--label",
            "bad/label",
        ])
        .output()
        .expect("run service install with invalid label");

    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("resume-cli service"));
    assert!(!stderr.contains(path_str(&data_dir)));
    assert!(!stderr.contains(path_str(&launch_agent_dir)));
    assert!(!stderr.contains(path_str(&daemon_binary)));

    remove_file(&daemon_binary);
}

fn temp_path(label: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!("resume-ir-s66-cli-{label}-{unique}"))
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

fn remove_file(path: &Path) {
    let _ = fs::remove_file(path);
}

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use meta_store::MetaStore;

#[test]
fn foreground_daemon_can_be_killed_and_restarted_without_path_leak() {
    let data_dir = temp_dir("daemon-kill-restart-data");

    let mut child = Command::new(env!("CARGO_BIN_EXE_resume-daemon"))
        .args(["--data-dir", path_str(&data_dir), "run", "--foreground"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start resume-daemon foreground");

    wait_until_metadata_store_ready(&mut child, &data_dir);
    child.kill().expect("kill foreground daemon");
    let killed = wait_child(child);
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

fn wait_until_metadata_store_ready(child: &mut Child, data_dir: &Path) {
    let metadata_store = data_dir.join("metadata.sqlite3");
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if metadata_store.exists()
            && MetaStore::open_data_dir(data_dir)
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

struct ChildOutput {
    success: bool,
    stdout: String,
    stderr: String,
}

fn wait_child(mut child: Child) -> ChildOutput {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(status) = child.try_wait().expect("poll daemon child") {
            let mut stdout = String::new();
            child
                .stdout
                .take()
                .expect("daemon stdout")
                .read_to_string(&mut stdout)
                .expect("read daemon stdout");
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

#![cfg(target_os = "macos")]

use std::fs;
use std::io::Read;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use index_fulltext::{publish_snapshot, FullTextError, IndexDocument, IndexSection};

const HOLDER_READY: &[u8] = b"resume-ir.installed-main-publication-lock.ready.v1\n";

#[test]
fn installed_acceptance_holder_contends_with_production_publication_lock() {
    let root = private_temp_dir("installed-acceptance-flock");
    publish_snapshot(&root, "generation-ready", [document("ready")]).unwrap();

    let mut holder = start_holder(&root.join("snapshot-publication.lock"));
    let contested = publish_snapshot(&root, "generation-contested", [document("contested")]);
    assert_eq!(contested.unwrap_err(), FullTextError::PublicationBusy);
    assert!(!root.join("snapshots/generation-contested").exists());

    stop_holder(&mut holder);
    publish_snapshot(&root, "generation-after-release", [document("released")]).unwrap();
    fs::remove_dir_all(root).unwrap();
}

fn start_holder(lock_path: &Path) -> Child {
    let helper = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../apps/desktop/scripts/macos-installed-main-acceptance/flock-holder.rb")
        .canonicalize()
        .unwrap();
    let mut child = Command::new("/usr/bin/ruby")
        .arg(helper)
        .arg(lock_path)
        .current_dir("/")
        .env_clear()
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();

    let mut stdout = child.stdout.take().unwrap();
    let (ready_tx, ready_rx) = mpsc::channel();
    let reader = thread::spawn(move || {
        let mut ready = vec![0; HOLDER_READY.len()];
        let result = stdout.read_exact(&mut ready).map(|_| ready);
        let _ = ready_tx.send(result);
    });
    let ready = ready_rx
        .recv_timeout(Duration::from_secs(2))
        .expect("flock holder readiness timed out")
        .expect("flock holder readiness read failed");
    reader.join().unwrap();
    assert_eq!(ready, HOLDER_READY);
    child
}

fn stop_holder(child: &mut Child) {
    drop(child.stdin.take());
    let deadline = Instant::now() + Duration::from_secs(2);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if Instant::now() >= deadline {
            child.kill().unwrap();
            let _ = child.wait();
            panic!("flock holder release timed out");
        }
        thread::sleep(Duration::from_millis(10));
    };
    let mut stderr = Vec::new();
    child
        .stderr
        .take()
        .unwrap()
        .read_to_end(&mut stderr)
        .unwrap();
    assert!(status.success(), "flock holder failed");
    assert!(stderr.is_empty(), "flock holder wrote stderr");
}

fn document(label: &str) -> IndexDocument {
    IndexDocument {
        doc_id: stable_id("doc_", label),
        resume_version_id: stable_id("ver_", label),
        file_name: format!("{label}.txt"),
        clean_text: format!("synthetic {label}"),
        sections: vec![IndexSection {
            section_type: "summary".to_string(),
            text: format!("synthetic {label}"),
        }],
    }
}

fn stable_id(prefix: &str, seed: &str) -> String {
    let mut left = 0xcbf2_9ce4_8422_2325_u64;
    let mut right = 0x6c62_272e_07bb_0142_u64;
    for byte in seed.bytes() {
        left = (left ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3);
        right = (right ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{prefix}{left:016x}{right:016x}")
}

fn private_temp_dir(label: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root =
        std::env::temp_dir().join(format!("resume-ir-{label}-{}-{nonce}", std::process::id()));
    fs::create_dir(&root).unwrap();
    fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
    root.canonicalize().unwrap()
}

//! Real subprocess recovery smoke tests for the local daemon.

use meta_store::MetadataStore;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[test]
fn killed_daemon_subprocess_recovers_expired_import_claim_on_next_run() -> Result<(), String> {
    let sandbox = unique_test_dir("daemon-subprocess-recovery")?;
    let data_dir = sandbox.join("data");
    let import_root = sandbox.join("private-root");
    fs::create_dir_all(&import_root).map_err(|error| error.to_string())?;
    fs::write(
        import_root.join("private-searchable.pdf"),
        text_layer_pdf_bytes_with("Subprocess recovery synthetic Java engineer"),
    )
    .map_err(|error| error.to_string())?;
    fs::write(import_root.join("private-scan.pdf"), image_only_pdf_bytes())
        .map_err(|error| error.to_string())?;

    let store = open_test_store(&data_dir)?;
    store
        .enqueue_import_root(&import_root)
        .map_err(|error| error.user_message().to_string())?;
    drop(store);

    let pause_file = sandbox.join("claim-observed");
    let child = Command::new(env!("CARGO_BIN_EXE_resume-daemon"))
        .args([
            "--data-dir",
            data_dir
                .to_str()
                .ok_or_else(|| "test data dir is not valid UTF-8".to_string())?,
            "--foreground",
            "--once",
        ])
        .env("RESUME_DAEMON_DEBUG_IMPORT_LEASE_MS", "200")
        .env("RESUME_DAEMON_DEBUG_PAUSE_AFTER_CLAIM_FILE", &pause_file)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| error.to_string())?;
    let mut child_guard = ChildGuard::new(child);

    wait_for_file(&pause_file, Duration::from_secs(5))?;
    let killed_output = child_guard.kill_and_wait()?;
    assert!(!killed_output.status.success());
    let killed_text = combined_output_text(&killed_output.stdout, &killed_output.stderr)?;
    assert_redacted(&killed_text, &import_root);
    assert!(!killed_text.contains("resume-daemon-local-import"));

    thread::sleep(Duration::from_millis(250));

    let recovered = Command::new(env!("CARGO_BIN_EXE_resume-daemon"))
        .args([
            "--data-dir",
            data_dir
                .to_str()
                .ok_or_else(|| "test data dir is not valid UTF-8".to_string())?,
            "--foreground",
            "--once",
        ])
        .env("RESUME_DAEMON_DEBUG_IMPORT_LEASE_MS", "5000")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|error| error.to_string())?;

    assert!(recovered.status.success());
    let recovered_text = combined_output_text(&recovered.stdout, &recovered.stderr)?;
    assert!(recovered_text.contains("claimed imports: 1"));
    assert!(recovered_text.contains("failed imports: 0"));
    assert!(recovered_text.contains("discovered documents: 2"));
    assert!(recovered_text.contains("searchable documents: 1"));
    assert!(recovered_text.contains("ocr required documents: 1"));
    assert!(recovered_text.contains("skipped documents: 0"));
    assert_redacted(&recovered_text, &import_root);
    assert!(!recovered_text.contains("resume-daemon-local-import"));

    let reopened = open_test_store(&data_dir)?;
    let status = reopened
        .status()
        .map_err(|error| error.user_message().to_string())?;
    assert_eq!(status.queued_import_task_count, 0);
    assert_eq!(status.searchable_document_count, 1);
    assert_eq!(status.ocr_required_document_count, 1);

    fs::remove_dir_all(&sandbox).map_err(|error| error.to_string())?;
    Ok(())
}

fn open_test_store(data_dir: &Path) -> Result<MetadataStore, String> {
    fs::create_dir_all(data_dir).map_err(|error| error.to_string())?;
    let store = MetadataStore::open(data_dir.join("metadata.sqlite"))
        .map_err(|error| error.user_message().to_string())?;
    store
        .run_migrations()
        .map_err(|error| error.user_message().to_string())?;
    Ok(store)
}

fn wait_for_file(path: &Path, timeout: Duration) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if path.is_file() {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(25));
    }
    Err("daemon subprocess did not report a claimed import before timeout".to_string())
}

fn combined_output_text(stdout: &[u8], stderr: &[u8]) -> Result<String, String> {
    let mut bytes = Vec::with_capacity(stdout.len() + stderr.len());
    bytes.extend_from_slice(stdout);
    bytes.extend_from_slice(stderr);
    String::from_utf8(bytes).map_err(|error| error.to_string())
}

struct ChildGuard {
    child: Option<Child>,
}

impl ChildGuard {
    fn new(child: Child) -> Self {
        Self { child: Some(child) }
    }

    fn kill_and_wait(&mut self) -> Result<Output, String> {
        let mut child = self
            .child
            .take()
            .ok_or_else(|| "daemon subprocess was already consumed".to_string())?;
        child.kill().map_err(|error| error.to_string())?;
        child.wait_with_output().map_err(|error| error.to_string())
    }
}

impl Drop for ChildGuard {
    fn drop(&mut self) {
        if let Some(child) = self.child.as_mut() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

fn assert_redacted(text: &str, import_root: &Path) {
    assert!(!text.contains(import_root.to_string_lossy().as_ref()));
    assert!(!text.contains("private-root"));
    assert!(!text.contains("private-searchable.pdf"));
    assert!(!text.contains("private-scan.pdf"));
    assert!(!text.contains("Subprocess recovery synthetic Java engineer"));
    assert!(!text.contains("import task lease expired"));
    assert!(!text.contains("local import worker failed"));
}

fn unique_test_dir(label: &str) -> Result<PathBuf, String> {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| error.to_string())?
        .as_nanos();
    let path =
        std::env::temp_dir().join(format!("resume-ir-{label}-{}-{stamp}", std::process::id()));
    fs::create_dir_all(&path).map_err(|error| error.to_string())?;
    Ok(path)
}

fn text_layer_pdf_bytes_with(text: &str) -> Vec<u8> {
    format!(
        "%PDF-1.4
1 0 obj
<< /Type /Page /Contents 2 0 R /Resources << /Font << /F1 3 0 R >> >> >>
endobj
2 0 obj
<< /Length 90 >>
stream
BT
/F1 12 Tf
72 720 Td
({text}) Tj
ET
endstream
endobj
3 0 obj
<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>
endobj
%%EOF"
    )
    .into_bytes()
}

fn image_only_pdf_bytes() -> Vec<u8> {
    b"%PDF-1.4
1 0 obj
<< /Type /Page /Resources << /XObject << /Im1 2 0 R >> >> /Contents 3 0 R >>
endobj
2 0 obj
<< /Type /XObject /Subtype /Image /Width 10 /Height 10 /ColorSpace /DeviceGray /BitsPerComponent 8 /Length 4 >>
stream
0000
endstream
endobj
3 0 obj
<< /Length 24 >>
stream
q 10 0 0 10 0 0 cm /Im1 Do Q
endstream
endobj
%%EOF"
        .to_vec()
}

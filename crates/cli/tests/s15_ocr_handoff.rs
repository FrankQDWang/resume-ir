use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use meta_store::{
    DocumentStatus, IngestJobKind, IngestJobStatus, MetaStore, OcrPageCacheKey, OcrPageCacheStatus,
    UnixTimestamp,
};

#[test]
fn import_scanned_pdf_creates_durable_ocr_document_job_without_searchable_text() {
    let data_dir = temp_dir("ocr-handoff-data");
    let fixture_root = fixture_root();

    let import = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "import",
            "--root",
            path_str(&fixture_root),
        ])
        .output()
        .expect("run resume-cli import");

    assert!(
        import.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&import.stdout),
        String::from_utf8_lossy(&import.stderr)
    );
    assert!(import.stderr.is_empty());
    let import_stdout = String::from_utf8_lossy(&import.stdout);
    assert!(import_stdout.contains("ocr required documents: 1"));
    assert!(import_stdout.contains("ocr jobs queued: 1"));
    assert!(!import_stdout.contains(path_str(&fixture_root)));

    let store = MetaStore::open(data_dir.join("metadata.sqlite3")).unwrap();
    store.run_migrations().unwrap();
    let scanned = store
        .visible_documents()
        .unwrap()
        .into_iter()
        .find(|document| document.file_name == "synthetic-scanned-resume.pdf")
        .expect("scanned synthetic fixture is persisted");
    assert_eq!(scanned.status, DocumentStatus::OcrRequired);

    let retryable = store.retryable_jobs().unwrap();
    assert_eq!(retryable.len(), 1);
    assert_eq!(retryable[0].kind, IngestJobKind::OcrDocument);
    assert_eq!(retryable[0].status, IngestJobStatus::Queued);

    let claimed = store
        .claim_next_job_by_kind(
            IngestJobKind::OcrDocument,
            UnixTimestamp::from_unix_seconds(1_900_000_000),
        )
        .unwrap()
        .expect("ocr document job can be claimed after restart");
    assert_eq!(claimed.kind, IngestJobKind::OcrDocument);
    assert_eq!(claimed.status, IngestJobStatus::Running);

    let search = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "search",
            "scanned",
            "--top-k",
            "20",
        ])
        .output()
        .expect("run resume-cli search for scanned text");
    assert!(search.status.success());
    let search_stdout = String::from_utf8_lossy(&search.stdout);
    assert!(search_stdout.contains("results: 0"));
    assert!(!search_stdout.contains("synthetic-scanned-resume.pdf"));

    remove_dir(&data_dir);
}

#[test]
fn repeated_import_does_not_duplicate_existing_ocr_document_jobs() {
    let data_dir = temp_dir("ocr-handoff-idempotent-data");
    let fixture_root = fixture_root();

    import_fixtures(&data_dir, &fixture_root);
    import_fixtures(&data_dir, &fixture_root);

    let store = MetaStore::open(data_dir.join("metadata.sqlite3")).unwrap();
    store.run_migrations().unwrap();
    let scanned = store
        .visible_documents()
        .unwrap()
        .into_iter()
        .find(|document| document.file_name == "synthetic-scanned-resume.pdf")
        .expect("scanned synthetic fixture is persisted");
    assert_eq!(
        store
            .retryable_jobs()
            .unwrap()
            .into_iter()
            .filter(|job| job.kind == IngestJobKind::OcrDocument && job.document_id == scanned.id)
            .count(),
        1
    );

    remove_dir(&data_dir);
}

#[test]
fn ocr_worker_without_command_reports_blocked_and_leaves_job_queued() {
    let data_dir = temp_dir("ocr-worker-no-command-data");
    let fixture_root = fixture_root();
    import_fixtures(&data_dir, &fixture_root);

    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args(["--data-dir", path_str(&data_dir), "ocr-worker", "--once"])
        .output()
        .expect("run ocr worker without command");

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).is_empty());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("ocr worker blocked: local OCR command not configured"));
    assert!(!stderr.contains(path_str(&data_dir)));
    assert!(!stderr.contains(path_str(&fixture_root)));

    let store = MetaStore::open(data_dir.join("metadata.sqlite3")).unwrap();
    store.run_migrations().unwrap();
    let retryable = store.retryable_jobs().unwrap();
    assert_eq!(retryable.len(), 1);
    assert_eq!(retryable[0].status, IngestJobStatus::Queued);

    remove_dir(&data_dir);
}

#[cfg(unix)]
#[test]
fn pause_and_resume_ocr_task_persistently_controls_worker_claims() {
    let data_dir = temp_dir("ocr-worker-pause-resume-data");
    let fixture_root = fixture_root();
    let command = write_fixture_executable(
        "fixture-ocr-worker-paused",
        r#"#!/bin/sh
printf 'resume-ir-ocr-v1\n'
printf 'confidence=0.81\n'
printf 'text:\n'
printf 'OCRS33PauseResumeToken worker text\n'
"#,
    );
    import_fixtures(&data_dir, &fixture_root);

    let pause = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args(["--data-dir", path_str(&data_dir), "pause", "--task", "ocr"])
        .output()
        .expect("pause ocr task");
    assert!(pause.status.success());
    assert!(pause.stderr.is_empty());
    let pause_stdout = String::from_utf8_lossy(&pause.stdout);
    assert!(pause_stdout.contains("task: ocr"));
    assert!(pause_stdout.contains("status: paused"));
    assert!(!pause_stdout.contains(path_str(&data_dir)));

    let paused_worker = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "ocr-worker",
            "--once",
            "--command",
            path_str(&command),
        ])
        .output()
        .expect("run paused ocr worker");
    assert!(paused_worker.status.success());
    assert!(paused_worker.stderr.is_empty());
    let paused_stdout = String::from_utf8_lossy(&paused_worker.stdout);
    assert!(paused_stdout.contains("ocr worker: paused"));
    assert!(paused_stdout.contains("documents processed: 0"));
    assert!(paused_stdout.contains("cache writes: 0"));
    assert!(!paused_stdout.contains("OCRS33PauseResumeToken"));
    assert!(!paused_stdout.contains(path_str(&data_dir)));
    assert!(!paused_stdout.contains(path_str(&fixture_root)));

    {
        let store = MetaStore::open(data_dir.join("metadata.sqlite3")).unwrap();
        store.run_migrations().unwrap();
        assert_eq!(scanned_document(&store).status, DocumentStatus::OcrRequired);
        let retryable = store.retryable_jobs().unwrap();
        assert_eq!(retryable.len(), 1);
        assert_eq!(retryable[0].status, IngestJobStatus::Queued);
    }

    let resume = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args(["--data-dir", path_str(&data_dir), "resume", "--task", "ocr"])
        .output()
        .expect("resume ocr task");
    assert!(resume.status.success());
    assert!(resume.stderr.is_empty());
    let resume_stdout = String::from_utf8_lossy(&resume.stdout);
    assert!(resume_stdout.contains("task: ocr"));
    assert!(resume_stdout.contains("status: running"));
    assert!(!resume_stdout.contains(path_str(&data_dir)));

    let resumed_worker = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "ocr-worker",
            "--once",
            "--command",
            path_str(&command),
        ])
        .output()
        .expect("run resumed ocr worker");
    assert!(
        resumed_worker.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&resumed_worker.stdout),
        String::from_utf8_lossy(&resumed_worker.stderr)
    );
    let resumed_stdout = String::from_utf8_lossy(&resumed_worker.stdout);
    assert!(resumed_stdout.contains("ocr worker: completed"));
    assert!(resumed_stdout.contains("documents processed: 1"));
    assert!(!resumed_stdout.contains("OCRS33PauseResumeToken"));

    let store = MetaStore::open(data_dir.join("metadata.sqlite3")).unwrap();
    store.run_migrations().unwrap();
    assert_eq!(scanned_document(&store).status, DocumentStatus::OcrDone);
    assert!(store.retryable_jobs().unwrap().is_empty());

    remove_dir(&data_dir);
}

#[cfg(unix)]
#[test]
fn ocr_worker_executes_local_command_and_persists_page_cache_without_searchable_text() {
    let data_dir = temp_dir("ocr-worker-command-data");
    let fixture_root = fixture_root();
    let command = write_fixture_executable(
        "fixture-ocr-worker",
        r#"#!/bin/sh
input_size="$(wc -c < "$RESUME_IR_OCR_INPUT_PATH" | tr -d ' ')"
printf 'resume-ir-ocr-v1\n'
printf 'confidence=0.73\n'
printf 'text:\n'
printf 'OCRS31UniqueToken worker text bytes=%s page=%s\n' "$input_size" "$RESUME_IR_OCR_PAGE_NO"
"#,
    );
    import_fixtures(&data_dir, &fixture_root);

    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "ocr-worker",
            "--once",
            "--command",
            path_str(&command),
            "--engine-profile",
            "fixture-engine",
        ])
        .output()
        .expect("run ocr worker with local command");

    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("ocr worker: completed"));
    assert!(stdout.contains("documents processed: 1"));
    assert!(stdout.contains("cache writes: 1"));
    assert!(!stdout.contains("OCRS31UniqueToken"));
    assert!(!stdout.contains(path_str(&data_dir)));
    assert!(!stdout.contains(path_str(&fixture_root)));

    let store = MetaStore::open(data_dir.join("metadata.sqlite3")).unwrap();
    store.run_migrations().unwrap();
    let scanned = scanned_document(&store);
    assert_eq!(scanned.status, DocumentStatus::OcrDone);
    assert!(store.retryable_jobs().unwrap().is_empty());
    let cache_key = OcrPageCacheKey::new(
        scanned.content_hash.expect("content hash"),
        1,
        300,
        "eng",
        "balanced",
    )
    .unwrap();
    let cache_entry = store
        .ocr_page_cache_entry(&cache_key)
        .unwrap()
        .expect("OCR cache entry");
    assert_eq!(cache_entry.status(), OcrPageCacheStatus::Succeeded);
    assert_eq!(cache_entry.confidence(), Some(0.73));
    assert_eq!(cache_entry.engine_profile(), Some("fixture-engine"));
    assert!(cache_entry.text().unwrap().contains("OCRS31UniqueToken"));

    let search = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "search",
            "OCRS31UniqueToken",
            "--top-k",
            "20",
        ])
        .output()
        .expect("run resume-cli search for cached OCR text");
    assert!(search.status.success());
    let search_stdout = String::from_utf8_lossy(&search.stdout);
    assert!(
        search_stdout.contains("results: 0"),
        "stdout:\n{search_stdout}"
    );

    remove_dir(&data_dir);
}

fn import_fixtures(data_dir: &Path, fixture_root: &Path) {
    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(data_dir),
            "import",
            "--root",
            path_str(fixture_root),
        ])
        .output()
        .expect("import fixtures");
    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn scanned_document(store: &MetaStore) -> meta_store::Document {
    store
        .visible_documents()
        .unwrap()
        .into_iter()
        .find(|document| document.file_name == "synthetic-scanned-resume.pdf")
        .expect("scanned synthetic fixture is persisted")
}

fn fixture_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("tests/fixtures/resumes")
}

fn temp_dir(label: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!("resume-ir-s15-cli-{label}-{unique}"));
    std::fs::create_dir_all(&path).unwrap();
    path
}

fn path_str(path: &Path) -> &str {
    path.to_str().unwrap()
}

fn remove_dir(path: &Path) {
    let _ = std::fs::remove_dir_all(path);
}

#[cfg(unix)]
fn write_fixture_executable(name: &str, body: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;

    let directory = temp_dir("ocr-worker-command-bin");
    let path = directory.join(name);
    std::fs::write(&path, body).unwrap();
    let mut permissions = std::fs::metadata(&path).unwrap().permissions();
    permissions.set_mode(0o700);
    std::fs::set_permissions(&path, permissions).unwrap();
    path
}

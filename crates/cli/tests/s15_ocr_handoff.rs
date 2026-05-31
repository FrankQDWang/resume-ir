use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use meta_store::{DocumentStatus, IngestJobKind, IngestJobStatus, MetaStore, UnixTimestamp};

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

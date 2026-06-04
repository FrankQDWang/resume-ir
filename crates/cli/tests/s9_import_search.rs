use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

#[cfg(windows)]
use std::sync::{Mutex, MutexGuard, OnceLock};

use meta_store::{
    EntityType, ImportRootKind, ImportRootPreset, ImportScanBudgetKind, ImportScanProfile,
    ImportTask, ImportTaskId, ImportTaskStatus, MetaStore, UnixTimestamp,
};
#[cfg(unix)]
use meta_store::{ImportScanErrorKind, ImportScanErrorOperation};

const LOCAL_DISCOVERY_ROOTS_ENV: &str = "RESUME_IR_LOCAL_DISCOVERY_ROOTS";

macro_rules! serialize_windows_s9_import_test {
    () => {
        #[cfg(windows)]
        let _guard = windows_s9_import_test_lock();
    };
}

#[cfg(windows)]
fn windows_s9_import_test_lock() -> MutexGuard<'static, ()> {
    static WINDOWS_S9_IMPORT_TEST_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    WINDOWS_S9_IMPORT_TEST_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[test]
fn import_fixtures_builds_searchable_index_and_reopens_snapshot() {
    serialize_windows_s9_import_test!();
    let data_dir = temp_dir("import-search-data");
    let fixture_root = fixture_root();
    let canonical_fixture_root = fs::canonicalize(&fixture_root).unwrap();

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
    assert!(import_stdout.contains("import task submitted"));
    assert!(import_stdout.contains("task id: imp_"));
    assert!(import_stdout.contains("status: completed"));
    assert!(import_stdout.contains("files discovered: 3"));
    assert!(import_stdout.contains("searchable documents: 2"));
    assert!(import_stdout.contains("ocr required documents: 1"));
    assert!(!import_stdout.contains(path_str(&fixture_root)));
    assert!(!import_stdout.contains(path_str(&canonical_fixture_root)));

    let status = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args(["--data-dir", path_str(&data_dir), "status"])
        .output()
        .expect("run resume-cli status");
    assert!(status.status.success());
    assert!(status.stderr.is_empty());
    let status_stdout = String::from_utf8_lossy(&status.stdout);
    assert!(status_stdout.contains("searchable documents: 2"));
    assert!(status_stdout.contains("ocr queue: 1"));
    assert!(status_stdout.contains("import tasks queued: 0"));
    assert!(status_stdout.contains("index health: ready"));
    assert!(status_stdout.contains("search index: available (full-text snapshot)"));

    let search = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args(["--data-dir", path_str(&data_dir), "search", "Java"])
        .output()
        .expect("run resume-cli search");
    assert!(search.status.success());
    assert!(search.stderr.is_empty());
    let search_stdout = String::from_utf8_lossy(&search.stdout);
    assert!(search_stdout.contains("results: 2"));
    assert!(search_stdout.contains("rank: 1"));
    assert!(search_stdout.contains("synthetic-java-platform.pdf"));
    assert!(search_stdout.contains("synthetic-java-engineer.docx"));
    assert!(!search_stdout.contains("query:"));

    let reopened_search = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args(["--data-dir", path_str(&data_dir), "search", "Java"])
        .output()
        .expect("run resume-cli search after reopen");
    assert!(reopened_search.status.success());
    let reopened_stdout = String::from_utf8_lossy(&reopened_search.stdout);
    assert!(reopened_stdout.contains("rank: 1"));

    let empty_root = temp_dir("empty-import-root");
    let empty_import = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "import",
            "--root",
            path_str(&empty_root),
        ])
        .output()
        .expect("run resume-cli import empty root");
    assert!(empty_import.status.success());
    let empty_import_stdout = String::from_utf8_lossy(&empty_import.stdout);
    assert!(empty_import_stdout.contains("files discovered: 0"));
    assert!(!empty_import_stdout.contains(path_str(&empty_root)));

    let search_after_empty_import = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args(["--data-dir", path_str(&data_dir), "search", "Java"])
        .output()
        .expect("run resume-cli search after empty import");
    assert!(search_after_empty_import.status.success());
    let search_after_empty_stdout = String::from_utf8_lossy(&search_after_empty_import.stdout);
    assert!(search_after_empty_stdout.contains("results: 2"));
    assert!(search_after_empty_stdout.contains("synthetic-java-platform.pdf"));
    assert!(search_after_empty_stdout.contains("synthetic-java-engineer.docx"));

    remove_dir(&data_dir);
    remove_dir(&empty_root);
}

#[test]
fn witness_imports_only_pdf_and_word_samples_without_persisting_private_data() {
    serialize_windows_s9_import_test!();
    let data_dir = temp_dir("witness-unused-data-dir");
    let private_root = temp_dir("witness-private-root");
    fs::copy(
        fixture_root().join("synthetic-java-platform.pdf"),
        private_root.join("real-person-platform.pdf"),
    )
    .unwrap();
    fs::copy(
        fixture_root().join("synthetic-java-engineer.docx"),
        private_root.join("real-person-engineer.docx"),
    )
    .unwrap();
    fs::write(
        private_root.join("real-person-legacy.doc"),
        b"Synthetic legacy Word resume\nSkills: Rust\n",
    )
    .unwrap();
    fs::write(
        private_root.join("real-person-not-a-resume.txt"),
        b"must not be copied by witness\n",
    )
    .unwrap();
    let canonical_private_root = fs::canonicalize(&private_root).unwrap();

    let witness = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "witness",
            "--root",
            path_str(&private_root),
            "--max-files",
            "10",
        ])
        .output()
        .expect("run resume-cli local witness");

    assert!(
        witness.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&witness.stdout),
        String::from_utf8_lossy(&witness.stderr)
    );
    assert!(witness.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&witness.stdout);
    assert!(stdout.contains("resume-ir local witness"));
    assert!(stdout.contains("source root: <redacted>"));
    assert!(stdout.contains("formats: pdf,docx,doc"));
    assert!(stdout.contains("files selected: 3"));
    assert!(stdout.contains("unsupported entries skipped: 1"));
    assert!(stdout.contains("witness import status: completed"));
    assert!(stdout.contains("private witness data: removed"));
    assert!(!stdout.contains(path_str(&data_dir)));
    assert!(!stdout.contains(path_str(&private_root)));
    assert!(!stdout.contains(path_str(&canonical_private_root)));
    assert!(!stdout.contains("real-person"));
    assert!(!stdout.contains("not-a-resume"));
    assert!(!data_dir.join("metadata.sqlite3").exists());

    remove_dir(&data_dir);
    remove_dir(&private_root);
}

#[test]
fn witness_probe_search_runs_private_query_without_leaking_query_or_paths() {
    serialize_windows_s9_import_test!();
    let data_dir = temp_dir("witness-probe-search-unused-data-dir");
    let private_root = temp_dir("witness-probe-search-private-root");
    fs::copy(
        fixture_root().join("synthetic-java-platform.pdf"),
        private_root.join("real-person-platform.pdf"),
    )
    .unwrap();
    fs::copy(
        fixture_root().join("synthetic-java-engineer.docx"),
        private_root.join("real-person-engineer.docx"),
    )
    .unwrap();
    let canonical_private_root = fs::canonicalize(&private_root).unwrap();

    let witness = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "witness",
            "--root",
            path_str(&private_root),
            "--max-files",
            "10",
            "--probe-search",
        ])
        .output()
        .expect("run resume-cli local witness with search probe");

    assert!(
        witness.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&witness.stdout),
        String::from_utf8_lossy(&witness.stderr)
    );
    assert!(witness.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&witness.stdout);
    assert!(stdout.contains("witness search status: completed"));
    assert!(stdout.contains("search probe hits: "));
    assert!(!stdout.contains("search probe hits: 0"));
    assert!(stdout.contains("private witness data: removed"));
    assert!(!stdout.contains("Java"));
    assert!(!stdout.contains(path_str(&data_dir)));
    assert!(!stdout.contains(path_str(&private_root)));
    assert!(!stdout.contains(path_str(&canonical_private_root)));
    assert!(!stdout.contains("real-person"));
    assert!(!stdout.contains("synthetic-java-platform.pdf"));
    assert!(!stdout.contains("synthetic-java-engineer.docx"));
    assert!(!data_dir.join("metadata.sqlite3").exists());

    remove_dir(&data_dir);
    remove_dir(&private_root);
}

#[test]
fn witness_probe_fields_reports_aggregate_counts_without_values_or_paths() {
    serialize_windows_s9_import_test!();
    let data_dir = temp_dir("witness-probe-fields-unused-data-dir");
    let private_root = temp_dir("witness-probe-fields-private-root");
    fs::copy(
        fixture_root().join("synthetic-java-engineer.docx"),
        private_root.join("real-person-engineer.docx"),
    )
    .unwrap();
    let canonical_private_root = fs::canonicalize(&private_root).unwrap();

    let witness = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "witness",
            "--root",
            path_str(&private_root),
            "--max-files",
            "10",
            "--probe-fields",
        ])
        .output()
        .expect("run resume-cli local witness with field probe");

    assert!(
        witness.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&witness.stdout),
        String::from_utf8_lossy(&witness.stderr)
    );
    assert!(witness.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&witness.stdout);
    assert!(stdout.contains("witness field status: completed"));
    assert!(stdout.contains("field probe documents: 1"));
    assert!(stdout.contains("field probe mentions: "));
    assert!(!stdout.contains("field probe mentions: 0"));
    assert!(stdout.contains("field probe email mentions: "));
    assert!(stdout.contains("field probe skill mentions: "));
    assert!(stdout.contains("field probe degree mentions: "));
    assert!(stdout.contains("private witness data: removed"));
    assert!(!stdout.contains("synthetic@example.test"));
    assert!(!stdout.contains("Java"));
    assert!(!stdout.contains(path_str(&data_dir)));
    assert!(!stdout.contains(path_str(&private_root)));
    assert!(!stdout.contains(path_str(&canonical_private_root)));
    assert!(!stdout.contains("real-person"));
    assert!(!stdout.contains("synthetic-java-engineer.docx"));
    assert!(!data_dir.join("metadata.sqlite3").exists());

    remove_dir(&data_dir);
    remove_dir(&private_root);
}

#[test]
fn witness_local_discovery_preset_uses_discovery_profile_without_path_leak() {
    serialize_windows_s9_import_test!();
    let data_dir = temp_dir("witness-local-discovery-unused-data-dir");
    let private_root = temp_dir("witness-local-discovery-private-root");
    fs::create_dir_all(private_root.join("Documents")).unwrap();
    fs::create_dir_all(private_root.join("node_modules")).unwrap();
    fs::copy(
        fixture_root().join("synthetic-java-platform.pdf"),
        private_root
            .join("Documents")
            .join("real-person-platform.pdf"),
    )
    .unwrap();
    fs::copy(
        fixture_root().join("synthetic-java-engineer.docx"),
        private_root
            .join("node_modules")
            .join("real-person-engineer.docx"),
    )
    .unwrap();
    let canonical_private_root = fs::canonicalize(&private_root).unwrap();

    let root_override = std::env::join_paths([&private_root]).unwrap();
    let witness = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .env(LOCAL_DISCOVERY_ROOTS_ENV, root_override)
        .args([
            "--data-dir",
            path_str(&data_dir),
            "witness",
            "--root-preset",
            "local-discovery",
            "--max-files",
            "10",
        ])
        .output()
        .expect("run resume-cli local discovery witness");

    assert!(
        witness.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&witness.stdout),
        String::from_utf8_lossy(&witness.stderr)
    );
    assert!(witness.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&witness.stdout);
    assert!(stdout.contains("resume-ir local witness"));
    assert!(stdout.contains("root preset: local-discovery"));
    assert!(stdout.contains("scan profile: discovery"));
    assert!(stdout.contains("files selected: 1"));
    assert!(stdout.contains("witness import status: completed"));
    assert!(stdout.contains("private witness data: removed"));
    assert!(!stdout.contains(path_str(&data_dir)));
    assert!(!stdout.contains(path_str(&private_root)));
    assert!(!stdout.contains(path_str(&canonical_private_root)));
    assert!(!stdout.contains("real-person"));
    assert!(!stdout.contains("node_modules"));
    assert!(!data_dir.join("metadata.sqlite3").exists());

    remove_dir(&data_dir);
    remove_dir(&private_root);
}

#[test]
fn witness_run_ocr_executes_local_command_without_output_or_path_leak() {
    serialize_windows_s9_import_test!();
    let data_dir = temp_dir("witness-ocr-unused-data-dir");
    let private_root = temp_dir("witness-ocr-private-root");
    fs::copy(
        fixture_root().join("synthetic-scanned-resume.pdf"),
        private_root.join("real-person-scanned.pdf"),
    )
    .unwrap();
    let command = write_fixture_executable(
        "fixture-witness-ocr",
        r#"#!/bin/sh
printf 'resume-ir-ocr-v1\n'
printf 'confidence=0.77\n'
printf 'text:\n'
printf 'WitnessOCRSecretToken local OCR text\n'
"#,
    );
    let canonical_private_root = fs::canonicalize(&private_root).unwrap();

    let witness = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "witness",
            "--root",
            path_str(&private_root),
            "--run-ocr",
            "--ocr-command",
            path_str(&command),
        ])
        .output()
        .expect("run resume-cli local witness with OCR");

    assert!(
        witness.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&witness.stdout),
        String::from_utf8_lossy(&witness.stderr)
    );
    assert!(witness.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&witness.stdout);
    assert!(stdout.contains("witness ocr status: completed"));
    assert!(stdout.contains("ocr documents processed: 1"));
    assert!(stdout.contains("ocr cache writes: 1"));
    assert!(!stdout.contains("WitnessOCRSecretToken"));
    assert!(!stdout.contains(path_str(&data_dir)));
    assert!(!stdout.contains(path_str(&private_root)));
    assert!(!stdout.contains(path_str(&canonical_private_root)));
    assert!(!stdout.contains(path_str(&command)));
    assert!(!stdout.contains("real-person"));
    assert!(!data_dir.join("metadata.sqlite3").exists());

    remove_dir(&data_dir);
    remove_dir(&private_root);
    remove_dir(command.parent().unwrap());
}

#[test]
fn witness_run_ocr_can_budget_documents_after_full_private_scan_without_path_leak() {
    serialize_windows_s9_import_test!();
    let data_dir = temp_dir("witness-ocr-budget-unused-data-dir");
    let private_root = temp_dir("witness-ocr-budget-private-root");
    fs::copy(
        fixture_root().join("synthetic-scanned-resume.pdf"),
        private_root.join("real-person-scanned-a.pdf"),
    )
    .unwrap();
    fs::copy(
        fixture_root().join("synthetic-scanned-resume.pdf"),
        private_root.join("real-person-scanned-b.pdf"),
    )
    .unwrap();
    let command = write_fixture_executable(
        "fixture-witness-ocr-budget",
        r#"#!/bin/sh
printf 'resume-ir-ocr-v1\n'
printf 'confidence=0.77\n'
printf 'text:\n'
printf 'WitnessOCRBudgetSecret local OCR text\n'
"#,
    );
    let canonical_private_root = fs::canonicalize(&private_root).unwrap();

    let witness = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "witness",
            "--root",
            path_str(&private_root),
            "--run-ocr",
            "--ocr-command",
            path_str(&command),
            "--ocr-max-documents",
            "1",
        ])
        .output()
        .expect("run resume-cli local witness with OCR document budget");

    assert!(
        witness.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&witness.stdout),
        String::from_utf8_lossy(&witness.stderr)
    );
    assert!(witness.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&witness.stdout);
    assert!(stdout.contains("witness ocr status: completed"));
    assert!(stdout.contains("ocr documents processed: 1"));
    assert!(stdout.contains("ocr cache writes: 1"));
    assert!(stdout.contains("ocr document budget exhausted: yes"));
    assert!(!stdout.contains("WitnessOCRBudgetSecret"));
    assert!(!stdout.contains(path_str(&data_dir)));
    assert!(!stdout.contains(path_str(&private_root)));
    assert!(!stdout.contains(path_str(&canonical_private_root)));
    assert!(!stdout.contains(path_str(&command)));
    assert!(!stdout.contains("real-person"));
    assert!(!data_dir.join("metadata.sqlite3").exists());

    remove_dir(&data_dir);
    remove_dir(&private_root);
    remove_dir(command.parent().unwrap());
}

#[test]
fn witness_run_ocr_budget_reports_failed_documents_without_stopping_or_leaking_paths() {
    serialize_windows_s9_import_test!();
    let data_dir = temp_dir("witness-ocr-partial-unused-data-dir");
    let private_root = temp_dir("witness-ocr-partial-private-root");
    fs::copy(
        fixture_root().join("synthetic-scanned-resume.pdf"),
        private_root.join("real-person-scanned-a.pdf"),
    )
    .unwrap();
    let mut second_pdf = fs::read(fixture_root().join("synthetic-scanned-resume.pdf")).unwrap();
    second_pdf.extend_from_slice(b"\n% second private fixture variant\n");
    fs::write(private_root.join("real-person-scanned-b.pdf"), second_pdf).unwrap();
    let counter_dir = temp_dir("witness-ocr-partial-counter");
    let counter_file = counter_dir.join("calls");
    let command = write_fixture_executable(
        "fixture-witness-ocr-partial",
        &format!(
            r#"#!/bin/sh
counter_file="{}"
count=0
if [ -f "$counter_file" ]; then
  count=$(cat "$counter_file")
fi
count=$((count + 1))
printf '%s' "$count" > "$counter_file"
if [ "$count" -eq 2 ]; then
  printf 'fixture OCR failure without private data\n' >&2
  exit 17
fi
printf 'resume-ir-ocr-v1\n'
printf 'confidence=0.77\n'
printf 'text:\n'
printf 'WitnessOCRPartialSecret local OCR text\n'
"#,
            path_str(&counter_file)
        ),
    );
    let canonical_private_root = fs::canonicalize(&private_root).unwrap();

    let witness = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "witness",
            "--root",
            path_str(&private_root),
            "--run-ocr",
            "--ocr-command",
            path_str(&command),
            "--ocr-max-documents",
            "2",
        ])
        .output()
        .expect("run resume-cli local witness with partial OCR failures");

    assert!(
        witness.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&witness.stdout),
        String::from_utf8_lossy(&witness.stderr)
    );
    assert!(witness.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&witness.stdout);
    assert!(stdout.contains("witness ocr status: completed"));
    assert!(stdout.contains("ocr documents processed: 1"));
    assert!(stdout.contains("ocr documents failed: 1"));
    assert!(stdout.contains("ocr cache writes: 1"));
    assert!(stdout.contains("ocr document budget exhausted: yes"));
    assert!(!stdout.contains("WitnessOCRPartialSecret"));
    assert!(!stdout.contains(path_str(&data_dir)));
    assert!(!stdout.contains(path_str(&private_root)));
    assert!(!stdout.contains(path_str(&canonical_private_root)));
    assert!(!stdout.contains(path_str(&command)));
    assert!(!stdout.contains(path_str(&counter_file)));
    assert!(!stdout.contains("real-person"));
    assert!(!data_dir.join("metadata.sqlite3").exists());

    remove_dir(&data_dir);
    remove_dir(&private_root);
    remove_dir(command.parent().unwrap());
    remove_dir(&counter_dir);
}

#[test]
fn witness_run_ocr_without_command_reports_blocked_without_persisting_private_data() {
    serialize_windows_s9_import_test!();
    let data_dir = temp_dir("witness-ocr-blocked-unused-data-dir");
    let private_root = temp_dir("witness-ocr-blocked-private-root");
    fs::copy(
        fixture_root().join("synthetic-scanned-resume.pdf"),
        private_root.join("real-person-scanned.pdf"),
    )
    .unwrap();
    let canonical_private_root = fs::canonicalize(&private_root).unwrap();

    let witness = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "witness",
            "--root",
            path_str(&private_root),
            "--run-ocr",
        ])
        .output()
        .expect("run resume-cli local witness with OCR blocked");

    assert!(
        witness.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&witness.stdout),
        String::from_utf8_lossy(&witness.stderr)
    );
    assert!(witness.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&witness.stdout);
    assert!(stdout.contains("witness ocr status: blocked"));
    assert!(stdout.contains("ocr block reason: local OCR command not configured"));
    assert!(stdout.contains("private witness data: removed"));
    assert!(!stdout.contains(path_str(&data_dir)));
    assert!(!stdout.contains(path_str(&private_root)));
    assert!(!stdout.contains(path_str(&canonical_private_root)));
    assert!(!stdout.contains("real-person"));
    assert!(!data_dir.join("metadata.sqlite3").exists());

    remove_dir(&data_dir);
    remove_dir(&private_root);
}

#[test]
fn import_txt_resume_builds_searchable_index_without_path_leakage() {
    serialize_windows_s9_import_test!();
    let data_dir = temp_dir("txt-import-data");
    let private_root = temp_dir("txt-import-private-root");
    let canonical_private_root = fs::canonicalize(&private_root).unwrap();
    fs::write(
        private_root.join("synthetic-rust-search.txt"),
        "Synthetic Candidate\nRust search infrastructure\nemail: candidate@example.test\n",
    )
    .unwrap();

    let import = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "import",
            "--root",
            path_str(&private_root),
        ])
        .output()
        .expect("run resume-cli import for txt resume");

    assert!(
        import.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&import.stdout),
        String::from_utf8_lossy(&import.stderr)
    );
    assert!(import.stderr.is_empty());
    let import_stdout = String::from_utf8_lossy(&import.stdout);
    assert!(import_stdout.contains("files discovered: 1"));
    assert!(import_stdout.contains("searchable documents: 1"));
    assert!(import_stdout.contains("failed documents: 0"));
    assert!(!import_stdout.contains(path_str(&private_root)));
    assert!(!import_stdout.contains(path_str(&canonical_private_root)));

    let search = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args(["--data-dir", path_str(&data_dir), "search", "Rust search"])
        .output()
        .expect("run resume-cli search for txt resume");
    assert!(search.status.success());
    assert!(search.stderr.is_empty());
    let search_stdout = String::from_utf8_lossy(&search.stdout);
    assert!(search_stdout.contains("results: 1"));
    assert!(search_stdout.contains("synthetic-rust-search.txt"));
    assert!(!search_stdout.contains("candidate@example.test"));
    assert!(!search_stdout.contains(path_str(&private_root)));
    assert!(!search_stdout.contains(path_str(&canonical_private_root)));

    let store = MetaStore::open_data_dir(&data_dir).unwrap();
    store.run_migrations().unwrap();
    let document = store
        .visible_documents()
        .unwrap()
        .into_iter()
        .find(|document| document.file_name == "synthetic-rust-search.txt")
        .unwrap();
    let version = store
        .latest_visible_resume_version_for_document(&document.id)
        .unwrap()
        .unwrap();
    let mentions = store.entity_mentions_for_version(&version.id).unwrap();
    let name = mentions
        .iter()
        .find(|mention| mention.entity_type == EntityType::Name)
        .unwrap();
    assert_eq!(
        name.normalized_value.as_deref(),
        Some("synthetic candidate")
    );
    assert_eq!(name.raw_value, "Synthetic Candidate");

    remove_dir(&data_dir);
    remove_dir(&private_root);
}

#[test]
fn import_blank_txt_resume_fails_without_queueing_ocr() {
    serialize_windows_s9_import_test!();
    let data_dir = temp_dir("blank-txt-import-data");
    let private_root = temp_dir("blank-txt-import-private-root");
    let canonical_private_root = fs::canonicalize(&private_root).unwrap();
    fs::write(private_root.join("synthetic-blank.txt"), " \n\t\r\n").unwrap();

    let import = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "import",
            "--root",
            path_str(&private_root),
        ])
        .output()
        .expect("run resume-cli import for blank txt resume");

    assert!(
        import.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&import.stdout),
        String::from_utf8_lossy(&import.stderr)
    );
    assert!(import.stderr.is_empty());
    let import_stdout = String::from_utf8_lossy(&import.stdout);
    assert!(import_stdout.contains("files discovered: 1"));
    assert!(import_stdout.contains("searchable documents: 0"));
    assert!(import_stdout.contains("ocr required documents: 0"));
    assert!(import_stdout.contains("ocr jobs queued: 0"));
    assert!(import_stdout.contains("failed documents: 1"));
    assert!(!import_stdout.contains(path_str(&private_root)));
    assert!(!import_stdout.contains(path_str(&canonical_private_root)));

    let status = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args(["--data-dir", path_str(&data_dir), "status"])
        .output()
        .expect("run resume-cli status after blank txt import");
    assert!(status.status.success());
    assert!(status.stderr.is_empty());
    let status_stdout = String::from_utf8_lossy(&status.stdout);
    assert!(status_stdout.contains("ocr queue: 0"));
    assert!(!status_stdout.contains(path_str(&private_root)));
    assert!(!status_stdout.contains(path_str(&canonical_private_root)));

    remove_dir(&data_dir);
    remove_dir(&private_root);
}

#[test]
fn import_enqueue_persists_task_without_running_foreground_import() {
    serialize_windows_s9_import_test!();
    let data_dir = temp_dir("enqueue-import-data");
    let fixture_root = fixture_root();
    let canonical_fixture_root = fs::canonicalize(&fixture_root).unwrap();

    let enqueue = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "import",
            "--enqueue",
            "--root",
            path_str(&fixture_root),
            "--max-files",
            "2",
        ])
        .output()
        .expect("enqueue import task");

    assert!(
        enqueue.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&enqueue.stdout),
        String::from_utf8_lossy(&enqueue.stderr)
    );
    assert!(enqueue.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&enqueue.stdout);
    assert!(stdout.contains("import task submitted"));
    assert!(stdout.contains("status: queued"));
    assert!(stdout.contains("roots queued: 1"));
    assert!(!stdout.contains("files discovered: 3"));
    assert!(!stdout.contains(path_str(&fixture_root)));
    assert!(!stdout.contains(path_str(&canonical_fixture_root)));

    let store = MetaStore::open_data_dir(&data_dir).unwrap();
    store.run_migrations().unwrap();
    let summary = store.status_summary().unwrap();
    assert_eq!(summary.import_tasks_queued, 1);
    assert_eq!(summary.searchable_documents, 0);
    let scope = store.latest_import_scan_scope().unwrap().unwrap();
    assert_eq!(scope.canonical_root_path, path_str(&canonical_fixture_root));
    assert_eq!(scope.files_discovered, 0);
    assert_eq!(scope.searchable_documents, 0);
    assert_eq!(scope.scan_budget_kind, Some(ImportScanBudgetKind::Files));
    assert_eq!(scope.scan_budget_limit, Some(2));
    assert_eq!(scope.scan_budget_observed, Some(0));
    assert!(!scope.scan_budget_exhausted);

    let search = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args(["--data-dir", path_str(&data_dir), "search", "Java"])
        .output()
        .expect("search before daemon import worker");
    assert!(search.status.success());
    assert!(search.stderr.is_empty());
    let search_stdout = String::from_utf8_lossy(&search.stdout);
    assert!(search_stdout.contains("search index not available yet"));
    assert!(search_stdout.contains("results: 0"));

    remove_dir(&data_dir);
}

#[test]
fn cancel_import_task_hides_queued_work_without_running_import_or_leaking_paths() {
    serialize_windows_s9_import_test!();
    let data_dir = temp_dir("cancel-import-data");
    let fixture_root = fixture_root();
    let canonical_fixture_root = fs::canonicalize(&fixture_root).unwrap();

    let enqueue = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "import",
            "--enqueue",
            "--root",
            path_str(&fixture_root),
        ])
        .output()
        .expect("enqueue import task");
    assert!(
        enqueue.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&enqueue.stdout),
        String::from_utf8_lossy(&enqueue.stderr)
    );
    let enqueue_stdout = String::from_utf8_lossy(&enqueue.stdout);
    let task_id = stdout_value(&enqueue_stdout, "task id: ");

    let cancel = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "cancel",
            "import",
            "--task-id",
            task_id,
        ])
        .output()
        .expect("cancel import task");
    assert!(
        cancel.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&cancel.stdout),
        String::from_utf8_lossy(&cancel.stderr)
    );
    assert!(cancel.stderr.is_empty());
    let cancel_stdout = String::from_utf8_lossy(&cancel.stdout);
    assert!(cancel_stdout.contains("import task cancelled"));
    assert!(cancel_stdout.contains("status: cancelled"));
    assert!(!cancel_stdout.contains(path_str(&fixture_root)));
    assert!(!cancel_stdout.contains(path_str(&canonical_fixture_root)));

    let status = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args(["--data-dir", path_str(&data_dir), "status"])
        .output()
        .expect("run resume-cli status after cancel");
    assert!(status.status.success());
    assert!(status.stderr.is_empty());
    let status_stdout = String::from_utf8_lossy(&status.stdout);
    assert!(status_stdout.contains("import tasks queued: 0"));
    assert!(status_stdout.contains("import tasks cancelled: 1"));
    assert!(status_stdout.contains("searchable documents: 0"));
    assert!(!status_stdout.contains(path_str(&fixture_root)));
    assert!(!status_stdout.contains(path_str(&canonical_fixture_root)));

    remove_dir(&data_dir);
}

#[test]
fn import_multiple_roots_builds_searchable_index_without_path_leak() {
    serialize_windows_s9_import_test!();
    let data_dir = temp_dir("multi-root-import-data");
    let first_root = temp_dir("multi-root-a-private");
    let second_root = temp_dir("multi-root-b-private");
    fs::copy(
        fixture_root().join("synthetic-java-platform.pdf"),
        first_root.join("synthetic-java-platform.pdf"),
    )
    .unwrap();
    fs::copy(
        fixture_root().join("synthetic-java-engineer.docx"),
        second_root.join("synthetic-java-engineer.docx"),
    )
    .unwrap();
    let canonical_first_root = fs::canonicalize(&first_root).unwrap();
    let canonical_second_root = fs::canonicalize(&second_root).unwrap();

    let import = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "import",
            "--root",
            path_str(&first_root),
            "--root",
            path_str(&second_root),
        ])
        .output()
        .expect("run resume-cli multi-root import");

    assert!(
        import.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&import.stdout),
        String::from_utf8_lossy(&import.stderr)
    );
    assert!(import.stderr.is_empty());
    let import_stdout = String::from_utf8_lossy(&import.stdout);
    assert!(import_stdout.contains("roots scanned: 2"));
    assert!(import_stdout.contains("files discovered: 2"));
    assert!(import_stdout.contains("searchable documents: 2"));
    assert!(!import_stdout.contains(path_str(&first_root)));
    assert!(!import_stdout.contains(path_str(&second_root)));
    assert!(!import_stdout.contains(path_str(&canonical_first_root)));
    assert!(!import_stdout.contains(path_str(&canonical_second_root)));

    let search = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args(["--data-dir", path_str(&data_dir), "search", "Java"])
        .output()
        .expect("run resume-cli search after multi-root import");
    assert!(search.status.success());
    assert!(search.stderr.is_empty());
    let search_stdout = String::from_utf8_lossy(&search.stdout);
    assert!(search_stdout.contains("results: 2"));
    assert!(search_stdout.contains("synthetic-java-platform.pdf"));
    assert!(search_stdout.contains("synthetic-java-engineer.docx"));

    remove_dir(&data_dir);
    remove_dir(&first_root);
    remove_dir(&second_root);
}

#[test]
fn explicit_root_import_without_max_files_has_no_default_scan_budget() {
    serialize_windows_s9_import_test!();
    let data_dir = temp_dir("explicit-root-no-budget-data");
    let private_root = temp_dir("explicit-root-no-budget-private-root");
    let canonical_private_root = fs::canonicalize(&private_root).unwrap();
    fs::copy(
        fixture_root().join("synthetic-java-platform.pdf"),
        private_root.join("synthetic-java-platform.pdf"),
    )
    .unwrap();

    let import = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "import",
            "--root",
            path_str(&private_root),
        ])
        .output()
        .expect("run explicit root import without max-files");

    assert!(
        import.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&import.stdout),
        String::from_utf8_lossy(&import.stderr)
    );
    assert!(import.stderr.is_empty());
    let import_stdout = String::from_utf8_lossy(&import.stdout);
    assert!(import_stdout.contains("scan profile: explicit"));
    assert!(import_stdout.contains("scan budget exhausted: no"));
    assert!(import_stdout.contains("scan file limit: none"));
    assert!(!import_stdout.contains(path_str(&private_root)));
    assert!(!import_stdout.contains(path_str(&canonical_private_root)));

    let store = MetaStore::open_data_dir(&data_dir).unwrap();
    store.run_migrations().unwrap();
    let scope = store
        .latest_import_scan_scope()
        .unwrap()
        .expect("scan scope persisted");
    assert_eq!(scope.root_kind, ImportRootKind::Explicit);
    assert_eq!(scope.scan_budget_kind, None);
    assert_eq!(scope.scan_budget_limit, None);
    assert_eq!(scope.scan_budget_observed, None);
    assert!(!scope.scan_budget_exhausted);

    remove_dir(&data_dir);
    remove_dir(&private_root);
}

#[test]
fn local_discovery_root_preset_uses_discovery_profile_without_path_leak() {
    serialize_windows_s9_import_test!();
    let data_dir = temp_dir("local-discovery-import-data");
    let local_root = temp_dir("local-discovery-private-root");
    let canonical_local_root = fs::canonicalize(&local_root).unwrap();
    fs::create_dir_all(local_root.join("Documents")).unwrap();
    fs::create_dir_all(local_root.join("node_modules")).unwrap();
    fs::copy(
        fixture_root().join("synthetic-java-platform.pdf"),
        local_root
            .join("Documents")
            .join("synthetic-java-platform.pdf"),
    )
    .unwrap();
    fs::copy(
        fixture_root().join("synthetic-java-engineer.docx"),
        local_root
            .join("node_modules")
            .join("synthetic-java-engineer.docx"),
    )
    .unwrap();

    let root_override = std::env::join_paths([&local_root]).unwrap();
    let import = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .env(LOCAL_DISCOVERY_ROOTS_ENV, root_override)
        .args([
            "--data-dir",
            path_str(&data_dir),
            "import",
            "--root-preset",
            "local-discovery",
        ])
        .output()
        .expect("run local discovery preset import");

    assert!(
        import.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&import.stdout),
        String::from_utf8_lossy(&import.stderr)
    );
    assert!(import.stderr.is_empty());
    let import_stdout = String::from_utf8_lossy(&import.stdout);
    assert!(import_stdout.contains("scan profile: discovery"));
    assert!(import_stdout.contains("roots scanned: 1"));
    assert!(import_stdout.contains("files discovered: 1"));
    assert!(import_stdout.contains("scan budget exhausted: no"));
    assert!(import_stdout.contains("scan file limit: 10000"));
    assert!(import_stdout.contains("searchable documents: 1"));
    assert!(!import_stdout.contains(path_str(&local_root)));
    assert!(!import_stdout.contains(path_str(&canonical_local_root)));

    let search = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args(["--data-dir", path_str(&data_dir), "search", "Java"])
        .output()
        .expect("run resume-cli search after local discovery import");
    assert!(search.status.success());
    assert!(search.stderr.is_empty());
    let search_stdout = String::from_utf8_lossy(&search.stdout);
    assert!(search_stdout.contains("results: 1"));
    assert!(search_stdout.contains("synthetic-java-platform.pdf"));
    assert!(!search_stdout.contains("synthetic-java-engineer.docx"));

    let store = MetaStore::open_data_dir(&data_dir).unwrap();
    store.run_migrations().unwrap();
    let scope = store
        .latest_import_scan_scope()
        .unwrap()
        .expect("scan scope persisted");
    assert_eq!(scope.root_kind, ImportRootKind::Preset);
    assert_eq!(scope.root_preset, Some(ImportRootPreset::LocalDiscovery));
    assert_eq!(scope.scan_profile, ImportScanProfile::Discovery);
    assert_eq!(scope.files_discovered, 1);
    assert_eq!(scope.ignored_entries, 1);
    assert_eq!(scope.scan_budget_kind, Some(ImportScanBudgetKind::Files));
    assert_eq!(scope.scan_budget_limit, Some(10000));
    assert_eq!(scope.scan_budget_observed, Some(1));
    assert!(!scope.scan_budget_exhausted);
    assert_eq!(scope.searchable_documents, 1);
    assert_eq!(scope.ocr_required_documents, 0);
    assert_eq!(scope.canonical_root_path, path_str(&canonical_local_root));
    assert_eq!(scope.requested_root_path, path_str(&local_root));
    assert!(!format!("{scope:?}").contains(path_str(&local_root)));

    remove_dir(&data_dir);
    remove_dir(&local_root);
}

#[test]
fn local_discovery_root_preset_allows_explicit_file_budget_override_without_path_leak() {
    serialize_windows_s9_import_test!();
    let data_dir = temp_dir("local-discovery-budgeted-data");
    let local_root = temp_dir("local-discovery-budgeted-private-root");
    let canonical_local_root = fs::canonicalize(&local_root).unwrap();
    fs::create_dir_all(local_root.join("Documents")).unwrap();
    fs::copy(
        fixture_root().join("synthetic-java-platform.pdf"),
        local_root
            .join("Documents")
            .join("synthetic-java-platform.pdf"),
    )
    .unwrap();
    fs::copy(
        fixture_root().join("synthetic-java-engineer.docx"),
        local_root
            .join("Documents")
            .join("synthetic-java-engineer.docx"),
    )
    .unwrap();

    let root_override = std::env::join_paths([&local_root]).unwrap();
    let import = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .env(LOCAL_DISCOVERY_ROOTS_ENV, root_override)
        .args([
            "--data-dir",
            path_str(&data_dir),
            "import",
            "--root-preset",
            "local-discovery",
            "--max-files",
            "1",
        ])
        .output()
        .expect("run budgeted local discovery preset import");

    assert!(
        import.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&import.stdout),
        String::from_utf8_lossy(&import.stderr)
    );
    assert!(import.stderr.is_empty());
    let import_stdout = String::from_utf8_lossy(&import.stdout);
    assert!(import_stdout.contains("files discovered: 1"));
    assert!(import_stdout.contains("scan budget exhausted: yes"));
    assert!(import_stdout.contains("scan file limit: 1"));
    assert!(!import_stdout.contains(path_str(&local_root)));
    assert!(!import_stdout.contains(path_str(&canonical_local_root)));

    let store = MetaStore::open_data_dir(&data_dir).unwrap();
    store.run_migrations().unwrap();
    let scope = store
        .latest_import_scan_scope()
        .unwrap()
        .expect("scan scope persisted");
    assert_eq!(scope.root_kind, ImportRootKind::Preset);
    assert_eq!(scope.root_preset, Some(ImportRootPreset::LocalDiscovery));
    assert_eq!(scope.scan_budget_kind, Some(ImportScanBudgetKind::Files));
    assert_eq!(scope.scan_budget_limit, Some(1));
    assert_eq!(scope.scan_budget_observed, Some(1));
    assert!(scope.scan_budget_exhausted);
    assert!(!format!("{scope:?}").contains(path_str(&local_root)));

    remove_dir(&data_dir);
    remove_dir(&local_root);
}

#[test]
fn import_max_files_limits_scan_and_persists_budget_state_without_path_leak() {
    serialize_windows_s9_import_test!();
    let data_dir = temp_dir("budgeted-import-data");
    let private_root = temp_dir("budgeted-import-private-root");
    let canonical_private_root = fs::canonicalize(&private_root).unwrap();
    fs::copy(
        fixture_root().join("synthetic-java-platform.pdf"),
        private_root.join("a-platform.pdf"),
    )
    .unwrap();
    fs::copy(
        fixture_root().join("synthetic-java-engineer.docx"),
        private_root.join("b-engineer.docx"),
    )
    .unwrap();
    fs::copy(
        fixture_root().join("synthetic-scanned-resume.pdf"),
        private_root.join("c-scanned.pdf"),
    )
    .unwrap();

    let import = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "import",
            "--root",
            path_str(&private_root),
            "--max-files",
            "1",
        ])
        .output()
        .expect("run budgeted import");

    assert!(
        import.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&import.stdout),
        String::from_utf8_lossy(&import.stderr)
    );
    assert!(import.stderr.is_empty());
    let import_stdout = String::from_utf8_lossy(&import.stdout);
    assert!(import_stdout.contains("files discovered: 1"));
    assert!(import_stdout.contains("scan budget exhausted: yes"));
    assert!(import_stdout.contains("scan file limit: 1"));
    assert!(!import_stdout.contains(path_str(&private_root)));
    assert!(!import_stdout.contains(path_str(&canonical_private_root)));

    let store = MetaStore::open_data_dir(&data_dir).unwrap();
    store.run_migrations().unwrap();
    let scope = store
        .latest_import_scan_scope()
        .unwrap()
        .expect("scan scope persisted");
    assert_eq!(scope.root_kind, ImportRootKind::Explicit);
    assert_eq!(scope.scan_budget_kind, Some(ImportScanBudgetKind::Files));
    assert_eq!(scope.scan_budget_limit, Some(1));
    assert_eq!(scope.scan_budget_observed, Some(1));
    assert!(scope.scan_budget_exhausted);
    assert_eq!(scope.files_discovered, 1);
    assert!(!format!("{scope:?}").contains(path_str(&private_root)));

    remove_dir(&data_dir);
    remove_dir(&private_root);
}

#[test]
fn multi_root_import_reports_budget_exhausted_when_later_root_hits_file_limit() {
    serialize_windows_s9_import_test!();
    let data_dir = temp_dir("multi-root-budgeted-data");
    let first_root = temp_dir("multi-root-budgeted-first-private-root");
    let second_root = temp_dir("multi-root-budgeted-second-private-root");
    let canonical_first_root = fs::canonicalize(&first_root).unwrap();
    let canonical_second_root = fs::canonicalize(&second_root).unwrap();
    fs::copy(
        fixture_root().join("synthetic-java-platform.pdf"),
        first_root.join("synthetic-java-platform.pdf"),
    )
    .unwrap();
    fs::copy(
        fixture_root().join("synthetic-java-platform.pdf"),
        second_root.join("a-platform.pdf"),
    )
    .unwrap();
    fs::copy(
        fixture_root().join("synthetic-java-engineer.docx"),
        second_root.join("b-engineer.docx"),
    )
    .unwrap();

    let import = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "import",
            "--root",
            path_str(&first_root),
            "--root",
            path_str(&second_root),
            "--max-files",
            "1",
        ])
        .output()
        .expect("run budgeted multi-root import");

    assert!(
        import.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&import.stdout),
        String::from_utf8_lossy(&import.stderr)
    );
    assert!(import.stderr.is_empty());
    let import_stdout = String::from_utf8_lossy(&import.stdout);
    assert!(import_stdout.contains("roots scanned: 2"));
    assert!(import_stdout.contains("files discovered: 2"));
    assert!(import_stdout.contains("scan budget exhausted: yes"));
    assert!(import_stdout.contains("scan file limit: 1"));
    assert!(!import_stdout.contains(path_str(&first_root)));
    assert!(!import_stdout.contains(path_str(&second_root)));
    assert!(!import_stdout.contains(path_str(&canonical_first_root)));
    assert!(!import_stdout.contains(path_str(&canonical_second_root)));

    remove_dir(&data_dir);
    remove_dir(&first_root);
    remove_dir(&second_root);
}

#[cfg(unix)]
#[test]
fn import_persists_scan_errors_without_path_leak() {
    serialize_windows_s9_import_test!();
    use std::os::unix::fs::PermissionsExt;

    let data_dir = temp_dir("scan-error-import-data");
    let private_root = temp_dir("scan-error-private-root");
    let canonical_private_root = fs::canonicalize(&private_root).unwrap();
    let unreadable_dir = private_root.join("unreadable-synthetic-subdir");
    fs::create_dir_all(&unreadable_dir).unwrap();
    fs::set_permissions(&unreadable_dir, fs::Permissions::from_mode(0o000)).unwrap();

    let import = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "import",
            "--root",
            path_str(&private_root),
        ])
        .output()
        .expect("run scan-error import");

    assert!(
        import.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&import.stdout),
        String::from_utf8_lossy(&import.stderr)
    );
    assert!(import.stderr.is_empty());
    let import_stdout = String::from_utf8_lossy(&import.stdout);
    assert!(import_stdout.contains("files discovered: 0"));
    assert!(import_stdout.contains("scan errors: 1"));
    assert!(!import_stdout.contains(path_str(&private_root)));
    assert!(!import_stdout.contains(path_str(&canonical_private_root)));

    let status = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args(["--data-dir", path_str(&data_dir), "status"])
        .output()
        .expect("run status after scan-error import");
    assert!(status.status.success());
    assert!(status.stderr.is_empty());
    let status_stdout = String::from_utf8_lossy(&status.stdout);
    assert!(status_stdout.contains("import scan errors: 1"));
    assert!(!status_stdout.contains(path_str(&private_root)));

    let store = MetaStore::open_data_dir(&data_dir).unwrap();
    store.run_migrations().unwrap();
    let scope = store
        .latest_import_scan_scope()
        .unwrap()
        .expect("scan scope persisted");
    assert_eq!(scope.scan_errors, 1);
    let errors = store
        .import_scan_errors_for_task(&scope.import_task_id)
        .unwrap();
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].kind, ImportScanErrorKind::PermissionDenied);
    assert_eq!(errors[0].operation, ImportScanErrorOperation::ReadDirectory);
    assert_eq!(errors[0].path_digest, None);
    assert!(!format!("{:?}", errors[0]).contains(path_str(&private_root)));

    fs::set_permissions(&unreadable_dir, fs::Permissions::from_mode(0o700)).unwrap();
    remove_dir(&data_dir);
    remove_dir(&private_root);
}

#[test]
fn import_reuses_recoverable_task_after_restart() {
    serialize_windows_s9_import_test!();
    let data_dir = temp_dir("import-restart-data");
    let fixture_root = fixture_root();
    let canonical_fixture_root = fs::canonicalize(&fixture_root).unwrap();
    let pending_task_id = seed_retryable_import_task(&data_dir, &fixture_root);

    let import = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "import",
            "--root",
            path_str(&fixture_root),
        ])
        .output()
        .expect("run resume-cli import after restart");

    assert!(
        import.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&import.stdout),
        String::from_utf8_lossy(&import.stderr)
    );
    assert!(import.stderr.is_empty());
    let import_stdout = String::from_utf8_lossy(&import.stdout);
    assert!(import_stdout.contains("status: completed"));
    assert!(import_stdout.contains(&pending_task_id.to_string()));
    assert!(!import_stdout.contains(path_str(&fixture_root)));
    assert!(!import_stdout.contains(path_str(&canonical_fixture_root)));

    let store = MetaStore::open_data_dir(&data_dir).unwrap();
    store.run_migrations().unwrap();
    let task = store.import_task_by_id(&pending_task_id).unwrap().unwrap();
    assert_eq!(task.status, ImportTaskStatus::Completed);
    assert_eq!(store.status_summary().unwrap().import_tasks_recoverable, 0);

    let search = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args(["--data-dir", path_str(&data_dir), "search", "Java"])
        .output()
        .expect("run resume-cli search after recovered import");
    assert!(search.status.success());
    let search_stdout = String::from_utf8_lossy(&search.stdout);
    assert!(search_stdout.contains("results: 2"));

    remove_dir(&data_dir);
}

#[test]
fn import_does_not_take_over_live_running_task() {
    serialize_windows_s9_import_test!();
    let data_dir = temp_dir("import-live-running-data");
    let fixture_root = fixture_root();
    let pending_task_id = seed_live_running_import_task(&data_dir, &fixture_root);

    let import = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "import",
            "--root",
            path_str(&fixture_root),
        ])
        .output()
        .expect("run resume-cli import while task is live");

    assert!(!import.status.success());
    assert!(import.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&import.stderr);
    assert!(stderr.contains("import task is already running"));
    assert!(!stderr.contains(path_str(&fixture_root)));

    let store = MetaStore::open_data_dir(&data_dir).unwrap();
    store.run_migrations().unwrap();
    let task = store.import_task_by_id(&pending_task_id).unwrap().unwrap();
    assert_eq!(task.status, ImportTaskStatus::Running);

    remove_dir(&data_dir);
}

#[test]
fn discovery_import_does_not_take_over_live_running_task_for_same_root() {
    serialize_windows_s9_import_test!();
    let data_dir = temp_dir("discovery-import-live-running-data");
    let fixture_root = fixture_root();
    let pending_task_id = seed_live_running_import_task(&data_dir, &fixture_root);

    let import = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "import",
            "--root",
            path_str(&fixture_root),
            "--profile",
            "discovery",
        ])
        .output()
        .expect("run discovery import while task is live");

    assert!(!import.status.success());
    assert!(import.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&import.stderr);
    assert!(stderr.contains("import task is already running"));
    assert!(!stderr.contains(path_str(&fixture_root)));

    let store = MetaStore::open_data_dir(&data_dir).unwrap();
    store.run_migrations().unwrap();
    let task = store.import_task_by_id(&pending_task_id).unwrap().unwrap();
    assert_eq!(task.status, ImportTaskStatus::Running);

    remove_dir(&data_dir);
}

#[test]
fn multi_root_import_does_not_take_over_live_running_task_for_any_root() {
    serialize_windows_s9_import_test!();
    let data_dir = temp_dir("multi-root-live-running-data");
    let fixture_root = fixture_root();
    let second_root = temp_dir("multi-root-live-second");
    let pending_task_id = seed_live_running_import_task(&data_dir, &fixture_root);

    let import = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "import",
            "--root",
            path_str(&fixture_root),
            "--root",
            path_str(&second_root),
        ])
        .output()
        .expect("run multi-root import while one task is live");

    assert!(!import.status.success());
    assert!(import.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&import.stderr);
    assert!(stderr.contains("import task is already running"));
    assert!(!stderr.contains(path_str(&fixture_root)));
    assert!(!stderr.contains(path_str(&second_root)));

    let store = MetaStore::open_data_dir(&data_dir).unwrap();
    store.run_migrations().unwrap();
    let task = store.import_task_by_id(&pending_task_id).unwrap().unwrap();
    assert_eq!(task.status, ImportTaskStatus::Running);

    remove_dir(&data_dir);
    remove_dir(&second_root);
}

#[test]
fn multi_root_import_reuses_recoverable_task_for_each_root() {
    serialize_windows_s9_import_test!();
    let data_dir = temp_dir("multi-root-recoverable-data");
    let fixture_root = fixture_root();
    let second_root = temp_dir("multi-root-recoverable-second");
    let canonical_fixture_root = fs::canonicalize(&fixture_root).unwrap();
    let canonical_second_root = fs::canonicalize(&second_root).unwrap();
    let pending_task_id = seed_retryable_import_task(&data_dir, &fixture_root);

    let import = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "import",
            "--root",
            path_str(&fixture_root),
            "--root",
            path_str(&second_root),
        ])
        .output()
        .expect("run multi-root import after restart");

    assert!(
        import.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&import.stdout),
        String::from_utf8_lossy(&import.stderr)
    );
    assert!(import.stderr.is_empty());
    let import_stdout = String::from_utf8_lossy(&import.stdout);
    assert!(import_stdout.contains(&pending_task_id.to_string()));
    assert!(import_stdout.contains("roots scanned: 2"));
    assert!(!import_stdout.contains(path_str(&fixture_root)));
    assert!(!import_stdout.contains(path_str(&second_root)));
    assert!(!import_stdout.contains(path_str(&canonical_fixture_root)));
    assert!(!import_stdout.contains(path_str(&canonical_second_root)));

    let store = MetaStore::open_data_dir(&data_dir).unwrap();
    store.run_migrations().unwrap();
    let task = store.import_task_by_id(&pending_task_id).unwrap().unwrap();
    assert_eq!(task.status, ImportTaskStatus::Completed);
    assert_eq!(store.status_summary().unwrap().import_tasks_recoverable, 0);

    remove_dir(&data_dir);
    remove_dir(&second_root);
}

fn seed_retryable_import_task(data_dir: &Path, fixture_root: &Path) -> ImportTaskId {
    let store = MetaStore::open_data_dir(data_dir).unwrap();
    store.run_migrations().unwrap();
    let queued_at = UnixTimestamp::from_unix_seconds(1_700_000_000);
    let started_at = UnixTimestamp::from_unix_seconds(1_700_000_010);
    let finished_at = UnixTimestamp::from_unix_seconds(1_700_000_020);
    let id = ImportTaskId::from_non_secret_parts(&["s9", "recoverable-import-task"]);
    let canonical_root = fs::canonicalize(fixture_root).unwrap();
    store
        .insert_import_task(&ImportTask {
            id: id.clone(),
            root_path: path_str(&canonical_root).to_string(),
            status: ImportTaskStatus::FailedRetryable,
            queued_at,
            started_at: Some(started_at),
            finished_at: Some(finished_at),
            updated_at: finished_at,
        })
        .unwrap();
    id
}

fn seed_live_running_import_task(data_dir: &Path, fixture_root: &Path) -> ImportTaskId {
    let store = MetaStore::open_data_dir(data_dir).unwrap();
    store.run_migrations().unwrap();
    let canonical_root = fs::canonicalize(fixture_root).unwrap();
    store
        .insert_import_task(&ImportTask {
            id: ImportTaskId::from_non_secret_parts(&["s9", "older-queued-import-task"]),
            root_path: path_str(&canonical_root).to_string(),
            status: ImportTaskStatus::Queued,
            queued_at: UnixTimestamp::from_unix_seconds(1_699_999_000),
            started_at: None,
            finished_at: None,
            updated_at: UnixTimestamp::from_unix_seconds(1_699_999_000),
        })
        .unwrap();

    let queued_at = UnixTimestamp::from_unix_seconds(1_700_000_000);
    let started_at = UnixTimestamp::from_unix_seconds(1_700_000_010);
    let id = ImportTaskId::from_non_secret_parts(&["s9", "live-running-import-task"]);
    store
        .insert_import_task(&ImportTask {
            id: id.clone(),
            root_path: path_str(&canonical_root).to_string(),
            status: ImportTaskStatus::Running,
            queued_at,
            started_at: Some(started_at),
            finished_at: None,
            updated_at: started_at,
        })
        .unwrap();
    id
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
    let path = std::env::temp_dir().join(format!("resume-ir-s9-cli-{label}-{unique}"));
    fs::create_dir_all(&path).unwrap();
    path
}

fn path_str(path: &Path) -> &str {
    path.to_str().unwrap()
}

fn stdout_value<'a>(output: &'a str, prefix: &str) -> &'a str {
    output
        .lines()
        .find_map(|line| line.strip_prefix(prefix))
        .unwrap_or_else(|| panic!("missing line with prefix {prefix:?} in:\n{output}"))
}

fn remove_dir(path: &Path) {
    let _ = fs::remove_dir_all(path);
}

#[cfg(unix)]
fn write_fixture_executable(name: &str, body: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;

    let directory = temp_dir("witness-ocr-command-bin");
    let path = directory.join(name);
    fs::write(&path, body).unwrap();
    let mut permissions = fs::metadata(&path).unwrap().permissions();
    permissions.set_mode(0o700);
    fs::set_permissions(&path, permissions).unwrap();
    path
}

#[cfg(windows)]
fn write_fixture_executable(name: &str, body: &str) -> PathBuf {
    let directory = temp_dir("witness-ocr-command-bin");
    let path = directory.join(format!("{name}.cmd"));
    fs::write(&path, windows_fixture_command_body(body)).unwrap();
    path
}

#[cfg(windows)]
fn windows_fixture_command_body(body: &str) -> String {
    if body.contains("WitnessOCRPartialSecret") {
        let counter_file = body
            .lines()
            .find_map(|line| {
                line.trim()
                    .strip_prefix("counter_file=\"")
                    .and_then(|value| value.strip_suffix('"'))
            })
            .expect("partial OCR fixture should define counter_file");
        return format!(
            concat!(
                "@echo off\r\n",
                "setlocal\r\n",
                "set \"counter_file={}\"\r\n",
                "set \"count=0\"\r\n",
                "if exist \"%counter_file%\" set /p count=<\"%counter_file%\"\r\n",
                "set /a count=count+1\r\n",
                ">\"%counter_file%\" <nul set /p \"=%count%\"\r\n",
                "if \"%count%\"==\"2\" (\r\n",
                "  1>&2 echo fixture OCR failure without private data\r\n",
                "  exit /b 17\r\n",
                ")\r\n",
                "echo resume-ir-ocr-v1\r\n",
                "echo confidence=0.77\r\n",
                "echo text:\r\n",
                "echo WitnessOCRPartialSecret local OCR text\r\n",
                "exit /b 0\r\n"
            ),
            counter_file
        );
    }

    let mut script = String::from("@echo off\r\n");
    for line in body.lines().map(str::trim) {
        if line.is_empty() || line == "#!/bin/sh" {
            continue;
        }
        let Some(literal) = line
            .strip_prefix("printf '")
            .and_then(|value| value.strip_suffix("'"))
        else {
            panic!("unsupported Windows OCR fixture shell line: {line}");
        };
        let literal = literal
            .strip_suffix("\\n")
            .expect("Windows OCR fixture printf lines should end with newline");
        script.push_str("echo ");
        script.push_str(&escape_batch_echo_literal(literal));
        script.push_str("\r\n");
    }
    script.push_str("exit /b 0\r\n");
    script
}

#[cfg(windows)]
fn escape_batch_echo_literal(literal: &str) -> String {
    literal
        .replace('^', "^^")
        .replace('&', "^&")
        .replace('|', "^|")
        .replace('<', "^<")
        .replace('>', "^>")
}

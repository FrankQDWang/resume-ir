use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use meta_store::{
    DocumentStatus, IngestJobKind, IngestJobStatus, MetaStore, OcrPageCacheEntry, OcrPageCacheKey,
    OcrPageCacheStatus, UnixTimestamp,
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
    assert_eq!(scanned_document(&store).status, DocumentStatus::Searchable);
    assert!(store.retryable_jobs().unwrap().is_empty());

    remove_dir(&data_dir);
}

#[cfg(unix)]
#[test]
fn ocr_worker_executes_local_command_persists_cache_and_indexes_searchable_text() {
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
    assert_eq!(scanned.status, DocumentStatus::Searchable);
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
    assert!(search.stderr.is_empty());
    let search_stdout = String::from_utf8_lossy(&search.stdout);
    assert!(
        search_stdout.contains("results: 1"),
        "stdout:\n{search_stdout}"
    );
    assert!(search_stdout.contains("synthetic-scanned-resume.pdf"));
    assert!(!search_stdout.contains(path_str(&data_dir)));
    assert!(!search_stdout.contains(path_str(&fixture_root)));

    remove_dir(&data_dir);
}

#[cfg(unix)]
#[test]
fn ocr_worker_processes_all_scanned_pdf_pages_before_indexing() {
    let data_dir = temp_dir("ocr-worker-multi-page-data");
    let fixture_root = temp_dir("ocr-worker-multi-page-fixtures");
    std::fs::write(
        fixture_root.join("synthetic-scanned-resume.pdf"),
        two_page_scanned_pdf_bytes(),
    )
    .unwrap();
    let render_command = write_fixture_executable(
        "fixture-pdf-render-multi-page",
        r#"#!/bin/sh
case "$RESUME_IR_PDF_RENDER_PAGE_NO" in
  1) printf 'S89_RENDERED_PAGE_1_BYTES' ;;
  2) printf 'S89_RENDERED_PAGE_2_BYTES' ;;
  *) printf 'PRIVATE_UNEXPECTED_RENDER_PAGE_%s\n' "$RESUME_IR_PDF_RENDER_PAGE_NO"; exit 23 ;;
esac
"#,
    );
    let command = write_fixture_executable(
        "fixture-ocr-worker-multi-page",
        r#"#!/bin/sh
input_bytes="$(cat "$RESUME_IR_OCR_INPUT_PATH")"
printf 'resume-ir-ocr-v1\n'
printf 'confidence=0.82\n'
printf 'text:\n'
case "$input_bytes:$RESUME_IR_OCR_PAGE_NO" in
  S89_RENDERED_PAGE_1_BYTES:1) printf 'S89PageOneToken first page text\n' ;;
  S89_RENDERED_PAGE_2_BYTES:2) printf 'S89PageTwoToken second page text\n' ;;
  *) printf 'PRIVATE_UNEXPECTED_OCR_INPUT_%s_PAGE_%s\n' "$input_bytes" "$RESUME_IR_OCR_PAGE_NO"; exit 19 ;;
esac
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
            "--render-command",
            path_str(&render_command),
            "--engine-profile",
            "fixture-multi-page-engine",
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
    assert!(stdout.contains("cache writes: 2"));
    assert!(stdout.contains("cache hits: 0"));
    assert!(!stdout.contains("S89PageOneToken"));
    assert!(!stdout.contains("S89PageTwoToken"));
    assert!(!stdout.contains("PRIVATE_UNEXPECTED_OCR_PAGE"));
    assert!(!stdout.contains("PRIVATE_UNEXPECTED_RENDER_PAGE"));
    assert!(!stdout.contains(path_str(&data_dir)));
    assert!(!stdout.contains(path_str(&fixture_root)));
    assert!(!stdout.contains(path_str(&command)));
    assert!(!stdout.contains(path_str(&render_command)));

    let store = MetaStore::open(data_dir.join("metadata.sqlite3")).unwrap();
    store.run_migrations().unwrap();
    let scanned = scanned_document(&store);
    assert_eq!(scanned.status, DocumentStatus::Searchable);
    assert!(store.retryable_jobs().unwrap().is_empty());
    let content_hash = scanned.content_hash.clone().expect("content hash");
    for (page_no, token) in [(1, "S89PageOneToken"), (2, "S89PageTwoToken")] {
        let cache_key =
            OcrPageCacheKey::new(content_hash.clone(), page_no, 300, "eng", "balanced").unwrap();
        let cache_entry = store
            .ocr_page_cache_entry(&cache_key)
            .unwrap()
            .expect("OCR cache entry");
        assert_eq!(cache_entry.status(), OcrPageCacheStatus::Succeeded);
        assert_eq!(cache_entry.confidence(), Some(0.82));
        assert_eq!(
            cache_entry.engine_profile(),
            Some("fixture-multi-page-engine")
        );
        assert!(cache_entry.text().unwrap().contains(token));
    }

    let version = store
        .latest_visible_resume_version_for_document(&scanned.id)
        .unwrap()
        .expect("OCR resume version");
    assert_eq!(version.page_count, Some(2));
    assert!(version.clean_text.unwrap().contains("S89PageOneToken"));
    assert!(version.raw_text.unwrap().contains("S89PageTwoToken"));

    for token in ["S89PageOneToken", "S89PageTwoToken"] {
        let search = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
            .args([
                "--data-dir",
                path_str(&data_dir),
                "search",
                token,
                "--top-k",
                "20",
            ])
            .output()
            .expect("run resume-cli search for multi-page OCR text");
        assert!(search.status.success());
        assert!(search.stderr.is_empty());
        let search_stdout = String::from_utf8_lossy(&search.stdout);
        assert!(
            search_stdout.contains("results: 1"),
            "stdout:\n{search_stdout}"
        );
        assert!(search_stdout.contains("synthetic-scanned-resume.pdf"));
        assert!(!search_stdout.contains(path_str(&data_dir)));
        assert!(!search_stdout.contains(path_str(&fixture_root)));
    }

    remove_dir(&data_dir);
    remove_dir(&fixture_root);
}

#[cfg(unix)]
#[test]
fn ocr_worker_command_crash_records_retryable_failure_without_leaking_outputs() {
    let data_dir = temp_dir("ocr-worker-crash-data");
    let fixture_root = fixture_root();
    let command = write_fixture_executable(
        "fixture-ocr-worker-crash",
        r#"#!/bin/sh
printf 'PRIVATE_OCR_WORKER_CRASH_STDOUT\n'
printf 'PRIVATE_OCR_WORKER_CRASH_STDERR\n' >&2
exit 17
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
        ])
        .output()
        .expect("run crashed ocr worker command");

    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("ocr worker blocked: local OCR command failed or unavailable"));
    assert!(!stderr.contains("PRIVATE_OCR_WORKER_CRASH_STDOUT"));
    assert!(!stderr.contains("PRIVATE_OCR_WORKER_CRASH_STDERR"));
    assert!(!stderr.contains(path_str(&data_dir)));
    assert!(!stderr.contains(path_str(&fixture_root)));
    assert!(!stderr.contains(path_str(&command)));

    let store = MetaStore::open(data_dir.join("metadata.sqlite3")).unwrap();
    store.run_migrations().unwrap();
    let scanned = scanned_document(&store);
    assert_eq!(scanned.status, DocumentStatus::OcrRequired);
    let jobs = store.retryable_jobs().unwrap();
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].status, IngestJobStatus::FailedRetryable);
    assert_eq!(jobs[0].attempt_count, 1);
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
        .expect("OCR retryable failure cache entry");
    assert_eq!(cache_entry.status(), OcrPageCacheStatus::FailedRetryable);
    assert_eq!(cache_entry.text(), None);
    assert_eq!(cache_entry.error_kind(), Some("EngineFailed"));

    let search = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "search",
            "PRIVATE_OCR_WORKER_CRASH_STDOUT",
            "--top-k",
            "20",
        ])
        .output()
        .expect("run resume-cli search after OCR crash");
    assert!(search.status.success());
    assert!(search.stderr.is_empty());
    let search_stdout = String::from_utf8_lossy(&search.stdout);
    assert!(
        search_stdout.contains("results: 0"),
        "stdout:\n{search_stdout}"
    );
    assert!(!search_stdout.contains("synthetic-scanned-resume.pdf"));
    assert!(!search_stdout.contains(path_str(&data_dir)));
    assert!(!search_stdout.contains(path_str(&fixture_root)));

    remove_dir(&data_dir);
}

#[cfg(unix)]
#[test]
fn ocr_worker_indexes_succeeded_cache_hit_without_invoking_command() {
    let data_dir = temp_dir("ocr-worker-cache-hit-data");
    let fixture_root = fixture_root();
    import_fixtures(&data_dir, &fixture_root);

    {
        let store = MetaStore::open(data_dir.join("metadata.sqlite3")).unwrap();
        store.run_migrations().unwrap();
        let scanned = scanned_document(&store);
        assert_eq!(scanned.status, DocumentStatus::OcrRequired);
        let cache_key = OcrPageCacheKey::new(
            scanned.content_hash.expect("content hash"),
            1,
            300,
            "eng",
            "balanced",
        )
        .unwrap();
        let cache_entry = OcrPageCacheEntry::succeeded(
            cache_key,
            "OCRS41CacheHitToken cached OCR text",
            0.84,
            "fixture-cache-engine",
            7,
            UnixTimestamp::from_unix_seconds(1_900_000_041),
        )
        .unwrap();
        store.upsert_ocr_page_cache_entry(&cache_entry).unwrap();
    }

    let command = write_fixture_executable(
        "fixture-ocr-worker-cache-hit-should-not-run",
        r#"#!/bin/sh
printf 'unexpected OCR command invocation\n' >&2
exit 42
"#,
    );
    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "ocr-worker",
            "--once",
            "--command",
            path_str(&command),
        ])
        .output()
        .expect("run ocr worker against succeeded cache entry");

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
    assert!(stdout.contains("cache writes: 0"));
    assert!(stdout.contains("cache hits: 1"));
    assert!(!stdout.contains("OCRS41CacheHitToken"));
    assert!(!stdout.contains(path_str(&data_dir)));
    assert!(!stdout.contains(path_str(&fixture_root)));

    let store = MetaStore::open(data_dir.join("metadata.sqlite3")).unwrap();
    store.run_migrations().unwrap();
    assert_eq!(scanned_document(&store).status, DocumentStatus::Searchable);
    assert!(store.retryable_jobs().unwrap().is_empty());

    let search = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "search",
            "OCRS41CacheHitToken",
            "--top-k",
            "20",
        ])
        .output()
        .expect("run resume-cli search for cache-hit OCR text");
    assert!(search.status.success());
    assert!(search.stderr.is_empty());
    let search_stdout = String::from_utf8_lossy(&search.stdout);
    assert!(
        search_stdout.contains("results: 1"),
        "stdout:\n{search_stdout}"
    );
    assert!(search_stdout.contains("synthetic-scanned-resume.pdf"));
    assert!(!search_stdout.contains(path_str(&data_dir)));
    assert!(!search_stdout.contains(path_str(&fixture_root)));

    remove_dir(&data_dir);
}

#[cfg(unix)]
#[test]
fn ocr_worker_empty_success_keeps_document_non_searchable() {
    let data_dir = temp_dir("ocr-worker-empty-text-data");
    let fixture_root = fixture_root();
    let command = write_fixture_executable(
        "fixture-ocr-worker-empty-text",
        r#"#!/bin/sh
printf 'resume-ir-ocr-v1\n'
printf 'confidence=0.66\n'
printf 'text:\n'
printf '    \n'
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
        ])
        .output()
        .expect("run ocr worker with empty OCR text");

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
    assert!(stdout.contains("cache hits: 0"));
    assert!(!stdout.contains(path_str(&data_dir)));
    assert!(!stdout.contains(path_str(&fixture_root)));

    let store = MetaStore::open(data_dir.join("metadata.sqlite3")).unwrap();
    store.run_migrations().unwrap();
    let scanned = scanned_document(&store);
    assert_eq!(scanned.status, DocumentStatus::OcrDone);
    assert!(store
        .resume_versions_for_document(&scanned.id)
        .unwrap()
        .is_empty());
    assert!(store.retryable_jobs().unwrap().is_empty());

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
        .expect("run resume-cli search after empty OCR text");
    assert!(search.status.success());
    assert!(search.stderr.is_empty());
    let search_stdout = String::from_utf8_lossy(&search.stdout);
    assert!(
        search_stdout.contains("results: 0"),
        "stdout:\n{search_stdout}"
    );
    assert!(!search_stdout.contains("synthetic-scanned-resume.pdf"));
    assert!(!search_stdout.contains(path_str(&data_dir)));
    assert!(!search_stdout.contains(path_str(&fixture_root)));

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

fn two_page_scanned_pdf_bytes() -> &'static [u8] {
    b"%PDF-1.4
1 0 obj << /Type /Catalog /Pages 2 0 R >> endobj
2 0 obj << /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >> endobj
3 0 obj << /Type /Page /Parent 2 0 R /Resources << /XObject << /Im1 5 0 R >> >> /Contents 7 0 R >> endobj
4 0 obj << /Type /Page /Parent 2 0 R /Resources << /XObject << /Im2 6 0 R >> >> /Contents 8 0 R >> endobj
5 0 obj << /Type /XObject /Subtype /Image /Width 10 /Height 10 /ColorSpace /DeviceGray /BitsPerComponent 8 /Length 4 >> stream
1111
endstream endobj
6 0 obj << /Type /XObject /Subtype /Image /Width 10 /Height 10 /ColorSpace /DeviceGray /BitsPerComponent 8 /Length 4 >> stream
2222
endstream endobj
7 0 obj << /Length 24 >> stream
q 10 0 0 10 0 0 cm /Im1 Do Q
endstream endobj
8 0 obj << /Length 24 >> stream
q 10 0 0 10 0 0 cm /Im2 Do Q
endstream endobj
%%EOF"
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

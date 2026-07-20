use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use import_pipeline::{
    current_import_processing_contract, finalize_migration_rebuild,
    prepare_migration_rebuild_artifacts, ImportOptions, SearchPublicationVectorization,
};
use meta_store::{
    ClassificationStatus, ContentDigest, CurrentClassifierEpoch, DataDirectoryOwnerAcquisition,
    DataDirectoryOwnerLease, Document, DocumentId, DocumentStatus, FileExtension,
    IngestJobFailureKind, IngestJobKind, IngestJobStatus, OcrPageCacheEntry, OcrPageCacheKey,
    OcrPageCacheStatus, OwnedMetaStore, ReadMetaStore, ReasonCode, SearchRepairReason,
    SourceRevision, SourceRevisionTriage, UnixTimestamp, CLASSIFIER_EPOCH,
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

    let store = create_owned_store(&data_dir);
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
        .claim_next_ocr_job(UnixTimestamp::from_unix_seconds(1_900_000_000))
        .unwrap()
        .expect("ocr document job can be claimed after restart");
    assert_eq!(claimed.job.kind, IngestJobKind::OcrDocument);
    assert_eq!(claimed.job.status, IngestJobStatus::Running);
    drop(store);

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

    let store = ReadMetaStore::open_data_dir(&data_dir).unwrap();
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

    let store = ReadMetaStore::open_data_dir(&data_dir).unwrap();
    let retryable = store.retryable_jobs().unwrap();
    assert_eq!(retryable.len(), 1);
    assert_eq!(retryable[0].status, IngestJobStatus::Queued);

    remove_dir(&data_dir);
}

#[test]
fn ocr_worker_repair_blocked_returns_not_ready_without_claiming() {
    let data_dir = temp_dir("ocr-worker-repair-blocked-data");
    let document_path = seed_ocr_pdf_document_with_bytes(
        &data_dir,
        b"synthetic repair-blocked OCR bytes".to_vec(),
        "repair-blocked",
        OcrFixturePublicationState::Unpublished,
    );
    let store = create_owned_store(&data_dir);
    let job_id = store.retryable_jobs().unwrap()[0].id.clone();
    store
        .block_migration_rebuild(
            SearchRepairReason::RuntimeInvariant,
            UnixTimestamp::from_unix_seconds(1_900_000_093),
        )
        .unwrap();
    drop(store);

    let missing_command = data_dir.join("synthetic-command-must-not-run");
    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "ocr-worker",
            "--once",
            "--command",
            path_str(&missing_command),
        ])
        .output()
        .expect("run repair-blocked OCR worker");

    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("ocr worker: not ready"));
    assert!(stdout.contains("documents processed: 0"));
    assert!(stdout.contains("cache writes: 0"));
    assert!(stdout.contains("cache hits: 0"));
    assert!(!stdout.contains(path_str(&data_dir)));
    assert!(!stdout.contains(path_str(&document_path)));
    assert!(!stdout.contains(path_str(&missing_command)));

    let store = ReadMetaStore::open_data_dir(&data_dir).unwrap();
    let still_queued = store.ingest_job_by_id(&job_id).unwrap().unwrap();
    assert_eq!(still_queued.status, IngestJobStatus::Queued);
    assert_eq!(still_queued.attempt_count, 0);
    assert_eq!(scanned_document(&store).status, DocumentStatus::OcrRequired);

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
printf 'SUMMARY\nSynthetic OCR fixture.\nEXPERIENCE\nBuilt systems.\nSKILLS\nSearch.\n'
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
        let store = ReadMetaStore::open_data_dir(&data_dir).unwrap();
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

    let store = ReadMetaStore::open_data_dir(&data_dir).unwrap();
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
printf 'SUMMARY\nSynthetic OCR fixture.\nEXPERIENCE\nBuilt systems.\nSKILLS\nSearch.\n'
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

    let store = ReadMetaStore::open_data_dir(&data_dir).unwrap();
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

    let metadata_bytes =
        std::fs::read(meta_store::metadata_store_path(&data_dir).unwrap()).unwrap();
    assert!(!metadata_bytes.starts_with(b"SQLite format 3"));
    assert!(!bytes_contain(&metadata_bytes, b"OCRS31UniqueToken"));
    assert!(!bytes_contain(&metadata_bytes, b"fixture-engine"));

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
  S89_RENDERED_PAGE_1_BYTES:1) printf 'SUMMARY\nSynthetic OCR fixture.\nEXPERIENCE\nBuilt systems.\nSKILLS\nSearch.\nS89PageOneToken first page text\n' ;;
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

    let store = ReadMetaStore::open_data_dir(&data_dir).unwrap();
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

    let projection = store
        .active_search_projection_for_document(&scanned.id)
        .unwrap()
        .expect("OCR search projection");
    let version = store
        .resume_version_by_id(&projection.resume_version_id)
        .unwrap()
        .expect("OCR resume version");
    assert_eq!(version.page_count, Some(2));
    let clean_text = version.clean_text.unwrap();
    assert!(clean_text.contains("S89PageOneToken"));
    assert!(clean_text.contains("S89PageTwoToken"));
    assert_eq!(version.raw_text, None);

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
fn ocr_worker_backpressures_scanned_pdf_above_page_limit_without_invoking_ocr() {
    let data_dir = temp_dir("ocr-worker-backpressure-data");
    let fixture_root = temp_dir("ocr-worker-backpressure-fixtures");
    std::fs::write(
        fixture_root.join("synthetic-scanned-resume.pdf"),
        two_page_scanned_pdf_bytes(),
    )
    .unwrap();
    let command = write_fixture_executable(
        "fixture-ocr-worker-backpressure",
        r#"#!/bin/sh
printf 'PRIVATE_OCR_BACKPRESSURE_INVOKED\n'
exit 31
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
            "--max-pages-per-document",
            "1",
        ])
        .output()
        .expect("run backpressured ocr worker");

    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("ocr worker blocked: OCR page count exceeds configured limit"));
    assert!(!stderr.contains("PRIVATE_OCR_BACKPRESSURE_INVOKED"));
    assert!(!stderr.contains(path_str(&data_dir)));
    assert!(!stderr.contains(path_str(&fixture_root)));
    assert!(!stderr.contains(path_str(&command)));

    let store = ReadMetaStore::open_data_dir(&data_dir).unwrap();
    let scanned = scanned_document(&store);
    assert_eq!(scanned.status, DocumentStatus::OcrRequired);
    let jobs = store.retryable_jobs().unwrap();
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].status, IngestJobStatus::FailedRetryable);
    assert_eq!(jobs[0].attempt_count, 1);
    assert_eq!(
        jobs[0].failure_kind,
        Some(IngestJobFailureKind::OcrPageBudgetExceeded)
    );

    let status = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args(["--data-dir", path_str(&data_dir), "status"])
        .output()
        .expect("run resume-cli status after OCR backpressure");
    assert!(status.status.success());
    assert!(status.stderr.is_empty());
    let status_stdout = String::from_utf8_lossy(&status.stdout);
    assert!(status_stdout.contains("ocr page budget blocked: 1"));
    assert!(status_stdout.contains(
        "ocr remediation: raise OCR max pages per document or skip oversized scanned PDFs"
    ));
    assert!(!status_stdout.contains(path_str(&data_dir)));
    assert!(!status_stdout.contains(path_str(&fixture_root)));
    assert!(!status_stdout.contains(path_str(&command)));

    let doctor = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args(["--data-dir", path_str(&data_dir), "doctor"])
        .output()
        .expect("run resume-cli doctor after OCR backpressure");
    assert!(doctor.status.success());
    assert!(doctor.stderr.is_empty());
    let doctor_stdout = String::from_utf8_lossy(&doctor.stdout);
    assert!(doctor_stdout.contains("ocr page budget blocked: 1"));
    assert!(doctor_stdout.contains(
        "ocr remediation: raise OCR max pages per document or skip oversized scanned PDFs"
    ));
    assert!(!doctor_stdout.contains(path_str(&data_dir)));
    assert!(!doctor_stdout.contains(path_str(&fixture_root)));
    assert!(!doctor_stdout.contains(path_str(&command)));

    let diagnostics = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "export-diagnostics",
            "--redact",
        ])
        .output()
        .expect("run resume-cli export-diagnostics after OCR backpressure");
    assert!(diagnostics.status.success());
    assert!(diagnostics.stderr.is_empty());
    let diagnostics_stdout = String::from_utf8_lossy(&diagnostics.stdout);
    assert!(diagnostics_stdout.contains("\"ocr_page_budget_blocked\": 1"));
    assert!(diagnostics_stdout.contains(
        "\"ocr_remediation\": \"raise OCR max pages per document or skip oversized scanned PDFs\""
    ));
    assert!(!diagnostics_stdout.contains(path_str(&data_dir)));
    assert!(!diagnostics_stdout.contains(path_str(&fixture_root)));
    assert!(!diagnostics_stdout.contains(path_str(&command)));

    let content_hash = scanned.content_hash.expect("content hash");
    for page_no in [1, 2] {
        let cache_key =
            OcrPageCacheKey::new(content_hash.clone(), page_no, 300, "eng", "balanced").unwrap();
        assert!(store.ocr_page_cache_entry(&cache_key).unwrap().is_none());
    }

    let search = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "search",
            "PRIVATE_OCR_BACKPRESSURE_INVOKED",
            "--top-k",
            "20",
        ])
        .output()
        .expect("run resume-cli search after OCR backpressure");
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
    remove_dir(&fixture_root);
}

#[cfg(unix)]
#[test]
fn ocr_worker_uses_pdftoppm_renderer_for_valid_pdf_before_ocr() {
    let Some(pdftoppm) = find_command("pdftoppm") else {
        eprintln!("skipping pdftoppm CLI worker witness because pdftoppm is not installed");
        return;
    };
    let data_dir = temp_dir("ocr-worker-pdftoppm-data");
    let fixture_root = temp_dir("ocr-worker-pdftoppm-fixtures");
    std::fs::write(
        fixture_root.join("synthetic-scanned-resume.pdf"),
        valid_blank_pdf_bytes(),
    )
    .unwrap();
    let command = write_fixture_executable(
        "fixture-ocr-worker-pdftoppm",
        r#"#!/bin/sh
header="$(head -c 2 "$RESUME_IR_OCR_INPUT_PATH")"
if [ "$header" != "P6" ]; then
  printf 'PRIVATE_UNEXPECTED_PDFFTOPPM_OCR_INPUT_%s\n' "$header"
  exit 19
fi
if [ "$RESUME_IR_OCR_PAGE_NO" != "1" ]; then
  printf 'PRIVATE_UNEXPECTED_PDFFTOPPM_OCR_PAGE_%s\n' "$RESUME_IR_OCR_PAGE_NO"
  exit 20
fi
printf 'resume-ir-ocr-v1\n'
printf 'confidence=0.87\n'
printf 'text:\n'
printf 'SUMMARY\nSynthetic OCR fixture.\nEXPERIENCE\nBuilt systems.\nSKILLS\nSearch.\n'
printf 'S91PdftoppmRenderedToken rendered page text\n'
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
            "--pdftoppm-command",
            path_str(&pdftoppm),
            "--engine-profile",
            "fixture-pdftoppm-engine",
            "--render-dpi",
            "72",
        ])
        .output()
        .expect("run ocr worker with pdftoppm renderer");

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
    assert!(!stdout.contains("S91PdftoppmRenderedToken"));
    assert!(!stdout.contains("PRIVATE_UNEXPECTED_PDFFTOPPM"));
    assert!(!stdout.contains(path_str(&data_dir)));
    assert!(!stdout.contains(path_str(&fixture_root)));
    assert!(!stdout.contains(path_str(&command)));
    assert!(!stdout.contains(path_str(&pdftoppm)));

    let store = ReadMetaStore::open_data_dir(&data_dir).unwrap();
    let scanned = scanned_document(&store);
    assert_eq!(scanned.status, DocumentStatus::Searchable);
    assert!(store.retryable_jobs().unwrap().is_empty());
    let cache_key = OcrPageCacheKey::new(
        scanned.content_hash.expect("content hash"),
        1,
        72,
        "eng",
        "balanced",
    )
    .unwrap();
    let cache_entry = store
        .ocr_page_cache_entry(&cache_key)
        .unwrap()
        .expect("OCR cache entry");
    assert_eq!(cache_entry.status(), OcrPageCacheStatus::Succeeded);
    assert_eq!(cache_entry.confidence(), Some(0.87));
    assert_eq!(
        cache_entry.engine_profile(),
        Some("fixture-pdftoppm-engine")
    );
    assert!(cache_entry
        .text()
        .unwrap()
        .contains("S91PdftoppmRenderedToken"));

    let search = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "search",
            "S91PdftoppmRenderedToken",
            "--top-k",
            "20",
        ])
        .output()
        .expect("run resume-cli search for pdftoppm OCR text");
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
    remove_dir(&fixture_root);
}

#[cfg(unix)]
#[test]
fn ocr_worker_uses_tesseract_for_rendered_image_before_indexing() {
    let Some(tesseract) = find_command("tesseract") else {
        eprintln!("skipping tesseract CLI worker witness because tesseract is not installed");
        return;
    };
    let Some(pango_view) = find_command("pango-view") else {
        eprintln!("skipping tesseract CLI worker witness because pango-view is not installed");
        return;
    };
    let data_dir = temp_dir("ocr-worker-tesseract-data");
    let private_document_path = seed_ocr_pdf_document_with_bytes(
        &data_dir,
        valid_blank_pdf_bytes(),
        "s92-cli-tesseract-content-hash",
        OcrFixturePublicationState::Ready,
    );
    let render_command = write_text_png_render_executable(
        "fixture-ocr-worker-tesseract-render",
        &pango_view,
        "SUMMARY\nS92 OCR TEST\nEXPERIENCE\nBuilt systems\nSKILLS\nSearch",
    );

    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "ocr-worker",
            "--once",
            "--tesseract-command",
            path_str(&tesseract),
            "--render-command",
            path_str(&render_command),
            "--engine-profile",
            "fixture-tesseract-engine",
            "--render-dpi",
            "200",
            "--page-timeout-ms",
            "10000",
        ])
        .output()
        .expect("run ocr worker with tesseract");

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
    assert!(!stdout.contains("S92 OCR TEST"));
    assert!(!stdout.contains(path_str(&data_dir)));
    assert!(!stdout.contains(path_str(&private_document_path)));
    assert!(!stdout.contains(path_str(&tesseract)));
    assert!(!stdout.contains(path_str(&render_command)));

    let store = ReadMetaStore::open_data_dir(&data_dir).unwrap();
    let scanned = scanned_document(&store);
    assert_eq!(scanned.status, DocumentStatus::Searchable);
    assert!(store.retryable_jobs().unwrap().is_empty());
    let cache_key = OcrPageCacheKey::new(
        scanned.content_hash.expect("content hash"),
        1,
        200,
        "eng",
        "balanced",
    )
    .unwrap();
    let cache_entry = store
        .ocr_page_cache_entry(&cache_key)
        .unwrap()
        .expect("OCR cache entry");
    assert_eq!(cache_entry.status(), OcrPageCacheStatus::Succeeded);
    assert_eq!(
        cache_entry.engine_profile(),
        Some("fixture-tesseract-engine")
    );
    let text = cache_entry.text().unwrap();
    assert!(text.contains("S92"), "OCR text: {text:?}");
    assert!(text.contains("OCR"), "OCR text: {text:?}");
    assert!(text.contains("TEST"), "OCR text: {text:?}");
    assert!(
        cache_entry
            .word_boxes()
            .iter()
            .any(|word_box| word_box.text() == "S92" && word_box.width() > 0),
        "OCR word boxes: {:?}",
        cache_entry.word_boxes()
    );

    let search = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "search",
            "S92",
            "--top-k",
            "20",
        ])
        .output()
        .expect("run resume-cli search for tesseract OCR text");
    assert!(search.status.success());
    assert!(search.stderr.is_empty());
    let search_stdout = String::from_utf8_lossy(&search.stdout);
    assert!(
        search_stdout.contains("results: 1"),
        "stdout:\n{search_stdout}"
    );
    assert!(search_stdout.contains("synthetic-scanned-resume.pdf"));
    assert!(!search_stdout.contains(path_str(&data_dir)));
    assert!(!search_stdout.contains(path_str(&private_document_path)));

    remove_dir(&data_dir);
}

#[cfg(unix)]
#[test]
fn ocr_worker_command_crash_becomes_permanent_after_max_attempts_without_leaks() {
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

    let store = ReadMetaStore::open_data_dir(&data_dir).unwrap();
    let scanned = scanned_document(&store);
    assert_eq!(scanned.status, DocumentStatus::OcrRequired);
    let jobs = store.retryable_jobs().unwrap();
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].status, IngestJobStatus::FailedRetryable);
    assert_eq!(jobs[0].attempt_count, 1);
    let job_id = jobs[0].id.clone();
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
    drop(store);

    for attempt in 2..=3 {
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
            .expect("rerun crashed ocr worker command");
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());

        let store = ReadMetaStore::open_data_dir(&data_dir).unwrap();
        let scanned = scanned_document(&store);
        let job = store.ingest_job_by_id(&job_id).unwrap().unwrap();
        let source_revision = SourceRevision::for_content(
            scanned.id.clone(),
            scanned
                .content_hash
                .as_deref()
                .unwrap()
                .parse::<ContentDigest>()
                .unwrap(),
            scanned.byte_size,
        );
        let classification = store
            .source_revision_triage(&source_revision.id, CLASSIFIER_EPOCH)
            .unwrap()
            .unwrap();
        assert_eq!(job.attempt_count, attempt);
        if attempt < 3 {
            assert_eq!(job.status, IngestJobStatus::FailedRetryable);
            assert_eq!(scanned.status, DocumentStatus::OcrRequired);
            assert_eq!(classification.status, ClassificationStatus::OcrBacklog);
        } else {
            assert_eq!(job.status, IngestJobStatus::FailedPermanent);
            assert_eq!(scanned.status, DocumentStatus::FailedPermanent);
            assert_eq!(classification.status, ClassificationStatus::OcrBacklog);
            assert_eq!(classification.reason_codes, vec![ReasonCode::OcrRequired]);
        }
    }

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
fn ocr_worker_blocks_missing_tesseract_language_before_invoking_engine_without_leaks() {
    let data_dir = temp_dir("ocr-worker-missing-lang-data");
    let fixture_root = fixture_root();
    let tesseract = write_fixture_executable(
        "fixture-ocr-worker-missing-lang-tesseract",
        r#"#!/bin/sh
if [ "$1" = "--list-langs" ]; then
  printf 'List of available languages (1):\n'
  printf 'eng\n'
  exit 0
fi
printf 'PRIVATE_TESSERACT_OCR_SHOULD_NOT_RUN\n' >&2
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
            "--tesseract-command",
            path_str(&tesseract),
            "--lang",
            "eng+chi_sim",
        ])
        .output()
        .expect("run OCR worker with missing Tesseract language pack");

    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("ocr worker blocked: requested OCR language pack is unavailable"));
    assert!(!stderr.contains("PRIVATE_TESSERACT_OCR_SHOULD_NOT_RUN"));
    assert!(!stderr.contains("chi_sim"));
    assert!(!stderr.contains(path_str(&data_dir)));
    assert!(!stderr.contains(path_str(&fixture_root)));
    assert!(!stderr.contains(path_str(&tesseract)));

    let store = ReadMetaStore::open_data_dir(&data_dir).unwrap();
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
        "eng+chi_sim",
        "balanced",
    )
    .unwrap();
    let cache_entry = store
        .ocr_page_cache_entry(&cache_key)
        .unwrap()
        .expect("OCR missing language cache entry");
    assert_eq!(cache_entry.status(), OcrPageCacheStatus::FailedRetryable);
    assert_eq!(cache_entry.text(), None);
    assert_eq!(cache_entry.error_kind(), Some("LanguageUnavailable"));

    let status = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args(["--data-dir", path_str(&data_dir), "status"])
        .output()
        .expect("run status after missing OCR language pack");
    assert!(status.status.success());
    assert!(status.stderr.is_empty());
    let status_stdout = String::from_utf8_lossy(&status.stdout);
    assert!(status_stdout.contains("ocr language unavailable: 1"));
    assert!(status_stdout.contains(
        "ocr language remediation: install requested OCR language packs or choose an installed OCR language"
    ));
    assert!(!status_stdout.contains("PRIVATE_TESSERACT_OCR_SHOULD_NOT_RUN"));
    assert!(!status_stdout.contains("chi_sim"));
    assert!(!status_stdout.contains(path_str(&data_dir)));
    assert!(!status_stdout.contains(path_str(&fixture_root)));
    assert!(!status_stdout.contains(path_str(&tesseract)));

    let doctor = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args(["--data-dir", path_str(&data_dir), "doctor"])
        .output()
        .expect("run doctor after missing OCR language pack");
    assert!(doctor.status.success());
    assert!(doctor.stderr.is_empty());
    let doctor_stdout = String::from_utf8_lossy(&doctor.stdout);
    assert!(doctor_stdout.contains("ocr language unavailable: 1"));
    assert!(doctor_stdout.contains(
        "ocr language remediation: install requested OCR language packs or choose an installed OCR language"
    ));
    assert!(!doctor_stdout.contains("PRIVATE_TESSERACT_OCR_SHOULD_NOT_RUN"));
    assert!(!doctor_stdout.contains("chi_sim"));
    assert!(!doctor_stdout.contains(path_str(&data_dir)));
    assert!(!doctor_stdout.contains(path_str(&fixture_root)));
    assert!(!doctor_stdout.contains(path_str(&tesseract)));

    let diagnostics = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "export-diagnostics",
            "--redact",
        ])
        .output()
        .expect("run diagnostics after missing OCR language pack");
    assert!(diagnostics.status.success());
    assert!(diagnostics.stderr.is_empty());
    let diagnostics_stdout = String::from_utf8_lossy(&diagnostics.stdout);
    assert!(diagnostics_stdout.contains("\"ocr_language_unavailable\": 1"));
    assert!(diagnostics_stdout.contains(
        "\"ocr_language_remediation\": \"install requested OCR language packs or choose an installed OCR language\""
    ));
    assert!(!diagnostics_stdout.contains("PRIVATE_TESSERACT_OCR_SHOULD_NOT_RUN"));
    assert!(!diagnostics_stdout.contains("chi_sim"));
    assert!(!diagnostics_stdout.contains(path_str(&data_dir)));
    assert!(!diagnostics_stdout.contains(path_str(&fixture_root)));
    assert!(!diagnostics_stdout.contains(path_str(&tesseract)));

    remove_dir(&data_dir);
}

#[cfg(unix)]
#[test]
fn ocr_worker_indexes_succeeded_cache_hit_without_invoking_command() {
    let data_dir = temp_dir("ocr-worker-cache-hit-data");
    let fixture_root = fixture_root();
    import_fixtures(&data_dir, &fixture_root);

    {
        let store = create_owned_store(&data_dir);
        let scanned = scanned_document_from_owned(&store);
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
            "SUMMARY\nSynthetic OCR fixture.\nEXPERIENCE\nBuilt systems.\nSKILLS\nSearch.\nOCRS41CacheHitToken cached OCR text",
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

    let store = ReadMetaStore::open_data_dir(&data_dir).unwrap();
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
fn ocr_worker_empty_success_excludes_document_without_publishing_a_version() {
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

    let store = ReadMetaStore::open_data_dir(&data_dir).unwrap();
    let scanned = scanned_document(&store);
    assert_eq!(scanned.status, DocumentStatus::Excluded);
    assert!(store
        .active_search_projection_for_document(&scanned.id)
        .unwrap()
        .is_none());
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

fn scanned_document(store: &ReadMetaStore) -> meta_store::Document {
    store
        .visible_documents()
        .unwrap()
        .into_iter()
        .find(|document| document.file_name == "synthetic-scanned-resume.pdf")
        .expect("scanned synthetic fixture is persisted")
}

fn scanned_document_from_owned(store: &OwnedMetaStore) -> meta_store::Document {
    store
        .visible_documents()
        .unwrap()
        .into_iter()
        .find(|document| document.file_name == "synthetic-scanned-resume.pdf")
        .expect("scanned synthetic fixture is persisted")
}

fn create_owned_store(data_dir: &Path) -> OwnedMetaStore {
    let owner = match DataDirectoryOwnerLease::try_acquire(data_dir).unwrap() {
        DataDirectoryOwnerAcquisition::Acquired(owner) => owner,
        DataDirectoryOwnerAcquisition::Contended => panic!("test store owner contended"),
    };
    owner.open_store().unwrap()
}

fn fixture_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("tests/fixtures/resumes")
}

fn two_page_scanned_pdf_bytes() -> Vec<u8> {
    build_valid_pdf(vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /Resources << /XObject << /Im1 5 0 R >> >> /MediaBox [0 0 72 72] /Contents 7 0 R >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /Resources << /XObject << /Im2 6 0 R >> >> /MediaBox [0 0 72 72] /Contents 8 0 R >>".to_vec(),
        pdf_stream_object(
            b"1111".to_vec(),
            Some(
                b"/Type /XObject /Subtype /Image /Width 10 /Height 10 /ColorSpace /DeviceGray /BitsPerComponent 8"
                    .to_vec(),
            ),
        ),
        pdf_stream_object(
            b"2222".to_vec(),
            Some(
                b"/Type /XObject /Subtype /Image /Width 10 /Height 10 /ColorSpace /DeviceGray /BitsPerComponent 8"
                    .to_vec(),
            ),
        ),
        pdf_stream_object(b"q 10 0 0 10 0 0 cm /Im1 Do Q\n".to_vec(), None),
        pdf_stream_object(b"q 10 0 0 10 0 0 cm /Im2 Do Q\n".to_vec(), None),
    ])
}

#[cfg(unix)]
fn find_command(name: &str) -> Option<PathBuf> {
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths)
            .map(|path| path.join(name))
            .find(|path| path.exists())
    })
}

#[cfg(unix)]
fn valid_blank_pdf_bytes() -> Vec<u8> {
    build_valid_pdf(vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 72 72] /Resources << >> >>".to_vec(),
    ])
}

fn build_valid_pdf(objects: Vec<Vec<u8>>) -> Vec<u8> {
    let object_count = objects.len();
    let mut output = Vec::new();
    output.extend_from_slice(b"%PDF-1.4\n");
    let mut offsets = Vec::with_capacity(objects.len());
    for (index, object) in objects.into_iter().enumerate() {
        offsets.push(output.len());
        output.extend_from_slice(format!("{} 0 obj\n", index + 1).as_bytes());
        output.extend_from_slice(&object);
        if !object.ends_with(b"\n") {
            output.push(b'\n');
        }
        output.extend_from_slice(b"endobj\n");
    }
    let xref = output.len();
    output.extend_from_slice(format!("xref\n0 {}\n", offsets.len() + 1).as_bytes());
    output.extend_from_slice(b"0000000000 65535 f\r\n");
    for offset in offsets {
        output.extend_from_slice(format!("{offset:010} 00000 n\r\n").as_bytes());
    }
    output.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            object_count + 1
        )
        .as_bytes(),
    );
    output
}

fn pdf_stream_object(payload: Vec<u8>, extra_dict: Option<Vec<u8>>) -> Vec<u8> {
    let mut object = Vec::new();
    object.extend_from_slice(b"<< ");
    if let Some(extra_dict) = extra_dict {
        object.extend_from_slice(&extra_dict);
        object.extend_from_slice(b" /Length ");
    } else {
        object.extend_from_slice(b"/Length ");
    }
    object.extend_from_slice(payload.len().to_string().as_bytes());
    object.extend_from_slice(b" >>\nstream\n");
    object.extend_from_slice(&payload);
    if !payload.ends_with(b"\n") {
        object.push(b'\n');
    }
    object.extend_from_slice(b"endstream");
    object
}

fn seed_ocr_pdf_document_with_bytes(
    data_dir: &Path,
    bytes: Vec<u8>,
    content_hash: &str,
    publication_state: OcrFixturePublicationState,
) -> PathBuf {
    let now = UnixTimestamp::from_unix_seconds(1_900_000_092);
    let content_digest = ContentDigest::from_bytes(&bytes);
    let private_root = data_dir.join("private-resumes");
    std::fs::create_dir_all(&private_root).unwrap();
    let document_path = private_root.join("synthetic-scanned-resume.pdf");
    std::fs::write(&document_path, &bytes).unwrap();
    let owner = match DataDirectoryOwnerLease::try_acquire(data_dir).unwrap() {
        DataDirectoryOwnerAcquisition::Acquired(owner) => owner,
        DataDirectoryOwnerAcquisition::Contended => panic!("synthetic data dir is unowned"),
    };
    let store = owner.open_store().unwrap();
    if publication_state == OcrFixturePublicationState::Ready {
        let contract = current_import_processing_contract(&ImportOptions::default()).unwrap();
        store
            .activate_migration_rebuild_contract(&contract, now)
            .unwrap();
        prepare_migration_rebuild_artifacts(
            &store,
            now,
            &import_pipeline::PipelineRunControl::default(),
        )
        .unwrap();
        finalize_migration_rebuild(
            &store,
            now,
            &contract,
            &SearchPublicationVectorization::default(),
            &import_pipeline::PipelineRunControl::default(),
        )
        .unwrap();
    }
    let doc_id = DocumentId::from_non_secret_parts(&["s15", content_hash]);
    store
        .upsert_document(&Document {
            id: doc_id.clone(),
            source_uri: format!("file://{}", path_str(&document_path)),
            normalized_path: path_str(&document_path).to_string(),
            file_name: "synthetic-scanned-resume.pdf".to_string(),
            extension: FileExtension::Pdf,
            byte_size: std::fs::metadata(&document_path).unwrap().len(),
            mtime: now,
            content_hash: Some(content_digest.as_str().to_string()),
            text_hash: None,
            is_deleted: false,
            created_at: now,
            updated_at: now,
            status: DocumentStatus::OcrRequired,
        })
        .unwrap();
    let source_revision = SourceRevision::for_content(
        doc_id.clone(),
        content_digest,
        std::fs::metadata(&document_path).unwrap().len(),
    );
    store.insert_source_revision(&source_revision).unwrap();
    store
        .insert_source_revision_triage(&SourceRevisionTriage {
            source_revision_id: source_revision.id.clone(),
            status: ClassificationStatus::OcrBacklog,
            triage_epoch: CLASSIFIER_EPOCH.to_string(),
            reason_codes: vec![ReasonCode::OcrRequired],
            triaged_at: now,
        })
        .unwrap();
    store
        .enqueue_ocr_job_for_source_triage(
            &source_revision.id,
            CurrentClassifierEpoch::parse(CLASSIFIER_EPOCH).unwrap(),
            now,
        )
        .unwrap();
    document_path
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum OcrFixturePublicationState {
    Unpublished,
    Ready,
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

fn bytes_contain(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
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

#[cfg(unix)]
fn write_text_png_render_executable(name: &str, pango_view: &Path, text: &str) -> PathBuf {
    let body = format!(
        r#"#!/bin/sh
set -eu
image="${{TMPDIR:-/tmp}}/resume-ir-s92-render-$$.png"
trap 'rm -f "$image"' EXIT
{} -q --font='Verdana Bold 48' --background=white --foreground=black --text={} --output="$image" >/dev/null 2>&1
cat "$image"
"#,
        shell_quote(path_str(pango_view)),
        shell_quote(text)
    );
    write_fixture_executable(name, &body)
}

#[cfg(unix)]
fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

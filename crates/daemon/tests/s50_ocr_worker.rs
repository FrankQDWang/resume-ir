use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use import_pipeline::{import_root_with_options, ImportOptions};
use meta_store::{
    DataDirectoryOwnerAcquisition, DataDirectoryOwnerLease, DocumentStatus, ImportTask,
    ImportTaskId, ImportTaskStatus, IngestJobFailureKind, IngestJobStatus, OcrPageCacheKey,
    OcrPageCacheStatus, OwnedMetaStore, ReadMetaStore, UnixTimestamp, WorkerTaskKind,
};
use search_runtime::{HitLimit, QueryCoordinator};
use serde_json::json;
use sha2::{Digest, Sha256};

mod support;

#[cfg(unix)]
#[test]
fn daemon_ocr_worker_reuses_the_bundled_classifier_model() {
    let data_dir = temp_dir("ocr-worker-classifier-data");
    seed_scanned_document(&data_dir);
    let command = write_fixture_executable(
        "fixture-daemon-ocr-classifier",
        r#"#!/bin/sh
printf 'resume-ir-ocr-v1\n'
printf 'confidence=0.80\n'
printf 'text:\n'
printf 'PROFILE\n'
printf 'Platform engineer with Rust experience.\n'
printf 'INVOICE\n'
"#,
    );
    let model = data_dir.join("bundled-classifier-model.json");
    write_synthetic_bundled_model(&model);

    let output = Command::new(env!("CARGO_BIN_EXE_resume-daemon"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "run",
            "--foreground",
            "--once",
            "--work-ocr-once",
            "--ocr-command",
            path_str(&command),
            "--resume-classifier-model",
            path_str(&model),
        ])
        .output()
        .expect("run daemon OCR worker with bundled classifier model");

    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    assert_eq!(
        scanned_document(&ReadMetaStore::open_data_dir(&data_dir).unwrap()).status,
        DocumentStatus::Searchable
    );

    remove_dir(&data_dir);
}

#[cfg(unix)]
#[test]
fn daemon_ocr_worker_once_executes_local_command_and_indexes_scanned_pdf() {
    let data_dir = temp_dir("ocr-worker-once-data");
    let private_document_path = seed_scanned_document(&data_dir);
    let command = write_fixture_executable(
        "fixture-daemon-ocr-worker-once",
        r#"#!/bin/sh
input_size="$(wc -c < "$RESUME_IR_OCR_INPUT_PATH" | tr -d ' ')"
printf 'resume-ir-ocr-v1\n'
printf 'confidence=0.71\n'
printf 'text:\n'
printf 'SUMMARY\n'
printf 'Synthetic OCR platform engineer.\n'
printf 'EXPERIENCE\n'
printf 'Built OCRS50DaemonOnceToken worker bytes=%s page=%s.\n' "$input_size" "$RESUME_IR_OCR_PAGE_NO"
printf 'SKILLS\n'
printf 'Rust search systems.\n'
"#,
    );

    let output = Command::new(env!("CARGO_BIN_EXE_resume-daemon"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "run",
            "--foreground",
            "--once",
            "--work-ocr-once",
            "--ocr-command",
            path_str(&command),
            "--ocr-engine-profile",
            "fixture-daemon-engine",
        ])
        .output()
        .expect("run daemon OCR worker once");

    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("ocr worker processed: 1"));
    assert!(stdout.contains("ocr worker cache writes: 1"));
    assert!(stdout.contains("ocr worker cache hits: 0"));
    assert!(stdout.contains("ocr worker failed: 0"));
    assert!(!stdout.contains("OCRS50DaemonOnceToken"));
    assert!(!stdout.contains(path_str(&data_dir)));
    assert!(!stdout.contains(path_str(&private_document_path)));
    assert!(!stdout.contains(path_str(&command)));

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
    assert_eq!(cache_entry.confidence(), Some(0.71));
    assert_eq!(cache_entry.engine_profile(), Some("fixture-daemon-engine"));
    assert!(cache_entry
        .text()
        .unwrap()
        .contains("OCRS50DaemonOnceToken"));

    let hits = search_fulltext(&data_dir, "OCRS50DaemonOnceToken");
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].file_name, "synthetic-scanned-resume.pdf");

    remove_dir(&data_dir);
}

#[cfg(unix)]
#[test]
fn daemon_ocr_publication_failure_keeps_the_exact_claim_retryable() {
    let data_dir = temp_dir("ocr-worker-publication-failure-data");
    let private_document_path = seed_scanned_document(&data_dir);
    let store = ReadMetaStore::open_data_dir(&data_dir).unwrap();
    let document = scanned_document(&store);
    let head_before = store.search_projection_state().unwrap();
    drop(store);

    let quoted_data_dir = shell_quote(path_str(&data_dir));
    let command = write_fixture_executable(
        "fixture-daemon-ocr-publication-failure",
        &format!(
            r#"#!/bin/sh
mv {quoted_data_dir}/search-index {quoted_data_dir}/search-index-valid
printf 'not-a-directory' > {quoted_data_dir}/search-index
printf 'resume-ir-ocr-v1\n'
printf 'confidence=0.71\n'
printf 'text:\n'
printf 'SUMMARY\n'
printf 'Synthetic OCR platform engineer.\n'
printf 'EXPERIENCE\n'
printf 'Built OCRS50PublicationFailureToken worker.\n'
printf 'SKILLS\n'
printf 'Rust search systems.\n'
"#
        ),
    );

    let output = Command::new(env!("CARGO_BIN_EXE_resume-daemon"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "run",
            "--foreground",
            "--once",
            "--work-ocr-once",
            "--ocr-command",
            path_str(&command),
            "--ocr-engine-profile",
            "fixture-daemon-engine",
        ])
        .output()
        .expect("run daemon OCR worker with publication failure");

    assert!(!output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    for private_value in [
        "OCRS50PublicationFailureToken",
        path_str(&data_dir),
        path_str(&private_document_path),
        path_str(&command),
    ] {
        assert!(!stdout.contains(private_value));
        assert!(!stderr.contains(private_value));
    }

    let store = ReadMetaStore::open_data_dir(&data_dir).unwrap();
    assert_eq!(
        store.document_by_id(&document.id).unwrap().unwrap().status,
        DocumentStatus::OcrRequired
    );
    assert!(store
        .resume_versions_for_document(&document.id)
        .unwrap()
        .is_empty());
    assert_eq!(
        store
            .active_search_projection_for_document(&document.id)
            .unwrap(),
        None
    );
    let retryable = store.retryable_jobs().unwrap();
    assert_eq!(retryable.len(), 1);
    assert_eq!(retryable[0].status, IngestJobStatus::FailedRetryable);
    assert_eq!(retryable[0].resume_version_id, None);
    let head_after = store.search_projection_state().unwrap();
    assert_eq!(head_after.generation, head_before.generation);
    assert_eq!(head_after.visible_epoch, head_before.visible_epoch);

    remove_dir(&data_dir);
}

fn write_synthetic_bundled_model(path: &Path) {
    let model = json!({
        "schema": "resume_ir_linear_promotion_v1",
        "classifier_epoch": "precision_first_v4",
        "feature_contract": "bounded_normalized_text_plus_structure_v1",
        "max_input_chars": 128,
        "threshold": 0.7,
        "intercept": 0.0,
        "features": [{"ngram": "pla", "idf": 1.0, "coefficient": 12.0}]
    });
    let model_json = serde_json::to_string(&model).unwrap();
    let model_sha256 = format!("{:x}", Sha256::digest(model_json.as_bytes()));
    fs::write(
        path,
        serde_json::to_vec(&json!({
            "model_json": model_json,
            "model_sha256": model_sha256
        }))
        .unwrap(),
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o644)).unwrap();
    }
}

#[cfg(unix)]
#[test]
fn daemon_ocr_worker_once_recovers_stale_running_job_after_restart() {
    let data_dir = temp_dir("ocr-worker-stale-running-recovery-data");
    let private_document_path = seed_scanned_document(&data_dir);
    let store = open_owned_store(&data_dir);
    let stale_claimed = store
        .claim_next_ocr_job(UnixTimestamp::from_unix_seconds(1_700_050_010))
        .unwrap()
        .expect("seed stale running OCR job");
    assert_eq!(stale_claimed.job.status, IngestJobStatus::Running);
    drop(store);

    let command = write_fixture_executable(
        "fixture-daemon-ocr-worker-stale-recovery",
        r#"#!/bin/sh
printf 'resume-ir-ocr-v1\n'
printf 'confidence=0.77\n'
printf 'text:\n'
printf 'SUMMARY\n'
printf 'Synthetic recovered OCR engineer.\n'
printf 'EXPERIENCE\n'
printf 'Built S50RecoveredStaleOcrJobToken page %s search services.\n' "$RESUME_IR_OCR_PAGE_NO"
printf 'SKILLS\n'
printf 'Rust recovery systems.\n'
"#,
    );

    let output = Command::new(env!("CARGO_BIN_EXE_resume-daemon"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "run",
            "--foreground",
            "--once",
            "--work-ocr-once",
            "--ocr-command",
            path_str(&command),
            "--ocr-engine-profile",
            "fixture-daemon-recovery-engine",
        ])
        .output()
        .expect("run daemon OCR worker once after stale running job");

    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("ingest jobs recovered stale running: 1"));
    assert!(stdout.contains("ocr worker processed: 1"));
    assert!(stdout.contains("ocr worker failed: 0"));
    assert!(!stdout.contains("S50RecoveredStaleOcrJobToken"));
    assert!(!stdout.contains(path_str(&data_dir)));
    assert!(!stdout.contains(path_str(&private_document_path)));
    assert!(!stdout.contains(path_str(&command)));

    let store = ReadMetaStore::open_data_dir(&data_dir).unwrap();
    let recovered_job = store
        .ingest_job_by_id(&stale_claimed.job.id)
        .unwrap()
        .expect("recovered OCR job remains persisted");
    assert_eq!(recovered_job.status, IngestJobStatus::Completed);
    assert_eq!(recovered_job.attempt_count, 2);
    let scanned = scanned_document(&store);
    assert_eq!(scanned.status, DocumentStatus::Searchable);
    assert!(search_fulltext(&data_dir, "S50RecoveredStaleOcrJobToken").len() == 1);

    remove_dir(&data_dir);
}

#[cfg(unix)]
#[test]
fn daemon_ocr_worker_once_renders_and_ocr_all_scanned_pdf_pages() {
    let data_dir = temp_dir("ocr-worker-render-multi-page-data");
    let private_document_path =
        seed_scanned_document_with_bytes(&data_dir, two_page_scanned_pdf_bytes());
    let render_command = write_fixture_executable(
        "fixture-daemon-pdf-render-multi-page",
        r#"#!/bin/sh
case "$RESUME_IR_PDF_RENDER_PAGE_NO" in
  1) printf 'S89_DAEMON_RENDERED_PAGE_1_BYTES' ;;
  2) printf 'S89_DAEMON_RENDERED_PAGE_2_BYTES' ;;
  *) printf 'PRIVATE_DAEMON_UNEXPECTED_RENDER_PAGE_%s\n' "$RESUME_IR_PDF_RENDER_PAGE_NO"; exit 23 ;;
esac
"#,
    );
    let command = write_fixture_executable(
        "fixture-daemon-ocr-worker-multi-page",
        r#"#!/bin/sh
input_bytes="$(cat "$RESUME_IR_OCR_INPUT_PATH")"
printf 'resume-ir-ocr-v1\n'
printf 'confidence=0.84\n'
printf 'text:\n'
case "$input_bytes:$RESUME_IR_OCR_PAGE_NO" in
  S89_DAEMON_RENDERED_PAGE_1_BYTES:1) printf 'SUMMARY\nSynthetic multi-page OCR engineer.\nEXPERIENCE\nBuilt S89DaemonPageOneToken search services.\n' ;;
  S89_DAEMON_RENDERED_PAGE_2_BYTES:2) printf 'SKILLS\nRust indexing with S89DaemonPageTwoToken.\n' ;;
  *) printf 'PRIVATE_DAEMON_UNEXPECTED_OCR_INPUT_%s_PAGE_%s\n' "$input_bytes" "$RESUME_IR_OCR_PAGE_NO"; exit 19 ;;
esac
"#,
    );

    let output = Command::new(env!("CARGO_BIN_EXE_resume-daemon"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "run",
            "--foreground",
            "--once",
            "--work-ocr-once",
            "--ocr-command",
            path_str(&command),
            "--ocr-render-command",
            path_str(&render_command),
            "--ocr-engine-profile",
            "fixture-daemon-render-engine",
        ])
        .output()
        .expect("run daemon OCR worker once");

    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("ocr worker processed: 1"));
    assert!(stdout.contains("ocr worker cache writes: 2"));
    assert!(stdout.contains("ocr worker cache hits: 0"));
    assert!(stdout.contains("ocr worker failed: 0"));
    assert!(!stdout.contains("S89DaemonPageOneToken"));
    assert!(!stdout.contains("S89DaemonPageTwoToken"));
    assert!(!stdout.contains("PRIVATE_DAEMON_UNEXPECTED"));
    assert!(!stdout.contains(path_str(&data_dir)));
    assert!(!stdout.contains(path_str(&private_document_path)));
    assert!(!stdout.contains(path_str(&command)));
    assert!(!stdout.contains(path_str(&render_command)));

    let store = ReadMetaStore::open_data_dir(&data_dir).unwrap();
    let scanned = scanned_document(&store);
    assert_eq!(scanned.status, DocumentStatus::Searchable);
    assert!(store.retryable_jobs().unwrap().is_empty());
    let content_hash = scanned.content_hash.clone().expect("content hash");
    for (page_no, token) in [(1, "S89DaemonPageOneToken"), (2, "S89DaemonPageTwoToken")] {
        let cache_key =
            OcrPageCacheKey::new(content_hash.clone(), page_no, 300, "eng", "balanced").unwrap();
        let cache_entry = store
            .ocr_page_cache_entry(&cache_key)
            .unwrap()
            .expect("OCR cache entry");
        assert_eq!(cache_entry.status(), OcrPageCacheStatus::Succeeded);
        assert_eq!(cache_entry.confidence(), Some(0.84));
        assert_eq!(
            cache_entry.engine_profile(),
            Some("fixture-daemon-render-engine")
        );
        assert!(cache_entry.text().unwrap().contains(token));
    }
    let version = active_resume_version(&store, &scanned.id);
    assert_eq!(version.page_count, Some(2));
    let clean_text = version.clean_text.unwrap();
    assert!(clean_text.contains("S89DaemonPageOneToken"));
    assert!(clean_text.contains("S89DaemonPageTwoToken"));
    assert_eq!(version.raw_text, None);

    assert_eq!(search_fulltext(&data_dir, "S89DaemonPageOneToken").len(), 1);
    assert_eq!(search_fulltext(&data_dir, "S89DaemonPageTwoToken").len(), 1);

    remove_dir(&data_dir);
}

#[cfg(unix)]
#[test]
fn daemon_ocr_worker_once_backpressures_scanned_pdf_above_page_limit_without_invoking_ocr() {
    let data_dir = temp_dir("ocr-worker-backpressure-data");
    let private_document_path =
        seed_scanned_document_with_bytes(&data_dir, two_page_scanned_pdf_bytes());
    let command = write_fixture_executable(
        "fixture-daemon-ocr-worker-backpressure",
        r#"#!/bin/sh
printf 'PRIVATE_DAEMON_OCR_BACKPRESSURE_INVOKED\n'
exit 31
"#,
    );

    let output = Command::new(env!("CARGO_BIN_EXE_resume-daemon"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "run",
            "--foreground",
            "--once",
            "--work-ocr-once",
            "--ocr-command",
            path_str(&command),
            "--ocr-max-pages-per-document",
            "1",
        ])
        .output()
        .expect("run daemon OCR worker once with page-count backpressure");

    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("ocr worker processed: 0"));
    assert!(stdout.contains("ocr worker cache writes: 0"));
    assert!(stdout.contains("ocr worker cache hits: 0"));
    assert!(stdout.contains("ocr worker failed: 1"));
    assert!(!stdout.contains("PRIVATE_DAEMON_OCR_BACKPRESSURE_INVOKED"));
    assert!(!stdout.contains(path_str(&data_dir)));
    assert!(!stdout.contains(path_str(&private_document_path)));
    assert!(!stdout.contains(path_str(&command)));

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
    let content_hash = scanned.content_hash.expect("content hash");
    for page_no in [1, 2] {
        let cache_key =
            OcrPageCacheKey::new(content_hash.clone(), page_no, 300, "eng", "balanced").unwrap();
        assert!(store.ocr_page_cache_entry(&cache_key).unwrap().is_none());
    }

    remove_dir(&data_dir);
}

#[cfg(unix)]
#[test]
fn daemon_ocr_worker_once_uses_pdftoppm_renderer_for_valid_pdf_before_ocr() {
    let Some(pdftoppm) = find_command("pdftoppm") else {
        eprintln!("skipping pdftoppm daemon worker witness because pdftoppm is not installed");
        return;
    };
    let data_dir = temp_dir("ocr-worker-pdftoppm-data");
    let private_document_path =
        seed_scanned_document_with_bytes(&data_dir, valid_blank_pdf_bytes());
    let command = write_fixture_executable(
        "fixture-daemon-ocr-worker-pdftoppm",
        r#"#!/bin/sh
header="$(head -c 2 "$RESUME_IR_OCR_INPUT_PATH")"
if [ "$header" != "P6" ]; then
  printf 'PRIVATE_DAEMON_UNEXPECTED_PDFFTOPPM_OCR_INPUT_%s\n' "$header"
  exit 19
fi
if [ "$RESUME_IR_OCR_PAGE_NO" != "1" ]; then
  printf 'PRIVATE_DAEMON_UNEXPECTED_PDFFTOPPM_OCR_PAGE_%s\n' "$RESUME_IR_OCR_PAGE_NO"
  exit 20
fi
printf 'resume-ir-ocr-v1\n'
printf 'confidence=0.89\n'
printf 'text:\n'
printf 'SUMMARY\n'
printf 'Synthetic rendered OCR engineer.\n'
printf 'EXPERIENCE\n'
printf 'Built S91DaemonPdftoppmRenderedToken search services.\n'
printf 'SKILLS\n'
printf 'Rust PDF indexing.\n'
"#,
    );

    let output = Command::new(env!("CARGO_BIN_EXE_resume-daemon"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "run",
            "--foreground",
            "--once",
            "--work-ocr-once",
            "--ocr-command",
            path_str(&command),
            "--ocr-pdftoppm-command",
            path_str(&pdftoppm),
            "--ocr-engine-profile",
            "fixture-daemon-pdftoppm-engine",
            "--ocr-render-dpi",
            "72",
        ])
        .output()
        .expect("run daemon OCR worker once with pdftoppm renderer");

    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("ocr worker processed: 1"));
    assert!(stdout.contains("ocr worker cache writes: 1"));
    assert!(stdout.contains("ocr worker cache hits: 0"));
    assert!(stdout.contains("ocr worker failed: 0"));
    assert!(!stdout.contains("S91DaemonPdftoppmRenderedToken"));
    assert!(!stdout.contains("PRIVATE_DAEMON_UNEXPECTED_PDFFTOPPM"));
    assert!(!stdout.contains(path_str(&data_dir)));
    assert!(!stdout.contains(path_str(&private_document_path)));
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
    assert_eq!(cache_entry.confidence(), Some(0.89));
    assert_eq!(
        cache_entry.engine_profile(),
        Some("fixture-daemon-pdftoppm-engine")
    );
    assert!(cache_entry
        .text()
        .unwrap()
        .contains("S91DaemonPdftoppmRenderedToken"));
    let version = active_resume_version(&store, &scanned.id);
    assert_eq!(version.page_count, Some(1));
    assert!(version
        .clean_text
        .unwrap()
        .contains("S91DaemonPdftoppmRenderedToken"));
    assert_eq!(
        search_fulltext(&data_dir, "S91DaemonPdftoppmRenderedToken").len(),
        1
    );

    remove_dir(&data_dir);
}

#[cfg(unix)]
#[test]
fn daemon_ocr_worker_once_uses_tesseract_for_rendered_image_before_indexing() {
    let Some(tesseract) = find_command("tesseract") else {
        eprintln!("skipping tesseract daemon worker witness because tesseract is not installed");
        return;
    };
    let Some(pango_view) = find_command("pango-view") else {
        eprintln!("skipping tesseract daemon worker witness because pango-view is not installed");
        return;
    };
    let data_dir = temp_dir("ocr-worker-tesseract-data");
    let private_document_path =
        seed_scanned_document_with_bytes(&data_dir, valid_blank_pdf_bytes());
    let render_command = write_text_png_render_executable(
        "fixture-daemon-ocr-worker-tesseract-render",
        &pango_view,
        "SUMMARY\nS92 OCR TEST\nEXPERIENCE\nBuilt SEARCH SYSTEMS\nSKILLS\nRUST",
    );

    let output = Command::new(env!("CARGO_BIN_EXE_resume-daemon"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "run",
            "--foreground",
            "--once",
            "--work-ocr-once",
            "--ocr-tesseract-command",
            path_str(&tesseract),
            "--ocr-render-command",
            path_str(&render_command),
            "--ocr-engine-profile",
            "fixture-daemon-tesseract-engine",
            "--ocr-render-dpi",
            "200",
            "--ocr-page-timeout-ms",
            "10000",
        ])
        .output()
        .expect("run daemon OCR worker once with tesseract");

    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("ocr worker processed: 1"));
    assert!(stdout.contains("ocr worker cache writes: 1"));
    assert!(stdout.contains("ocr worker cache hits: 0"));
    assert!(stdout.contains("ocr worker failed: 0"));
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
        Some("fixture-daemon-tesseract-engine")
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
    assert_eq!(search_fulltext(&data_dir, "S92").len(), 1);

    remove_dir(&data_dir);
}

#[cfg(unix)]
#[test]
fn daemon_ocr_worker_command_crash_becomes_permanent_after_max_attempts_without_leaks() {
    let data_dir = temp_dir("ocr-worker-crash-data");
    let private_document_path = seed_scanned_document(&data_dir);
    let command = write_fixture_executable(
        "fixture-daemon-ocr-worker-crash",
        r#"#!/bin/sh
printf 'PRIVATE_DAEMON_OCR_CRASH_STDOUT\n'
printf 'PRIVATE_DAEMON_OCR_CRASH_STDERR\n' >&2
exit 17
"#,
    );

    let output = Command::new(env!("CARGO_BIN_EXE_resume-daemon"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "run",
            "--foreground",
            "--once",
            "--work-ocr-once",
            "--ocr-command",
            path_str(&command),
        ])
        .output()
        .expect("run daemon OCR worker once with crashing command");

    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("ocr worker processed: 0"));
    assert!(stdout.contains("ocr worker cache writes: 0"));
    assert!(stdout.contains("ocr worker cache hits: 0"));
    assert!(stdout.contains("ocr worker failed: 1"));
    assert!(!stdout.contains("PRIVATE_DAEMON_OCR_CRASH_STDOUT"));
    assert!(!stdout.contains("PRIVATE_DAEMON_OCR_CRASH_STDERR"));
    assert!(!stdout.contains(path_str(&data_dir)));
    assert!(!stdout.contains(path_str(&private_document_path)));
    assert!(!stdout.contains(path_str(&command)));

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
        let output = Command::new(env!("CARGO_BIN_EXE_resume-daemon"))
            .args([
                "--data-dir",
                path_str(&data_dir),
                "run",
                "--foreground",
                "--once",
                "--work-ocr-once",
                "--ocr-command",
                path_str(&command),
            ])
            .output()
            .expect("rerun daemon OCR worker with crashing command");
        assert!(output.status.success());
        assert!(output.stderr.is_empty());
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(stdout.contains("ocr worker failed: 1"));

        let store = ReadMetaStore::open_data_dir(&data_dir).unwrap();
        let scanned = scanned_document(&store);
        let job = store.ingest_job_by_id(&job_id).unwrap().unwrap();
        assert_eq!(job.attempt_count, attempt);
        if attempt < 3 {
            assert_eq!(job.status, IngestJobStatus::FailedRetryable);
            assert_eq!(scanned.status, DocumentStatus::OcrRequired);
        } else {
            assert_eq!(job.status, IngestJobStatus::FailedPermanent);
            assert_eq!(scanned.status, DocumentStatus::FailedPermanent);
        }
    }

    remove_dir(&data_dir);
}

#[cfg(unix)]
#[test]
fn daemon_ocr_worker_once_blocks_missing_tesseract_language_before_engine_without_leaks() {
    let data_dir = temp_dir("ocr-worker-missing-lang-data");
    let private_document_path = seed_scanned_document(&data_dir);
    let tesseract = write_fixture_executable(
        "fixture-daemon-ocr-worker-missing-lang-tesseract",
        r#"#!/bin/sh
if [ "$1" = "--list-langs" ]; then
  printf 'List of available languages (1):\n'
  printf 'eng\n'
  exit 0
fi
printf 'PRIVATE_DAEMON_TESSERACT_OCR_SHOULD_NOT_RUN\n' >&2
exit 17
"#,
    );

    let output = Command::new(env!("CARGO_BIN_EXE_resume-daemon"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "run",
            "--foreground",
            "--once",
            "--work-ocr-once",
            "--ocr-tesseract-command",
            path_str(&tesseract),
            "--ocr-lang",
            "eng+chi_sim",
        ])
        .output()
        .expect("run daemon OCR worker once with missing Tesseract language pack");

    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("ocr worker processed: 0"));
    assert!(stdout.contains("ocr worker cache writes: 0"));
    assert!(stdout.contains("ocr worker cache hits: 0"));
    assert!(stdout.contains("ocr worker failed: 1"));
    assert!(!stdout.contains("PRIVATE_DAEMON_TESSERACT_OCR_SHOULD_NOT_RUN"));
    assert!(!stdout.contains("chi_sim"));
    assert!(!stdout.contains(path_str(&data_dir)));
    assert!(!stdout.contains(path_str(&private_document_path)));
    assert!(!stdout.contains(path_str(&tesseract)));

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

    remove_dir(&data_dir);
}

#[cfg(unix)]
#[test]
fn daemon_ocr_worker_once_respects_pause_without_claiming_or_invoking_command() {
    let data_dir = temp_dir("ocr-worker-paused-data");
    let private_document_path = seed_scanned_document(&data_dir);
    let missing_command = data_dir.join("private-bin").join("missing-ocr-command");
    let store = open_owned_store(&data_dir);
    store
        .set_worker_task_paused(
            WorkerTaskKind::Ocr,
            true,
            UnixTimestamp::from_unix_seconds(1_800_050_001),
        )
        .unwrap();
    drop(store);

    let output = Command::new(env!("CARGO_BIN_EXE_resume-daemon"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "run",
            "--foreground",
            "--once",
            "--work-ocr-once",
            "--ocr-command",
            path_str(&missing_command),
        ])
        .output()
        .expect("run paused daemon OCR worker once");

    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("ocr worker paused: true"));
    assert!(stdout.contains("ocr worker processed: 0"));
    assert!(stdout.contains("ocr worker cache writes: 0"));
    assert!(stdout.contains("ocr worker failed: 0"));
    assert!(!stdout.contains(path_str(&data_dir)));
    assert!(!stdout.contains(path_str(&private_document_path)));
    assert!(!stdout.contains(path_str(&missing_command)));

    let store = ReadMetaStore::open_data_dir(&data_dir).unwrap();
    let scanned = scanned_document(&store);
    assert_eq!(scanned.status, DocumentStatus::OcrRequired);
    let jobs = store.retryable_jobs().unwrap();
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].status, IngestJobStatus::Queued);

    remove_dir(&data_dir);
}

#[cfg(unix)]
#[test]
fn daemon_ocr_worker_loop_serves_status_ipc_while_indexing_scanned_pdf() {
    let data_dir = temp_dir("ocr-worker-loop-data");
    let private_document_path = seed_scanned_document(&data_dir);
    let command = write_fixture_executable(
        "fixture-daemon-ocr-worker-loop",
        r#"#!/bin/sh
printf 'resume-ir-ocr-v1\n'
printf 'confidence=0.74\n'
printf 'text:\n'
printf 'SUMMARY\n'
printf 'Synthetic background OCR engineer.\n'
printf 'EXPERIENCE\n'
printf 'Built OCRS50DaemonLoopToken background search services.\n'
printf 'SKILLS\n'
printf 'Rust worker systems.\n'
"#,
    );

    let mut child = Command::new(env!("CARGO_BIN_EXE_resume-daemon"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "run",
            "--foreground",
            "--work-ocr",
            "--ocr-command",
            path_str(&command),
            "--worker-interval-ms",
            "25",
            "--ipc-listen",
            "127.0.0.1:0",
            "--max-requests",
            "40",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start daemon OCR worker loop with IPC");

    let stdout = child.stdout.take().expect("daemon stdout");
    let mut stdout = BufReader::new(stdout);
    let endpoint = read_ipc_endpoint(&mut child, &mut stdout);
    let (used_requests, completed_response) = wait_for_searchable_documents(&endpoint, 1, 40);
    drain_status_requests(&endpoint, 40 - used_requests);
    let mut daemon_stdout = String::new();
    stdout
        .read_to_string(&mut daemon_stdout)
        .expect("read daemon stdout tail");

    let output = wait_child(child);
    assert!(output.success, "stderr:\n{}", output.stderr);
    assert!(output.stderr.is_empty());
    assert!(daemon_stdout.contains("ocr worker processed: 1"));
    assert!(!daemon_stdout.contains("OCRS50DaemonLoopToken"));
    assert!(!daemon_stdout.contains(path_str(&data_dir)));
    assert!(!daemon_stdout.contains(path_str(&private_document_path)));
    assert!(!daemon_stdout.contains(path_str(&command)));
    assert!(!completed_response.contains(path_str(&data_dir)));
    assert!(!completed_response.contains(path_str(&private_document_path)));

    let hits = search_fulltext(&data_dir, "OCRS50DaemonLoopToken");
    assert_eq!(hits.len(), 1);

    remove_dir(&data_dir);
}

#[cfg(unix)]
#[test]
fn daemon_ocr_worker_loop_batches_multiple_jobs_in_one_tick() {
    let data_dir = temp_dir("ocr-worker-loop-batch-data");
    let private_document_path_a = seed_scanned_document_fixture(
        &data_dir,
        "batch-a",
        "synthetic-scanned-resume-batch-a.pdf",
        single_page_scanned_pdf_bytes(),
    );
    let private_document_path_b = seed_scanned_document_fixture(
        &data_dir,
        "batch-b",
        "synthetic-scanned-resume-batch-b.pdf",
        single_page_scanned_pdf_bytes(),
    );
    let command = write_fixture_executable(
        "fixture-daemon-ocr-worker-loop-batch",
        r#"#!/bin/sh
printf 'resume-ir-ocr-v1\n'
printf 'confidence=0.72\n'
printf 'text:\n'
printf 'SUMMARY\n'
printf 'Synthetic batch OCR engineer.\n'
printf 'EXPERIENCE\n'
printf 'Built OCRS50DaemonBatchToken page %s search services.\n' "$RESUME_IR_OCR_PAGE_NO"
printf 'SKILLS\n'
printf 'Rust batch systems.\n'
"#,
    );

    let output = Command::new(env!("CARGO_BIN_EXE_resume-daemon"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "run",
            "--foreground",
            "--work-ocr",
            "--ocr-command",
            path_str(&command),
            "--worker-interval-ms",
            "1",
            "--max-worker-ticks",
            "1",
            "--ocr-jobs-per-tick",
            "2",
        ])
        .output()
        .expect("run daemon OCR worker loop with batch budget");

    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("ocr worker processed: 2"));
    assert!(stdout.contains("ocr worker cache writes: 1"));
    assert!(stdout.contains("ocr worker cache hits: 1"));
    assert!(stdout.contains("ocr worker failed: 0"));
    assert!(!stdout.contains("OCRS50DaemonBatchToken"));
    assert!(!stdout.contains(path_str(&data_dir)));
    assert!(!stdout.contains(path_str(&private_document_path_a)));
    assert!(!stdout.contains(path_str(&private_document_path_b)));
    assert!(!stdout.contains(path_str(&command)));

    let hits = search_fulltext(&data_dir, "OCRS50DaemonBatchToken");
    assert_eq!(hits.len(), 2);

    remove_dir(&data_dir);
}

fn seed_scanned_document(data_dir: &Path) -> PathBuf {
    seed_scanned_document_with_bytes(data_dir, single_page_scanned_pdf_bytes())
}

fn seed_scanned_document_with_bytes(data_dir: &Path, bytes: impl AsRef<[u8]>) -> PathBuf {
    seed_scanned_document_fixture(
        data_dir,
        "scanned-document",
        "synthetic-scanned-resume.pdf",
        bytes.as_ref(),
    )
}

fn seed_scanned_document_fixture(
    data_dir: &Path,
    id_suffix: &str,
    file_name: &str,
    bytes: impl AsRef<[u8]>,
) -> PathBuf {
    let now = UnixTimestamp::from_unix_seconds(1_800_050_000);
    let private_root = data_dir.join("private-resumes");
    fs::create_dir_all(&private_root).unwrap();
    let document_path = private_root.join(file_name);
    fs::write(&document_path, bytes.as_ref()).unwrap();
    let store = open_owned_store(data_dir);
    let task = ImportTask {
        id: ImportTaskId::from_non_secret_parts(&["s50", id_suffix]),
        root_path: path_str(&private_root).to_string(),
        status: ImportTaskStatus::Running,
        queued_at: now,
        started_at: Some(now),
        finished_at: None,
        updated_at: now,
    };
    support::insert_import_task(&store, &task);
    import_root_with_options(
        data_dir,
        &store,
        &task,
        &private_root,
        now,
        ImportOptions::default(),
    )
    .unwrap();
    document_path
}

fn open_owned_store(data_dir: &Path) -> OwnedMetaStore {
    let owner = match DataDirectoryOwnerLease::try_acquire(data_dir).unwrap() {
        DataDirectoryOwnerAcquisition::Acquired(owner) => owner,
        DataDirectoryOwnerAcquisition::Contended => panic!("synthetic data dir is owned"),
    };
    owner.open_store().unwrap()
}

fn single_page_scanned_pdf_bytes() -> Vec<u8> {
    build_valid_pdf(vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /Resources << /XObject << /Im1 4 0 R >> >> /MediaBox [0 0 72 72] /Contents 5 0 R >>".to_vec(),
        pdf_stream_object(
            b"1111".to_vec(),
            Some(
                b"/Type /XObject /Subtype /Image /Width 10 /Height 10 /ColorSpace /DeviceGray /BitsPerComponent 8"
                    .to_vec(),
            ),
        ),
        pdf_stream_object(b"q 10 0 0 10 0 0 cm /Im1 Do Q\n".to_vec(), None),
    ])
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
    let mut offsets = Vec::with_capacity(object_count);
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
    output.extend_from_slice(format!("xref\n0 {}\n", object_count + 1).as_bytes());
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

fn scanned_document(store: &ReadMetaStore) -> meta_store::Document {
    store
        .visible_documents()
        .unwrap()
        .into_iter()
        .find(|document| document.file_name == "synthetic-scanned-resume.pdf")
        .expect("scanned synthetic fixture is persisted")
}

fn active_resume_version(
    store: &ReadMetaStore,
    document_id: &meta_store::DocumentId,
) -> meta_store::ResumeVersion {
    let version_id = store
        .with_search_metadata_snapshot(|snapshot| {
            snapshot
                .validated_active_projections()
                .map_err(|_| ())?
                .into_iter()
                .find(|projection| &projection.document_id == document_id)
                .map(|projection| projection.resume_version_id)
                .ok_or(())
        })
        .expect("document has an active immutable resume version");
    store
        .resume_version_by_id(&version_id)
        .unwrap()
        .expect("active resume version exists")
}

fn search_fulltext(data_dir: &Path, query: &str) -> Vec<search_runtime::FullTextCandidate> {
    let mut coordinator = QueryCoordinator::open(data_dir).unwrap();
    coordinator
        .with_query(|scope| scope.fulltext_candidates(query, HitLimit::new(20)?, None))
        .expect("generation-pinned full-text query")
}

fn http_get(endpoint: &str) -> String {
    let rest = endpoint
        .strip_prefix("http://")
        .expect("endpoint has http scheme");
    let (addr, path) = rest.split_once('/').expect("endpoint has path");
    let request = format!("GET /{path} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n");
    let mut stream = TcpStream::connect(addr).expect("connect daemon ipc");
    stream
        .write_all(request.as_bytes())
        .expect("write status request");
    let mut response = String::new();
    stream
        .read_to_string(&mut response)
        .expect("read status response");
    response
}

fn wait_for_searchable_documents(
    endpoint: &str,
    expected_searchable: usize,
    max_requests: usize,
) -> (usize, String) {
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut requests = 0_usize;
    let needle = format!("\"searchable_documents\":{expected_searchable}");
    loop {
        requests += 1;
        let response = http_get(endpoint);
        if response.contains(&needle) {
            return (requests, response);
        }
        assert!(
            requests < max_requests,
            "daemon OCR worker did not reach searchable count; last response:\n{response}"
        );
        assert!(
            Instant::now() < deadline,
            "daemon OCR worker timed out; last response:\n{response}"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn drain_status_requests(endpoint: &str, count: usize) {
    for _ in 0..count {
        let _ = http_get(endpoint);
    }
}

fn read_ipc_endpoint(child: &mut Child, stdout: &mut BufReader<impl Read>) -> String {
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut line = String::new();
    while Instant::now() < deadline {
        line.clear();
        let bytes = stdout.read_line(&mut line).expect("read daemon stdout");
        if bytes == 0 {
            if let Ok(Some(status)) = child.try_wait() {
                panic!("daemon exited before endpoint: {status}");
            }
            continue;
        }
        if let Some(endpoint) = line.trim().strip_prefix("ipc status endpoint: ") {
            return endpoint.to_string();
        }
    }
    panic!("daemon did not print ipc status endpoint");
}

fn wait_child(child: Child) -> ChildOutput {
    let output = child.wait_with_output().expect("wait daemon");
    ChildOutput {
        success: output.status.success(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

struct ChildOutput {
    success: bool,
    stderr: String,
}

fn temp_dir(label: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!("resume-ir-s50-daemon-{label}-{unique}"));
    fs::create_dir_all(&path).unwrap();
    path
}

fn path_str(path: &Path) -> &str {
    path.to_str().unwrap()
}

fn remove_dir(path: &Path) {
    let _ = fs::remove_dir_all(path);
}

#[cfg(unix)]
fn write_fixture_executable(name: &str, body: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;

    let directory = temp_dir("ocr-worker-command-bin");
    let path = directory.join(name);
    fs::write(&path, body).unwrap();
    let mut permissions = fs::metadata(&path).unwrap().permissions();
    permissions.set_mode(0o700);
    fs::set_permissions(&path, permissions).unwrap();
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

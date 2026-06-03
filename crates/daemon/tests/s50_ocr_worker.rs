use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use index_fulltext::{FullTextIndex, SearchQuery};
use meta_store::{
    Document, DocumentId, DocumentStatus, FileExtension, IngestJobStatus, MetaStore,
    OcrPageCacheKey, OcrPageCacheStatus, UnixTimestamp, WorkerTaskKind,
};

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
printf 'OCRS50DaemonOnceToken worker bytes=%s page=%s\n' "$input_size" "$RESUME_IR_OCR_PAGE_NO"
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
  S89_DAEMON_RENDERED_PAGE_1_BYTES:1) printf 'S89DaemonPageOneToken first page text\n' ;;
  S89_DAEMON_RENDERED_PAGE_2_BYTES:2) printf 'S89DaemonPageTwoToken second page text\n' ;;
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

    let store = MetaStore::open(data_dir.join("metadata.sqlite3")).unwrap();
    store.run_migrations().unwrap();
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
    let version = store
        .latest_visible_resume_version_for_document(&scanned.id)
        .unwrap()
        .expect("OCR resume version");
    assert_eq!(version.page_count, Some(2));
    assert!(version
        .clean_text
        .unwrap()
        .contains("S89DaemonPageOneToken"));
    assert!(version.raw_text.unwrap().contains("S89DaemonPageTwoToken"));

    assert_eq!(search_fulltext(&data_dir, "S89DaemonPageOneToken").len(), 1);
    assert_eq!(search_fulltext(&data_dir, "S89DaemonPageTwoToken").len(), 1);

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
        seed_scanned_document_with_bytes(&data_dir, &valid_blank_pdf_bytes());
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
printf 'S91DaemonPdftoppmRenderedToken rendered daemon page text\n'
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

    let store = MetaStore::open(data_dir.join("metadata.sqlite3")).unwrap();
    store.run_migrations().unwrap();
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
    let version = store
        .latest_visible_resume_version_for_document(&scanned.id)
        .unwrap()
        .expect("OCR resume version");
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
        seed_scanned_document_with_bytes(&data_dir, &valid_blank_pdf_bytes());
    let render_command = write_text_png_render_executable(
        "fixture-daemon-ocr-worker-tesseract-render",
        &pango_view,
        "S92 OCR TEST",
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

    let store = MetaStore::open(data_dir.join("metadata.sqlite3")).unwrap();
    store.run_migrations().unwrap();
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
    assert_eq!(search_fulltext(&data_dir, "S92").len(), 1);

    remove_dir(&data_dir);
}

#[cfg(unix)]
#[test]
fn daemon_ocr_worker_once_records_command_crash_as_retryable_without_leaks() {
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

    remove_dir(&data_dir);
}

#[cfg(unix)]
#[test]
fn daemon_ocr_worker_once_respects_pause_without_claiming_or_invoking_command() {
    let data_dir = temp_dir("ocr-worker-paused-data");
    let private_document_path = seed_scanned_document(&data_dir);
    let missing_command = data_dir.join("private-bin").join("missing-ocr-command");
    let store = MetaStore::open(data_dir.join("metadata.sqlite3")).unwrap();
    store.run_migrations().unwrap();
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

    let store = MetaStore::open(data_dir.join("metadata.sqlite3")).unwrap();
    store.run_migrations().unwrap();
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
printf 'OCRS50DaemonLoopToken background worker text\n'
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

fn seed_scanned_document(data_dir: &Path) -> PathBuf {
    seed_scanned_document_with_bytes(data_dir, single_page_scanned_pdf_bytes())
}

fn seed_scanned_document_with_bytes(data_dir: &Path, bytes: &[u8]) -> PathBuf {
    let now = UnixTimestamp::from_unix_seconds(1_800_050_000);
    let private_root = data_dir.join("private-resumes");
    fs::create_dir_all(&private_root).unwrap();
    let document_path = private_root.join("synthetic-scanned-resume.pdf");
    fs::write(&document_path, bytes).unwrap();
    let store = MetaStore::open(data_dir.join("metadata.sqlite3")).unwrap();
    store.run_migrations().unwrap();
    let doc_id = DocumentId::from_non_secret_parts(&["s50", "scanned-document"]);
    store
        .upsert_document(&Document {
            id: doc_id.clone(),
            source_uri: format!("file://{}", path_str(&document_path)),
            normalized_path: path_str(&document_path).to_string(),
            file_name: "synthetic-scanned-resume.pdf".to_string(),
            extension: FileExtension::Pdf,
            byte_size: fs::metadata(&document_path).unwrap().len(),
            mtime: now,
            content_hash: Some("s50-synthetic-scanned-content-hash".to_string()),
            text_hash: None,
            is_deleted: false,
            created_at: now,
            updated_at: now,
            status: DocumentStatus::OcrRequired,
        })
        .unwrap();
    store.enqueue_ocr_job_for_document(&doc_id, now).unwrap();
    document_path
}

fn single_page_scanned_pdf_bytes() -> &'static [u8] {
    b"%PDF-1.4
1 0 obj << /Type /Catalog /Pages 2 0 R >> endobj
2 0 obj << /Type /Pages /Kids [3 0 R] /Count 1 >> endobj
3 0 obj << /Type /Page /Parent 2 0 R /Resources << /XObject << /Im1 4 0 R >> >> /Contents 5 0 R >> endobj
4 0 obj << /Type /XObject /Subtype /Image /Width 10 /Height 10 /ColorSpace /DeviceGray /BitsPerComponent 8 /Length 4 >> stream
1111
endstream endobj
5 0 obj << /Length 24 >> stream
q 10 0 0 10 0 0 cm /Im1 Do Q
endstream endobj
%%EOF"
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
    let mut output = Vec::new();
    output.extend_from_slice(b"%PDF-1.4\n");
    let object_1 = output.len();
    output.extend_from_slice(b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n");
    let object_2 = output.len();
    output.extend_from_slice(b"2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n");
    let object_3 = output.len();
    output.extend_from_slice(
        b"3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 72 72] /Resources << >> >>\nendobj\n",
    );
    let xref = output.len();
    output.extend_from_slice(b"xref\n0 4\n");
    output.extend_from_slice(b"0000000000 65535 f \n");
    for offset in [object_1, object_2, object_3] {
        output.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    output.extend_from_slice(
        format!("trailer\n<< /Size 4 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n").as_bytes(),
    );
    output
}

fn scanned_document(store: &MetaStore) -> meta_store::Document {
    store
        .visible_documents()
        .unwrap()
        .into_iter()
        .find(|document| document.file_name == "synthetic-scanned-resume.pdf")
        .expect("scanned synthetic fixture is persisted")
}

fn search_fulltext(data_dir: &Path, query: &str) -> Vec<index_fulltext::SearchHit> {
    let index = FullTextIndex::open_active(&data_dir.join("search-index"))
        .unwrap()
        .expect("active full-text index");
    index
        .search(SearchQuery::new(query).with_limit(20))
        .expect("search full-text index")
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

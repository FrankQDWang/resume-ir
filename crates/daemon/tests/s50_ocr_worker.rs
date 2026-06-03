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
    let now = UnixTimestamp::from_unix_seconds(1_800_050_000);
    let private_root = data_dir.join("private-resumes");
    fs::create_dir_all(&private_root).unwrap();
    let document_path = private_root.join("synthetic-scanned-resume.pdf");
    fs::write(&document_path, b"%PDF-1.4 synthetic scanned page bytes").unwrap();
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

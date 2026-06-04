use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::str::FromStr;
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use index_vector::{PersistentVectorIndex, VectorDocument, VectorIndex};
use meta_store::{
    DocumentId, MetaStore, OcrPageCacheEntry, OcrPageCacheKey, OcrWordBox, UnixTimestamp,
};

#[test]
fn delete_soft_tombstones_document_and_removes_it_from_default_search() {
    let _guard = s14_test_lock();
    let data_dir = temp_dir("delete-search-data");
    let fixture_root = fixture_root();
    import_fixtures(&data_dir, &fixture_root);

    let before = search(&data_dir, "Java");
    assert!(before.contains("results: 2"));
    let deleted_doc_id = doc_id_for_file(&before, "synthetic-java-engineer.docx");

    let delete = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "delete",
            "--doc-id",
            &deleted_doc_id,
        ])
        .output()
        .expect("run resume-cli delete");

    assert!(
        delete.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&delete.stdout),
        String::from_utf8_lossy(&delete.stderr)
    );
    assert!(delete.stderr.is_empty());
    let delete_stdout = String::from_utf8_lossy(&delete.stdout);
    assert!(delete_stdout.contains("delete completed"));
    assert!(delete_stdout.contains("status: deleted"));
    assert!(delete_stdout.contains("index rebuilt: true"));
    assert!(!delete_stdout.contains(path_str(&fixture_root)));
    assert!(!delete_stdout.contains(path_str(&data_dir)));

    let after = search(&data_dir, "Java");
    assert!(after.contains("results: 1"));
    assert!(!after.contains("synthetic-java-engineer.docx"));
    assert!(after.contains("synthetic-java-platform.pdf"));

    let status = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args(["--data-dir", path_str(&data_dir), "status"])
        .output()
        .expect("run resume-cli status after delete");
    assert!(status.status.success());
    let status_stdout = String::from_utf8_lossy(&status.stdout);
    assert!(status_stdout.contains("searchable documents: 1"));
    assert!(status_stdout.contains("index health: ready"));

    let reopened = search(&data_dir, "Java");
    assert!(reopened.contains("results: 1"));
    assert!(!reopened.contains("synthetic-java-engineer.docx"));

    remove_dir(&data_dir);
}

#[test]
fn reimport_marks_missing_files_deleted_and_default_search_hides_stale_hits() {
    let _guard = s14_test_lock();
    let data_dir = temp_dir("reimport-delete-data");
    let fixture_root = temp_dir("reimport-fixtures");
    copy_fixture_tree(&fixture_root);

    import_fixtures(&data_dir, &fixture_root);
    let before = search(&data_dir, "Java");
    assert!(before.contains("results: 2"));

    fs::remove_file(fixture_root.join("synthetic-java-engineer.docx")).unwrap();
    import_fixtures(&data_dir, &fixture_root);

    let after = search(&data_dir, "Java");
    assert!(after.contains("results: 1"));
    assert!(!after.contains("synthetic-java-engineer.docx"));
    assert!(after.contains("synthetic-java-platform.pdf"));

    let status = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args(["--data-dir", path_str(&data_dir), "status"])
        .output()
        .expect("run resume-cli status after reimport delete");
    assert!(status.status.success());
    let status_stdout = String::from_utf8_lossy(&status.stdout);
    assert!(status_stdout.contains("searchable documents: 1"));

    remove_dir(&data_dir);
    remove_dir(&fixture_root);
}

#[test]
fn budgeted_reimport_does_not_mark_unscanned_missing_files_deleted() {
    let _guard = s14_test_lock();
    let data_dir = temp_dir("budgeted-reimport-delete-data");
    let fixture_root = temp_dir("budgeted-reimport-fixtures");
    copy_fixture_tree(&fixture_root);

    import_fixtures(&data_dir, &fixture_root);
    let before = search(&data_dir, "Java");
    assert!(before.contains("results: 2"));

    fs::remove_file(fixture_root.join("synthetic-java-engineer.docx")).unwrap();
    let budgeted = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "import",
            "--root",
            path_str(&fixture_root),
            "--max-files",
            "1",
        ])
        .output()
        .expect("run budgeted reimport");
    assert!(
        budgeted.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&budgeted.stdout),
        String::from_utf8_lossy(&budgeted.stderr)
    );
    assert!(budgeted.stderr.is_empty());
    let budgeted_stdout = String::from_utf8_lossy(&budgeted.stdout);
    assert!(budgeted_stdout.contains("scan budget exhausted: yes"));
    assert!(budgeted_stdout.contains("deleted documents: 0"));
    assert!(!budgeted_stdout.contains(path_str(&fixture_root)));

    let after = search(&data_dir, "Java");
    assert!(after.contains("results: 2"));
    assert!(after.contains("synthetic-java-engineer.docx"));
    assert!(after.contains("synthetic-java-platform.pdf"));

    remove_dir(&data_dir);
    remove_dir(&fixture_root);
}

#[test]
fn multi_root_reimport_marks_missing_files_deleted_per_root() {
    let _guard = s14_test_lock();
    let data_dir = temp_dir("multi-root-reimport-delete-data");
    let first_root = temp_dir("multi-root-delete-a");
    let second_root = temp_dir("multi-root-delete-b");
    fs::copy(
        fixture_file("synthetic-java-platform.pdf"),
        first_root.join("synthetic-java-platform.pdf"),
    )
    .unwrap();
    fs::copy(
        fixture_file("synthetic-java-engineer.docx"),
        second_root.join("synthetic-java-engineer.docx"),
    )
    .unwrap();

    import_multi_root_fixtures(&data_dir, &first_root, &second_root);
    let before = search(&data_dir, "Java");
    assert!(before.contains("results: 2"));

    fs::remove_file(first_root.join("synthetic-java-platform.pdf")).unwrap();
    import_multi_root_fixtures(&data_dir, &first_root, &second_root);

    let after = search(&data_dir, "Java");
    assert!(after.contains("results: 1"));
    assert!(!after.contains("synthetic-java-platform.pdf"));
    assert!(after.contains("synthetic-java-engineer.docx"));

    remove_dir(&data_dir);
    remove_dir(&first_root);
    remove_dir(&second_root);
}

#[test]
fn discovery_profile_reuses_root_scan_without_deleting_skipped_directories() {
    let _guard = s14_test_lock();
    let data_dir = temp_dir("discovery-reimport-data");
    let fixture_root = temp_dir("discovery-reimport-fixtures");
    fs::create_dir_all(fixture_root.join("Documents")).unwrap();
    fs::create_dir_all(fixture_root.join("node_modules")).unwrap();
    fs::copy(
        fixture_file("synthetic-java-platform.pdf"),
        fixture_root
            .join("Documents")
            .join("synthetic-java-platform.pdf"),
    )
    .unwrap();
    fs::copy(
        fixture_file("synthetic-java-engineer.docx"),
        fixture_root
            .join("node_modules")
            .join("synthetic-java-engineer.docx"),
    )
    .unwrap();

    import_fixtures(&data_dir, &fixture_root);
    let before = search(&data_dir, "Java");
    assert!(before.contains("results: 2"));

    let discovery = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
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
        .expect("run discovery profile import");
    assert!(
        discovery.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&discovery.stdout),
        String::from_utf8_lossy(&discovery.stderr)
    );
    assert!(discovery.stderr.is_empty());
    let discovery_stdout = String::from_utf8_lossy(&discovery.stdout);
    assert!(discovery_stdout.contains("scan profile: discovery"));
    assert!(discovery_stdout.contains("files discovered: 1"));
    assert!(!discovery_stdout.contains(path_str(&fixture_root)));

    let after = search(&data_dir, "Java");
    assert!(after.contains("results: 2"));
    assert!(after.contains("synthetic-java-platform.pdf"));
    assert!(after.contains("synthetic-java-engineer.docx"));

    fs::remove_file(
        fixture_root
            .join("Documents")
            .join("synthetic-java-platform.pdf"),
    )
    .unwrap();
    let discovery_after_delete = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
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
        .expect("run discovery profile import after delete");
    assert!(
        discovery_after_delete.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&discovery_after_delete.stdout),
        String::from_utf8_lossy(&discovery_after_delete.stderr)
    );

    let after_delete = search(&data_dir, "Java");
    assert!(after_delete.contains("results: 1"));
    assert!(!after_delete.contains("synthetic-java-platform.pdf"));
    assert!(after_delete.contains("synthetic-java-engineer.docx"));

    remove_dir(&data_dir);
    remove_dir(&fixture_root);
}

#[test]
fn default_search_hydrates_metadata_to_hide_deleted_stale_index_hits() {
    let _guard = s14_test_lock();
    let data_dir = temp_dir("stale-index-delete-data");
    let fixture_root = fixture_root();
    import_fixtures(&data_dir, &fixture_root);

    let before = search(&data_dir, "Java");
    assert!(before.contains("results: 2"));
    let deleted_doc_id = doc_id_for_file(&before, "synthetic-java-engineer.docx");

    let store = MetaStore::open_data_dir(&data_dir).unwrap();
    store.run_migrations().unwrap();
    store
        .mark_document_deleted(
            &DocumentId::from_str(&deleted_doc_id).unwrap(),
            UnixTimestamp::from_unix_seconds(1_900_000_000),
        )
        .unwrap();

    let after = search(&data_dir, "Java");
    assert!(after.contains("results: 1"));
    assert!(!after.contains("synthetic-java-engineer.docx"));
    assert!(after.contains("synthetic-java-platform.pdf"));

    remove_dir(&data_dir);
}

#[test]
fn purge_deleted_removes_tombstoned_metadata_old_snapshots_and_vectors_without_path_leak() {
    let _guard = s14_test_lock();
    let data_dir = temp_dir("purge-deleted-data");
    let fixture_root = fixture_root();
    import_fixtures(&data_dir, &fixture_root);

    let before = search(&data_dir, "Java");
    assert!(before.contains("results: 2"));
    let deleted_doc_id = doc_id_for_file(&before, "synthetic-java-engineer.docx");
    let live_doc_id = doc_id_for_file(&before, "synthetic-java-platform.pdf");

    let vector_index = PersistentVectorIndex::open(data_dir.join("vector-index"), 4).unwrap();
    vector_index
        .upsert(vec![
            VectorDocument::new_for_model(
                "fixture-model",
                "fixture-model:deleted-doc",
                deleted_doc_id.clone(),
                vec![1.0, 0.0, 0.0, 0.0],
            )
            .unwrap(),
            VectorDocument::new_for_model(
                "fixture-model",
                "fixture-model:live-doc",
                live_doc_id.clone(),
                vec![0.0, 1.0, 0.0, 0.0],
            )
            .unwrap(),
        ])
        .unwrap();
    assert_eq!(vector_index.snapshot().unwrap().vector_count(), 2);

    let deleted_document_id = DocumentId::from_str(&deleted_doc_id).unwrap();
    let (ocr_cache_key, ocr_job_id, embedding_job_id) = {
        let store = MetaStore::open_data_dir(&data_dir).unwrap();
        store.run_migrations().unwrap();
        let deleted_document = store
            .document_by_id(&deleted_document_id)
            .unwrap()
            .expect("deleted candidate document before tombstone");
        let deleted_version = store
            .resume_versions_for_document(&deleted_document.id)
            .unwrap()
            .into_iter()
            .next()
            .expect("deleted candidate version before tombstone");
        let content_hash = deleted_document.content_hash.clone().expect("content hash");
        let ocr_cache_key = OcrPageCacheKey::new(content_hash, 1, 300, "eng", "balanced").unwrap();
        let ocr_cache_entry = OcrPageCacheEntry::succeeded_with_word_boxes(
            ocr_cache_key.clone(),
            "PRIVATE_PURGE_OCR_TEXT_SHOULD_NOT_SURVIVE",
            0.91,
            "fixture-ocr-engine",
            17,
            vec![OcrWordBox::new("PRIVATE_PURGE_WORD_BOX", 1, 2, 3, 4, 0.88).unwrap()],
            UnixTimestamp::from_unix_seconds(1_800_014_000),
        )
        .unwrap();
        assert_eq!(ocr_cache_entry.word_boxes().len(), 1);
        store.upsert_ocr_page_cache_entry(&ocr_cache_entry).unwrap();
        let ocr_job = store
            .enqueue_ocr_job_for_document(
                &deleted_document.id,
                UnixTimestamp::from_unix_seconds(1_800_014_001),
            )
            .unwrap()
            .job;
        let embedding_job = store
            .enqueue_embedding_job_for_resume_version(
                &deleted_document.id,
                &deleted_version.id,
                "fixture-purge-model",
                4,
                UnixTimestamp::from_unix_seconds(1_800_014_002),
            )
            .unwrap()
            .job;
        (ocr_cache_key, ocr_job.id, embedding_job.id)
    };

    let delete = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "delete",
            "--doc-id",
            &deleted_doc_id,
        ])
        .output()
        .expect("run resume-cli delete before purge");
    assert!(delete.status.success());
    assert!(snapshot_dir_count(&data_dir) >= 2);

    let purge = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args(["--data-dir", path_str(&data_dir), "purge", "--deleted"])
        .output()
        .expect("run resume-cli purge");

    assert!(
        purge.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&purge.stdout),
        String::from_utf8_lossy(&purge.stderr)
    );
    assert!(purge.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&purge.stdout);
    assert!(stdout.contains("purge completed"));
    assert!(stdout.contains("scope: deleted"));
    assert!(stdout.contains("purged documents: 1"));
    assert!(stdout.contains("index rebuilt: true"));
    assert!(stdout.contains("vector documents purged: 1"));
    assert!(stdout.contains("ingest jobs purged: 2"));
    assert!(stdout.contains("embedding job specs purged: 1"));
    assert!(stdout.contains("ocr cache entries purged: 1"));
    assert!(stdout.contains("ocr word boxes purged: 1"));
    assert!(stdout.contains("metadata vacuum: yes"));
    assert!(!stdout.contains(path_str(&data_dir)));
    assert!(!stdout.contains(path_str(&fixture_root)));
    assert!(!stdout.contains("PRIVATE_PURGE_OCR_TEXT"));
    assert!(!stdout.contains("PRIVATE"));

    let store = MetaStore::open_data_dir(&data_dir).unwrap();
    store.run_migrations().unwrap();
    assert!(store
        .document_by_id(&DocumentId::from_str(&deleted_doc_id).unwrap())
        .unwrap()
        .is_none());
    assert!(store
        .document_by_id(&DocumentId::from_str(&live_doc_id).unwrap())
        .unwrap()
        .is_some());
    assert!(store
        .ocr_page_cache_entry(&ocr_cache_key)
        .unwrap()
        .is_none());
    assert!(store.ingest_job_by_id(&ocr_job_id).unwrap().is_none());
    assert!(store.ingest_job_by_id(&embedding_job_id).unwrap().is_none());
    assert_eq!(snapshot_dir_count(&data_dir), 1);
    let reopened_vector = PersistentVectorIndex::open(data_dir.join("vector-index"), 4).unwrap();
    assert_eq!(reopened_vector.snapshot().unwrap().vector_count(), 1);

    let after = search(&data_dir, "Java");
    assert!(after.contains("results: 1"));
    assert!(!after.contains("synthetic-java-engineer.docx"));
    assert!(after.contains("synthetic-java-platform.pdf"));

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

fn import_multi_root_fixtures(data_dir: &Path, first_root: &Path, second_root: &Path) {
    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(data_dir),
            "import",
            "--root",
            path_str(first_root),
            "--root",
            path_str(second_root),
        ])
        .output()
        .expect("import multi-root fixtures");
    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn s14_test_lock() -> MutexGuard<'static, ()> {
    static S14_TEST_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    S14_TEST_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn copy_fixture_tree(target_root: &Path) {
    for entry in fs::read_dir(fixture_root()).unwrap() {
        let entry = entry.unwrap();
        let source = entry.path();
        if source.is_file() {
            fs::copy(&source, target_root.join(entry.file_name())).unwrap();
        }
    }
}

fn search(data_dir: &Path, query: &str) -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args(["--data-dir", path_str(data_dir), "search", query])
        .output()
        .expect("run resume-cli search");
    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn doc_id_for_file(search_output: &str, file_name: &str) -> String {
    let mut current_doc_id = None;
    for line in search_output.lines() {
        if let Some(doc_id) = line.strip_prefix("doc_id: ") {
            current_doc_id = Some(doc_id.to_string());
        }
        if line == format!("file_name: {file_name}") {
            return current_doc_id.expect("file line follows doc id");
        }
    }

    panic!("file not found in search output: {file_name}");
}

fn fixture_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("tests/fixtures/resumes")
}

fn fixture_file(name: &str) -> PathBuf {
    fixture_root().join(name)
}

fn snapshot_dir_count(data_dir: &Path) -> usize {
    let snapshots = data_dir.join("search-index").join("snapshots");
    match fs::read_dir(snapshots) {
        Ok(entries) => entries
            .filter_map(Result::ok)
            .filter(|entry| {
                entry
                    .file_type()
                    .map(|file_type| file_type.is_dir())
                    .unwrap_or(false)
            })
            .count(),
        Err(_) => 0,
    }
}

fn temp_dir(label: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!("resume-ir-s14-cli-{label}-{unique}"));
    fs::create_dir_all(&path).unwrap();
    path
}

fn path_str(path: &Path) -> &str {
    path.to_str().unwrap()
}

fn remove_dir(path: &Path) {
    let _ = fs::remove_dir_all(path);
}

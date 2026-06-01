use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::str::FromStr;
use std::time::{SystemTime, UNIX_EPOCH};

use meta_store::{DocumentId, MetaStore, UnixTimestamp};

#[test]
fn delete_soft_tombstones_document_and_removes_it_from_default_search() {
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
fn discovery_profile_reuses_root_scan_without_deleting_skipped_directories() {
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
    let data_dir = temp_dir("stale-index-delete-data");
    let fixture_root = fixture_root();
    import_fixtures(&data_dir, &fixture_root);

    let before = search(&data_dir, "Java");
    assert!(before.contains("results: 2"));
    let deleted_doc_id = doc_id_for_file(&before, "synthetic-java-engineer.docx");

    let store = MetaStore::open(data_dir.join("metadata.sqlite3")).unwrap();
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

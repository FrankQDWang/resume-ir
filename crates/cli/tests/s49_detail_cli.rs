mod support;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use meta_store::{ReadMetaStore, SearchSelection};

use support::{assert_import_succeeded, import_text_resumes};

const PRIVATE_EMAIL: &str = "candidate@example.test";
const PRIVATE_PHONE: &str = "155-555-0199";

#[test]
fn detail_local_reads_the_exact_active_selection_and_redacts_contacts() {
    let data_dir = temp_dir("detail-local-data");
    let source_root = temp_dir("detail-local-source");
    let text = resume_text("Version A", "Rust");
    let imported = import_text_resumes(&data_dir, &source_root, &[("synthetic-detail.txt", &text)]);
    assert_import_succeeded(&imported);
    let selection = active_selection(&data_dir, "synthetic-detail.txt");

    let output = detail(&data_dir, &selection);
    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("resume detail"));
    assert!(stdout.contains(&format!("doc_id: {}", selection.document_id)));
    assert!(stdout.contains(&format!("version_id: {}", selection.resume_version_id)));
    assert!(stdout.contains(&format!("visible_epoch: {}", selection.visible_epoch)));
    assert!(stdout.contains("field: skill"));
    assert!(stdout.contains("snippet:"));
    assert!(!stdout.contains(PRIVATE_EMAIL));
    assert!(!stdout.contains(PRIVATE_PHONE));
    assert!(!stdout.contains(path_str(&source_root)));

    remove_dir(&data_dir);
    remove_dir(&source_root);
}

#[test]
fn detail_local_rejects_a_selection_after_the_document_changes_version() {
    let data_dir = temp_dir("detail-stale-data");
    let source_root = temp_dir("detail-stale-source");
    let version_a = resume_text("Version A", "Rust");
    assert_import_succeeded(&import_text_resumes(
        &data_dir,
        &source_root,
        &[("synthetic-detail.txt", &version_a)],
    ));
    let old_selection = active_selection(&data_dir, "synthetic-detail.txt");

    let version_b = resume_text("PRIVATE_VERSION_B_MUST_NOT_LEAK", "Java");
    assert_import_succeeded(&import_text_resumes(
        &data_dir,
        &source_root,
        &[("synthetic-detail.txt", &version_b)],
    ));
    let new_selection = active_selection(&data_dir, "synthetic-detail.txt");
    assert_ne!(
        old_selection.resume_version_id,
        new_selection.resume_version_id
    );

    let output = detail(&data_dir, &old_selection);
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("detail selection is stale; refresh search"));
    assert!(!stderr.contains("PRIVATE_VERSION_B_MUST_NOT_LEAK"));

    remove_dir(&data_dir);
    remove_dir(&source_root);
}

#[test]
fn detail_local_rejects_a_selection_after_delete_without_returning_text() {
    let data_dir = temp_dir("detail-deleted-data");
    let source_root = temp_dir("detail-deleted-source");
    let text = resume_text("PRIVATE_DELETED_VERSION", "Rust");
    assert_import_succeeded(&import_text_resumes(
        &data_dir,
        &source_root,
        &[("synthetic-detail.txt", &text)],
    ));
    let selection = active_selection(&data_dir, "synthetic-detail.txt");
    let deleted = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "delete",
            "--doc-id",
            selection.document_id.as_str(),
        ])
        .output()
        .expect("delete synthetic detail document");
    assert!(deleted.status.success());

    let output = detail(&data_dir, &selection);
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("detail selection was not found"));
    assert!(!stderr.contains("PRIVATE_DELETED_VERSION"));

    remove_dir(&data_dir);
    remove_dir(&source_root);
}

fn active_selection(data_dir: &Path, file_name: &str) -> SearchSelection {
    let store = ReadMetaStore::open_data_dir(data_dir).unwrap();
    let document = store
        .visible_documents()
        .unwrap()
        .into_iter()
        .find(|document| document.file_name == file_name)
        .unwrap();
    let projection = store
        .active_search_projection_for_document(&document.id)
        .unwrap()
        .unwrap();
    let visible_epoch = store.search_projection_state().unwrap().visible_epoch;
    SearchSelection {
        document_id: projection.document_id,
        resume_version_id: projection.resume_version_id,
        visible_epoch,
    }
}

fn detail(data_dir: &Path, selection: &SearchSelection) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(data_dir),
            "detail",
            "--doc-id",
            selection.document_id.as_str(),
            "--version-id",
            selection.resume_version_id.as_str(),
            "--visible-epoch",
            &selection.visible_epoch.to_string(),
        ])
        .output()
        .expect("run resume-cli detail")
}

fn resume_text(version: &str, skill: &str) -> String {
    format!(
        "SUMMARY\nSynthetic Candidate {version}\nEmail: {PRIVATE_EMAIL}\nPhone: {PRIVATE_PHONE}\nEXPERIENCE\nBuilt {skill} systems\nSKILLS\n{skill}"
    )
}

fn temp_dir(label: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!("resume-ir-s49-cli-{label}-{unique}"));
    fs::create_dir_all(&path).unwrap();
    path
}

fn path_str(path: &Path) -> &str {
    path.to_str().unwrap()
}

fn remove_dir(path: &Path) {
    let _ = fs::remove_dir_all(path);
}

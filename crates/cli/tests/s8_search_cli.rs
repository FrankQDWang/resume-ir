mod support;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use meta_store::ReadMetaStore;

use support::{assert_import_succeeded, import_text_resumes};

#[test]
fn search_cli_rejects_query_bounds_before_index_access_without_query_echo() {
    let data_dir = temp_dir("query-bounds");
    let query = (0..17)
        .map(|index| format!("private-term-{index}"))
        .collect::<Vec<_>>()
        .join(" ");

    let output = search(&data_dir, &query);

    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("search query is outside semantic bounds"));
    assert!(!stderr.contains(&query));

    remove_dir(&data_dir);
}

#[test]
fn search_cli_reads_the_atomically_published_generation_without_query_echo() {
    let data_dir = temp_dir("published-generation");
    let source_root = temp_dir("published-generation-source");
    let long_file_name = format!(
        "candidate@example.test-{}-PRIVATE_TRAILING.txt",
        "候".repeat(60)
    );
    assert_import_succeeded(&import_text_resumes(
        &data_dir,
        &source_root,
        &[(
            long_file_name,
            "SUMMARY\nSynthetic Candidate\nEXPERIENCE\nBuilt Java payment search systems\nSKILLS\nJava".to_string(),
        )],
    ));

    let output = search(&data_dir, "Java payment");

    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("results: 1"));
    assert!(stdout.contains("rank: 1"));
    assert!(stdout.contains("doc_id: doc_"));
    assert!(stdout.contains("version_id: ver_"));
    assert!(stdout.contains("visible_epoch:"));
    assert!(stdout.contains("snippet:"));
    assert!(!stdout.contains("candidate@example.test"));
    assert!(!stdout.contains("PRIVATE_TRAILING"));
    assert!(!stdout.contains("query:"));

    remove_dir(&data_dir);
    remove_dir(&source_root);
}

#[test]
fn search_cli_ignores_an_unpublished_artifact_directory() {
    let data_dir = temp_dir("unpublished-generation");
    let source_root = temp_dir("unpublished-generation-source");
    assert_import_succeeded(&import_text_resumes(
        &data_dir,
        &source_root,
        &[(
            "published.txt",
            "SUMMARY\nPublished Candidate\nEXPERIENCE\nBuilt Java committed generation sentinel\nSKILLS\nJava",
        )],
    ));
    let unpublished = data_dir
        .join("search-index")
        .join("snapshots")
        .join("unpublished-generation");
    fs::create_dir_all(&unpublished).unwrap();
    fs::write(
        unpublished.join("PRIVATE_UNPUBLISHED_ARTIFACT"),
        b"Rust unpublished generation sentinel",
    )
    .unwrap();

    let output = search(&data_dir, "Java committed sentinel");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("results: 1"));
    assert!(stdout.contains("published.txt"));
    assert!(!stdout.contains("PRIVATE_UNPUBLISHED_ARTIFACT"));
    assert!(!stdout.contains("unpublished generation"));

    remove_dir(&data_dir);
    remove_dir(&source_root);
}

#[test]
fn search_cli_fails_closed_when_the_active_fulltext_artifact_is_corrupt() {
    let data_dir = temp_dir("corrupt-generation");
    let source_root = temp_dir("corrupt-generation-source");
    assert_import_succeeded(&import_text_resumes(
        &data_dir,
        &source_root,
        &[(
            "private-corrupt.txt",
            "SUMMARY\nCorrupt Candidate\nEXPERIENCE\nBuilt PRIVATE_CORRUPT_SENTINEL systems\nSKILLS\nRust",
        )],
    ));
    let store = ReadMetaStore::open_data_dir(&data_dir).unwrap();
    let generation = store.search_projection_state().unwrap().generation.unwrap();
    fs::write(
        data_dir
            .join("search-index")
            .join("snapshots")
            .join(generation)
            .join("fulltext.snapshot.enc"),
        b"corrupt active snapshot",
    )
    .unwrap();

    let output = search(&data_dir, "PRIVATE_CORRUPT_SENTINEL");

    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(!String::from_utf8_lossy(&output.stderr).contains("PRIVATE_CORRUPT_SENTINEL"));

    remove_dir(&data_dir);
    remove_dir(&source_root);
}

fn search(data_dir: &Path, query: &str) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args(["--data-dir", path_str(data_dir), "search", query])
        .output()
        .expect("run resume-cli search")
}

fn temp_dir(label: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!("resume-ir-s8-cli-{label}-{unique}"));
    fs::create_dir_all(&path).unwrap();
    path
}

fn path_str(path: &Path) -> &str {
    path.to_str().unwrap()
}

fn remove_dir(path: &Path) {
    let _ = fs::remove_dir_all(path);
}

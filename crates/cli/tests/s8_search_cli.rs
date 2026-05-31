use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use index_fulltext::{FullTextIndex, IndexDocument, IndexSection};

#[test]
fn search_cli_reads_existing_fulltext_index_without_query_echo() {
    let data_dir = temp_dir("search-cli-data");
    let index_dir = data_dir.join("search-index");
    let index = FullTextIndex::open_or_create(&index_dir).unwrap();
    index
        .replace_documents([IndexDocument {
            doc_id: "doc_cli_java".to_string(),
            version_id: "ver_cli_java".to_string(),
            file_name: "synthetic-cli-java.pdf".to_string(),
            clean_text: "Java payment search platform".to_string(),
            sections: vec![IndexSection {
                section_type: "experience".to_string(),
                text: "Java payment".to_string(),
            }],
            is_deleted: false,
        }])
        .unwrap();
    index.commit().unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args(["--data-dir", path_str(&data_dir), "search", "Java payment"])
        .output()
        .expect("run resume-cli search");

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("results: 1"));
    assert!(stdout.contains("rank: 1"));
    assert!(stdout.contains("doc_id: doc_cli_java"));
    assert!(stdout.contains("file_name: synthetic-cli-java.pdf"));
    assert!(stdout.contains("snippet:"));
    assert!(!stdout.contains("query:"));

    remove_dir(&data_dir);
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

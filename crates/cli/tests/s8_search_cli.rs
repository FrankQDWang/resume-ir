use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use index_fulltext::{FullTextIndex, IndexDocument, IndexSection};
use meta_store::{
    Document, DocumentId, DocumentStatus, FileExtension, MetaStore, ResumeVersion, ResumeVersionId,
    ResumeVisibility, UnixTimestamp,
};

#[test]
fn search_cli_reads_existing_fulltext_index_without_query_echo() {
    let data_dir = temp_dir("search-cli-data");
    let document_id = DocumentId::from_non_secret_parts(&["s8", "cli-java"]);
    let version_id = ResumeVersionId::from_non_secret_parts(&["s8", "cli-java-version"]);
    seed_visible_metadata(&data_dir, document_id.clone(), version_id.clone());

    let index_dir = data_dir.join("search-index");
    let index = FullTextIndex::open_or_create(&index_dir).unwrap();
    index
        .replace_documents([IndexDocument {
            doc_id: document_id.to_string(),
            version_id: version_id.to_string(),
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
    assert!(stdout.contains(&format!("doc_id: {document_id}")));
    assert!(stdout.contains("file_name: synthetic-cli-java.pdf"));
    assert!(stdout.contains("snippet:"));
    assert!(!stdout.contains("query:"));

    remove_dir(&data_dir);
}

fn seed_visible_metadata(data_dir: &Path, document_id: DocumentId, version_id: ResumeVersionId) {
    let now = UnixTimestamp::from_unix_seconds(1_800_001_000);
    let store = MetaStore::open(data_dir.join("metadata.sqlite3")).unwrap();
    store.run_migrations().unwrap();
    store
        .upsert_document(&Document {
            id: document_id.clone(),
            source_uri: "synthetic://cli-java".to_string(),
            normalized_path: "synthetic/cli-java.pdf".to_string(),
            file_name: "synthetic-cli-java.pdf".to_string(),
            extension: FileExtension::Pdf,
            byte_size: 123,
            mtime: now,
            content_hash: Some("synthetic-cli-java-hash".to_string()),
            text_hash: None,
            is_deleted: false,
            created_at: now,
            updated_at: now,
            status: DocumentStatus::Searchable,
        })
        .unwrap();
    store
        .upsert_resume_version(&ResumeVersion {
            id: version_id,
            document_id,
            candidate_id: None,
            parse_version: "parser-v1".to_string(),
            schema_version: "schema-v1".to_string(),
            language_set: vec!["en".to_string()],
            page_count: Some(1),
            raw_text: Some("Java payment search platform".to_string()),
            clean_text: Some("Java payment search platform".to_string()),
            quality_score: Some(0.8),
            visibility: ResumeVisibility::Searchable,
        })
        .unwrap();
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

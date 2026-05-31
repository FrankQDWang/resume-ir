use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use index_fulltext::{FullTextIndex, IndexDocument, IndexSection};
use meta_store::{
    CandidateId, Document, DocumentId, DocumentStatus, EntityMention, EntityMentionId, EntityType,
    FileExtension, MetaStore, ResumeVersion, ResumeVersionId, ResumeVisibility, UnixTimestamp,
};

#[test]
fn search_folds_versions_with_the_same_assigned_candidate_id() {
    let data_dir = temp_dir("candidate-folding-data");
    let candidate_id = CandidateId::from_non_secret_parts(&["s18", "assigned-candidate"]);
    let old_document_id = DocumentId::from_non_secret_parts(&["s18", "candidate-a-old-doc"]);
    let old_version_id = ResumeVersionId::from_non_secret_parts(&["s18", "candidate-a-old-ver"]);
    let current_document_id =
        DocumentId::from_non_secret_parts(&["s18", "candidate-a-current-doc"]);
    let current_version_id =
        ResumeVersionId::from_non_secret_parts(&["s18", "candidate-a-current-ver"]);
    let distinct_document_id = DocumentId::from_non_secret_parts(&["s18", "candidate-b-doc"]);
    let distinct_version_id = ResumeVersionId::from_non_secret_parts(&["s18", "candidate-b-ver"]);
    let second_distinct_document_id =
        DocumentId::from_non_secret_parts(&["s18", "candidate-c-doc"]);
    let second_distinct_version_id =
        ResumeVersionId::from_non_secret_parts(&["s18", "candidate-c-ver"]);

    seed_document(
        &data_dir,
        old_document_id.clone(),
        old_version_id.clone(),
        Some(candidate_id.clone()),
        "synthetic-candidate-a-old.pdf",
        "Java backend",
    );
    seed_document(
        &data_dir,
        current_document_id.clone(),
        current_version_id.clone(),
        Some(candidate_id),
        "synthetic-candidate-a-current.pdf",
        "Java Java Java backend platform",
    );
    seed_document(
        &data_dir,
        distinct_document_id.clone(),
        distinct_version_id.clone(),
        None,
        "synthetic-candidate-b.pdf",
        "Java backend search",
    );
    seed_document(
        &data_dir,
        second_distinct_document_id.clone(),
        second_distinct_version_id.clone(),
        None,
        "synthetic-candidate-c.pdf",
        "Java backend observability",
    );
    seed_index(
        &data_dir,
        [
            (
                old_document_id,
                old_version_id,
                "synthetic-candidate-a-old.pdf",
                "Java backend",
            ),
            (
                current_document_id,
                current_version_id,
                "synthetic-candidate-a-current.pdf",
                "Java Java Java backend platform",
            ),
            (
                distinct_document_id,
                distinct_version_id,
                "synthetic-candidate-b.pdf",
                "Java backend search",
            ),
            (
                second_distinct_document_id,
                second_distinct_version_id,
                "synthetic-candidate-c.pdf",
                "Java backend observability",
            ),
        ],
    );

    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "search",
            "Java",
            "--top-k",
            "10",
        ])
        .output()
        .expect("run candidate-folding search");

    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("results: 3"));
    assert!(stdout.contains("rank: 1"));
    assert!(stdout.contains("rank: 2"));
    assert!(stdout.contains("rank: 3"));
    assert!(stdout.contains("synthetic-candidate-a-current.pdf"));
    assert!(!stdout.contains("synthetic-candidate-a-old.pdf"));
    assert!(stdout.contains("synthetic-candidate-b.pdf"));
    assert!(stdout.contains("synthetic-candidate-c.pdf"));

    let filtered = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "search",
            "Java",
            "--skills-any",
            "java",
            "--top-k",
            "10",
        ])
        .output()
        .expect("run filtered candidate-folding search");

    assert!(
        filtered.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&filtered.stdout),
        String::from_utf8_lossy(&filtered.stderr)
    );
    assert!(filtered.stderr.is_empty());
    let filtered_stdout = String::from_utf8_lossy(&filtered.stdout);
    assert!(filtered_stdout.contains("results: 3"));
    assert!(filtered_stdout.contains("synthetic-candidate-a-current.pdf"));
    assert!(!filtered_stdout.contains("synthetic-candidate-a-old.pdf"));
    assert!(filtered_stdout.contains("synthetic-candidate-b.pdf"));
    assert!(filtered_stdout.contains("synthetic-candidate-c.pdf"));

    remove_dir(&data_dir);
}

fn seed_document(
    data_dir: &Path,
    document_id: DocumentId,
    version_id: ResumeVersionId,
    candidate_id: Option<CandidateId>,
    file_name: &str,
    clean_text: &str,
) {
    let now = UnixTimestamp::from_unix_seconds(1_800_018_000);
    let store = MetaStore::open(data_dir.join("metadata.sqlite3")).unwrap();
    store.run_migrations().unwrap();
    store
        .upsert_document(&Document {
            id: document_id.clone(),
            source_uri: format!("synthetic://{file_name}"),
            normalized_path: format!("synthetic/{file_name}"),
            file_name: file_name.to_string(),
            extension: FileExtension::Pdf,
            byte_size: clean_text.len() as u64,
            mtime: now,
            content_hash: Some(format!("{file_name}-hash")),
            text_hash: None,
            is_deleted: false,
            created_at: now,
            updated_at: now,
            status: DocumentStatus::Searchable,
        })
        .unwrap();
    store
        .upsert_resume_version(&ResumeVersion {
            id: version_id.clone(),
            document_id,
            candidate_id,
            parse_version: "parser-v1".to_string(),
            schema_version: "schema-v1".to_string(),
            language_set: vec!["en".to_string()],
            page_count: Some(1),
            raw_text: Some(clean_text.to_string()),
            clean_text: Some(clean_text.to_string()),
            quality_score: Some(0.8),
            visibility: ResumeVisibility::Searchable,
        })
        .unwrap();
    store
        .replace_entity_mentions(
            &version_id,
            &[EntityMention {
                id: EntityMentionId::from_non_secret_parts(&["s18", version_id.as_str(), "java"]),
                resume_version_id: version_id.clone(),
                section_id: None,
                entity_type: EntityType::Skill,
                raw_value: "Java".to_string(),
                normalized_value: Some("java".to_string()),
                span_start: Some(0),
                span_end: Some(4),
                confidence: 0.95,
                extractor: "s18-synthetic".to_string(),
            }],
        )
        .unwrap();
}

fn seed_index<const N: usize>(
    data_dir: &Path,
    documents: [(DocumentId, ResumeVersionId, &str, &str); N],
) {
    let index_dir = data_dir.join("search-index");
    let index = FullTextIndex::open_or_create(&index_dir).unwrap();
    index
        .replace_documents(documents.into_iter().map(
            |(document_id, version_id, file_name, clean_text)| IndexDocument {
                doc_id: document_id.to_string(),
                version_id: version_id.to_string(),
                file_name: file_name.to_string(),
                clean_text: clean_text.to_string(),
                sections: vec![IndexSection {
                    section_type: "experience".to_string(),
                    text: clean_text.to_string(),
                }],
                is_deleted: false,
            },
        ))
        .unwrap();
    index.commit().unwrap();
}

fn temp_dir(label: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!("resume-ir-s18-cli-{label}-{unique}"));
    fs::create_dir_all(&path).unwrap();
    path
}

fn path_str(path: &Path) -> &str {
    path.to_str().unwrap()
}

fn remove_dir(path: &Path) {
    let _ = fs::remove_dir_all(path);
}

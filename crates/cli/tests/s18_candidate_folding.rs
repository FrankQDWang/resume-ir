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

#[test]
fn search_marks_soft_duplicate_hints_without_low_confidence_folding() {
    let data_dir = temp_dir("soft-dedupe-data");
    let first_document_id = DocumentId::from_non_secret_parts(&["s18", "soft-first-doc"]);
    let first_version_id = ResumeVersionId::from_non_secret_parts(&["s18", "soft-first-ver"]);
    let second_document_id = DocumentId::from_non_secret_parts(&["s18", "soft-second-doc"]);
    let second_version_id = ResumeVersionId::from_non_secret_parts(&["s18", "soft-second-ver"]);
    let distinct_document_id = DocumentId::from_non_secret_parts(&["s18", "soft-distinct-doc"]);
    let distinct_version_id = ResumeVersionId::from_non_secret_parts(&["s18", "soft-distinct-ver"]);

    seed_document_with_mentions(
        &data_dir,
        first_document_id.clone(),
        first_version_id.clone(),
        None,
        "synthetic-soft-a.pdf",
        "Java backend payments",
        &[
            SeedMention::new(
                EntityType::Name,
                "Synthetic Candidate",
                "synthetic candidate",
            ),
            SeedMention::new(
                EntityType::School,
                "Synthetic University",
                "synthetic university",
            ),
            SeedMention::new(EntityType::Company, "Example Labs", "example labs"),
            SeedMention::new(EntityType::Skill, "Java", "java"),
            SeedMention::new(EntityType::Skill, "Payments", "payments"),
        ],
    );
    seed_document_with_mentions(
        &data_dir,
        second_document_id.clone(),
        second_version_id.clone(),
        None,
        "synthetic-soft-b.pdf",
        "Java backend search",
        &[
            SeedMention::new(
                EntityType::Name,
                "Synthetic Candidate",
                "synthetic candidate",
            ),
            SeedMention::new(
                EntityType::School,
                "Synthetic University",
                "synthetic university",
            ),
            SeedMention::new(EntityType::Skill, "Java", "java"),
            SeedMention::new(EntityType::Skill, "Search", "search"),
        ],
    );
    seed_document_with_mentions(
        &data_dir,
        distinct_document_id.clone(),
        distinct_version_id.clone(),
        None,
        "synthetic-distinct.pdf",
        "Java backend observability",
        &[
            SeedMention::new(
                EntityType::Name,
                "Different Candidate",
                "different candidate",
            ),
            SeedMention::new(
                EntityType::School,
                "Synthetic University",
                "synthetic university",
            ),
            SeedMention::new(EntityType::Skill, "Java", "java"),
        ],
    );
    seed_index(
        &data_dir,
        [
            (
                first_document_id,
                first_version_id,
                "synthetic-soft-a.pdf",
                "Java backend payments",
            ),
            (
                second_document_id,
                second_version_id,
                "synthetic-soft-b.pdf",
                "Java backend search",
            ),
            (
                distinct_document_id,
                distinct_version_id,
                "synthetic-distinct.pdf",
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
        .expect("run soft dedupe search");

    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("results: 3"));
    assert!(stdout.contains("synthetic-soft-a.pdf"));
    assert!(stdout.contains("synthetic-soft-b.pdf"));
    assert!(stdout.contains("synthetic-distinct.pdf"));
    assert_eq!(
        stdout.matches("soft_dedupe: suspected_versions=1").count(),
        2
    );
    assert!(stdout.contains("folded=false"));
    assert!(!stdout.contains("Synthetic Candidate"));
    assert!(!stdout.contains("synthetic candidate"));
    assert!(!stdout.contains("Synthetic University"));
    assert!(!stdout.contains("Example Labs"));

    remove_dir(&data_dir);
}

#[test]
fn candidate_review_merge_and_split_control_default_search_folding_without_value_leak() {
    let data_dir = temp_dir("candidate-review-data");
    let first_document_id = DocumentId::from_non_secret_parts(&["s18", "review-first-doc"]);
    let first_version_id = ResumeVersionId::from_non_secret_parts(&["s18", "review-first-ver"]);
    let second_document_id = DocumentId::from_non_secret_parts(&["s18", "review-second-doc"]);
    let second_version_id = ResumeVersionId::from_non_secret_parts(&["s18", "review-second-ver"]);
    let distinct_document_id = DocumentId::from_non_secret_parts(&["s18", "review-distinct-doc"]);
    let distinct_version_id =
        ResumeVersionId::from_non_secret_parts(&["s18", "review-distinct-ver"]);

    seed_document_with_mentions(
        &data_dir,
        first_document_id.clone(),
        first_version_id.clone(),
        None,
        "synthetic-review-a.pdf",
        "Java backend payments",
        &[
            SeedMention::new(
                EntityType::Name,
                "Private Review Candidate",
                "private review candidate",
            ),
            SeedMention::new(
                EntityType::School,
                "Private Review University",
                "private review university",
            ),
            SeedMention::new(
                EntityType::Company,
                "Private Review Labs",
                "private review labs",
            ),
            SeedMention::new(EntityType::Skill, "Java", "java"),
            SeedMention::new(EntityType::Skill, "Payments", "payments"),
        ],
    );
    seed_document_with_mentions(
        &data_dir,
        second_document_id.clone(),
        second_version_id.clone(),
        None,
        "synthetic-review-b.pdf",
        "Java backend search",
        &[
            SeedMention::new(
                EntityType::Name,
                "Private Review Candidate",
                "private review candidate",
            ),
            SeedMention::new(
                EntityType::School,
                "Private Review University",
                "private review university",
            ),
            SeedMention::new(EntityType::Skill, "Java", "java"),
            SeedMention::new(EntityType::Skill, "Search", "search"),
        ],
    );
    seed_document_with_mentions(
        &data_dir,
        distinct_document_id.clone(),
        distinct_version_id.clone(),
        None,
        "synthetic-review-distinct.pdf",
        "Java backend observability",
        &[
            SeedMention::new(EntityType::Name, "Distinct Review", "distinct review"),
            SeedMention::new(
                EntityType::School,
                "Private Review University",
                "private review university",
            ),
            SeedMention::new(EntityType::Skill, "Java", "java"),
        ],
    );
    seed_index(
        &data_dir,
        [
            (
                first_document_id,
                first_version_id.clone(),
                "synthetic-review-a.pdf",
                "Java backend payments",
            ),
            (
                second_document_id,
                second_version_id.clone(),
                "synthetic-review-b.pdf",
                "Java backend search",
            ),
            (
                distinct_document_id,
                distinct_version_id,
                "synthetic-review-distinct.pdf",
                "Java backend observability",
            ),
        ],
    );

    let list = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "candidate-review",
            "list",
            "--limit",
            "5",
        ])
        .output()
        .expect("run candidate-review list");
    assert!(
        list.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&list.stdout),
        String::from_utf8_lossy(&list.stderr)
    );
    assert!(list.stderr.is_empty());
    let list_stdout = String::from_utf8_lossy(&list.stdout);
    assert!(list_stdout.contains("candidate review suggestions: 1"));
    assert!(list_stdout.contains("suggestion: 1"));
    assert!(list_stdout.contains("versions: 2"));
    assert!(list_stdout.contains("paths: <redacted>"));
    assert!(!list_stdout.contains("Private Review Candidate"));
    assert!(!list_stdout.contains("Private Review University"));
    assert!(!list_stdout.contains("Private Review Labs"));
    assert!(!list_stdout.contains(path_str(&data_dir)));

    let merge = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "candidate-review",
            "merge",
            "--version",
            first_version_id.as_str(),
            "--version",
            second_version_id.as_str(),
            "--confidence",
            "0.91",
        ])
        .output()
        .expect("run candidate-review merge");
    assert!(
        merge.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&merge.stdout),
        String::from_utf8_lossy(&merge.stderr)
    );
    assert!(merge.stderr.is_empty());
    let merge_stdout = String::from_utf8_lossy(&merge.stdout);
    assert!(merge_stdout.contains("candidate review merge: completed"));
    assert!(merge_stdout.contains("versions assigned: 2"));
    assert!(merge_stdout.contains("paths: <redacted>"));
    assert!(!merge_stdout.contains("Private Review Candidate"));
    assert!(!merge_stdout.contains(path_str(&data_dir)));

    let folded = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "search",
            "Java",
            "--top-k",
            "10",
        ])
        .output()
        .expect("run folded search after review merge");
    assert!(folded.status.success());
    assert!(folded.stderr.is_empty());
    let folded_stdout = String::from_utf8_lossy(&folded.stdout);
    assert!(folded_stdout.contains("results: 2"));
    assert_eq!(folded_stdout.matches("soft_dedupe:").count(), 0);

    let candidate_id = searchable_versions(&data_dir)
        .into_iter()
        .find(|version| version.id == first_version_id)
        .and_then(|version| version.candidate_id)
        .expect("merged candidate id");

    let split = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "candidate-review",
            "split",
            "--candidate",
            candidate_id.as_str(),
        ])
        .output()
        .expect("run candidate-review split");
    assert!(
        split.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&split.stdout),
        String::from_utf8_lossy(&split.stderr)
    );
    assert!(split.stderr.is_empty());
    let split_stdout = String::from_utf8_lossy(&split.stdout);
    assert!(split_stdout.contains("candidate review split: completed"));
    assert!(split_stdout.contains("versions unassigned: 2"));
    assert!(split_stdout.contains("paths: <redacted>"));
    assert!(!split_stdout.contains(path_str(&data_dir)));

    let unfolded = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "search",
            "Java",
            "--top-k",
            "10",
        ])
        .output()
        .expect("run unfolded search after review split");
    assert!(unfolded.status.success());
    assert!(unfolded.stderr.is_empty());
    let unfolded_stdout = String::from_utf8_lossy(&unfolded.stdout);
    assert!(unfolded_stdout.contains("results: 3"));
    assert_eq!(
        unfolded_stdout
            .matches("soft_dedupe: suspected_versions=1")
            .count(),
        2
    );

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
    let store = MetaStore::open_data_dir(data_dir).unwrap();
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

fn searchable_versions(data_dir: &Path) -> Vec<ResumeVersion> {
    let store = MetaStore::open_data_dir(data_dir).unwrap();
    store.run_migrations().unwrap();
    let mut versions = Vec::new();
    for document in store.visible_documents().unwrap() {
        versions.extend(store.resume_versions_for_document(&document.id).unwrap());
    }
    versions.sort_by(|left, right| left.id.as_str().cmp(right.id.as_str()));
    versions
}

fn seed_document_with_mentions(
    data_dir: &Path,
    document_id: DocumentId,
    version_id: ResumeVersionId,
    candidate_id: Option<CandidateId>,
    file_name: &str,
    clean_text: &str,
    mentions: &[SeedMention],
) {
    let now = UnixTimestamp::from_unix_seconds(1_800_018_000);
    let store = MetaStore::open_data_dir(data_dir).unwrap();
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
    let mentions = mentions
        .iter()
        .enumerate()
        .map(|(index, mention)| EntityMention {
            id: EntityMentionId::from_non_secret_parts(&[
                "s18",
                version_id.as_str(),
                &index.to_string(),
            ]),
            resume_version_id: version_id.clone(),
            section_id: None,
            entity_type: mention.entity_type.clone(),
            raw_value: mention.raw_value.to_string(),
            normalized_value: Some(mention.normalized_value.to_string()),
            span_start: Some(index),
            span_end: Some(index + mention.raw_value.len()),
            confidence: mention.confidence,
            extractor: "s18-synthetic".to_string(),
        })
        .collect::<Vec<_>>();
    store
        .replace_entity_mentions(&version_id, &mentions)
        .unwrap();
}

struct SeedMention {
    entity_type: EntityType,
    raw_value: &'static str,
    normalized_value: &'static str,
    confidence: f32,
}

impl SeedMention {
    fn new(
        entity_type: EntityType,
        raw_value: &'static str,
        normalized_value: &'static str,
    ) -> Self {
        Self {
            entity_type,
            raw_value,
            normalized_value,
            confidence: 0.95,
        }
    }
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

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use meta_store::{
    Document, DocumentId, DocumentStatus, EntityMention, EntityMentionId, EntityType,
    FileExtension, MetaStore, ResumeVersion, ResumeVersionId, ResumeVisibility, UnixTimestamp,
};

#[test]
fn detail_local_prints_redacted_fields_and_short_snippet_without_private_paths() {
    let data_dir = temp_dir("detail-local-data");
    let doc_id = DocumentId::from_non_secret_parts(&["s49", "detail-local-doc"]);
    let version_id = ResumeVersionId::from_non_secret_parts(&["s49", "detail-local-version"]);
    let old_version_id = ResumeVersionId::from_non_secret_parts(&["s49", "detail-old-version"]);
    let private_path = "/Users/frank/private/resumes/candidate@example.test-java.pdf";
    let private_tail = "PRIVATE_TRAILING_MARKER_SHOULD_NOT_APPEAR";
    seed_detail_resume(
        &data_dir,
        &doc_id,
        &old_version_id,
        &version_id,
        private_path,
        private_tail,
    );

    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "detail",
            "--doc-id",
            doc_id.as_str(),
        ])
        .output()
        .expect("run resume-cli detail");

    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("resume detail"));
    assert!(stdout.contains(&format!("doc_id: {doc_id}")));
    assert!(stdout.contains(&format!("version_id: {version_id}")));
    assert!(!stdout.contains(old_version_id.as_str()));
    assert!(stdout.contains("file_name:"));
    assert!(stdout.contains("extension: pdf"));
    assert!(stdout.contains("document status: searchable"));
    assert!(stdout.contains("visibility: searchable"));
    assert!(stdout.contains("byte_size: 4096"));
    assert!(stdout.contains("fields:"));
    assert!(stdout.contains("field: degree"));
    assert!(stdout.contains("field: skill"));
    assert!(stdout.contains("field: company"));
    assert!(stdout.contains("field: title"));
    assert!(stdout.contains("field: certificate"));
    assert!(stdout.contains("field: other"));
    assert!(stdout.contains("snippet:"));
    assert!(!stdout.contains("candidate@example.test"));
    assert!(!stdout.contains("155-555-0199"));
    assert!(!stdout.contains(private_path));
    assert!(!stdout.contains("private/resumes"));
    assert!(!stdout.contains("OLD_VERSION_SHOULD_NOT_APPEAR"));
    assert!(!stdout.contains(private_tail));

    remove_dir(&data_dir);
}

#[test]
fn detail_local_rejects_deleted_documents_without_returning_version_data() {
    let data_dir = temp_dir("detail-local-deleted-data");
    let doc_id = DocumentId::from_non_secret_parts(&["s49", "detail-deleted-doc"]);
    let version_id = ResumeVersionId::from_non_secret_parts(&["s49", "detail-deleted-version"]);
    seed_deleted_resume(&data_dir, &doc_id, &version_id);

    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "detail",
            "--doc-id",
            doc_id.as_str(),
        ])
        .output()
        .expect("run resume-cli detail deleted doc");

    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("detail document was not found"));
    assert!(!stderr.contains("DELETED_VERSION_SHOULD_NOT_APPEAR"));

    remove_dir(&data_dir);
}

fn seed_detail_resume(
    data_dir: &Path,
    document_id: &DocumentId,
    old_version_id: &ResumeVersionId,
    version_id: &ResumeVersionId,
    private_path: &str,
    private_tail: &str,
) {
    let now = UnixTimestamp::from_unix_seconds(1_800_049_000);
    let store = MetaStore::open_data_dir(data_dir).unwrap();
    store.run_migrations().unwrap();
    store
        .upsert_document(&Document {
            id: document_id.clone(),
            source_uri: format!("file://{private_path}"),
            normalized_path: private_path.to_string(),
            file_name: "candidate@example.test-java.pdf".to_string(),
            extension: FileExtension::Pdf,
            byte_size: 4096,
            mtime: now,
            content_hash: Some("s49-detail-content-hash".to_string()),
            text_hash: None,
            is_deleted: false,
            created_at: now,
            updated_at: now,
            status: DocumentStatus::Searchable,
        })
        .unwrap();
    store
        .upsert_resume_version(&ResumeVersion {
            id: old_version_id.clone(),
            document_id: document_id.clone(),
            candidate_id: None,
            parse_version: "parser-v0".to_string(),
            schema_version: "schema-v1".to_string(),
            language_set: vec!["en".to_string()],
            page_count: Some(1),
            raw_text: Some("OLD_VERSION_SHOULD_NOT_APPEAR".to_string()),
            clean_text: Some("OLD_VERSION_SHOULD_NOT_APPEAR".to_string()),
            quality_score: Some(0.3),
            visibility: ResumeVisibility::Searchable,
        })
        .unwrap();
    store
        .upsert_resume_version(&ResumeVersion {
            id: version_id.clone(),
            document_id: document_id.clone(),
            candidate_id: None,
            parse_version: "parser-v1".to_string(),
            schema_version: "schema-v1".to_string(),
            language_set: vec!["en".to_string()],
            page_count: Some(2),
            raw_text: Some(format!(
                "raw candidate@example.test 155-555-0199 {private_tail}"
            )),
            clean_text: Some(format!(
                "Java platform engineer candidate@example.test 155-555-0199 {private_path} \
                 led payment routing with Rust and Kubernetes. {} {}",
                "skill evidence ".repeat(30),
                private_tail
            )),
            quality_score: Some(0.91),
            visibility: ResumeVisibility::Searchable,
        })
        .unwrap();
    store
        .replace_entity_mentions(
            version_id,
            &[
                entity_mention(version_id, "degree", EntityType::Degree, "master", 0.96),
                entity_mention(version_id, "skill", EntityType::Skill, "Kubernetes", 0.94),
                entity_mention(
                    version_id,
                    "company",
                    EntityType::Company,
                    "Acme Payments",
                    0.9,
                ),
                entity_mention(
                    version_id,
                    "title",
                    EntityType::Title,
                    "Staff Engineer",
                    0.9,
                ),
                entity_mention(
                    version_id,
                    "certificate",
                    EntityType::Certificate,
                    "AWS Certified Developer",
                    0.86,
                ),
                entity_mention(
                    version_id,
                    "email",
                    EntityType::Email,
                    "candidate@example.test",
                    0.99,
                ),
                entity_mention(version_id, "phone", EntityType::Phone, "155-555-0199", 0.99),
                entity_mention(
                    version_id,
                    "other",
                    EntityType::Other("/Users/frank/private/entity-type".to_string()),
                    "/Users/frank/private/field-value",
                    0.8,
                ),
            ],
        )
        .unwrap();
}

fn seed_deleted_resume(data_dir: &Path, document_id: &DocumentId, version_id: &ResumeVersionId) {
    let now = UnixTimestamp::from_unix_seconds(1_800_049_001);
    let store = MetaStore::open_data_dir(data_dir).unwrap();
    store.run_migrations().unwrap();
    store
        .upsert_document(&Document {
            id: document_id.clone(),
            source_uri: "file:///Users/frank/private/deleted.pdf".to_string(),
            normalized_path: "/Users/frank/private/deleted.pdf".to_string(),
            file_name: "deleted.pdf".to_string(),
            extension: FileExtension::Pdf,
            byte_size: 512,
            mtime: now,
            content_hash: Some("s49-deleted-content-hash".to_string()),
            text_hash: None,
            is_deleted: true,
            created_at: now,
            updated_at: now,
            status: DocumentStatus::Deleted,
        })
        .unwrap();
    store
        .upsert_resume_version(&ResumeVersion {
            id: version_id.clone(),
            document_id: document_id.clone(),
            candidate_id: None,
            parse_version: "parser-v1".to_string(),
            schema_version: "schema-v1".to_string(),
            language_set: vec!["en".to_string()],
            page_count: Some(1),
            raw_text: Some("DELETED_VERSION_SHOULD_NOT_APPEAR".to_string()),
            clean_text: Some("DELETED_VERSION_SHOULD_NOT_APPEAR".to_string()),
            quality_score: Some(0.1),
            visibility: ResumeVisibility::Searchable,
        })
        .unwrap();
}

fn entity_mention(
    version_id: &ResumeVersionId,
    label: &str,
    entity_type: EntityType,
    value: &str,
    confidence: f32,
) -> EntityMention {
    EntityMention {
        id: EntityMentionId::from_non_secret_parts(&["s49", version_id.as_str(), label]),
        resume_version_id: version_id.clone(),
        section_id: None,
        entity_type,
        raw_value: value.to_string(),
        normalized_value: Some(value.to_string()),
        span_start: Some(0),
        span_end: Some(value.len()),
        confidence,
        extractor: "s49-test".to_string(),
    }
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

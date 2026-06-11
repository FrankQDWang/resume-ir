use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use meta_store::{
    Document, DocumentId, DocumentStatus, EntityMention, EntityMentionId, EntityType,
    FileExtension, MetaStore, ResumeVersion, ResumeVersionId, ResumeVisibility, UnixTimestamp,
};

#[test]
fn benchmark_query_set_draft_writes_local_private_queries_without_stdout_leaks() {
    let data_dir = temp_dir("query-set-data");
    let out_dir = temp_dir("query-set-private-out");
    let query_set = out_dir.join("private-query-set.local.jsonl");
    seed_searchable_document_with_mentions(
        &data_dir,
        "alpha-private-resume.pdf",
        &[
            mention(
                EntityType::Name,
                "Alice Private Candidate",
                "alice private",
                0.99,
            ),
            mention(
                EntityType::Email,
                "alice.private@example.test",
                "alice.private@example.test",
                0.99,
            ),
            mention(EntityType::Phone, "+1 415 555 0132", "+14155550132", 0.99),
            mention(
                EntityType::Company,
                "Private Payments Incorporated",
                "private_payments",
                0.94,
            ),
            mention(
                EntityType::Title,
                "Backend Platform Engineer",
                "backend_platform_engineer",
                0.96,
            ),
            mention(EntityType::Skill, "Rust", "rust", 0.97),
            mention(EntityType::Skill, "Tantivy", "tantivy", 0.91),
        ],
    );
    seed_searchable_document_with_mentions(
        &data_dir,
        "beta-private-resume.docx",
        &[
            mention(
                EntityType::School,
                "Private Technical University",
                "private_technical_university",
                0.95,
            ),
            mention(
                EntityType::Major,
                "Computer Science",
                "computer_science",
                0.94,
            ),
            mention(EntityType::Certificate, "CKA", "cka", 0.95),
            mention(EntityType::Location, "Shanghai", "shanghai", 0.91),
            mention(
                EntityType::Title,
                "Search Engineer",
                "search_engineer",
                0.95,
            ),
        ],
    );

    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "benchmark-query-set",
            "draft",
            "--out",
            path_str(&query_set),
            "--max-queries",
            "5",
            "--min-queries",
            "4",
        ])
        .output()
        .expect("draft local private query set");

    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("query set: written"));
    assert!(stdout.contains("schema: resume-ir.query-set.jsonl.v1"));
    assert!(stdout.contains("privacy boundary: local_only_private_query_set"));
    assert!(stdout.contains("queries: 5"));
    assert!(stdout.contains("query set sha256: "));
    assert!(stdout.contains("queries: <redacted>"));
    for forbidden in [
        path_str(&data_dir),
        path_str(&out_dir),
        "Alice",
        "alice.private@example.test",
        "+14155550132",
        "Private Payments",
        "backend_platform_engineer",
        "private_technical_university",
    ] {
        assert!(!stdout.contains(forbidden), "stdout leaked {forbidden}");
    }

    let query_set_text = fs::read_to_string(&query_set).unwrap();
    let lines = query_set_text.lines().collect::<Vec<_>>();
    assert_eq!(lines.len(), 5);
    let mut sample_ids = Vec::new();
    let mut queries = Vec::new();
    for line in lines {
        let value: serde_json::Value = serde_json::from_str(line).unwrap();
        sample_ids.push(value["sample_id"].as_str().unwrap().to_string());
        queries.push(value["query"].as_str().unwrap().to_string());
    }
    assert_eq!(sample_ids[0], "local-query-000001");
    assert!(queries.iter().any(|query| query.contains("rust")));
    assert!(queries
        .iter()
        .any(|query| query.contains("backend_platform_engineer")));
    assert!(queries
        .iter()
        .any(|query| query.contains("private_technical_university")));
    for forbidden in [
        path_str(&data_dir),
        path_str(&out_dir),
        "alpha-private-resume",
        "beta-private-resume",
        "Alice Private Candidate",
        "alice.private@example.test",
        "+14155550132",
    ] {
        assert!(
            !query_set_text.contains(forbidden),
            "query set leaked {forbidden}"
        );
    }

    remove_dir(&data_dir);
    remove_dir(&out_dir);
}

#[test]
fn benchmark_query_set_draft_rejects_insufficient_queries_without_path_leak() {
    let data_dir = temp_dir("query-set-insufficient-data");
    let out_dir = temp_dir("query-set-insufficient-private-out");
    let query_set = out_dir.join("private-query-set.local.jsonl");
    seed_searchable_document_with_mentions(
        &data_dir,
        "insufficient-private-resume.pdf",
        &[mention(
            EntityType::Email,
            "insufficient@example.test",
            "insufficient@example.test",
            0.99,
        )],
    );

    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "benchmark-query-set",
            "draft",
            "--out",
            path_str(&query_set),
            "--max-queries",
            "10",
            "--min-queries",
            "1",
        ])
        .output()
        .expect("reject insufficient query set");

    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("query set blocked: not enough local field queries"));
    assert!(!stderr.contains(path_str(&data_dir)));
    assert!(!stderr.contains(path_str(&out_dir)));
    assert!(!stderr.contains("insufficient-private-resume"));
    assert!(!stderr.contains("insufficient@example.test"));
    assert!(!query_set.exists());

    remove_dir(&data_dir);
    remove_dir(&out_dir);
}

#[test]
fn benchmark_query_set_draft_can_use_explicit_keyword_fallback_for_smoke() {
    let data_dir = temp_dir("query-set-keyword-fallback-data");
    let out_dir = temp_dir("query-set-keyword-fallback-private-out");
    let query_set = out_dir.join("private-query-set.local.jsonl");
    seed_searchable_document_with_mentions_and_text(
        &data_dir,
        "ocr-only-private-resume.pdf",
        &[mention(
            EntityType::Email,
            "ocr.only@example.test",
            "ocr.only@example.test",
            0.99,
        )],
        "This local OCR candidate has rust indexing retrieval and offline ranking experience.",
    );

    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "benchmark-query-set",
            "draft",
            "--out",
            path_str(&query_set),
            "--max-queries",
            "3",
            "--min-queries",
            "1",
            "--allow-keyword-fallback",
        ])
        .output()
        .expect("draft local private query set with keyword fallback");

    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("query set: written"));
    assert!(stdout.contains("schema: resume-ir.query-set.jsonl.v1"));
    assert!(stdout.contains("privacy boundary: local_only_private_query_set"));
    assert!(stdout.contains("query fallback: keyword"));
    assert!(stdout.contains("queries: 3"));
    assert!(stdout.contains("queries: <redacted>"));
    for forbidden in [
        path_str(&data_dir),
        path_str(&out_dir),
        "ocr.only@example.test",
        "ocr-only-private-resume",
    ] {
        assert!(!stdout.contains(forbidden), "stdout leaked {forbidden}");
    }

    let query_set_text = fs::read_to_string(&query_set).unwrap();
    let lines = query_set_text.lines().collect::<Vec<_>>();
    assert_eq!(lines.len(), 3);
    assert!(query_set_text.contains("rust"));
    assert!(query_set_text.contains("indexing"));
    for forbidden in [
        path_str(&data_dir),
        path_str(&out_dir),
        "ocr.only@example.test",
        "ocr-only-private-resume",
    ] {
        assert!(
            !query_set_text.contains(forbidden),
            "query set leaked {forbidden}"
        );
    }

    remove_dir(&data_dir);
    remove_dir(&out_dir);
}

fn seed_searchable_document_with_mentions(
    data_dir: &Path,
    file_name: &str,
    mentions: &[SeedMention],
) {
    seed_searchable_document_with_mentions_and_text(
        data_dir,
        file_name,
        mentions,
        &format!("synthetic text for {file_name}"),
    );
}

fn seed_searchable_document_with_mentions_and_text(
    data_dir: &Path,
    file_name: &str,
    mentions: &[SeedMention],
    text: &str,
) {
    let store = MetaStore::open_data_dir(data_dir).unwrap();
    store.run_migrations().unwrap();
    let document_id = DocumentId::from_non_secret_parts(&["s304", file_name]);
    let version_id = ResumeVersionId::from_non_secret_parts(&["s304", file_name, "version"]);
    let now = UnixTimestamp::from_unix_seconds(1_800_304_000);
    store
        .upsert_document(&Document {
            id: document_id.clone(),
            source_uri: format!("synthetic://{file_name}"),
            normalized_path: format!("synthetic/{file_name}"),
            file_name: file_name.to_string(),
            extension: FileExtension::Pdf,
            byte_size: 256,
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
            document_id: document_id.clone(),
            candidate_id: None,
            parse_version: "parser-v1".to_string(),
            schema_version: "schema-v1".to_string(),
            language_set: vec!["en".to_string()],
            page_count: Some(1),
            raw_text: Some(text.to_string()),
            clean_text: Some(text.to_string()),
            quality_score: Some(0.8),
            visibility: ResumeVisibility::Searchable,
        })
        .unwrap();
    let mentions = mentions
        .iter()
        .enumerate()
        .map(|(index, seed)| EntityMention {
            id: EntityMentionId::from_non_secret_parts(&["s304", file_name, &index.to_string()]),
            resume_version_id: version_id.clone(),
            section_id: None,
            entity_type: seed.entity_type.clone(),
            raw_value: seed.raw_value.to_string(),
            normalized_value: Some(seed.normalized_value.to_string()),
            span_start: Some(index),
            span_end: Some(index + seed.raw_value.len()),
            confidence: seed.confidence,
            extractor: "s304-synthetic".to_string(),
        })
        .collect::<Vec<_>>();
    store
        .replace_entity_mentions(&version_id, &mentions)
        .unwrap();
}

fn mention(
    entity_type: EntityType,
    raw_value: &'static str,
    normalized_value: &'static str,
    confidence: f32,
) -> SeedMention {
    SeedMention {
        entity_type,
        raw_value,
        normalized_value,
        confidence,
    }
}

struct SeedMention {
    entity_type: EntityType,
    raw_value: &'static str,
    normalized_value: &'static str,
    confidence: f32,
}

fn temp_dir(label: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!("resume-ir-s304-{label}-{unique}"));
    fs::create_dir_all(&path).unwrap();
    path
}

fn path_str(path: &Path) -> &str {
    path.to_str().unwrap()
}

fn remove_dir(path: &Path) {
    let _ = fs::remove_dir_all(path);
}

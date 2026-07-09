use std::any::TypeId;
use std::str::FromStr;

use core_domain::{
    normalize_query_set_query, query_set_query_in_semantic_bounds, Candidate, CandidateId,
    ContactHash, Document, DocumentId, DocumentStatus, EntityMention, EntityMentionId, EntityType,
    ErrorKind, FileExtension, IdParseError, QuerySetSampleShape, QuerySetSampleShapeMetadata,
    QuerySetSourceKind, RedactionLevel, ResumeIrError, ResumeVersion, ResumeVersionId,
    ResumeVisibility, Section, SectionId, SectionType, SourceComponent, UnixTimestamp,
    VectorQuantization, VectorRecord, VectorRecordId, VectorScope, QUERY_SET_MAX_QUERY_BYTES,
    QUERY_SET_MAX_TERMS,
};

#[test]
fn opaque_ids_have_golden_generation_and_validated_hydration() {
    let document_id =
        DocumentId::from_non_secret_parts(&["synthetic-root", "resume-placeholder.txt"]);
    let same_document_id =
        DocumentId::from_non_secret_parts(&["synthetic-root", "resume-placeholder.txt"]);
    let other_document_id =
        DocumentId::from_non_secret_parts(&["synthetic-root", "other-placeholder.txt"]);

    assert_eq!(document_id.as_str(), "doc_4fcf3ffdf8561b4c56698040d8f36503");
    assert_eq!(document_id, same_document_id);
    assert_ne!(document_id, other_document_id);
    assert_eq!(
        DocumentId::from_str(document_id.as_str()).unwrap(),
        document_id
    );
    assert_eq!(
        DocumentId::try_from(document_id.as_str().to_string()).unwrap(),
        document_id
    );

    assert!(matches!(
        DocumentId::from_str("ver_4fcf3ffdf8561b4c56698040d8f36503"),
        Err(IdParseError::InvalidPrefix { .. })
    ));
    assert!(matches!(
        DocumentId::from_str("doc_4fcf3ffdf8561b4c"),
        Err(IdParseError::InvalidLength { .. })
    ));
    assert!(matches!(
        DocumentId::from_str("doc_4fcf3ffdf8561b4c56698040d8f3650z"),
        Err(IdParseError::InvalidHexDigest)
    ));

    assert_ne!(TypeId::of::<DocumentId>(), TypeId::of::<ResumeVersionId>());
}

#[test]
fn contact_hash_only_hydrates_external_keyed_digests() {
    let digest = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    let hash = ContactHash::from_keyed_digest(digest).unwrap();

    assert_eq!(hash.as_str(), digest);
    assert_eq!(hash.to_string(), "<redacted>");
    assert!(!hash.to_string().contains(digest));
    assert_eq!(ContactHash::try_from(digest.to_string()).unwrap(), hash);
    assert_eq!(
        ContactHash::from_keyed_digest(digest.to_ascii_uppercase())
            .unwrap()
            .as_str(),
        digest
    );
    assert!(ContactHash::from_keyed_digest("0123").is_err());
    assert!(ContactHash::from_keyed_digest(
        "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdez"
    )
    .is_err());
}

#[test]
fn query_set_sample_shape_is_the_shared_freeze_and_runner_bucket_semantics() {
    let field_filter = QuerySetSampleShape::from_query("rust backend shanghai");
    assert_eq!(field_filter.term_count(), 3);
    assert_eq!(field_filter.bucket(), "field_filter");
    assert!(field_filter.has_location());
    assert!(field_filter.has_skill());
    assert!(!field_filter.has_boolean());

    let hybrid = QuerySetSampleShape::from_query("rust AND backend");
    assert_eq!(hybrid.bucket(), "hybrid");
    assert!(hybrid.has_boolean());

    let semantic = QuerySetSampleShape::from_query("\"distributed systems\"");
    assert_eq!(semantic.term_count(), 1);
    assert_eq!(semantic.bucket(), "semantic");
    assert!(semantic.has_phrase());

    assert_eq!(
        QuerySetSampleShape::from_query("rust backend").bucket(),
        "and_2"
    );
    assert_eq!(
        QuerySetSampleShape::from_query("rust backend search ranking index systems storage")
            .bucket(),
        "and_6_16"
    );

    let declared = QuerySetSampleShape::from_metadata(QuerySetSampleShapeMetadata {
        term_count: 3,
        has_boolean: false,
        has_location: true,
        has_years: false,
        has_degree: false,
        has_skill: true,
        has_phrase: false,
    });
    assert_eq!(declared, field_filter);
}

#[test]
fn query_set_query_semantics_are_shared_execution_caps() {
    let max_term_query = (1..=QUERY_SET_MAX_TERMS)
        .map(|index| format!("term{index}"))
        .collect::<Vec<_>>()
        .join(" ");
    assert!(query_set_query_in_semantic_bounds(&max_term_query));

    let too_many_terms = format!("{max_term_query} overflow");
    assert!(!query_set_query_in_semantic_bounds(&too_many_terms));
    assert!(!query_set_query_in_semantic_bounds(""));

    let max_bytes = "a".repeat(QUERY_SET_MAX_QUERY_BYTES);
    assert!(query_set_query_in_semantic_bounds(&max_bytes));

    let too_many_bytes = "a".repeat(QUERY_SET_MAX_QUERY_BYTES + 1);
    assert!(!query_set_query_in_semantic_bounds(&too_many_bytes));
}

#[test]
fn query_set_query_normalization_dedupes_normalized_logical_terms() {
    assert_eq!(
        normalize_query_set_query("  ｒｕｓｔ   backend   rust backend  ").as_deref(),
        Some("rust backend")
    );
    assert_eq!(
        normalize_query_set_query("“distributed   systems” \"distributed systems\"").as_deref(),
        Some("\"distributed systems\"")
    );
    assert_eq!(normalize_query_set_query("").as_deref(), None);

    let too_many_terms = (0..=QUERY_SET_MAX_TERMS)
        .map(|index| format!("term{index}"))
        .collect::<Vec<_>>()
        .join(" ");
    assert_eq!(normalize_query_set_query(&too_many_terms).as_deref(), None);
}

#[test]
fn query_set_source_kind_is_shared_agent_replay_boundary() {
    assert_eq!(
        QuerySetSourceKind::from_str("trace_source_search_v1").unwrap(),
        QuerySetSourceKind::TraceSourceSearchV1
    );
    assert!(QuerySetSourceKind::TraceSourceSearchV1.is_agent_query_replay());
    assert_eq!(
        QuerySetSourceKind::TraceSourceSearchV1.as_str(),
        "trace_source_search_v1"
    );

    assert!(QuerySetSourceKind::from_str("local_field").is_err());
    assert!(QuerySetSourceKind::from_str("local_field_or_keyword_fallback").is_err());
    assert!(QuerySetSourceKind::from_str("query_history").is_err());
}

#[test]
fn domain_models_match_design_required_fields() {
    let discovered_at = UnixTimestamp::from_unix_seconds(1_800_000_000);
    let updated_at = UnixTimestamp::from_unix_seconds(1_800_000_120);
    let candidate_id = CandidateId::from_non_secret_parts(&["candidate-placeholder"]);
    let document_id =
        DocumentId::from_non_secret_parts(&["synthetic-root", "resume-placeholder.txt"]);
    let version_id = ResumeVersionId::from_non_secret_parts(&[
        document_id.as_str(),
        "sha256:SYNTHETIC_CONTENT_HASH",
        "parser-v1",
        "schema-v1",
    ]);
    let section_id = SectionId::from_non_secret_parts(&[version_id.as_str(), "skill", "0"]);
    let mention_id = EntityMentionId::from_non_secret_parts(&[
        version_id.as_str(),
        section_id.as_str(),
        "skill-token",
    ]);
    let vector_id = VectorRecordId::from_non_secret_parts(&[
        section_id.as_str(),
        "synthetic-vector-model",
        "fp16",
    ]);

    let candidate = Candidate {
        id: candidate_id.clone(),
        primary_name: Some("Synthetic Candidate".to_string()),
        phone_hash: Some(
            ContactHash::from_keyed_digest(
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            )
            .unwrap(),
        ),
        email_hash: Some(
            ContactHash::from_keyed_digest(
                "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            )
            .unwrap(),
        ),
        dedupe_key: Some("synthetic-dedupe-key".to_string()),
        merge_confidence: Some(0.91),
        version_count: 1,
    };
    let document = Document {
        id: document_id.clone(),
        source_uri: "file:///synthetic/resume-placeholder.txt".to_string(),
        normalized_path: "/synthetic/resume-placeholder.txt".to_string(),
        file_name: "resume-placeholder.txt".to_string(),
        extension: FileExtension::Txt,
        byte_size: 4096,
        mtime: discovered_at,
        content_hash: Some("sha256:SYNTHETIC_DOCUMENT_HASH".to_string()),
        text_hash: Some("sha256:SYNTHETIC_TEXT_HASH".to_string()),
        is_deleted: false,
        created_at: discovered_at,
        updated_at,
        status: DocumentStatus::Discovered,
    };
    let version = ResumeVersion {
        id: version_id.clone(),
        document_id: document.id.clone(),
        candidate_id: Some(candidate.id.clone()),
        parse_version: "parser-v1".to_string(),
        schema_version: "schema-v1".to_string(),
        language_set: vec!["en".to_string()],
        page_count: Some(2),
        raw_text: Some("SYNTHETIC RAW TEXT".to_string()),
        clean_text: Some("SYNTHETIC CLEAN TEXT".to_string()),
        quality_score: Some(0.87),
        visibility: ResumeVisibility::Searchable,
    };
    let section = Section {
        id: section_id.clone(),
        resume_version_id: version.id.clone(),
        section_type: SectionType::Skill,
        order_no: 3,
        page_no: Some(1),
        text: "SYNTHETIC SKILL TOKEN".to_string(),
        char_start: Some(10),
        char_end: Some(31),
        confidence: 0.93,
    };
    let mention = EntityMention {
        id: mention_id.clone(),
        resume_version_id: version.id.clone(),
        section_id: Some(section.id.clone()),
        entity_type: EntityType::Skill,
        raw_value: "SYNTHETIC_SKILL_TOKEN".to_string(),
        normalized_value: Some("synthetic-skill-token".to_string()),
        span_start: Some(18),
        span_end: Some(39),
        confidence: 0.88,
        extractor: "synthetic-rule-extractor".to_string(),
    };
    let vector = VectorRecord {
        id: vector_id,
        resume_version_id: version.id,
        section_id: Some(section.id),
        vector_scope: VectorScope::Section,
        model_id: "synthetic-vector-model".to_string(),
        dim: 384,
        quantization: VectorQuantization::Fp16,
        created_at: updated_at,
    };

    assert_eq!(version.candidate_id, Some(candidate_id));
    assert_eq!(
        document.normalized_path,
        "/synthetic/resume-placeholder.txt"
    );
    assert_eq!(document.extension, FileExtension::Txt);
    assert_eq!(version.visibility, ResumeVisibility::Searchable);
    assert_eq!(section.resume_version_id, version_id.clone());
    assert_eq!(mention.section_id, Some(section_id));
    assert_eq!(vector.resume_version_id, version_id);
    assert_eq!(vector.dim, 384);
    assert_eq!(candidate.version_count, 1);
}

#[test]
fn pii_bearing_domain_debug_output_is_redacted() {
    let timestamp = UnixTimestamp::from_unix_seconds(1_800_000_000);
    let candidate_id = CandidateId::from_non_secret_parts(&["candidate-debug-placeholder"]);
    let document_id = DocumentId::from_non_secret_parts(&["debug-root", "debug-resume.txt"]);
    let version_id = ResumeVersionId::from_non_secret_parts(&[document_id.as_str(), "debug-v1"]);
    let section_id = SectionId::from_non_secret_parts(&[version_id.as_str(), "debug-section"]);
    let mention_id =
        EntityMentionId::from_non_secret_parts(&[section_id.as_str(), "debug-mention"]);
    let contact_digest = "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";

    let candidate = Candidate {
        id: candidate_id.clone(),
        primary_name: Some("SYNTHETIC_DEBUG_NAME".to_string()),
        phone_hash: Some(ContactHash::from_keyed_digest(contact_digest).unwrap()),
        email_hash: None,
        dedupe_key: Some("SYNTHETIC_DEDUPE_KEY".to_string()),
        merge_confidence: Some(0.7),
        version_count: 1,
    };
    let document = Document {
        id: document_id.clone(),
        source_uri: "file:///private/synthetic-debug-resume.txt".to_string(),
        normalized_path: "/private/synthetic-debug-resume.txt".to_string(),
        file_name: "synthetic-debug-resume.txt".to_string(),
        extension: FileExtension::Txt,
        byte_size: 12,
        mtime: timestamp,
        content_hash: Some("SYNTHETIC_CONTENT_HASH_DEBUG".to_string()),
        text_hash: Some("SYNTHETIC_TEXT_HASH_DEBUG".to_string()),
        is_deleted: false,
        created_at: timestamp,
        updated_at: timestamp,
        status: DocumentStatus::Discovered,
    };
    let version = ResumeVersion {
        id: version_id.clone(),
        document_id: document_id.clone(),
        candidate_id: Some(candidate_id),
        parse_version: "parser-v1".to_string(),
        schema_version: "schema-v1".to_string(),
        language_set: vec!["en".to_string()],
        page_count: Some(1),
        raw_text: Some("SYNTHETIC_RAW_RESUME_DEBUG_TEXT".to_string()),
        clean_text: Some("SYNTHETIC_CLEAN_RESUME_DEBUG_TEXT".to_string()),
        quality_score: Some(0.8),
        visibility: ResumeVisibility::Partial,
    };
    let section = Section {
        id: section_id.clone(),
        resume_version_id: version_id.clone(),
        section_type: SectionType::Experience,
        order_no: 1,
        page_no: Some(1),
        text: "SYNTHETIC_SECTION_DEBUG_TEXT".to_string(),
        char_start: Some(0),
        char_end: Some(28),
        confidence: 0.9,
    };
    let mention = EntityMention {
        id: mention_id,
        resume_version_id: version_id,
        section_id: Some(section_id),
        entity_type: EntityType::Company,
        raw_value: "SYNTHETIC_ENTITY_DEBUG_VALUE".to_string(),
        normalized_value: Some("synthetic-entity-debug-value".to_string()),
        span_start: Some(0),
        span_end: Some(28),
        confidence: 0.86,
        extractor: "synthetic-rule-extractor".to_string(),
    };

    let debug = format!("{candidate:?}\n{document:?}\n{version:?}\n{section:?}\n{mention:?}");

    for sensitive in [
        "SYNTHETIC_DEBUG_NAME",
        contact_digest,
        "SYNTHETIC_DEDUPE_KEY",
        "file:///private/synthetic-debug-resume.txt",
        "/private/synthetic-debug-resume.txt",
        "synthetic-debug-resume.txt",
        "SYNTHETIC_CONTENT_HASH_DEBUG",
        "SYNTHETIC_TEXT_HASH_DEBUG",
        "SYNTHETIC_RAW_RESUME_DEBUG_TEXT",
        "SYNTHETIC_CLEAN_RESUME_DEBUG_TEXT",
        "SYNTHETIC_SECTION_DEBUG_TEXT",
        "SYNTHETIC_ENTITY_DEBUG_VALUE",
        "synthetic-entity-debug-value",
    ] {
        assert!(!debug.contains(sensitive), "debug leaked {sensitive}");
    }

    assert!(debug.contains("<redacted>"));
}

#[test]
fn document_status_covers_lifecycle_state_machine() {
    let statuses = [
        DocumentStatus::Discovered,
        DocumentStatus::Fingerprinted,
        DocumentStatus::ParseQueued,
        DocumentStatus::ParseRunning,
        DocumentStatus::TextExtracted,
        DocumentStatus::OcrRequired,
        DocumentStatus::OcrRunning,
        DocumentStatus::OcrDone,
        DocumentStatus::TextCleaned,
        DocumentStatus::FieldsExtracted,
        DocumentStatus::EmbeddingDone,
        DocumentStatus::IndexedPartial,
        DocumentStatus::Searchable,
        DocumentStatus::FailedRetryable,
        DocumentStatus::FailedPermanent,
        DocumentStatus::Deleted,
    ];

    assert_eq!(statuses.len(), 16);
}

#[test]
fn error_display_and_debug_redact_sensitive_diagnostics() {
    let diagnostic = "parser note includes SYNTHETIC_SECRET_TOKEN";
    let error = ResumeIrError::new(
        ErrorKind::CorruptedDocument,
        true,
        "Could not read this placeholder resume.",
        diagnostic,
        RedactionLevel::Sensitive,
        SourceComponent::CoreDomain,
    );

    assert_eq!(error.kind, ErrorKind::CorruptedDocument);
    assert!(error.retryable);
    assert_eq!(
        error.user_message,
        "Could not read this placeholder resume."
    );
    assert_eq!(error.diagnostic_message(), diagnostic);
    assert_eq!(error.redaction_level, RedactionLevel::Sensitive);
    assert_eq!(error.source_component, SourceComponent::CoreDomain);

    let display = error.to_string();
    let debug = format!("{error:?}");

    assert!(!display.contains("SYNTHETIC_SECRET_TOKEN"));
    assert!(!debug.contains("SYNTHETIC_SECRET_TOKEN"));
    assert!(display.contains("Could not read this placeholder resume."));
    assert!(debug.contains("<redacted>"));
}

#[test]
fn error_kind_matches_layering_design_list() {
    let kinds = [
        ErrorKind::ConfigError,
        ErrorKind::IoError,
        ErrorKind::PermissionDenied,
        ErrorKind::UnsupportedFormat,
        ErrorKind::EncryptedDocument,
        ErrorKind::CorruptedDocument,
        ErrorKind::ParserTimeout,
        ErrorKind::OcrTimeout,
        ErrorKind::ModelError,
        ErrorKind::IndexCorrupted,
        ErrorKind::SchemaMismatch,
        ErrorKind::ResourceExhausted,
        ErrorKind::Cancelled,
        ErrorKind::InternalBug,
    ];

    assert_eq!(kinds.len(), 14);
}

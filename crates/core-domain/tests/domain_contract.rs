#![allow(missing_docs)]

use core_domain::{
    Candidate, CandidateId, Document, DocumentExtension, DocumentId, EntityMention,
    EntityMentionId, EntityType, ErrorKind, RedactionLevel, ResumeError, ResumeVersion,
    ResumeVersionId, ResumeVisibility, Section, SectionId, SectionType, SourceComponent,
};

#[test]
fn generated_ids_are_typed_and_unique() {
    let first = DocumentId::new();
    let second = DocumentId::new();
    let candidate = CandidateId::new();

    assert_ne!(first, second);
    assert_ne!(first.to_string(), candidate.to_string());
    assert!(first.to_string().starts_with("doc_"));
    assert!(candidate.to_string().starts_with("cand_"));
}

#[test]
fn error_carries_redaction_and_audience_fields() {
    let error = ResumeError::new(
        ErrorKind::PermissionDenied,
        false,
        "Cannot read this file. Check local permissions.",
        "permission denied opening /redacted/path",
        RedactionLevel::Safe,
        SourceComponent::CoreDomain,
    );

    assert_eq!(error.kind(), ErrorKind::PermissionDenied);
    assert!(!error.retryable());
    assert_eq!(
        error.user_message(),
        "Cannot read this file. Check local permissions."
    );
    assert_eq!(error.redaction_level(), RedactionLevel::Safe);
    assert_eq!(error.source_component(), SourceComponent::CoreDomain);
    assert!(error.local_diagnostic_message().contains("/redacted/path"));
}

#[test]
fn pii_bearing_debug_output_is_redacted() {
    let doc_id = DocumentId::new();
    let version_id = ResumeVersionId::new();
    let section_id = SectionId::new();

    let document = Document {
        doc_id: doc_id.clone(),
        source_uri: "/synthetic/private/source/candidate_file.pdf".to_string(),
        normalized_path: "/synthetic/private/source/candidate_file.pdf".to_string(),
        file_name: "candidate_file.pdf".to_string(),
        extension: DocumentExtension::Pdf,
        byte_size: 42,
        mtime: "2026-05-31T00:00:00Z".to_string(),
        content_hash: Some("content-hash".to_string()),
        text_hash: Some("text-hash".to_string()),
        is_deleted: false,
        created_at: "2026-05-31T00:00:00Z".to_string(),
        updated_at: "2026-05-31T00:00:00Z".to_string(),
    };
    let version = ResumeVersion {
        version_id: version_id.clone(),
        doc_id: doc_id.clone(),
        candidate_id: Some(CandidateId::new()),
        parse_version: "parser-v1".to_string(),
        schema_version: "schema-v1".to_string(),
        language_set: vec!["en".to_string()],
        page_count: Some(1),
        raw_text: Some("private raw text token".to_string()),
        clean_text: Some("private clean text token".to_string()),
        quality_score: Some(0.9),
        visibility: ResumeVisibility::Searchable,
    };
    let candidate = Candidate {
        candidate_id: CandidateId::new(),
        primary_name: Some("candidate display token".to_string()),
        phone_hash: Some("phone-hash".to_string()),
        email_hash: Some("email-hash".to_string()),
        dedupe_key: Some("dedupe-key".to_string()),
        merge_confidence: Some(0.9),
        version_count: 1,
    };
    let section = Section {
        section_id: section_id.clone(),
        version_id: version_id.clone(),
        section_type: SectionType::Experience,
        order_no: 1,
        page_no: Some(1),
        text: "section confidential token".to_string(),
        char_start: Some(0),
        char_end: Some(19),
        confidence: 0.9,
    };
    let mention = EntityMention {
        mention_id: EntityMentionId::new(),
        version_id,
        section_id: Some(section_id),
        entity_type: EntityType::Email,
        raw_value: "entity-secret-token".to_string(),
        normalized_value: Some("entity-secret-token".to_string()),
        span_start: Some(0),
        span_end: Some(21),
        confidence: 0.99,
        extractor: "rule".to_string(),
    };

    assert_debug_redacts(&document, &["/synthetic/private", "candidate_file.pdf"]);
    assert_debug_redacts(&version, &["private raw text", "private clean text"]);
    assert_debug_redacts(&candidate, &["candidate display token"]);
    assert_debug_redacts(&section, &["section confidential token"]);
    assert_debug_redacts(&mention, &["entity-secret-token"]);
}

#[test]
fn sensitive_error_debug_and_default_diagnostic_are_redacted() {
    let error = ResumeError::new(
        ErrorKind::CorruptedDocument,
        false,
        "Could not parse this file.",
        "failed near sensitive diagnostic token",
        RedactionLevel::Sensitive,
        SourceComponent::CoreDomain,
    );

    assert_eq!(
        error.redacted_diagnostic_message(),
        "[redacted sensitive diagnostic]"
    );
    assert!(error
        .local_diagnostic_message()
        .contains("failed near sensitive diagnostic token"));
    assert_debug_redacts(&error, &["sensitive diagnostic token"]);
}

fn assert_debug_redacts(value: &impl std::fmt::Debug, forbidden: &[&str]) {
    let debug_output = format!("{value:?}");

    for token in forbidden {
        assert!(
            !debug_output.contains(token),
            "debug output leaked {token:?}: {debug_output}"
        );
    }
}

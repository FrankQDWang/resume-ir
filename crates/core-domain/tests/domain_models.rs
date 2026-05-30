use core_domain::{
    Candidate, CandidateId, Document, DocumentExtension, DocumentId, EntityMention,
    EntityMentionId, EntityType, Quantization, ResumeVersion, ResumeVersionId, Section, SectionId,
    SectionType, VectorRecord, VectorRecordId, VectorScope, Visibility,
};
use std::time::SystemTime;

#[test]
fn generated_ids_have_type_specific_prefixes_and_are_unique() {
    let first_doc = DocumentId::new();
    let second_doc = DocumentId::new();
    let version = ResumeVersionId::new();
    let candidate = CandidateId::new();

    assert_ne!(first_doc, second_doc);
    assert!(first_doc.as_str().starts_with("doc_"));
    assert!(version.as_str().starts_with("ver_"));
    assert!(candidate.as_str().starts_with("cand_"));
}

#[test]
fn domain_models_keep_identity_types_distinct() {
    let now = SystemTime::UNIX_EPOCH;
    let doc_id = DocumentId::new();
    let version_id = ResumeVersionId::new();
    let section_id = SectionId::new();

    let document = Document {
        doc_id: doc_id.clone(),
        source_uri: "file:///fixtures/resume.docx".to_owned(),
        normalized_path: "/fixtures/resume.docx".to_owned(),
        file_name: "resume.docx".to_owned(),
        extension: DocumentExtension::Docx,
        byte_size: 128,
        mtime: now,
        content_hash: None,
        text_hash: None,
        is_deleted: false,
        created_at: now,
        updated_at: now,
    };

    let version = ResumeVersion {
        version_id: version_id.clone(),
        doc_id,
        candidate_id: None,
        parse_version: "parser-v1".to_owned(),
        schema_version: "schema-v1".to_owned(),
        language_set: vec!["zh".to_owned(), "en".to_owned()],
        page_count: Some(2),
        raw_text: None,
        clean_text: Some("Java backend engineer".to_owned()),
        quality_score: Some(0.95),
        visibility: Visibility::Searchable,
    };

    let candidate = Candidate {
        candidate_id: CandidateId::new(),
        primary_name: Some("Test Candidate".to_owned()),
        phone_hash: Some("phone_hash".to_owned()),
        email_hash: None,
        dedupe_key: None,
        merge_confidence: None,
        version_count: 1,
    };

    let section = Section {
        section_id: section_id.clone(),
        version_id: version_id.clone(),
        section_type: SectionType::Experience,
        order_no: 1,
        page_no: Some(1),
        text: "Built payment systems".to_owned(),
        char_start: Some(0),
        char_end: Some(21),
        confidence: 0.9,
    };

    let mention = EntityMention {
        mention_id: EntityMentionId::new(),
        version_id: version_id.clone(),
        section_id: Some(section_id),
        entity_type: EntityType::Skill,
        raw_value: "Java".to_owned(),
        normalized_value: Some("java".to_owned()),
        span_start: Some(0),
        span_end: Some(4),
        confidence: 0.99,
        extractor: "rule".to_owned(),
    };

    let vector = VectorRecord {
        vector_id: VectorRecordId::new(),
        version_id,
        section_id: mention.section_id.clone(),
        vector_scope: VectorScope::Section,
        model_id: "fake-embedder".to_owned(),
        dim: 384,
        quantization: Quantization::Fp16,
        created_at: now,
    };

    assert_eq!(document.extension, DocumentExtension::Docx);
    assert_eq!(version.visibility, Visibility::Searchable);
    assert_eq!(candidate.version_count, 1);
    assert_eq!(section.section_type, SectionType::Experience);
    assert_eq!(mention.entity_type, EntityType::Skill);
    assert_eq!(vector.vector_scope, VectorScope::Section);
}

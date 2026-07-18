use std::str::FromStr;

use core_domain::{
    ActiveSearchProjection, ContentDigest, DocumentId, ResumeVersion, ResumeVersionId,
    SearchProjectionDigest, SearchProjectionDigestError, SearchSelection, SourceRevision,
    SourceRevisionId,
};

fn digest(hex: char) -> ContentDigest {
    ContentDigest::from_str(&format!("sha256:{}", hex.to_string().repeat(64))).unwrap()
}

#[test]
fn resume_version_is_bound_to_source_and_normalized_content() {
    let document_id = DocumentId::from_non_secret_parts(&["s807", "immutable-version"]);
    let source_revision_id = SourceRevisionId::from_content_identity(&document_id, &digest('1'));
    let normalized_text_hash = ContentDigest::from_bytes(b"synthetic normalized resume");
    let id = ResumeVersionId::from_content_identity(
        &document_id,
        &source_revision_id,
        &normalized_text_hash,
        "parser-v1",
        "schema-v27",
    );
    let version = ResumeVersion {
        id,
        document_id,
        source_revision_id,
        normalized_text_hash,
        parse_version: "parser-v1".to_string(),
        schema_version: "schema-v27".to_string(),
        language_set: vec!["en".to_string()],
        page_count: Some(1),
        raw_text: Some("synthetic raw resume".to_string()),
        clean_text: Some("synthetic normalized resume".to_string()),
        quality_score: Some(0.9),
    };

    let debug = format!("{version:?}");
    assert!(!debug.contains("synthetic raw resume"));
    assert!(!debug.contains("synthetic normalized resume"));
}

#[test]
fn source_and_resume_identity_change_with_content() {
    let document_id = DocumentId::from_non_secret_parts(&["s807", "document"]);
    let source_a = SourceRevisionId::from_content_identity(&document_id, &digest('a'));
    let source_b = SourceRevisionId::from_content_identity(&document_id, &digest('b'));
    assert_ne!(source_a, source_b);

    let version_a = ResumeVersionId::from_content_identity(
        &document_id,
        &source_a,
        &digest('c'),
        "parser-v1",
        "schema-v27",
    );
    let version_b = ResumeVersionId::from_content_identity(
        &document_id,
        &source_b,
        &digest('d'),
        "parser-v1",
        "schema-v27",
    );
    assert_ne!(version_a, version_b);
}

#[test]
fn content_digest_is_strict_and_redacted() {
    assert!(ContentDigest::from_str(&format!("sha256:{}", "a".repeat(64))).is_ok());
    assert!(ContentDigest::from_str(&format!("sha256:{}", "A".repeat(64))).is_err());
    assert!(ContentDigest::from_str(&format!("sha1:{}", "a".repeat(64))).is_err());

    let value = digest('e');
    assert_eq!(format!("{value:?}"), "ContentDigest(<redacted>)");
    assert_eq!(format!("{value}"), "<redacted-content-digest>");
}

#[test]
fn selection_and_projection_bind_the_exact_immutable_version() {
    let document_id = DocumentId::from_non_secret_parts(&["s807", "selection-document"]);
    let revision = SourceRevision::for_content(document_id.clone(), digest('f'), 128);
    let version_id = ResumeVersionId::from_content_identity(
        &document_id,
        &revision.id,
        &ContentDigest::from_bytes(b"synthetic normalized text"),
        "parser-v1",
        "schema-v27",
    );
    let selection = SearchSelection {
        document_id: document_id.clone(),
        resume_version_id: version_id.clone(),
        visible_epoch: 7,
    };
    let projection = ActiveSearchProjection {
        document_id,
        resume_version_id: version_id,
    };

    assert_eq!(selection.document_id, projection.document_id);
    assert_eq!(selection.resume_version_id, projection.resume_version_id);
    assert_eq!(selection.visible_epoch, 7);
}

#[test]
fn projection_digest_is_order_independent_and_version_exact() {
    let doc_a = DocumentId::from_non_secret_parts(&["s807", "projection-a"]);
    let doc_b = DocumentId::from_non_secret_parts(&["s807", "projection-b"]);
    let version_a = ResumeVersionId::from_non_secret_parts(&["s807", "version-a"]);
    let version_b = ResumeVersionId::from_non_secret_parts(&["s807", "version-b"]);
    let version_b_next = ResumeVersionId::from_non_secret_parts(&["s807", "version-b-next"]);

    let first = SearchProjectionDigest::from_pairs([
        (doc_a.as_str(), version_a.as_str()),
        (doc_b.as_str(), version_b.as_str()),
    ])
    .unwrap();
    let reordered = SearchProjectionDigest::from_pairs([
        (doc_b.as_str(), version_b.as_str()),
        (doc_a.as_str(), version_a.as_str()),
    ])
    .unwrap();
    let changed = SearchProjectionDigest::from_pairs([
        (doc_a.as_str(), version_a.as_str()),
        (doc_b.as_str(), version_b_next.as_str()),
    ])
    .unwrap();

    assert_eq!(first, reordered);
    assert_ne!(first, changed);
    assert_eq!(first.as_str().len(), "sha256:".len() + 64);
    assert_eq!(format!("{first:?}"), "SearchProjectionDigest(<redacted>)");
}

#[test]
fn projection_digest_rejects_ambiguous_or_invalid_identity() {
    let document = DocumentId::from_non_secret_parts(&["s807", "projection-duplicate"]);
    let other_document = DocumentId::from_non_secret_parts(&["s807", "projection-other"]);
    let version = ResumeVersionId::from_non_secret_parts(&["s807", "projection-version"]);
    let other_version =
        ResumeVersionId::from_non_secret_parts(&["s807", "projection-other-version"]);

    assert_eq!(
        SearchProjectionDigest::from_pairs([
            (document.as_str(), version.as_str()),
            (document.as_str(), other_version.as_str()),
        ]),
        Err(SearchProjectionDigestError::DuplicateDocument)
    );
    assert_eq!(
        SearchProjectionDigest::from_pairs([
            (document.as_str(), version.as_str()),
            (other_document.as_str(), version.as_str()),
        ]),
        Err(SearchProjectionDigestError::DuplicateResumeVersion)
    );
    assert_eq!(
        SearchProjectionDigest::from_pairs([("not-a-doc", version.as_str())]),
        Err(SearchProjectionDigestError::InvalidIdentity)
    );
}

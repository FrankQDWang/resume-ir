use crate::{
    ClassificationStatus, ContactHash, ContentDigest, DataDirectoryOwnerAcquisition,
    DataDirectoryOwnerLease, Document, DocumentId, DocumentStatus, EntityMention, EntityMentionId,
    EntityType, FileExtension, ImmutableIngestStage, MetaStoreErrorClass, OwnedMetaStore,
    ReasonCode, ResumeVersion, ResumeVersionClassification, ResumeVersionId, ReviewDisposition,
    SourceRevision, SourceRevisionTriage, UnixTimestamp, CLASSIFIER_EPOCH,
    MAX_ENTITY_MENTIONS_PER_VERSION,
};

fn owned_store() -> (tempfile::TempDir, OwnedMetaStore) {
    let directory = tempfile::tempdir().unwrap();
    let owner = match DataDirectoryOwnerLease::try_acquire(directory.path()).unwrap() {
        DataDirectoryOwnerAcquisition::Acquired(owner) => owner,
        DataDirectoryOwnerAcquisition::Contended => panic!("synthetic data directory contended"),
    };
    let store = owner.open_store().unwrap();
    (directory, store)
}

fn classified_fixture(
    label: &str,
) -> (
    Document,
    SourceRevision,
    ResumeVersion,
    ResumeVersionClassification,
) {
    let timestamp = UnixTimestamp::from_unix_seconds(1_900_200_000);
    let source = format!("synthetic source {label}");
    let mut document = Document {
        id: DocumentId::from_non_secret_parts(&["immutable-ingest-stage", label]),
        source_uri: format!("synthetic://immutable-ingest/{label}"),
        normalized_path: format!("synthetic/immutable-ingest/{label}.txt"),
        file_name: format!("{label}.txt"),
        extension: FileExtension::Txt,
        byte_size: source.len() as u64,
        mtime: timestamp,
        content_hash: None,
        text_hash: None,
        is_deleted: false,
        created_at: timestamp,
        updated_at: timestamp,
        status: DocumentStatus::FieldsExtracted,
    };
    let source_revision = SourceRevision::for_content(
        document.id.clone(),
        ContentDigest::from_bytes(source.as_bytes()),
        source.len() as u64,
    );
    document.content_hash = Some(source_revision.content_hash.as_str().to_string());
    let clean_text = format!("synthetic normalized resume {label}");
    let normalized_text_hash = ContentDigest::from_bytes(clean_text.as_bytes());
    let version = ResumeVersion {
        id: ResumeVersionId::from_content_identity(
            &document.id,
            &source_revision.id,
            &normalized_text_hash,
            "parser-v1",
            "schema-v28",
        ),
        document_id: document.id.clone(),
        source_revision_id: source_revision.id.clone(),
        normalized_text_hash,
        parse_version: "parser-v1".to_string(),
        schema_version: "schema-v28".to_string(),
        language_set: vec!["en".to_string()],
        page_count: Some(1),
        raw_text: None,
        clean_text: Some(clean_text),
        quality_score: Some(0.9),
    };
    let classification = ResumeVersionClassification {
        resume_version_id: version.id.clone(),
        status: ClassificationStatus::ResumeCandidate,
        classifier_epoch: CLASSIFIER_EPOCH.to_string(),
        reason_codes: vec![ReasonCode::CorroboratedResumeSignals],
        classified_at: UnixTimestamp::from_unix_seconds(1_900_200_001),
        review_disposition: ReviewDisposition::NotRequired,
    };
    (document, source_revision, version, classification)
}

fn mention(version: &ResumeVersion, label: &str, raw_value: String) -> EntityMention {
    EntityMention {
        id: EntityMentionId::from_non_secret_parts(&[
            "immutable-ingest-stage",
            version.id.as_str(),
            label,
        ]),
        resume_version_id: version.id.clone(),
        section_id: None,
        entity_type: EntityType::Skill,
        normalized_value: Some(raw_value.clone()),
        raw_value,
        span_start: None,
        span_end: None,
        confidence: 0.9,
        extractor: "synthetic-extractor".to_string(),
    }
}

#[test]
fn oversized_mention_rolls_back_every_classified_stage_row() {
    let (_directory, store) = owned_store();
    let (document, source_revision, version, classification) =
        classified_fixture("oversized-mention");
    let oversized = mention(&version, "oversized", "x".repeat(4_097));
    let email_hash = ContactHash::from_keyed_digest("a".repeat(64)).unwrap();
    let mut previous_document = document.clone();
    previous_document.content_hash = Some(
        ContentDigest::from_bytes(b"previous synthetic source")
            .as_str()
            .to_string(),
    );
    previous_document.status = DocumentStatus::Searchable;
    previous_document.updated_at = UnixTimestamp::from_unix_seconds(1_900_199_999);
    store.upsert_document(&previous_document).unwrap();

    assert_eq!(
        store
            .stage_immutable_ingest(ImmutableIngestStage::ClassifiedResume {
                document: &document,
                source_revision: &source_revision,
                version: &version,
                classification: &classification,
                mentions: std::slice::from_ref(&oversized),
                email_hash: Some(&email_hash),
                phone_hash: None,
            })
            .unwrap_err()
            .class(),
        MetaStoreErrorClass::InvalidValue
    );

    assert_eq!(
        store.document_by_id(&document.id).unwrap(),
        Some(previous_document)
    );
    assert_eq!(
        store.source_revision_by_id(&source_revision.id).unwrap(),
        None
    );
    assert_eq!(store.resume_version_by_id(&version.id).unwrap(), None);
    assert_eq!(
        store
            .resume_version_classification(&version.id, CLASSIFIER_EPOCH)
            .unwrap(),
        None
    );
    assert!(store
        .entity_mentions_for_version(&version.id)
        .unwrap()
        .is_empty());
    assert_eq!(
        store.candidate_assignment_for_version(&version.id).unwrap(),
        None
    );
    assert_eq!(store.candidate_by_contact_hash(&email_hash).unwrap(), None);
}

#[test]
fn invalid_source_triage_rolls_back_document_and_revision() {
    let (_directory, store) = owned_store();
    let (document, source_revision, _version, _classification) =
        classified_fixture("invalid-triage");
    let triage = SourceRevisionTriage {
        source_revision_id: source_revision.id.clone(),
        status: ClassificationStatus::ResumeCandidate,
        triage_epoch: CLASSIFIER_EPOCH.to_string(),
        reason_codes: vec![ReasonCode::CorroboratedResumeSignals],
        triaged_at: UnixTimestamp::from_unix_seconds(1_900_200_001),
    };

    assert_eq!(
        store
            .stage_immutable_ingest(ImmutableIngestStage::SourceTriage {
                document: &document,
                source_revision: &source_revision,
                triage: &triage,
            })
            .unwrap_err()
            .class(),
        MetaStoreErrorClass::InvalidValue
    );
    assert_eq!(store.document_by_id(&document.id).unwrap(), None);
    assert_eq!(
        store.source_revision_by_id(&source_revision.id).unwrap(),
        None
    );
}

#[test]
fn excessive_mention_count_rolls_back_every_classified_stage_row() {
    let (_directory, store) = owned_store();
    let (document, source_revision, version, classification) =
        classified_fixture("excessive-mention-count");
    let mentions = (0..=MAX_ENTITY_MENTIONS_PER_VERSION)
        .map(|ordinal| {
            mention(
                &version,
                &ordinal.to_string(),
                format!("synthetic-skill-{ordinal}"),
            )
        })
        .collect::<Vec<_>>();

    assert_eq!(
        store
            .stage_immutable_ingest(ImmutableIngestStage::ClassifiedResume {
                document: &document,
                source_revision: &source_revision,
                version: &version,
                classification: &classification,
                mentions: &mentions,
                email_hash: None,
                phone_hash: None,
            })
            .unwrap_err()
            .class(),
        MetaStoreErrorClass::InvalidValue
    );
    assert_eq!(store.document_by_id(&document.id).unwrap(), None);
    assert_eq!(
        store.source_revision_by_id(&source_revision.id).unwrap(),
        None
    );
    assert_eq!(store.resume_version_by_id(&version.id).unwrap(), None);
    assert_eq!(
        store
            .resume_version_classification(&version.id, CLASSIFIER_EPOCH)
            .unwrap(),
        None
    );
}

#[test]
fn same_decision_retries_are_idempotent_for_both_stage_variants() {
    let (_directory, store) = owned_store();
    let (document, source_revision, version, classification) =
        classified_fixture("idempotent-classified");
    let skill = mention(&version, "skill", "rust".to_string());
    let email_hash = ContactHash::from_keyed_digest("b".repeat(64)).unwrap();
    let stage = || ImmutableIngestStage::ClassifiedResume {
        document: &document,
        source_revision: &source_revision,
        version: &version,
        classification: &classification,
        mentions: std::slice::from_ref(&skill),
        email_hash: Some(&email_hash),
        phone_hash: None,
    };

    store.stage_immutable_ingest(stage()).unwrap();
    let first_candidate_id = store
        .candidate_assignment_for_version(&version.id)
        .unwrap()
        .unwrap();
    let mut retried_classification = classification.clone();
    retried_classification.classified_at = UnixTimestamp::from_unix_seconds(1_900_200_999);
    store
        .stage_immutable_ingest(ImmutableIngestStage::ClassifiedResume {
            document: &document,
            source_revision: &source_revision,
            version: &version,
            classification: &retried_classification,
            mentions: std::slice::from_ref(&skill),
            email_hash: Some(&email_hash),
            phone_hash: None,
        })
        .unwrap();
    assert_eq!(
        store.entity_mentions_for_version(&version.id).unwrap(),
        vec![skill]
    );
    let stored_classification = store
        .resume_version_classification(&version.id, CLASSIFIER_EPOCH)
        .unwrap()
        .unwrap();
    assert_eq!(
        stored_classification.classified_at,
        classification.classified_at
    );
    let retried_candidate_id = store
        .candidate_assignment_for_version(&version.id)
        .unwrap()
        .unwrap();
    assert_eq!(retried_candidate_id, first_candidate_id);
    assert_eq!(
        store
            .candidate_by_id(&retried_candidate_id)
            .unwrap()
            .unwrap()
            .email_hash,
        Some(email_hash)
    );

    let (triage_document, triage_revision, _version, _classification) =
        classified_fixture("idempotent-triage");
    let triage = SourceRevisionTriage {
        source_revision_id: triage_revision.id.clone(),
        status: ClassificationStatus::OcrBacklog,
        triage_epoch: CLASSIFIER_EPOCH.to_string(),
        reason_codes: vec![ReasonCode::OcrRequired],
        triaged_at: UnixTimestamp::from_unix_seconds(1_900_200_001),
    };
    store
        .stage_immutable_ingest(ImmutableIngestStage::SourceTriage {
            document: &triage_document,
            source_revision: &triage_revision,
            triage: &triage,
        })
        .unwrap();
    let mut retried_triage = triage.clone();
    retried_triage.triaged_at = UnixTimestamp::from_unix_seconds(1_900_200_999);
    store
        .stage_immutable_ingest(ImmutableIngestStage::SourceTriage {
            document: &triage_document,
            source_revision: &triage_revision,
            triage: &retried_triage,
        })
        .unwrap();
    let stored_triage = store
        .source_revision_triage(&triage_revision.id, CLASSIFIER_EPOCH)
        .unwrap()
        .unwrap();
    assert_eq!(stored_triage.triaged_at, triage.triaged_at);
}

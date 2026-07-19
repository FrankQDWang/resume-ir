use core_domain::{
    ActiveSearchProjection, ContentDigest, Document, DocumentId, DocumentStatus, FileExtension,
    ResumeVersion, ResumeVersionId, SearchProjectionDigest, SourceRevision, UnixTimestamp,
};
use meta_store::{
    ClassificationStatus, CurrentClassifierEpoch, FullTextSnapshotDescriptor, IngestJobStatus,
    OcrSearchPublicationCommit, ProjectedDocumentSnapshot, ReasonCode, ResumeVersionClassification,
    ReviewDisposition, SearchPublicationCommit, SearchPublicationDraft, SearchPublicationFailure,
    SearchPublicationOutcome, SearchPublicationState, SearchPublicationValidation,
    SourceRevisionTriage, TerminalDocumentUpdate, VectorSnapshotDescriptor, CLASSIFIER_EPOCH,
};

mod support;

fn now(seconds: i64) -> UnixTimestamp {
    UnixTimestamp::from_unix_seconds(seconds)
}

#[test]
fn failed_ocr_commit_rolls_back_inserted_facts_and_abandons_validated_publication() {
    let (_directory, store) = support::owned_store();
    support::ensure_ready_empty_search_owned(&store, now(1_910_000_000));
    let head_before = store.search_projection_state().unwrap();
    let base_generation = head_before.generation.clone().unwrap();

    let mut document = Document {
        id: DocumentId::from_non_secret_parts(&["ocr-atomic", "rollback"]),
        source_uri: "synthetic://ocr-atomic/rollback.pdf".to_string(),
        normalized_path: "synthetic/ocr-atomic/rollback.pdf".to_string(),
        file_name: "rollback.pdf".to_string(),
        extension: FileExtension::Pdf,
        byte_size: 128,
        mtime: now(1_910_000_001),
        content_hash: None,
        text_hash: None,
        is_deleted: false,
        created_at: now(1_910_000_001),
        updated_at: now(1_910_000_001),
        status: DocumentStatus::OcrRequired,
    };
    let source_revision = SourceRevision::for_content(
        document.id.clone(),
        ContentDigest::from_bytes(b"synthetic scanned source"),
        document.byte_size,
    );
    document.content_hash = Some(source_revision.content_hash.as_str().to_string());
    store.upsert_document(&document).unwrap();
    store.insert_source_revision(&source_revision).unwrap();
    store
        .insert_source_revision_triage(&SourceRevisionTriage {
            source_revision_id: source_revision.id.clone(),
            status: ClassificationStatus::OcrBacklog,
            triage_epoch: CLASSIFIER_EPOCH.to_string(),
            reason_codes: vec![ReasonCode::OcrRequired],
            triaged_at: now(1_910_000_002),
        })
        .unwrap();
    let classifier_epoch = CurrentClassifierEpoch::parse(CLASSIFIER_EPOCH).unwrap();
    store
        .enqueue_ocr_job_for_source_triage(
            &source_revision.id,
            classifier_epoch,
            now(1_910_000_003),
        )
        .unwrap();
    let claimed = store
        .claim_next_ocr_job(now(1_910_000_004))
        .unwrap()
        .unwrap();

    let clean_text = "Synthetic Candidate\nRust search systems";
    let normalized_text_hash = ContentDigest::from_bytes(clean_text.as_bytes());
    let version = ResumeVersion {
        id: ResumeVersionId::from_content_identity(
            &document.id,
            &source_revision.id,
            &normalized_text_hash,
            "ocr-parser-v1",
            "schema-v28",
        ),
        document_id: document.id.clone(),
        source_revision_id: source_revision.id.clone(),
        normalized_text_hash,
        parse_version: "ocr-parser-v1".to_string(),
        schema_version: "schema-v28".to_string(),
        language_set: vec!["en".to_string()],
        page_count: Some(1),
        raw_text: None,
        clean_text: Some(clean_text.to_string()),
        quality_score: Some(0.91),
    };
    let classification = ResumeVersionClassification {
        resume_version_id: version.id.clone(),
        status: ClassificationStatus::ResumeCandidate,
        classifier_epoch: CLASSIFIER_EPOCH.to_string(),
        reason_codes: vec![ReasonCode::CorroboratedResumeSignals],
        classified_at: now(1_910_000_005),
        review_disposition: ReviewDisposition::NotRequired,
    };
    let projection = ActiveSearchProjection {
        document_id: document.id.clone(),
        resume_version_id: version.id.clone(),
    };
    let projections = [projection.clone()];
    let projection_digest = SearchProjectionDigest::from_pairs([(
        projection.document_id.as_str(),
        projection.resume_version_id.as_str(),
    )])
    .unwrap();
    let empty_coverage = SearchProjectionDigest::from_pairs::<_, &str, &str>([]).unwrap();
    let generation = "ocr-atomic-rollback";
    let session = store.wait_for_search_publication_session().unwrap();
    assert_eq!(
        session
            .begin_search_publication(&SearchPublicationDraft {
                generation: generation.to_string(),
                base_generation: Some(base_generation),
                expected_visible_epoch: head_before.visible_epoch,
                classifier_epoch: CLASSIFIER_EPOCH.to_string(),
                projection_digest: projection_digest.clone(),
                now: now(1_910_000_006),
            })
            .unwrap(),
        SearchPublicationOutcome::Applied
    );
    let fulltext = FullTextSnapshotDescriptor::new(
        generation.to_string(),
        1,
        projection_digest.clone(),
        ContentDigest::from_bytes(b"ocr-atomic-fulltext"),
    );
    let vector = VectorSnapshotDescriptor::disabled(
        generation.to_string(),
        1,
        projection_digest,
        empty_coverage,
        ContentDigest::from_bytes(b"ocr-atomic-vector"),
    );
    session
        .validate_search_publication(&SearchPublicationValidation {
            generation,
            fulltext: &fulltext,
            vector: &vector,
            now: now(1_910_000_007),
        })
        .unwrap();
    let terminal = TerminalDocumentUpdate {
        document_id: document.id.clone(),
        expected_status: DocumentStatus::OcrRequired,
        expected_is_deleted: false,
        expected_content_hash: source_revision.content_hash.clone(),
        terminal_status: DocumentStatus::Searchable,
        terminal_is_deleted: false,
    };
    let terminals = [terminal];
    let commit_now = now(1_910_000_008);
    let mut projected_document = document.clone();
    projected_document.status = DocumentStatus::Searchable;
    projected_document.updated_at = commit_now;
    let projected_documents = [ProjectedDocumentSnapshot::Replacement {
        projection: projection.clone(),
        document: projected_document,
    }];

    let error = session
        .commit_ocr_search_publication(&OcrSearchPublicationCommit {
            search: SearchPublicationCommit {
                generation,
                terminal_documents: &terminals,
                projections: &projections,
                projected_documents: &projected_documents,
                // The validated descriptor disables vectors, so this exact
                // projection forces a failure after the immutable facts were
                // inserted into the transaction.
                vector_coverage: &projections,
                now: commit_now,
            },
            claimed: &claimed,
            source_revision: &source_revision,
            version: &version,
            classification: &classification,
            mentions: &[],
            email_hash: None,
            phone_hash: None,
        })
        .unwrap_err();

    assert_eq!(
        error.search_publication_failure(),
        Some(SearchPublicationFailure::VectorCoverageMismatch)
    );
    assert!(store.resume_version_by_id(&version.id).unwrap().is_none());
    assert!(store
        .resume_version_classification(&version.id, CLASSIFIER_EPOCH)
        .unwrap()
        .is_none());
    assert_eq!(
        store.document_by_id(&document.id).unwrap().unwrap().status,
        DocumentStatus::OcrRequired
    );
    assert_eq!(
        store
            .active_search_projection_for_document(&document.id)
            .unwrap(),
        None
    );
    let job = store.ingest_job_by_id(&claimed.job.id).unwrap().unwrap();
    assert_eq!(job.status, IngestJobStatus::Running);
    assert_eq!(job.resume_version_id, None);
    let head_after = store.search_projection_state().unwrap();
    assert_eq!(head_after.generation, head_before.generation);
    assert_eq!(head_after.visible_epoch, head_before.visible_epoch);
    assert_eq!(
        store.search_publication(generation).unwrap().unwrap().state,
        SearchPublicationState::Abandoned
    );
}

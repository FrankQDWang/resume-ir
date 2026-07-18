use meta_store::{
    ActiveSearchProjection, ClassificationStatus, ContentDigest, Document, DocumentId,
    DocumentStatus, FileExtension, FullTextSnapshotDescriptor, IdentityInsertOutcome, MetaStore,
    ReasonCode, ResumeVersion, ResumeVersionClassification, ResumeVersionId, ReviewDisposition,
    SearchProjectionDigest, SearchPublicationCommit, SearchPublicationDraft,
    SearchPublicationOutcome, SearchPublicationValidation, SourceRevision, TerminalDocumentUpdate,
    UnixTimestamp, VectorSnapshotDescriptor, CLASSIFIER_EPOCH,
};

fn timestamp(seconds: i64) -> UnixTimestamp {
    UnixTimestamp::from_unix_seconds(seconds)
}

fn document(status: DocumentStatus) -> Document {
    let now = timestamp(1_800_100_000);
    Document {
        id: DocumentId::from_non_secret_parts(&["excluded-document-status"]),
        source_uri: "synthetic://excluded/document.txt".to_string(),
        normalized_path: "synthetic/excluded/document.txt".to_string(),
        file_name: "document.txt".to_string(),
        extension: FileExtension::Txt,
        byte_size: 32,
        mtime: now,
        content_hash: None,
        text_hash: None,
        is_deleted: false,
        created_at: now,
        updated_at: now,
        status,
    }
}

#[test]
fn excluded_status_round_trips_in_v27_without_deletion() {
    let store = MetaStore::open_in_memory().unwrap();
    store.run_migrations().unwrap();
    let excluded = document(DocumentStatus::Excluded);

    store.upsert_document(&excluded).unwrap();

    let persisted = store.document_by_id(&excluded.id).unwrap().unwrap();
    assert_eq!(persisted.status, DocumentStatus::Excluded);
    assert!(!persisted.is_deleted);
    assert_eq!(store.schema_version().unwrap(), 27);
}

#[test]
fn publication_can_atomically_remove_an_active_version_as_excluded() {
    let store = MetaStore::open_in_memory().unwrap();
    store.run_migrations().unwrap();
    let mut staged = document(DocumentStatus::FieldsExtracted);
    let revision = SourceRevision::for_content(
        staged.id.clone(),
        ContentDigest::from_bytes(b"synthetic excluded source"),
        25,
    );
    staged.content_hash = Some(revision.content_hash.as_str().to_string());
    store.upsert_document(&staged).unwrap();
    assert_eq!(
        store.insert_source_revision(&revision).unwrap(),
        IdentityInsertOutcome::Inserted
    );
    let normalized_text_hash = ContentDigest::from_bytes(b"synthetic resume text");
    let version = ResumeVersion {
        id: ResumeVersionId::from_content_identity(
            &staged.id,
            &revision.id,
            &normalized_text_hash,
            "parser-v1",
            "schema-v27",
        ),
        document_id: staged.id.clone(),
        source_revision_id: revision.id.clone(),
        normalized_text_hash,
        parse_version: "parser-v1".to_string(),
        schema_version: "schema-v27".to_string(),
        language_set: vec!["en".to_string()],
        page_count: Some(1),
        raw_text: Some("synthetic resume text".to_string()),
        clean_text: Some("synthetic resume text".to_string()),
        quality_score: Some(0.9),
    };
    assert_eq!(
        store.insert_resume_version(&version).unwrap(),
        IdentityInsertOutcome::Inserted
    );
    assert_eq!(
        store
            .insert_resume_version_classification(&ResumeVersionClassification {
                resume_version_id: version.id.clone(),
                status: ClassificationStatus::ResumeCandidate,
                classifier_epoch: CLASSIFIER_EPOCH.to_string(),
                reason_codes: vec![ReasonCode::CorroboratedResumeSignals],
                classified_at: timestamp(1_800_100_001),
                review_disposition: ReviewDisposition::NotRequired,
            })
            .unwrap(),
        IdentityInsertOutcome::Inserted
    );

    let projection = ActiveSearchProjection {
        document_id: staged.id.clone(),
        resume_version_id: version.id.clone(),
    };
    publish(
        &store,
        "excluded-generation-1",
        None,
        0,
        std::slice::from_ref(&projection),
        &[TerminalDocumentUpdate {
            document_id: staged.id.clone(),
            expected_status: DocumentStatus::FieldsExtracted,
            expected_is_deleted: false,
            expected_content_hash: revision.content_hash.clone(),
            terminal_status: DocumentStatus::Searchable,
            terminal_is_deleted: false,
        }],
    );
    publish(
        &store,
        "excluded-generation-2",
        Some("excluded-generation-1"),
        1,
        &[],
        &[TerminalDocumentUpdate {
            document_id: staged.id.clone(),
            expected_status: DocumentStatus::Searchable,
            expected_is_deleted: false,
            expected_content_hash: revision.content_hash,
            terminal_status: DocumentStatus::Excluded,
            terminal_is_deleted: false,
        }],
    );

    let persisted = store.document_by_id(&staged.id).unwrap().unwrap();
    assert_eq!(persisted.status, DocumentStatus::Excluded);
    assert!(!persisted.is_deleted);
    assert_eq!(store.status_summary().unwrap().searchable_documents, 0);
}

fn publish(
    store: &MetaStore,
    generation: &str,
    base_generation: Option<&str>,
    expected_visible_epoch: u64,
    projections: &[ActiveSearchProjection],
    terminal_documents: &[TerminalDocumentUpdate],
) {
    let projection_digest =
        SearchProjectionDigest::from_pairs(projections.iter().map(|projection| {
            (
                projection.document_id.as_str(),
                projection.resume_version_id.as_str(),
            )
        }))
        .unwrap();
    let empty_coverage = SearchProjectionDigest::from_pairs::<_, &str, &str>([]).unwrap();
    assert_eq!(
        store
            .begin_search_publication(&SearchPublicationDraft {
                generation: generation.to_string(),
                base_generation: base_generation.map(str::to_string),
                expected_visible_epoch,
                classifier_epoch: CLASSIFIER_EPOCH.to_string(),
                projection_digest: projection_digest.clone(),
                now: timestamp(1_800_100_010 + expected_visible_epoch as i64),
            })
            .unwrap(),
        SearchPublicationOutcome::Applied
    );
    let fulltext = FullTextSnapshotDescriptor::new(
        generation.to_string(),
        projections.len() as u64,
        projection_digest.clone(),
        ContentDigest::from_bytes(format!("fulltext:{generation}").as_bytes()),
    );
    let vector = VectorSnapshotDescriptor::disabled(
        generation.to_string(),
        projections.len() as u64,
        projection_digest,
        empty_coverage,
        ContentDigest::from_bytes(format!("vector:{generation}").as_bytes()),
    );
    store
        .validate_search_publication(&SearchPublicationValidation {
            generation,
            fulltext: &fulltext,
            vector: &vector,
            now: timestamp(1_800_100_020 + expected_visible_epoch as i64),
        })
        .unwrap();
    assert_eq!(
        store
            .commit_search_publication(&SearchPublicationCommit {
                generation,
                terminal_documents,
                projections,
                vector_coverage: &[],
                now: timestamp(1_800_100_030 + expected_visible_epoch as i64),
            })
            .unwrap(),
        SearchPublicationOutcome::Applied
    );
}

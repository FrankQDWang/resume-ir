use std::{fs, num::NonZeroUsize, path::Path, sync::mpsc, thread, time::Duration};

use core_domain::{
    ActiveSearchProjection, Candidate, CandidateId, ContactHash, ContentDigest, Document,
    DocumentId, DocumentStatus, EntityMention, EntityMentionId, EntityType, FileExtension,
    ResumeVersion, ResumeVersionId, SearchProjectionDigest, SearchSelection, SourceRevision,
    UnixTimestamp,
};
use meta_store::migration_test_support::{begin_owned_store_write_race, OwnedStoreWriteRace};
use meta_store::{
    ArtifactRepairAttemptAcquire, ArtifactRepairKey, BoundedFilterSelection, ClassificationStatus,
    ClassifierEpochSource, CurrentClassifierEpoch, DataDirectoryOwnerAcquisition,
    DataDirectoryOwnerLease, EnabledVectorSnapshotDescriptor, ExactHitHydration,
    ExactHitHydrationFailureKind, FullTextSnapshotDescriptor, IdentityInsertOutcome,
    ImmutableIngestStage, MetaStoreErrorClass, MigrationRebuildPublicationAttemptAcquire,
    OcrAttemptFailure, OcrAttemptFailureOutcome, OcrJobDiscardReason, OwnedMetaStore,
    ProjectedDocumentSnapshot, ReasonCode, ResumeVersionClassification, ReviewDisposition,
    SearchArtifactExpectation, SearchFilterCase, SearchMetadataTransactionError,
    SearchMetadataUnavailable, SearchProjectionFilter, SearchProjectionPredicate,
    SearchProjectionServiceState, SearchPublicationCommit, SearchPublicationDraft,
    SearchPublicationFailure, SearchPublicationOutcome, SearchPublicationPrunePolicy,
    SearchPublicationRetirementFailureOutcome, SearchPublicationRetirementPlan,
    SearchPublicationSession, SearchPublicationState, SearchPublicationValidation,
    SearchSelectionDetailsResolution, SearchSelectionResolution, SearchTextBytePageRequest,
    SearchTextBytePageResolution, SearchTextPageCursor, SearchTextPageRequest,
    SearchTextPageResolution, SourceRevisionTriage, TerminalDocumentUpdate,
    VectorSnapshotDescriptor, MAX_BOUNDED_FILTER_SELECTION, MAX_SEARCH_TEXT_PAGE_CODE_POINTS,
};
use tempfile::TempDir;

mod support;

fn now(seconds: i64) -> UnixTimestamp {
    UnixTimestamp::from_unix_seconds(seconds)
}

fn document(label: &str) -> Document {
    let timestamp = now(1_800_000_000);
    Document {
        id: DocumentId::from_non_secret_parts(&["s807", label]),
        source_uri: format!("synthetic://s807/{label}"),
        normalized_path: format!("synthetic/s807/{label}.txt"),
        file_name: format!("{label}.txt"),
        extension: FileExtension::Txt,
        byte_size: 128,
        mtime: timestamp,
        content_hash: None,
        text_hash: None,
        is_deleted: false,
        created_at: timestamp,
        updated_at: timestamp,
        status: DocumentStatus::Discovered,
    }
}

fn revision(document: &Document, source: &[u8]) -> SourceRevision {
    SourceRevision::for_content(
        document.id.clone(),
        ContentDigest::from_bytes(source),
        source.len() as u64,
    )
}

fn version(document: &Document, revision: &SourceRevision, text: &str) -> ResumeVersion {
    let normalized_text_hash = ContentDigest::from_bytes(text.as_bytes());
    let id = ResumeVersionId::from_content_identity(
        &document.id,
        &revision.id,
        &normalized_text_hash,
        "parser-v1",
        "schema-v27",
    );
    ResumeVersion {
        id,
        document_id: document.id.clone(),
        source_revision_id: revision.id.clone(),
        normalized_text_hash,
        parse_version: "parser-v1".to_string(),
        schema_version: "schema-v27".to_string(),
        language_set: vec!["en".to_string()],
        page_count: Some(1),
        raw_text: Some(text.to_string()),
        clean_text: Some(text.to_string()),
        quality_score: Some(0.9),
    }
}

fn seed_version(
    store: &OwnedMetaStore,
    document: &Document,
    revision: &SourceRevision,
    version: &ResumeVersion,
) {
    let mut staged = document.clone();
    staged.content_hash = Some(revision.content_hash.as_str().to_string());
    staged.byte_size = revision.byte_size;
    staged.status = DocumentStatus::FieldsExtracted;
    store.upsert_document(&staged).unwrap();
    assert_eq!(
        store.insert_source_revision(revision).unwrap(),
        IdentityInsertOutcome::Inserted
    );
    assert_eq!(
        store.insert_resume_version(version).unwrap(),
        IdentityInsertOutcome::Inserted
    );
    insert_resume_candidate_classification(store, version);
}

fn insert_resume_candidate_classification(store: &OwnedMetaStore, version: &ResumeVersion) {
    assert!(matches!(
        store
            .insert_resume_version_classification(&ResumeVersionClassification {
                resume_version_id: version.id.clone(),
                status: ClassificationStatus::ResumeCandidate,
                classifier_epoch: resume_classifier::CLASSIFIER_EPOCH.to_string(),
                reason_codes: vec![ReasonCode::CorroboratedResumeSignals],
                classified_at: now(1_800_000_005),
                review_disposition: ReviewDisposition::NotRequired,
            })
            .unwrap(),
        IdentityInsertOutcome::Inserted | IdentityInsertOutcome::AlreadyPresent
    ));
}

fn publish(
    store: &OwnedMetaStore,
    generation: &str,
    expected_generation: Option<&str>,
    expected_epoch: u64,
    _documents: &[Document],
    projections: &[ActiveSearchProjection],
) -> SearchPublicationOutcome {
    let migration_barrier = expected_generation
        .is_none()
        .then(|| support::acquire_migration_rebuild_barrier_owned(store, now(1_799_999_999)));
    let projection_digest =
        SearchProjectionDigest::from_pairs(projections.iter().map(|projection| {
            (
                projection.document_id.as_str(),
                projection.resume_version_id.as_str(),
            )
        }))
        .unwrap();
    let empty_coverage = SearchProjectionDigest::from_pairs::<_, &str, &str>([]).unwrap();
    let draft = SearchPublicationDraft {
        generation: generation.to_string(),
        base_generation: expected_generation.map(str::to_string),
        expected_visible_epoch: expected_epoch,
        classifier_epoch: resume_classifier::CLASSIFIER_EPOCH.to_string(),
        projection_digest: projection_digest.clone(),
        now: now(1_800_000_010 + expected_epoch as i64),
    };
    let mut session = store.wait_for_search_publication_session().unwrap();
    if let Some(barrier) = migration_barrier.as_ref() {
        assert!(matches!(
            session
                .acquire_migration_rebuild_publication_attempt(
                    barrier,
                    now(1_800_000_000 + expected_epoch as i64),
                )
                .unwrap(),
            MigrationRebuildPublicationAttemptAcquire::Started(_)
                | MigrationRebuildPublicationAttemptAcquire::InProgress
        ));
    }
    assert_eq!(
        session.begin_search_publication(&draft).unwrap(),
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
    session
        .validate_search_publication(&SearchPublicationValidation {
            generation,
            fulltext: &fulltext,
            vector: &vector,
            now: now(1_800_000_020 + expected_epoch as i64),
        })
        .unwrap();
    let terminal_documents = projections
        .iter()
        .filter_map(|projection| {
            let document = store
                .document_by_id(&projection.document_id)
                .unwrap()
                .unwrap();
            (document.status != DocumentStatus::Searchable).then(|| {
                let version = store
                    .resume_version_by_id(&projection.resume_version_id)
                    .unwrap()
                    .unwrap();
                let revision = store
                    .source_revision_by_id(&version.source_revision_id)
                    .unwrap()
                    .unwrap();
                TerminalDocumentUpdate {
                    document_id: projection.document_id.clone(),
                    expected_status: document.status,
                    expected_is_deleted: document.is_deleted,
                    expected_content_hash: revision.content_hash,
                    terminal_status: DocumentStatus::Searchable,
                    terminal_is_deleted: false,
                }
            })
        })
        .collect::<Vec<_>>();
    let commit_now = now(1_800_000_040 + expected_epoch as i64);
    let projected_documents = support::projected_documents_for_commit(
        store,
        projections,
        &terminal_documents,
        commit_now,
    );
    let commit = SearchPublicationCommit {
        generation,
        terminal_documents: &terminal_documents,
        projections,
        projected_documents: &projected_documents,
        vector_coverage: &[],
        now: commit_now,
    };
    match migration_barrier.as_ref() {
        Some(barrier) => session
            .commit_migration_rebuild_search_publication(&commit, barrier)
            .unwrap(),
        None => session.commit_search_publication(&commit).unwrap(),
    }
}

fn prepare_publication(
    store: &OwnedMetaStore,
    generation: &str,
    expected_generation: Option<&str>,
    expected_epoch: u64,
    classifier_epoch: &str,
    projections: &[ActiveSearchProjection],
) -> SearchPublicationSession {
    let migration_barrier = expected_generation
        .is_none()
        .then(|| support::acquire_migration_rebuild_barrier_owned(store, now(1_800_009_999)));
    let artifact_key = if expected_generation.is_some()
        && store.search_projection_state().unwrap().service_state
            == SearchProjectionServiceState::Repairing
    {
        store.artifact_repair_context().unwrap().map(|context| {
            ArtifactRepairKey::new(
                context.generation,
                context.publication_fingerprint,
                context.visible_epoch,
            )
        })
    } else {
        None
    };
    let mut session = store.wait_for_search_publication_session().unwrap();
    if let Some(barrier) = migration_barrier.as_ref() {
        assert!(matches!(
            session
                .acquire_migration_rebuild_publication_attempt(barrier, now(1_800_009_999))
                .unwrap(),
            MigrationRebuildPublicationAttemptAcquire::Started(_)
                | MigrationRebuildPublicationAttemptAcquire::InProgress
        ));
    }
    if let Some(key) = artifact_key.as_ref() {
        assert!(matches!(
            session
                .acquire_artifact_repair_attempt(key, now(1_800_009_999))
                .unwrap(),
            ArtifactRepairAttemptAcquire::Started(_) | ArtifactRepairAttemptAcquire::InProgress
        ));
    }
    prepare_publication_in_session(
        &session,
        generation,
        expected_generation,
        expected_epoch,
        classifier_epoch,
        projections,
    );
    session
}

fn prepare_publication_in_session(
    session: &SearchPublicationSession,
    generation: &str,
    expected_generation: Option<&str>,
    expected_epoch: u64,
    classifier_epoch: &str,
    projections: &[ActiveSearchProjection],
) {
    let projection_digest =
        SearchProjectionDigest::from_pairs(projections.iter().map(|projection| {
            (
                projection.document_id.as_str(),
                projection.resume_version_id.as_str(),
            )
        }))
        .unwrap();
    let draft = SearchPublicationDraft {
        generation: generation.to_string(),
        base_generation: expected_generation.map(str::to_string),
        expected_visible_epoch: expected_epoch,
        classifier_epoch: classifier_epoch.to_string(),
        projection_digest: projection_digest.clone(),
        now: now(1_800_010_000 + expected_epoch as i64),
    };
    assert_eq!(
        session.begin_search_publication(&draft).unwrap(),
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
        SearchProjectionDigest::from_pairs::<_, &str, &str>([]).unwrap(),
        ContentDigest::from_bytes(format!("vector:{generation}").as_bytes()),
    );
    session
        .validate_search_publication(&SearchPublicationValidation {
            generation,
            fulltext: &fulltext,
            vector: &vector,
            now: now(1_800_010_100 + expected_epoch as i64),
        })
        .unwrap();
}

fn terminal_updates_for(
    store: &OwnedMetaStore,
    projections: &[ActiveSearchProjection],
) -> Vec<TerminalDocumentUpdate> {
    projections
        .iter()
        .filter_map(|projection| {
            let document = store
                .document_by_id(&projection.document_id)
                .unwrap()
                .unwrap();
            (document.status != DocumentStatus::Searchable).then(|| {
                let version = store
                    .resume_version_by_id(&projection.resume_version_id)
                    .unwrap()
                    .unwrap();
                let revision = store
                    .source_revision_by_id(&version.source_revision_id)
                    .unwrap()
                    .unwrap();
                TerminalDocumentUpdate {
                    document_id: projection.document_id.clone(),
                    expected_status: document.status,
                    expected_is_deleted: document.is_deleted,
                    expected_content_hash: revision.content_hash,
                    terminal_status: DocumentStatus::Searchable,
                    terminal_is_deleted: false,
                }
            })
        })
        .collect()
}

fn open_owned_store(data_dir: &Path) -> OwnedMetaStore {
    let owner = match DataDirectoryOwnerLease::try_acquire(data_dir).unwrap() {
        DataDirectoryOwnerAcquisition::Acquired(owner) => owner,
        DataDirectoryOwnerAcquisition::Contended => panic!("synthetic data directory contended"),
    };
    owner.open_store().unwrap()
}

fn owned_store() -> (TempDir, OwnedMetaStore) {
    let directory = tempfile::tempdir().unwrap();
    let data_dir = directory.path().join("data");
    fs::create_dir_all(&data_dir).unwrap();
    let store = open_owned_store(&data_dir);
    (directory, store)
}

fn resolve_in_snapshot(
    store: &OwnedMetaStore,
    selection: &SearchSelection,
) -> SearchSelectionResolution {
    store
        .with_search_metadata_snapshot(|snapshot| {
            Ok::<_, ()>(snapshot.resolve_search_selection(selection).unwrap())
        })
        .unwrap()
}

fn hydrated_document(store: &OwnedMetaStore, projection: &ActiveSearchProjection) -> Document {
    store
        .with_search_metadata_snapshot(|snapshot| {
            let hydrated = snapshot
                .hydrate_exact_hits(
                    std::slice::from_ref(projection),
                    NonZeroUsize::new(1).unwrap(),
                )
                .unwrap();
            let ExactHitHydration::Hydrated(mut hits) = hydrated else {
                panic!("exact active projection did not hydrate");
            };
            Ok::<_, ()>(hits.remove(0).document)
        })
        .unwrap()
}

#[test]
fn initial_empty_publication_becomes_ready_epoch_one() {
    let (_directory, store) = owned_store();
    let unavailable: SearchMetadataTransactionError<()> =
        store.with_search_metadata_snapshot(|_| Ok(())).unwrap_err();
    assert_eq!(
        unavailable.unavailable(),
        Some(SearchMetadataUnavailable::Repairing(
            meta_store::SearchRepairReason::MigrationRebuild
        ))
    );

    assert_eq!(
        publish(&store, "empty-generation", None, 0, &[], &[]),
        SearchPublicationOutcome::Applied
    );
    let state = store.search_projection_state().unwrap();
    assert_eq!(state.service_state, SearchProjectionServiceState::Ready);
    assert_eq!(state.visible_epoch, 1);
    let publication = state.publication.unwrap();
    assert_eq!(publication.state, meta_store::SearchPublicationState::Ready);
    assert!(publication.publication_fingerprint.is_some());
    assert_eq!(publication.fulltext.unwrap().document_count(), 0);
    let observed = store
        .with_search_metadata_snapshot(|snapshot| {
            Ok::<_, ()>((
                snapshot.head().visible_epoch,
                snapshot.validated_active_projections().unwrap(),
            ))
        })
        .unwrap();
    assert_eq!(observed, (1, Vec::new()));
}

#[test]
fn ready_head_journal_and_active_projection_are_database_guarded() {
    let (_directory, store) = owned_store();
    let make_document = document;
    let make_revision = revision;
    let make_version = version;
    let document = document("ready-head-guards");
    let revision = revision(&document, b"ready head guard source");
    let version = version(&document, &revision, "ready head guard text");
    seed_version(&store, &document, &revision, &version);
    let projection = ActiveSearchProjection {
        document_id: document.id.clone(),
        resume_version_id: version.id,
    };
    assert_eq!(
        publish(
            &store,
            "ready-head-guards-v1",
            None,
            0,
            std::slice::from_ref(&document),
            std::slice::from_ref(&projection),
        ),
        SearchPublicationOutcome::Applied
    );

    store
        .begin_artifact_repair("ready-head-guards-v1", 1, now(1_800_020_000))
        .unwrap();
    let unavailable: SearchMetadataTransactionError<()> =
        store.with_search_metadata_snapshot(|_| Ok(())).unwrap_err();
    assert_eq!(
        unavailable.unavailable(),
        Some(SearchMetadataUnavailable::Repairing(
            meta_store::SearchRepairReason::ArtifactUnavailable
        ))
    );
    let session = prepare_publication(
        &store,
        "ready-head-guards-v2",
        Some("ready-head-guards-v1"),
        1,
        resume_classifier::CLASSIFIER_EPOCH,
        std::slice::from_ref(&projection),
    );
    let unauthorized_document = make_document("ready-head-guards-unauthorized");
    let unauthorized_revision = make_revision(
        &unauthorized_document,
        b"ready head guard unauthorized source",
    );
    let unauthorized_version = make_version(
        &unauthorized_document,
        &unauthorized_revision,
        "ready head guard unauthorized text",
    );
    seed_version(
        &store,
        &unauthorized_document,
        &unauthorized_revision,
        &unauthorized_version,
    );
    assert_eq!(
        session
            .commit_search_publication(&SearchPublicationCommit {
                generation: "ready-head-guards-v2",
                terminal_documents: &[],
                projections: &[
                    projection.clone(),
                    ActiveSearchProjection {
                        document_id: unauthorized_document.id,
                        resume_version_id: unauthorized_version.id,
                    }
                ],
                projected_documents: &[],
                vector_coverage: &[],
                now: now(1_800_019_999),
            })
            .unwrap_err()
            .search_publication_failure(),
        Some(SearchPublicationFailure::ProjectionMismatch)
    );
    let terminal_updates = terminal_updates_for(&store, std::slice::from_ref(&projection));
    assert_eq!(
        session
            .commit_search_publication(&SearchPublicationCommit {
                generation: "ready-head-guards-v2",
                terminal_documents: &terminal_updates,
                projections: std::slice::from_ref(&projection),
                projected_documents: &support::projected_documents_for_commit(
                    &store,
                    std::slice::from_ref(&projection),
                    &terminal_updates,
                    now(1_800_020_001),
                ),
                vector_coverage: &[],
                now: now(1_800_020_001),
            })
            .unwrap(),
        SearchPublicationOutcome::Applied
    );
}

#[test]
fn metadata_snapshot_reports_repair_blocked_as_a_typed_error() {
    let (_directory, store) = owned_store();
    store
        .block_migration_rebuild(
            meta_store::SearchRepairReason::SourceUnavailable,
            now(1_800_019_999),
        )
        .unwrap();

    let error: SearchMetadataTransactionError<()> =
        store.with_search_metadata_snapshot(|_| Ok(())).unwrap_err();
    assert_eq!(
        error.unavailable(),
        Some(SearchMetadataUnavailable::RepairBlocked(
            meta_store::SearchRepairReason::SourceUnavailable
        ))
    );
}

#[test]
fn metadata_snapshot_releases_transaction_after_operation_error_and_panic() {
    let (_directory, store) = owned_store();
    publish(&store, "snapshot-raii", None, 0, &[], &[]);

    let operation_error = store
        .with_search_metadata_snapshot(|_| Err::<(), _>("synthetic-operation-error"))
        .unwrap_err();
    assert_eq!(
        operation_error.operation_error(),
        Some(&"synthetic-operation-error")
    );
    let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _: Result<(), SearchMetadataTransactionError<()>> = store
            .with_search_metadata_snapshot(|_| -> Result<(), ()> {
                panic!("synthetic snapshot panic")
            });
    }));
    assert!(panic.is_err());

    assert_eq!(
        store
            .with_search_metadata_snapshot(|snapshot| {
                Ok::<_, ()>(snapshot.head().generation.clone())
            })
            .unwrap(),
        "snapshot-raii"
    );
}

#[test]
fn detail_reads_are_selection_bound_and_sealed_mentions_are_immutable() {
    let (_directory, store) = owned_store();
    let staged_document = document("bounded-selection-staged");
    let staged_revision = revision(&staged_document, b"bounded staged source");
    let staged_version = version(
        &staged_document,
        &staged_revision,
        "bounded staged private body",
    );
    let document = document("bounded-selection-details");
    let revision = revision(&document, b"bounded selection source");
    let version = version(&document, &revision, "bounded selection private body");
    seed_version(&store, &document, &revision, &version);
    let candidate = Candidate {
        id: CandidateId::from_non_secret_parts(&["s807", "bounded-selection-candidate"]),
        primary_name: Some("Synthetic Candidate".to_string()),
        phone_hash: None,
        email_hash: None,
        dedupe_key: None,
        merge_confidence: Some(1.0),
        version_count: 0,
    };
    store.upsert_candidate(&candidate).unwrap();
    store
        .insert_candidate_assignment(&version.id, &candidate.id)
        .unwrap();
    let mention = EntityMention {
        id: EntityMentionId::from_non_secret_parts(&[version.id.as_str(), "bounded"]),
        resume_version_id: version.id.clone(),
        section_id: None,
        entity_type: EntityType::Skill,
        raw_value: "bounded skill".to_string(),
        normalized_value: Some("bounded skill".to_string()),
        span_start: None,
        span_end: None,
        confidence: 0.9,
        extractor: "synthetic".to_string(),
    };
    store
        .insert_entity_mentions(&version.id, std::slice::from_ref(&mention))
        .unwrap();
    let projection = ActiveSearchProjection {
        document_id: document.id.clone(),
        resume_version_id: version.id.clone(),
    };
    publish(
        &store,
        "bounded-selection-details-v1",
        None,
        0,
        std::slice::from_ref(&document),
        std::slice::from_ref(&projection),
    );
    let selection = SearchSelection {
        document_id: document.id.clone(),
        resume_version_id: version.id.clone(),
        visible_epoch: 1,
    };
    let details = store
        .with_search_metadata_snapshot(|snapshot| {
            Ok::<_, ()>(snapshot.selection_details(&selection).unwrap())
        })
        .unwrap();
    let SearchSelectionDetailsResolution::Current(details) = details else {
        panic!("current selection details expected");
    };
    assert_eq!(details.selection, selection);
    assert_eq!(details.version.source_revision_id, revision.id);
    assert_eq!(details.version.source_content_hash, revision.content_hash);
    assert_eq!(details.version.source_byte_size, revision.byte_size);
    assert_eq!(
        details.version.normalized_text_hash,
        version.normalized_text_hash
    );
    assert_eq!(details.version.parse_version, version.parse_version);
    assert_eq!(details.version.schema_version, version.schema_version);
    assert_eq!(details.candidate_id, Some(candidate.id.clone()));
    assert_eq!(details.mentions, vec![mention]);

    seed_version(&store, &staged_document, &staged_revision, &staged_version);
    store
        .insert_candidate_assignment(&staged_version.id, &candidate.id)
        .unwrap();
    let staged_selection = SearchSelection {
        document_id: staged_document.id,
        resume_version_id: staged_version.id,
        visible_epoch: 1,
    };
    assert_eq!(
        store
            .with_search_metadata_snapshot(|snapshot| {
                Ok::<_, ()>(snapshot.selection_details(&staged_selection).unwrap())
            })
            .unwrap(),
        SearchSelectionDetailsResolution::NotFound
    );

    let late_mention = EntityMention {
        id: EntityMentionId::from_non_secret_parts(&[version.id.as_str(), "late"]),
        resume_version_id: version.id.clone(),
        section_id: None,
        entity_type: EntityType::Skill,
        raw_value: "late mutation".to_string(),
        normalized_value: Some("late mutation".to_string()),
        span_start: None,
        span_end: None,
        confidence: 0.9,
        extractor: "synthetic".to_string(),
    };
    assert!(store
        .insert_entity_mentions(&version.id, &[late_mention])
        .is_err());
}

#[test]
fn clean_text_pages_use_opaque_unicode_code_point_cursors() {
    let (_directory, store) = owned_store();
    let document = document("unicode-page");
    let revision = revision(&document, b"unicode page source");
    let version = version(&document, &revision, "甲🙂éZ");
    seed_version(&store, &document, &revision, &version);
    let projection = ActiveSearchProjection {
        document_id: document.id.clone(),
        resume_version_id: version.id.clone(),
    };
    publish(
        &store,
        "unicode-page-v1",
        None,
        0,
        std::slice::from_ref(&document),
        std::slice::from_ref(&projection),
    );
    let selection = SearchSelection {
        document_id: document.id,
        resume_version_id: version.id,
        visible_epoch: 1,
    };
    let first_request = SearchTextPageRequest::new(selection.clone(), None, 2).unwrap();
    let first = store
        .with_search_metadata_snapshot(|snapshot| {
            Ok::<_, ()>(snapshot.clean_text_page(&first_request).unwrap())
        })
        .unwrap();
    let SearchTextPageResolution::Current(first) = first else {
        panic!("first text page expected");
    };
    assert_eq!(first.text, "甲🙂");
    assert_eq!(first.total_code_points, 4);
    let cursor_token = first.next_cursor.unwrap().to_opaque_token();
    let cursor = SearchTextPageCursor::from_opaque_token(&cursor_token).unwrap();
    let second_request = SearchTextPageRequest::new(selection.clone(), Some(cursor), 2).unwrap();
    let second = store
        .with_search_metadata_snapshot(|snapshot| {
            Ok::<_, ()>(snapshot.clean_text_page(&second_request).unwrap())
        })
        .unwrap();
    let SearchTextPageResolution::Current(second) = second else {
        panic!("second text page expected");
    };
    assert_eq!(second.text, "éZ");
    assert_eq!(second.total_code_points, 4);
    assert_eq!(second.next_cursor, None);

    let beyond = SearchTextPageCursor::from_opaque_token("cp1:0000000000000005").unwrap();
    let beyond_request = SearchTextPageRequest::new(selection.clone(), Some(beyond), 2).unwrap();
    assert_eq!(
        store
            .with_search_metadata_snapshot(|snapshot| {
                Ok::<_, ()>(snapshot.clean_text_page(&beyond_request).unwrap())
            })
            .unwrap(),
        SearchTextPageResolution::InvalidOffset
    );
    assert!(SearchTextPageRequest::new(
        first.selection,
        None,
        MAX_SEARCH_TEXT_PAGE_CODE_POINTS + 1,
    )
    .is_err());

    let byte_first_request = SearchTextBytePageRequest::new(selection.clone(), 0, 4).unwrap();
    let byte_first = store
        .with_search_metadata_snapshot(|snapshot| {
            Ok::<_, ()>(snapshot.clean_text_byte_page(&byte_first_request).unwrap())
        })
        .unwrap();
    let SearchTextBytePageResolution::Current(byte_first) = byte_first else {
        panic!("first byte page expected");
    };
    assert_eq!(byte_first.text, "甲");
    assert_eq!(byte_first.next_offset_bytes, 3);
    assert_eq!(byte_first.total_bytes, "甲🙂éZ".len() as u64);

    let byte_second_request =
        SearchTextBytePageRequest::new(selection.clone(), byte_first.next_offset_bytes, 4).unwrap();
    let byte_second = store
        .with_search_metadata_snapshot(|snapshot| {
            Ok::<_, ()>(snapshot.clean_text_byte_page(&byte_second_request).unwrap())
        })
        .unwrap();
    let SearchTextBytePageResolution::Current(byte_second) = byte_second else {
        panic!("second byte page expected");
    };
    assert_eq!(byte_second.text, "🙂");
    assert_eq!(byte_second.next_offset_bytes, 7);

    let invalid_boundary = SearchTextBytePageRequest::new(selection, 1, 4).unwrap();
    assert_eq!(
        store
            .with_search_metadata_snapshot(|snapshot| {
                Ok::<_, ()>(snapshot.clean_text_byte_page(&invalid_boundary).unwrap())
            })
            .unwrap(),
        SearchTextBytePageResolution::InvalidOffset
    );
    let beyond_end =
        SearchTextBytePageRequest::new(byte_first.selection, byte_first.total_bytes + 1, 4)
            .unwrap();
    assert_eq!(
        store
            .with_search_metadata_snapshot(|snapshot| {
                Ok::<_, ()>(snapshot.clean_text_byte_page(&beyond_end).unwrap())
            })
            .unwrap(),
        SearchTextBytePageResolution::InvalidOffset
    );
}

#[test]
fn request_snapshot_opens_without_a_projection_scan_and_explicit_audit_accepts_consistent_state() {
    let (_directory, store) = owned_store();
    let document = document("snapshot-o1-open");
    let revision = revision(&document, b"snapshot o1 source");
    let version = version(&document, &revision, "snapshot o1 text");
    seed_version(&store, &document, &revision, &version);
    let projection = ActiveSearchProjection {
        document_id: document.id,
        resume_version_id: version.id,
    };
    publish(
        &store,
        "snapshot-o1-open-v1",
        None,
        0,
        &[],
        std::slice::from_ref(&projection),
    );
    store.validate_search_projection_integrity().unwrap();

    assert_eq!(
        store
            .with_search_metadata_snapshot(|snapshot| Ok::<_, ()>(snapshot.head().visible_epoch))
            .unwrap(),
        1
    );
    store.validate_search_projection_integrity().unwrap();
}

#[test]
fn descriptor_mismatch_never_validates_publication() {
    let (_directory, store) = owned_store();
    let barrier = support::acquire_migration_rebuild_barrier_owned(&store, now(1_800_019_999));
    let mut session = store.wait_for_search_publication_session().unwrap();
    assert!(matches!(
        session
            .acquire_migration_rebuild_publication_attempt(&barrier, now(1_800_019_999))
            .unwrap(),
        MigrationRebuildPublicationAttemptAcquire::Started(_)
    ));
    let projection_digest = SearchProjectionDigest::from_pairs::<_, &str, &str>([]).unwrap();
    let wrong_projection_digest = SearchProjectionDigest::from_pairs([(
        DocumentId::from_non_secret_parts(&["s807", "descriptor-doc"]),
        ResumeVersionId::from_non_secret_parts(&["s807", "descriptor-version"]),
    )])
    .unwrap();
    let draft = SearchPublicationDraft {
        generation: "descriptor-mismatch".to_string(),
        base_generation: None,
        expected_visible_epoch: 0,
        classifier_epoch: resume_classifier::CLASSIFIER_EPOCH.to_string(),
        projection_digest: projection_digest.clone(),
        now: now(1_800_020_000),
    };
    session.begin_search_publication(&draft).unwrap();
    let fulltext = FullTextSnapshotDescriptor::new(
        draft.generation.clone(),
        0,
        projection_digest.clone(),
        ContentDigest::from_bytes(b"fulltext-logical"),
    );
    let vector = VectorSnapshotDescriptor::disabled(
        draft.generation.clone(),
        0,
        wrong_projection_digest,
        projection_digest,
        ContentDigest::from_bytes(b"vector-logical"),
    );
    let error = session
        .validate_search_publication(&SearchPublicationValidation {
            generation: &draft.generation,
            fulltext: &fulltext,
            vector: &vector,
            now: now(1_800_020_001),
        })
        .unwrap_err();
    assert_eq!(
        error.search_publication_failure(),
        Some(SearchPublicationFailure::DescriptorMismatch)
    );
    assert_eq!(
        store
            .search_publication(&draft.generation)
            .unwrap()
            .unwrap()
            .state,
        meta_store::SearchPublicationState::Preparing
    );
}

#[test]
fn enabled_vector_dimension_matches_the_artifact_contract_boundary() {
    let (_directory, store) = owned_store();
    let barrier = support::acquire_migration_rebuild_barrier_owned(&store, now(1_800_020_009));
    let mut session = store.wait_for_search_publication_session().unwrap();
    assert!(matches!(
        session
            .acquire_migration_rebuild_publication_attempt(&barrier, now(1_800_020_009))
            .unwrap(),
        MigrationRebuildPublicationAttemptAcquire::Started(_)
    ));
    let projection_digest = SearchProjectionDigest::from_pairs::<_, &str, &str>([]).unwrap();
    for (generation, dimension, expected_failure) in [
        ("vector-dimension-max", 65_536, None),
        (
            "vector-dimension-too-large",
            65_537,
            Some(SearchPublicationFailure::InvalidDescriptor),
        ),
    ] {
        session
            .begin_search_publication(&SearchPublicationDraft {
                generation: generation.to_string(),
                base_generation: None,
                expected_visible_epoch: 0,
                classifier_epoch: resume_classifier::CLASSIFIER_EPOCH.to_string(),
                projection_digest: projection_digest.clone(),
                now: now(1_800_020_010),
            })
            .unwrap();
        let fulltext = FullTextSnapshotDescriptor::new(
            generation.to_string(),
            0,
            projection_digest.clone(),
            ContentDigest::from_bytes(format!("fulltext:{generation}").as_bytes()),
        );
        let vector = VectorSnapshotDescriptor::enabled(EnabledVectorSnapshotDescriptor {
            generation: generation.to_string(),
            model_id: "synthetic-model".to_string(),
            dimension,
            projection_count: 0,
            projection_digest: projection_digest.clone(),
            coverage_digest: projection_digest.clone(),
            vector_count: 0,
            document_count: 0,
            resume_version_count: 0,
            logical_content_digest: ContentDigest::from_bytes(
                format!("vector:{generation}").as_bytes(),
            ),
        });
        let result = session.validate_search_publication(&SearchPublicationValidation {
            generation,
            fulltext: &fulltext,
            vector: &vector,
            now: now(1_800_020_011),
        });
        assert_eq!(
            result
                .err()
                .and_then(|error| error.search_publication_failure()),
            expected_failure
        );
        session
            .abandon_search_publication(generation, now(1_800_020_012))
            .unwrap();
    }
}

#[test]
fn enabled_vector_coverage_is_an_exact_projection_subset() {
    let (_directory, store) = owned_store();
    let migration_barrier =
        support::acquire_migration_rebuild_barrier_owned(&store, now(1_799_999_999));
    let mut session = store.wait_for_search_publication_session().unwrap();
    assert!(matches!(
        session
            .acquire_migration_rebuild_publication_attempt(&migration_barrier, now(1_799_999_999),)
            .unwrap(),
        MigrationRebuildPublicationAttemptAcquire::Started(_)
    ));
    let document_a = document("vector-coverage-a");
    let revision_a = revision(&document_a, b"vector coverage source A");
    let version_a = version(&document_a, &revision_a, "vector coverage normalized A");
    seed_version(&store, &document_a, &revision_a, &version_a);
    let document_b = document("vector-coverage-b");
    let revision_b = revision(&document_b, b"vector coverage source B");
    let version_b = version(&document_b, &revision_b, "vector coverage normalized B");
    seed_version(&store, &document_b, &revision_b, &version_b);
    let projection_a = ActiveSearchProjection {
        document_id: document_a.id,
        resume_version_id: version_a.id,
    };
    let projection_b = ActiveSearchProjection {
        document_id: document_b.id,
        resume_version_id: version_b.id,
    };
    let projections = [projection_a.clone(), projection_b.clone()];
    let projection_digest = SearchProjectionDigest::from_pairs(
        projections
            .iter()
            .map(|item| (item.document_id.as_str(), item.resume_version_id.as_str())),
    )
    .unwrap();
    let coverage_digest = SearchProjectionDigest::from_pairs([(
        projection_a.document_id.as_str(),
        projection_a.resume_version_id.as_str(),
    )])
    .unwrap();
    let draft = SearchPublicationDraft {
        generation: "vector-coverage".to_string(),
        base_generation: None,
        expected_visible_epoch: 0,
        classifier_epoch: resume_classifier::CLASSIFIER_EPOCH.to_string(),
        projection_digest: projection_digest.clone(),
        now: now(1_800_020_005),
    };
    assert_eq!(
        session.begin_search_publication(&draft).unwrap(),
        SearchPublicationOutcome::Applied
    );
    let fulltext = FullTextSnapshotDescriptor::new(
        draft.generation.clone(),
        2,
        projection_digest.clone(),
        ContentDigest::from_bytes(b"vector-coverage-fulltext"),
    );
    let vector = VectorSnapshotDescriptor::enabled(EnabledVectorSnapshotDescriptor {
        generation: draft.generation.clone(),
        model_id: "synthetic-model-v1".to_string(),
        dimension: 3,
        projection_count: 2,
        projection_digest,
        coverage_digest,
        vector_count: 2,
        document_count: 1,
        resume_version_count: 1,
        logical_content_digest: ContentDigest::from_bytes(b"vector-coverage-vector"),
    });
    session
        .validate_search_publication(&SearchPublicationValidation {
            generation: &draft.generation,
            fulltext: &fulltext,
            vector: &vector,
            now: now(1_800_020_006),
        })
        .unwrap();
    let invalid_coverage = ActiveSearchProjection {
        document_id: projection_a.document_id.clone(),
        resume_version_id: projection_b.resume_version_id.clone(),
    };
    let terminal_documents = terminal_updates_for(&store, &projections);
    assert_eq!(
        session
            .commit_migration_rebuild_search_publication(
                &SearchPublicationCommit {
                    generation: &draft.generation,
                    terminal_documents: &terminal_documents,
                    projections: &projections,
                    projected_documents: &support::projected_documents_for_commit(
                        &store,
                        &projections,
                        &terminal_documents,
                        now(1_800_020_007),
                    ),
                    vector_coverage: &[invalid_coverage],
                    now: now(1_800_020_007),
                },
                &migration_barrier,
            )
            .unwrap_err()
            .search_publication_failure(),
        Some(SearchPublicationFailure::VectorCoverageMismatch)
    );
    assert_eq!(
        session
            .commit_migration_rebuild_search_publication(
                &SearchPublicationCommit {
                    generation: &draft.generation,
                    terminal_documents: &terminal_documents,
                    projections: &projections,
                    projected_documents: &support::projected_documents_for_commit(
                        &store,
                        &projections,
                        &terminal_documents,
                        now(1_800_020_008),
                    ),
                    vector_coverage: std::slice::from_ref(&projection_a),
                    now: now(1_800_020_008),
                },
                &migration_barrier,
            )
            .unwrap(),
        SearchPublicationOutcome::Applied
    );
    let persisted = store
        .search_publication(&draft.generation)
        .unwrap()
        .unwrap();
    let vector = persisted.vector.unwrap();
    assert_eq!(vector.projection_count(), 2);
    assert_eq!(vector.document_count(), 1);
    assert_eq!(vector.resume_version_count(), 1);
    assert_eq!(vector.vector_count(), 2);
}

#[test]
fn terminal_document_state_must_match_projection_membership() {
    let (_directory, store) = owned_store();
    let session = store.wait_for_search_publication_session().unwrap();
    let projected_document = document("terminal-projected");
    let projected_version =
        ResumeVersionId::from_non_secret_parts(&["s807", "terminal-projected-version"]);
    let projection = ActiveSearchProjection {
        document_id: projected_document.id.clone(),
        resume_version_id: projected_version,
    };
    let projected_but_not_searchable = TerminalDocumentUpdate {
        document_id: projected_document.id.clone(),
        expected_status: DocumentStatus::FieldsExtracted,
        expected_is_deleted: false,
        expected_content_hash: ContentDigest::from_bytes(b"projected"),
        terminal_status: DocumentStatus::FailedPermanent,
        terminal_is_deleted: false,
    };
    let error = session
        .commit_search_publication(&SearchPublicationCommit {
            generation: "terminal-shape-projected",
            terminal_documents: &[projected_but_not_searchable],
            projections: std::slice::from_ref(&projection),
            projected_documents: &[ProjectedDocumentSnapshot::RetainedUnchanged {
                projection: projection.clone(),
            }],
            vector_coverage: &[],
            now: now(1_800_020_002),
        })
        .unwrap_err();
    assert_eq!(
        error.search_publication_failure(),
        Some(SearchPublicationFailure::InvalidDocumentState)
    );

    let unprojected_searchable = TerminalDocumentUpdate {
        document_id: DocumentId::from_non_secret_parts(&["s807", "terminal-unprojected"]),
        expected_status: DocumentStatus::FieldsExtracted,
        expected_is_deleted: false,
        expected_content_hash: ContentDigest::from_bytes(b"unprojected"),
        terminal_status: DocumentStatus::Searchable,
        terminal_is_deleted: false,
    };
    let error = session
        .commit_search_publication(&SearchPublicationCommit {
            generation: "terminal-shape-unprojected",
            terminal_documents: &[unprojected_searchable],
            projections: std::slice::from_ref(&projection),
            projected_documents: &[ProjectedDocumentSnapshot::RetainedUnchanged {
                projection: projection.clone(),
            }],
            vector_coverage: &[],
            now: now(1_800_020_003),
        })
        .unwrap_err();
    assert_eq!(
        error.search_publication_failure(),
        Some(SearchPublicationFailure::InvalidDocumentState)
    );

    for unstable_status in [
        DocumentStatus::ParseRunning,
        DocumentStatus::OcrRunning,
        DocumentStatus::FailedRetryable,
    ] {
        let unstable_removal = TerminalDocumentUpdate {
            document_id: DocumentId::from_non_secret_parts(&["s807", "terminal-unstable"]),
            expected_status: DocumentStatus::Searchable,
            expected_is_deleted: false,
            expected_content_hash: ContentDigest::from_bytes(b"unstable"),
            terminal_status: unstable_status,
            terminal_is_deleted: false,
        };
        assert_eq!(
            session
                .commit_search_publication(&SearchPublicationCommit {
                    generation: "terminal-shape-unstable",
                    terminal_documents: &[unstable_removal],
                    projections: &[],
                    projected_documents: &[],
                    vector_coverage: &[],
                    now: now(1_800_020_004),
                })
                .unwrap_err()
                .search_publication_failure(),
            Some(SearchPublicationFailure::InvalidDocumentState)
        );
    }
}

#[test]
fn projection_transition_requires_exact_terminal_updates() {
    let (_directory, store) = owned_store();
    let migration_barrier =
        support::acquire_migration_rebuild_barrier_owned(&store, now(1_799_999_999));
    let document_a = document("projection-transition-a");
    let revision_a1 = revision(&document_a, b"projection transition A1");
    let version_a1 = version(&document_a, &revision_a1, "projection transition A1");
    seed_version(&store, &document_a, &revision_a1, &version_a1);
    let projection_a1 = ActiveSearchProjection {
        document_id: document_a.id.clone(),
        resume_version_id: version_a1.id,
    };

    let session = prepare_publication(
        &store,
        "transition-generation-1",
        None,
        0,
        resume_classifier::CLASSIFIER_EPOCH,
        std::slice::from_ref(&projection_a1),
    );
    assert_eq!(
        session
            .commit_migration_rebuild_search_publication(
                &SearchPublicationCommit {
                    generation: "transition-generation-1",
                    terminal_documents: &[],
                    projections: std::slice::from_ref(&projection_a1),
                    projected_documents: &support::projected_documents_for_commit(
                        &store,
                        std::slice::from_ref(&projection_a1),
                        &[],
                        now(1_800_020_101),
                    ),
                    vector_coverage: &[],
                    now: now(1_800_020_101),
                },
                &migration_barrier,
            )
            .unwrap_err()
            .search_publication_failure(),
        Some(SearchPublicationFailure::InvalidProjectionTransition)
    );
    let activation = terminal_updates_for(&store, std::slice::from_ref(&projection_a1));
    assert_eq!(
        session
            .commit_migration_rebuild_search_publication(
                &SearchPublicationCommit {
                    generation: "transition-generation-1",
                    terminal_documents: &activation,
                    projections: std::slice::from_ref(&projection_a1),
                    projected_documents: &support::projected_documents_for_commit(
                        &store,
                        std::slice::from_ref(&projection_a1),
                        &activation,
                        now(1_800_020_102),
                    ),
                    vector_coverage: &[],
                    now: now(1_800_020_102),
                },
                &migration_barrier,
            )
            .unwrap(),
        SearchPublicationOutcome::Applied
    );
    drop(session);

    let revision_a2 = revision(&document_a, b"projection transition A2");
    let version_a2 = version(&document_a, &revision_a2, "projection transition A2");
    let mut staged_a2 = document_a.clone();
    staged_a2.content_hash = Some(revision_a2.content_hash.as_str().to_string());
    staged_a2.byte_size = revision_a2.byte_size;
    staged_a2.status = DocumentStatus::FieldsExtracted;
    store.upsert_document(&staged_a2).unwrap();
    store.insert_source_revision(&revision_a2).unwrap();
    store.insert_resume_version(&version_a2).unwrap();
    insert_resume_candidate_classification(&store, &version_a2);
    let projection_a2 = ActiveSearchProjection {
        document_id: document_a.id.clone(),
        resume_version_id: version_a2.id,
    };
    let session = prepare_publication(
        &store,
        "transition-generation-2",
        Some("transition-generation-1"),
        1,
        resume_classifier::CLASSIFIER_EPOCH,
        std::slice::from_ref(&projection_a2),
    );
    assert_eq!(
        session
            .commit_search_publication(&SearchPublicationCommit {
                generation: "transition-generation-2",
                terminal_documents: &[],
                projections: std::slice::from_ref(&projection_a2),
                projected_documents: &support::projected_documents_for_commit(
                    &store,
                    std::slice::from_ref(&projection_a2),
                    &[],
                    now(1_800_020_103),
                ),
                vector_coverage: &[],
                now: now(1_800_020_103),
            })
            .unwrap_err()
            .search_publication_failure(),
        Some(SearchPublicationFailure::InvalidProjectionTransition)
    );
    let replacement = terminal_updates_for(&store, std::slice::from_ref(&projection_a2));
    assert_eq!(
        session
            .commit_search_publication(&SearchPublicationCommit {
                generation: "transition-generation-2",
                terminal_documents: &replacement,
                projections: std::slice::from_ref(&projection_a2),
                projected_documents: &support::projected_documents_for_commit(
                    &store,
                    std::slice::from_ref(&projection_a2),
                    &replacement,
                    now(1_800_020_104),
                ),
                vector_coverage: &[],
                now: now(1_800_020_104),
            })
            .unwrap(),
        SearchPublicationOutcome::Applied
    );
    drop(session);

    let session = prepare_publication(
        &store,
        "transition-generation-3",
        Some("transition-generation-2"),
        2,
        resume_classifier::CLASSIFIER_EPOCH,
        std::slice::from_ref(&projection_a2),
    );
    let retained_terminal = TerminalDocumentUpdate {
        document_id: document_a.id.clone(),
        expected_status: DocumentStatus::Searchable,
        expected_is_deleted: false,
        expected_content_hash: revision_a2.content_hash.clone(),
        terminal_status: DocumentStatus::Searchable,
        terminal_is_deleted: false,
    };
    assert_eq!(
        session
            .commit_search_publication(&SearchPublicationCommit {
                generation: "transition-generation-3",
                terminal_documents: std::slice::from_ref(&retained_terminal),
                projections: std::slice::from_ref(&projection_a2),
                projected_documents: &support::projected_documents_for_commit(
                    &store,
                    std::slice::from_ref(&projection_a2),
                    std::slice::from_ref(&retained_terminal),
                    now(1_800_020_105),
                ),
                vector_coverage: &[],
                now: now(1_800_020_105),
            })
            .unwrap_err()
            .search_publication_failure(),
        Some(SearchPublicationFailure::InvalidProjectionTransition)
    );
    assert_eq!(
        session
            .commit_search_publication(&SearchPublicationCommit {
                generation: "transition-generation-3",
                terminal_documents: &[],
                projections: std::slice::from_ref(&projection_a2),
                projected_documents: &support::projected_documents_for_commit(
                    &store,
                    std::slice::from_ref(&projection_a2),
                    &[],
                    now(1_800_020_106),
                ),
                vector_coverage: &[],
                now: now(1_800_020_106),
            })
            .unwrap(),
        SearchPublicationOutcome::Applied
    );
    drop(session);

    let session = prepare_publication(
        &store,
        "transition-generation-4",
        Some("transition-generation-3"),
        3,
        resume_classifier::CLASSIFIER_EPOCH,
        &[],
    );
    assert_eq!(
        session
            .commit_search_publication(&SearchPublicationCommit {
                generation: "transition-generation-4",
                terminal_documents: &[],
                projections: &[],
                projected_documents: &[],
                vector_coverage: &[],
                now: now(1_800_020_107),
            })
            .unwrap_err()
            .search_publication_failure(),
        Some(SearchPublicationFailure::InvalidProjectionTransition)
    );
    assert_eq!(
        store.searchable_document_ids().unwrap(),
        vec![document_a.id.clone()]
    );
    let removal = TerminalDocumentUpdate {
        document_id: document_a.id.clone(),
        expected_status: DocumentStatus::Searchable,
        expected_is_deleted: false,
        expected_content_hash: revision_a2.content_hash,
        terminal_status: DocumentStatus::FailedPermanent,
        terminal_is_deleted: false,
    };
    assert_eq!(
        session
            .commit_search_publication(&SearchPublicationCommit {
                generation: "transition-generation-4",
                terminal_documents: &[removal],
                projections: &[],
                projected_documents: &[],
                vector_coverage: &[],
                now: now(1_800_020_108),
            })
            .unwrap(),
        SearchPublicationOutcome::Applied
    );
    drop(session);

    let document_b = document("projection-transition-b");
    let revision_b = revision(&document_b, b"projection transition B");
    let version_b = version(&document_b, &revision_b, "projection transition B");
    seed_version(&store, &document_b, &revision_b, &version_b);
    let session = prepare_publication(
        &store,
        "transition-generation-5",
        Some("transition-generation-4"),
        4,
        resume_classifier::CLASSIFIER_EPOCH,
        &[],
    );
    let never_active_terminal = TerminalDocumentUpdate {
        document_id: document_b.id.clone(),
        expected_status: DocumentStatus::FieldsExtracted,
        expected_is_deleted: false,
        expected_content_hash: revision_b.content_hash,
        terminal_status: DocumentStatus::FailedPermanent,
        terminal_is_deleted: false,
    };
    assert_eq!(
        session
            .commit_search_publication(&SearchPublicationCommit {
                generation: "transition-generation-5",
                terminal_documents: &[never_active_terminal],
                projections: &[],
                projected_documents: &[],
                vector_coverage: &[],
                now: now(1_800_020_109),
            })
            .unwrap(),
        SearchPublicationOutcome::Applied
    );
    assert_eq!(
        store
            .document_by_id(&document_b.id)
            .unwrap()
            .unwrap()
            .status,
        DocumentStatus::FailedPermanent
    );
}

#[test]
fn confirmed_deletion_is_only_published_by_atomic_cas() {
    let (_directory, store) = owned_store();
    let document = document("atomic-delete");
    let revision = revision(&document, b"atomic delete source");
    let version = version(&document, &revision, "atomic delete normalized");
    seed_version(&store, &document, &revision, &version);
    let mention = EntityMention {
        id: EntityMentionId::from_non_secret_parts(&[version.id.as_str(), "atomic-delete"]),
        resume_version_id: version.id.clone(),
        section_id: None,
        entity_type: EntityType::Skill,
        raw_value: "synthetic-private-skill".to_string(),
        normalized_value: Some("synthetic-private-skill".to_string()),
        span_start: None,
        span_end: None,
        confidence: 0.9,
        extractor: "atomic-delete-test".to_string(),
    };
    store
        .insert_entity_mentions(&version.id, &[mention])
        .unwrap();
    let projection = ActiveSearchProjection {
        document_id: document.id.clone(),
        resume_version_id: version.id.clone(),
    };
    publish(
        &store,
        "atomic-delete-generation-1",
        None,
        0,
        std::slice::from_ref(&document),
        std::slice::from_ref(&projection),
    );
    let selection = SearchSelection {
        document_id: document.id.clone(),
        resume_version_id: version.id,
        visible_epoch: 1,
    };

    let mut direct_delete = store.document_by_id(&document.id).unwrap().unwrap();
    direct_delete.is_deleted = true;
    direct_delete.status = DocumentStatus::Deleted;
    direct_delete.updated_at = now(1_800_020_009);
    assert_eq!(
        store.upsert_document(&direct_delete).unwrap_err().class(),
        MetaStoreErrorClass::InvalidTransition
    );
    assert_eq!(
        store.purge_deleted_documents().unwrap().deleted_documents,
        0
    );
    assert!(matches!(
        store
            .with_search_metadata_snapshot(|snapshot| {
                Ok::<_, ()>(snapshot.resolve_search_selection(&selection).unwrap())
            })
            .unwrap(),
        SearchSelectionResolution::Current { .. }
    ));

    let session = prepare_publication(
        &store,
        "atomic-delete-generation-2",
        Some("atomic-delete-generation-1"),
        1,
        resume_classifier::CLASSIFIER_EPOCH,
        &[],
    );
    let deletion = TerminalDocumentUpdate {
        document_id: document.id.clone(),
        expected_status: DocumentStatus::Searchable,
        expected_is_deleted: false,
        expected_content_hash: revision.content_hash,
        terminal_status: DocumentStatus::Deleted,
        terminal_is_deleted: true,
    };
    assert_eq!(
        session
            .commit_search_publication(&SearchPublicationCommit {
                generation: "atomic-delete-generation-2",
                terminal_documents: &[deletion],
                projections: &[],
                projected_documents: &[],
                vector_coverage: &[],
                now: now(1_800_020_010),
            })
            .unwrap(),
        SearchPublicationOutcome::Applied
    );
    let deleted = store.document_by_id(&document.id).unwrap().unwrap();
    assert!(deleted.is_deleted);
    assert_eq!(deleted.status, DocumentStatus::Deleted);
    assert_eq!(
        store
            .with_search_metadata_snapshot(|snapshot| {
                Ok::<_, ()>(snapshot.resolve_search_selection(&selection).unwrap())
            })
            .unwrap(),
        SearchSelectionResolution::NotFound
    );
    assert_eq!(
        store.purge_deleted_documents().unwrap().deleted_documents,
        1
    );
    assert!(store.document_by_id(&document.id).unwrap().is_none());
    assert!(store
        .resume_version_by_id(&selection.resume_version_id)
        .unwrap()
        .is_none());
    assert!(store.source_revision_by_id(&revision.id).unwrap().is_none());
    assert!(store
        .entity_mentions_for_version(&selection.resume_version_id)
        .unwrap()
        .is_empty());
}

#[test]
fn deletion_upsert_rechecks_projection_after_a_competing_writer() {
    let (_directory, store) = owned_store();
    let document = document("delete-upsert-race");
    let revision = revision(&document, b"delete upsert race source");
    let version = version(&document, &revision, "delete upsert race normalized");
    seed_version(&store, &document, &revision, &version);
    let projection = ActiveSearchProjection {
        document_id: document.id.clone(),
        resume_version_id: version.id,
    };
    let _publication_session = prepare_publication(
        &store,
        "synthetic-delete-upsert-race-generation",
        None,
        0,
        resume_classifier::CLASSIFIER_EPOCH,
        std::slice::from_ref(&projection),
    );
    let contender = store.open_sibling().unwrap();
    let mut deletion = store.document_by_id(&document.id).unwrap().unwrap();
    deletion.is_deleted = true;
    deletion.status = DocumentStatus::Deleted;

    let held_writer = begin_owned_store_write_race(
        &store,
        OwnedStoreWriteRace::StageSyntheticProjectionCommit {
            projection: projection.clone(),
            generation: "synthetic-delete-upsert-race-generation".to_string(),
        },
    )
    .unwrap();

    let (started_tx, started_rx) = mpsc::channel();
    let deletion_thread = thread::spawn(move || {
        started_tx.send(()).unwrap();
        contender.upsert_document(&deletion)
    });
    started_rx.recv_timeout(Duration::from_secs(1)).unwrap();
    thread::sleep(Duration::from_millis(100));
    assert!(!deletion_thread.is_finished());
    held_writer.commit().unwrap();

    assert_eq!(
        deletion_thread.join().unwrap().unwrap_err().class(),
        MetaStoreErrorClass::InvalidTransition
    );
    let persisted = store.document_by_id(&document.id).unwrap().unwrap();
    assert!(!persisted.is_deleted);
    assert_ne!(persisted.status, DocumentStatus::Deleted);
}

#[test]
fn purge_reads_tombstones_after_acquiring_the_writer_lock() {
    let (_directory, store) = owned_store();
    let document = document("purge-restore-race");
    store.upsert_document(&document).unwrap();
    let mut tombstone = document.clone();
    tombstone.is_deleted = true;
    tombstone.status = DocumentStatus::Deleted;
    store.upsert_document(&tombstone).unwrap();
    let contender = store.open_sibling().unwrap();
    let held_writer = begin_owned_store_write_race(
        &store,
        OwnedStoreWriteRace::RestoreSyntheticDocument {
            document_id: document.id.clone(),
        },
    )
    .unwrap();

    let (started_tx, started_rx) = mpsc::channel();
    let purge_thread = thread::spawn(move || {
        started_tx.send(()).unwrap();
        contender.purge_deleted_documents()
    });
    started_rx.recv_timeout(Duration::from_secs(1)).unwrap();
    thread::sleep(Duration::from_millis(100));
    assert!(!purge_thread.is_finished());
    held_writer.commit().unwrap();

    assert_eq!(purge_thread.join().unwrap().unwrap().deleted_documents, 0);
    let restored = store.document_by_id(&document.id).unwrap().unwrap();
    assert!(!restored.is_deleted);
    assert_eq!(restored.status, DocumentStatus::Discovered);
}

#[test]
fn failed_terminal_update_rolls_back_in_cas_seal_and_head() {
    let (_directory, store) = owned_store();
    let migration_barrier =
        support::acquire_migration_rebuild_barrier_owned(&store, now(1_799_999_999));
    let document = document("terminal-cas-rollback");
    let revision = revision(&document, b"terminal source");
    let version = version(&document, &revision, "terminal normalized");
    seed_version(&store, &document, &revision, &version);
    let projection = ActiveSearchProjection {
        document_id: document.id.clone(),
        resume_version_id: version.id.clone(),
    };
    let session = prepare_publication(
        &store,
        "terminal-cas-rollback",
        None,
        0,
        resume_classifier::CLASSIFIER_EPOCH,
        std::slice::from_ref(&projection),
    );
    let invalid_update = TerminalDocumentUpdate {
        document_id: document.id.clone(),
        expected_status: DocumentStatus::ParseRunning,
        expected_is_deleted: false,
        expected_content_hash: revision.content_hash,
        terminal_status: DocumentStatus::Searchable,
        terminal_is_deleted: false,
    };
    let error = session
        .commit_migration_rebuild_search_publication(
            &SearchPublicationCommit {
                generation: "terminal-cas-rollback",
                terminal_documents: std::slice::from_ref(&invalid_update),
                projections: std::slice::from_ref(&projection),
                projected_documents: &support::projected_documents_for_commit(
                    &store,
                    std::slice::from_ref(&projection),
                    std::slice::from_ref(&invalid_update),
                    now(1_800_020_010),
                ),
                vector_coverage: &[],
                now: now(1_800_020_010),
            },
            &migration_barrier,
        )
        .unwrap_err();
    assert_eq!(
        error.search_publication_failure(),
        Some(SearchPublicationFailure::InvalidDocumentState)
    );
    assert_eq!(
        store.search_projection_state().unwrap().service_state,
        SearchProjectionServiceState::Repairing
    );
    let late_mention = EntityMention {
        id: EntityMentionId::from_non_secret_parts(&[version.id.as_str(), "after-rollback"]),
        resume_version_id: version.id.clone(),
        section_id: None,
        entity_type: EntityType::Skill,
        raw_value: "Rust".to_string(),
        normalized_value: Some("rust".to_string()),
        span_start: None,
        span_end: None,
        confidence: 0.9,
        extractor: "rollback-proof".to_string(),
    };
    assert_eq!(
        store
            .insert_entity_mentions(&version.id, &[late_mention])
            .unwrap(),
        IdentityInsertOutcome::Inserted
    );
}

#[test]
fn metadata_snapshot_remains_all_old_while_writer_commits_new_head() {
    let (_directory, reader) = owned_store();
    let writer = reader.open_sibling().unwrap();
    let document = document("snapshot-isolation");
    let revision = revision(&document, b"snapshot source");
    let version = version(&document, &revision, "snapshot normalized");
    seed_version(&reader, &document, &revision, &version);
    let projection = ActiveSearchProjection {
        document_id: document.id.clone(),
        resume_version_id: version.id.clone(),
    };
    publish(
        &reader,
        "snapshot-generation-1",
        None,
        0,
        std::slice::from_ref(&document),
        std::slice::from_ref(&projection),
    );
    let commit_projection = projection.clone();
    let prepared_projection = projection.clone();
    let (prepared_tx, prepared_rx) = mpsc::channel();
    let (commit_tx, commit_rx) = mpsc::channel();
    let (finished_tx, finished_rx) = mpsc::channel();
    let writer_thread = thread::spawn(move || {
        let session = prepare_publication(
            &writer,
            "snapshot-generation-2",
            Some("snapshot-generation-1"),
            1,
            resume_classifier::CLASSIFIER_EPOCH,
            std::slice::from_ref(&prepared_projection),
        );
        prepared_tx.send(()).unwrap();
        commit_rx.recv().unwrap();
        let result = session.commit_search_publication(&SearchPublicationCommit {
            generation: "snapshot-generation-2",
            terminal_documents: &[],
            projections: std::slice::from_ref(&commit_projection),
            projected_documents: &support::projected_documents_for_commit(
                &writer,
                std::slice::from_ref(&commit_projection),
                &[],
                now(1_800_020_020),
            ),
            vector_coverage: &[],
            now: now(1_800_020_020),
        });
        finished_tx.send(()).unwrap();
        result
    });
    prepared_rx.recv_timeout(Duration::from_secs(1)).unwrap();

    let observed = reader
        .with_search_metadata_snapshot(|snapshot| {
            assert_eq!(snapshot.head().generation, "snapshot-generation-1");
            commit_tx.send(()).unwrap();
            assert!(finished_rx
                .recv_timeout(Duration::from_millis(100))
                .is_err());
            Ok::<_, ()>((
                snapshot.head().generation.clone(),
                snapshot
                    .active_projection_for_document(&document.id)
                    .unwrap()
                    .unwrap(),
            ))
        })
        .unwrap();
    assert_eq!(observed.0, "snapshot-generation-1");
    assert_eq!(observed.1, projection);
    assert_eq!(
        writer_thread.join().unwrap().unwrap(),
        SearchPublicationOutcome::Applied
    );
    assert_eq!(
        reader
            .search_projection_state()
            .unwrap()
            .generation
            .as_deref(),
        Some("snapshot-generation-2")
    );
}

#[test]
fn unrelated_publication_preserves_a_staging_documents_old_projection() {
    let (_directory, store) = owned_store();
    let document_a = document("staging-retained-a");
    let revision_a1 = revision(&document_a, b"retained source A1");
    let version_a1 = version(&document_a, &revision_a1, "retained normalized A1");
    seed_version(&store, &document_a, &revision_a1, &version_a1);
    let projection_a1 = ActiveSearchProjection {
        document_id: document_a.id.clone(),
        resume_version_id: version_a1.id.clone(),
    };
    publish(
        &store,
        "retained-generation-1",
        None,
        0,
        std::slice::from_ref(&document_a),
        std::slice::from_ref(&projection_a1),
    );

    let revision_a2 = revision(&document_a, b"retained source A2");
    let version_a2 = version(&document_a, &revision_a2, "retained normalized A2");
    let mut staging_a = document_a.clone();
    staging_a.content_hash = Some(revision_a2.content_hash.as_str().to_string());
    staging_a.byte_size = revision_a2.byte_size;
    staging_a.status = DocumentStatus::FieldsExtracted;
    store.upsert_document(&staging_a).unwrap();
    store.insert_source_revision(&revision_a2).unwrap();
    store.insert_resume_version(&version_a2).unwrap();
    insert_resume_candidate_classification(&store, &version_a2);

    let document_b = document("staging-retained-b");
    let revision_b = revision(&document_b, b"retained source B");
    let version_b = version(&document_b, &revision_b, "retained normalized B");
    seed_version(&store, &document_b, &revision_b, &version_b);
    let projection_b = ActiveSearchProjection {
        document_id: document_b.id.clone(),
        resume_version_id: version_b.id,
    };
    let projections = [projection_a1.clone(), projection_b.clone()];
    let session = prepare_publication(
        &store,
        "retained-generation-2",
        Some("retained-generation-1"),
        1,
        resume_classifier::CLASSIFIER_EPOCH,
        &projections,
    );
    let invalid_retained_terminal = TerminalDocumentUpdate {
        document_id: document_a.id.clone(),
        expected_status: DocumentStatus::FieldsExtracted,
        expected_is_deleted: false,
        expected_content_hash: revision_a2.content_hash.clone(),
        terminal_status: DocumentStatus::Searchable,
        terminal_is_deleted: false,
    };
    assert_eq!(
        session
            .commit_search_publication(&SearchPublicationCommit {
                generation: "retained-generation-2",
                terminal_documents: std::slice::from_ref(&invalid_retained_terminal),
                projections: &projections,
                projected_documents: &support::projected_documents_for_commit(
                    &store,
                    &projections,
                    std::slice::from_ref(&invalid_retained_terminal),
                    now(1_800_020_029),
                ),
                vector_coverage: &[],
                now: now(1_800_020_029),
            })
            .unwrap_err()
            .search_publication_failure(),
        Some(SearchPublicationFailure::InvalidProjectionTransition)
    );
    let terminal_b = terminal_updates_for(&store, std::slice::from_ref(&projection_b));
    assert_eq!(
        session
            .commit_search_publication(&SearchPublicationCommit {
                generation: "retained-generation-2",
                terminal_documents: &terminal_b,
                projections: &projections,
                projected_documents: &support::projected_documents_for_commit(
                    &store,
                    &projections,
                    &terminal_b,
                    now(1_800_020_030),
                ),
                vector_coverage: &[],
                now: now(1_800_020_030),
            })
            .unwrap(),
        SearchPublicationOutcome::Applied
    );
    assert_eq!(
        store
            .document_by_id(&document_a.id)
            .unwrap()
            .unwrap()
            .status,
        DocumentStatus::FieldsExtracted
    );
    let selection_a1 = SearchSelection {
        document_id: document_a.id,
        resume_version_id: version_a1.id,
        visible_epoch: 1,
    };
    assert!(matches!(
        store
            .with_search_metadata_snapshot(|snapshot| {
                Ok::<_, ()>(snapshot.resolve_search_selection(&selection_a1).unwrap())
            })
            .unwrap(),
        SearchSelectionResolution::Current { .. }
    ));
}

#[test]
fn immutable_stage_metadata_is_query_invisible_until_exact_publication_cas() {
    let (_directory, store) = owned_store();
    let document_a = document("projection-metadata-stage");
    let revision_a = revision(&document_a, b"projection metadata source A");
    let version_a = version(&document_a, &revision_a, "projection metadata normalized A");
    seed_version(&store, &document_a, &revision_a, &version_a);
    let projection_a = ActiveSearchProjection {
        document_id: document_a.id.clone(),
        resume_version_id: version_a.id.clone(),
    };
    assert_eq!(
        publish(
            &store,
            "projection-metadata-generation-a",
            None,
            0,
            std::slice::from_ref(&document_a),
            std::slice::from_ref(&projection_a),
        ),
        SearchPublicationOutcome::Applied
    );
    let published_a = hydrated_document(&store, &projection_a);
    assert_eq!(published_a.status, DocumentStatus::Searchable);
    assert_eq!(
        published_a.content_hash.as_deref(),
        Some(revision_a.content_hash.as_str())
    );

    let revision_b = revision(&document_a, b"projection metadata source B");
    let version_b = version(&document_a, &revision_b, "projection metadata normalized B");
    let mut staged_b = document_a.clone();
    staged_b.source_uri = "synthetic://s807/projection-metadata-stage-b".to_string();
    staged_b.normalized_path = "synthetic/s807/projection-metadata-stage-b.pdf".to_string();
    staged_b.file_name = "projection-metadata-stage-b.pdf".to_string();
    staged_b.extension = FileExtension::Pdf;
    staged_b.byte_size = revision_b.byte_size;
    staged_b.mtime = now(1_800_030_001);
    staged_b.content_hash = Some(revision_b.content_hash.as_str().to_string());
    staged_b.text_hash = Some(version_b.normalized_text_hash.as_str().to_string());
    staged_b.updated_at = now(1_800_030_001);
    staged_b.status = DocumentStatus::FieldsExtracted;
    let classification_b = ResumeVersionClassification {
        resume_version_id: version_b.id.clone(),
        status: ClassificationStatus::ResumeCandidate,
        classifier_epoch: resume_classifier::CLASSIFIER_EPOCH.to_string(),
        reason_codes: vec![ReasonCode::CorroboratedResumeSignals],
        classified_at: now(1_800_030_002),
        review_disposition: ReviewDisposition::NotRequired,
    };
    store
        .stage_immutable_ingest(ImmutableIngestStage::ClassifiedResume {
            document: &staged_b,
            source_revision: &revision_b,
            version: &version_b,
            classification: &classification_b,
            mentions: &[],
            email_hash: None,
            phone_hash: None,
        })
        .unwrap();

    assert_eq!(hydrated_document(&store, &projection_a), published_a);

    let projection_b = ActiveSearchProjection {
        document_id: document_a.id.clone(),
        resume_version_id: version_b.id.clone(),
    };
    let session = prepare_publication(
        &store,
        "projection-metadata-generation-b",
        Some("projection-metadata-generation-a"),
        1,
        resume_classifier::CLASSIFIER_EPOCH,
        std::slice::from_ref(&projection_b),
    );
    let invalid_terminal = TerminalDocumentUpdate {
        document_id: document_a.id.clone(),
        expected_status: DocumentStatus::FieldsExtracted,
        expected_is_deleted: false,
        expected_content_hash: revision_a.content_hash.clone(),
        terminal_status: DocumentStatus::Searchable,
        terminal_is_deleted: false,
    };
    assert_eq!(
        session
            .commit_search_publication(&SearchPublicationCommit {
                generation: "projection-metadata-generation-b",
                terminal_documents: std::slice::from_ref(&invalid_terminal),
                projections: std::slice::from_ref(&projection_b),
                projected_documents: &support::projected_documents_for_commit(
                    &store,
                    std::slice::from_ref(&projection_b),
                    std::slice::from_ref(&invalid_terminal),
                    now(1_800_030_003),
                ),
                vector_coverage: &[],
                now: now(1_800_030_003),
            })
            .unwrap_err()
            .search_publication_failure(),
        Some(SearchPublicationFailure::InvalidDocumentState)
    );
    assert_eq!(hydrated_document(&store, &projection_a), published_a);

    let terminal_b = terminal_updates_for(&store, std::slice::from_ref(&projection_b));
    assert_eq!(
        session
            .commit_search_publication(&SearchPublicationCommit {
                generation: "projection-metadata-generation-b",
                terminal_documents: &terminal_b,
                projections: std::slice::from_ref(&projection_b),
                projected_documents: &support::projected_documents_for_commit(
                    &store,
                    std::slice::from_ref(&projection_b),
                    &terminal_b,
                    now(1_800_030_004),
                ),
                vector_coverage: &[],
                now: now(1_800_030_004),
            })
            .unwrap(),
        SearchPublicationOutcome::Applied
    );

    assert_eq!(
        resolve_in_snapshot(
            &store,
            &SearchSelection {
                document_id: projection_a.document_id.clone(),
                resume_version_id: projection_a.resume_version_id.clone(),
                visible_epoch: 1,
            },
        ),
        SearchSelectionResolution::Stale
    );
    let published_b = hydrated_document(&store, &projection_b);
    assert_eq!(published_b.source_uri, staged_b.source_uri);
    assert_eq!(published_b.normalized_path, staged_b.normalized_path);
    assert_eq!(published_b.file_name, staged_b.file_name);
    assert_eq!(published_b.extension, staged_b.extension);
    assert_eq!(published_b.byte_size, staged_b.byte_size);
    assert_eq!(published_b.mtime, staged_b.mtime);
    assert_eq!(published_b.content_hash, staged_b.content_hash);
    assert_eq!(published_b.text_hash, staged_b.text_hash);
    assert_eq!(published_b.status, DocumentStatus::Searchable);
}

#[test]
fn same_version_metadata_change_requires_exact_snapshot_and_advances_head_atomically() {
    let (_directory, store) = owned_store();
    let document = document("same-version-metadata");
    let revision = revision(&document, b"same version metadata source");
    let version = version(&document, &revision, "same version metadata text");
    seed_version(&store, &document, &revision, &version);
    let projection = ActiveSearchProjection {
        document_id: document.id.clone(),
        resume_version_id: version.id.clone(),
    };
    assert_eq!(
        publish(
            &store,
            "same-version-metadata-a",
            None,
            0,
            std::slice::from_ref(&document),
            std::slice::from_ref(&projection),
        ),
        SearchPublicationOutcome::Applied
    );
    let published = hydrated_document(&store, &projection);

    let mut renamed = published.clone();
    renamed.source_uri = "synthetic://s807/same-version-renamed.txt".to_string();
    renamed.normalized_path = "synthetic/s807/same-version-renamed.txt".to_string();
    renamed.file_name = "same-version-renamed.txt".to_string();
    renamed.mtime = now(1_800_030_101);
    renamed.updated_at = now(1_800_030_101);
    store.upsert_document(&renamed).unwrap();
    assert_eq!(hydrated_document(&store, &projection), published);

    let session = prepare_publication(
        &store,
        "same-version-metadata-b",
        Some("same-version-metadata-a"),
        1,
        resume_classifier::CLASSIFIER_EPOCH,
        std::slice::from_ref(&projection),
    );
    let mut mismatched = renamed.clone();
    mismatched.file_name = "not-the-staged-name.txt".to_string();
    let mismatched_action = [ProjectedDocumentSnapshot::MetadataChanged {
        projection: projection.clone(),
        document: mismatched,
    }];
    let error = session
        .commit_search_publication(&SearchPublicationCommit {
            generation: "same-version-metadata-b",
            terminal_documents: &[],
            projections: std::slice::from_ref(&projection),
            projected_documents: &mismatched_action,
            vector_coverage: &[],
            now: now(1_800_030_102),
        })
        .unwrap_err();
    assert_eq!(
        error.search_publication_failure(),
        Some(SearchPublicationFailure::InvalidDocumentState)
    );
    assert_eq!(hydrated_document(&store, &projection), published);
    assert_eq!(
        store
            .search_projection_state()
            .unwrap()
            .generation
            .as_deref(),
        Some("same-version-metadata-a")
    );

    let changed_action = [ProjectedDocumentSnapshot::MetadataChanged {
        projection: projection.clone(),
        document: renamed.clone(),
    }];
    assert_eq!(
        session
            .commit_search_publication(&SearchPublicationCommit {
                generation: "same-version-metadata-b",
                terminal_documents: &[],
                projections: std::slice::from_ref(&projection),
                projected_documents: &changed_action,
                vector_coverage: &[],
                now: now(1_800_030_102),
            })
            .unwrap(),
        SearchPublicationOutcome::Applied
    );
    assert_eq!(hydrated_document(&store, &projection), renamed);
    let head = store.search_projection_state().unwrap();
    assert_eq!(head.generation.as_deref(), Some("same-version-metadata-b"));
    assert_eq!(head.visible_epoch, 2);
    assert_eq!(
        store.resume_versions_for_document(&document.id).unwrap(),
        vec![version]
    );
    drop(session);

    let noop_session = prepare_publication(
        &store,
        "same-version-metadata-noop",
        Some("same-version-metadata-b"),
        2,
        resume_classifier::CLASSIFIER_EPOCH,
        std::slice::from_ref(&projection),
    );
    let noop_action = [ProjectedDocumentSnapshot::MetadataChanged {
        projection: projection.clone(),
        document: renamed,
    }];
    let error = noop_session
        .commit_search_publication(&SearchPublicationCommit {
            generation: "same-version-metadata-noop",
            terminal_documents: &[],
            projections: std::slice::from_ref(&projection),
            projected_documents: &noop_action,
            vector_coverage: &[],
            now: now(1_800_030_103),
        })
        .unwrap_err();
    assert_eq!(
        error.search_publication_failure(),
        Some(SearchPublicationFailure::InvalidDocumentState)
    );
    assert_eq!(store.search_projection_state().unwrap(), head);
}

#[test]
fn v29_identity_and_derived_rows_are_insert_once() {
    let (_directory, store) = owned_store();
    assert_eq!(store.schema_version().unwrap(), 29);
    assert_eq!(
        store.search_projection_state().unwrap().service_state,
        SearchProjectionServiceState::Repairing
    );

    let document = document("immutable");
    let revision = revision(&document, b"synthetic source A");
    let version = version(&document, &revision, "synthetic normalized A");
    seed_version(&store, &document, &revision, &version);
    assert_eq!(
        store.insert_source_revision(&revision).unwrap(),
        IdentityInsertOutcome::AlreadyPresent
    );
    assert_eq!(
        store.insert_resume_version(&version).unwrap(),
        IdentityInsertOutcome::AlreadyPresent
    );

    let mut changed_revision = revision.clone();
    changed_revision.byte_size += 1;
    assert_eq!(
        store
            .insert_source_revision(&changed_revision)
            .unwrap_err()
            .class(),
        MetaStoreErrorClass::ImmutableIdentityConflict
    );
    let mut changed_version = version.clone();
    changed_version.raw_text = Some("different payload".to_string());
    assert_eq!(
        store
            .insert_resume_version(&changed_version)
            .unwrap_err()
            .class(),
        MetaStoreErrorClass::ImmutableIdentityConflict
    );
    let mut detached_hash = version.clone();
    detached_hash.clean_text = Some("different canonical text".to_string());
    assert_eq!(
        store
            .insert_resume_version(&detached_hash)
            .unwrap_err()
            .class(),
        MetaStoreErrorClass::InvalidValue
    );

    let mention = EntityMention {
        id: EntityMentionId::from_non_secret_parts(&[version.id.as_str(), "skill"]),
        resume_version_id: version.id.clone(),
        section_id: None,
        entity_type: EntityType::Skill,
        raw_value: "Rust".to_string(),
        normalized_value: Some("rust".to_string()),
        span_start: Some(0),
        span_end: Some(4),
        confidence: 0.9,
        extractor: "synthetic-v1".to_string(),
    };
    assert_eq!(
        store
            .insert_entity_mentions(&version.id, std::slice::from_ref(&mention))
            .unwrap(),
        IdentityInsertOutcome::Inserted
    );
    assert_eq!(
        store
            .insert_entity_mentions(&version.id, std::slice::from_ref(&mention))
            .unwrap(),
        IdentityInsertOutcome::AlreadyPresent
    );
    let mut changed_mention = mention;
    changed_mention.raw_value = "Go".to_string();
    assert_eq!(
        store
            .insert_entity_mentions(&version.id, &[changed_mention])
            .unwrap_err()
            .class(),
        MetaStoreErrorClass::ImmutableIdentityConflict
    );
}

#[test]
fn ocr_triage_and_final_version_classification_are_independently_immutable() {
    let (_directory, store) = owned_store();
    let mut document = document("classification");
    let revision = revision(&document, b"classified source");
    document.content_hash = Some(revision.content_hash.as_str().to_string());
    store.upsert_document(&document).unwrap();
    store.insert_source_revision(&revision).unwrap();
    let triage = SourceRevisionTriage {
        source_revision_id: revision.id.clone(),
        status: ClassificationStatus::OcrBacklog,
        triage_epoch: resume_classifier::CLASSIFIER_EPOCH.to_string(),
        reason_codes: vec![ReasonCode::OcrRequired],
        triaged_at: now(1_800_000_100),
    };
    assert_eq!(
        store.insert_source_revision_triage(&triage).unwrap(),
        IdentityInsertOutcome::Inserted
    );
    assert_eq!(
        store.insert_source_revision_triage(&triage).unwrap(),
        IdentityInsertOutcome::AlreadyPresent
    );
    let mut changed_triage = triage.clone();
    changed_triage.status = ClassificationStatus::Failed;
    changed_triage.reason_codes = vec![ReasonCode::ParserFailed];
    assert_eq!(
        store
            .insert_source_revision_triage(&changed_triage)
            .unwrap_err()
            .class(),
        MetaStoreErrorClass::ImmutableIdentityConflict
    );

    let version = version(&document, &revision, "OCR normalized resume");
    store.insert_resume_version(&version).unwrap();
    let classification = ResumeVersionClassification {
        resume_version_id: version.id.clone(),
        status: ClassificationStatus::ResumeCandidate,
        classifier_epoch: resume_classifier::CLASSIFIER_EPOCH.to_string(),
        reason_codes: vec![ReasonCode::CorroboratedResumeSignals],
        classified_at: now(1_800_000_110),
        review_disposition: ReviewDisposition::NotRequired,
    };
    assert_eq!(
        store
            .insert_resume_version_classification(&classification)
            .unwrap(),
        IdentityInsertOutcome::Inserted
    );
    assert_eq!(
        store
            .source_revision_triage(&revision.id, resume_classifier::CLASSIFIER_EPOCH)
            .unwrap()
            .unwrap()
            .status,
        ClassificationStatus::OcrBacklog
    );
    assert_eq!(
        store
            .resume_version_classification(&version.id, resume_classifier::CLASSIFIER_EPOCH)
            .unwrap()
            .unwrap()
            .status,
        ClassificationStatus::ResumeCandidate
    );
    let mut changed_classification = classification;
    changed_classification.status = ClassificationStatus::NonResume;
    changed_classification.reason_codes = vec![ReasonCode::CorroboratedNonResumeSignals];
    assert_eq!(
        store
            .insert_resume_version_classification(&changed_classification)
            .unwrap_err()
            .class(),
        MetaStoreErrorClass::ImmutableIdentityConflict
    );
    let promoted_epoch = format!("{}0123456789ab", resume_classifier::PROMOTED_EPOCH_PREFIX);
    changed_classification.classifier_epoch = promoted_epoch.clone();
    assert_eq!(
        store
            .insert_resume_version_classification(&changed_classification)
            .unwrap(),
        IdentityInsertOutcome::Inserted
    );
    let deterministic_counts = store
        .classification_counts(resume_classifier::CLASSIFIER_EPOCH)
        .unwrap();
    assert_eq!(deterministic_counts.resume_candidate, 1);
    assert_eq!(deterministic_counts.non_resume, 0);
    assert_eq!(deterministic_counts.ocr_backlog, 1);
    let promoted_counts = store.classification_counts(&promoted_epoch).unwrap();
    assert_eq!(promoted_counts.resume_candidate, 0);
    assert_eq!(promoted_counts.non_resume, 1);
    assert_eq!(promoted_counts.ocr_backlog, 0);
    assert!(store
        .resume_version_has_resume_candidate_classification_at_epoch(
            &version.id,
            resume_classifier::CLASSIFIER_EPOCH,
        )
        .unwrap());
    assert!(!store
        .resume_version_has_resume_candidate_classification_at_epoch(&version.id, &promoted_epoch)
        .unwrap());
    assert_eq!(
        store
            .classification_counts("precision_first_v27_replay")
            .unwrap_err()
            .class(),
        MetaStoreErrorClass::InvalidValue
    );
}

#[test]
fn non_resume_classification_is_bound_to_one_exact_version() {
    let (_directory, store) = owned_store();
    let document = document("non-resume-version-bound");
    let revision = revision(&document, b"same source");
    store.upsert_document(&document).unwrap();
    store.insert_source_revision(&revision).unwrap();
    let first = version(&document, &revision, "normalized non-resume A");
    let second = version(&document, &revision, "normalized non-resume B");
    store.insert_resume_version(&first).unwrap();
    store.insert_resume_version(&second).unwrap();
    let classification = ResumeVersionClassification {
        resume_version_id: first.id.clone(),
        status: ClassificationStatus::NonResume,
        classifier_epoch: resume_classifier::CLASSIFIER_EPOCH.to_string(),
        reason_codes: vec![ReasonCode::CorroboratedNonResumeSignals],
        classified_at: now(1_800_000_120),
        review_disposition: ReviewDisposition::NotRequired,
    };
    store
        .insert_resume_version_classification(&classification)
        .unwrap();
    assert_eq!(
        store
            .resume_version_classification(&first.id, resume_classifier::CLASSIFIER_EPOCH)
            .unwrap()
            .unwrap()
            .status,
        ClassificationStatus::NonResume
    );
    assert!(store
        .resume_version_classification(&second.id, resume_classifier::CLASSIFIER_EPOCH)
        .unwrap()
        .is_none());
}

#[test]
fn ocr_job_identity_is_bound_to_exact_source_revision_and_triage_epoch() {
    let (_directory, store) = owned_store();
    let mut document = document("ocr-job-spec");
    document.status = DocumentStatus::OcrRequired;
    let first_revision = revision(&document, b"ocr source A");
    document.content_hash = Some(first_revision.content_hash.as_str().to_string());
    store.upsert_document(&document).unwrap();
    store.insert_source_revision(&first_revision).unwrap();
    let deterministic_triage = SourceRevisionTriage {
        source_revision_id: first_revision.id.clone(),
        status: ClassificationStatus::OcrBacklog,
        triage_epoch: resume_classifier::CLASSIFIER_EPOCH.to_string(),
        reason_codes: vec![ReasonCode::OcrRequired],
        triaged_at: now(1_800_000_130),
    };
    store
        .insert_source_revision_triage(&deterministic_triage)
        .unwrap();
    let promoted_epoch = format!("{}0123456789ab", resume_classifier::PROMOTED_EPOCH_PREFIX);
    let promoted_triage = SourceRevisionTriage {
        triage_epoch: promoted_epoch.clone(),
        ..deterministic_triage
    };
    store
        .insert_source_revision_triage(&promoted_triage)
        .unwrap();

    let deterministic_epoch = CurrentClassifierEpoch::parse(resume_classifier::CLASSIFIER_EPOCH)
        .expect("deterministic epoch is current");
    let promoted_epoch_value =
        CurrentClassifierEpoch::parse(&promoted_epoch).expect("promoted epoch is current");
    let deterministic_job = store
        .enqueue_ocr_job_for_source_triage(
            &first_revision.id,
            deterministic_epoch,
            now(1_800_000_140),
        )
        .unwrap()
        .job;
    let promoted_job = store
        .enqueue_ocr_job_for_source_triage(
            &first_revision.id,
            promoted_epoch_value,
            now(1_800_000_141),
        )
        .unwrap()
        .job;
    assert_ne!(deterministic_job.id, promoted_job.id);
    assert_eq!(
        store
            .ocr_job_for_source_triage(&first_revision.id, deterministic_epoch)
            .unwrap()
            .unwrap()
            .id,
        deterministic_job.id
    );

    let claimed = store
        .claim_next_ocr_job(now(1_800_000_142))
        .unwrap()
        .unwrap();
    assert_eq!(claimed.source_revision_id(), &first_revision.id);
    assert_eq!(claimed.triage_epoch(), resume_classifier::CLASSIFIER_EPOCH);
    assert_eq!(
        claimed.source_fingerprint(),
        first_revision.content_hash.as_str()
    );
    assert!(store.ocr_claim_is_current(&claimed).unwrap());
    let promoted_claimed = store
        .claim_next_ocr_job(now(1_800_000_143))
        .unwrap()
        .unwrap();
    assert_eq!(promoted_claimed.job.id, promoted_job.id);
    assert_eq!(promoted_claimed.triage_epoch(), promoted_epoch);

    let second_revision = revision(&document, b"ocr source B");
    document.content_hash = Some(second_revision.content_hash.as_str().to_string());
    store.upsert_document(&document).unwrap();
    store.insert_source_revision(&second_revision).unwrap();
    store
        .insert_source_revision_triage(&SourceRevisionTriage {
            source_revision_id: second_revision.id.clone(),
            status: ClassificationStatus::OcrBacklog,
            triage_epoch: resume_classifier::CLASSIFIER_EPOCH.to_string(),
            reason_codes: vec![ReasonCode::OcrRequired],
            triaged_at: now(1_800_000_150),
        })
        .unwrap();
    assert!(!store.ocr_claim_is_current(&claimed).unwrap());
    assert_eq!(
        store
            .finish_ocr_attempt_failure(&claimed, OcrAttemptFailure::Retryable, now(1_800_000_151),)
            .unwrap(),
        OcrAttemptFailureOutcome::Superseded
    );
    assert_eq!(
        store
            .finish_ocr_attempt_failure(
                &promoted_claimed,
                OcrAttemptFailure::Retryable,
                now(1_800_000_152),
            )
            .unwrap(),
        OcrAttemptFailureOutcome::Superseded
    );
    assert_eq!(
        store.ocr_job_discard_reason(&claimed.job.id).unwrap(),
        Some(OcrJobDiscardReason::SourceRevisionNoLongerCurrent)
    );
    assert_eq!(
        store
            .ingest_job_by_id(&claimed.job.id)
            .unwrap()
            .unwrap()
            .status,
        meta_store::IngestJobStatus::Completed
    );
    assert!(store
        .claim_next_ocr_job(now(1_800_000_152))
        .unwrap()
        .is_none());
    assert_eq!(
        store.ocr_job_discard_reason(&promoted_job.id).unwrap(),
        Some(OcrJobDiscardReason::SourceRevisionNoLongerCurrent)
    );
    assert!(!store
        .retryable_jobs()
        .unwrap()
        .iter()
        .any(|job| job.id == deterministic_job.id || job.id == promoted_job.id));
    assert!(!store
        .jobs_requiring_recovery()
        .unwrap()
        .iter()
        .any(|job| job.id == deterministic_job.id || job.id == promoted_job.id));
    assert_eq!(
        store
            .recover_stale_running_ingest_jobs(now(1_800_000_153), now(1_800_000_152),)
            .unwrap(),
        0
    );

    let second_job = store
        .enqueue_ocr_job_for_source_triage(
            &second_revision.id,
            deterministic_epoch,
            now(1_800_000_154),
        )
        .unwrap()
        .job;
    assert_ne!(second_job.id, deterministic_job.id);
    assert_ne!(second_job.id, promoted_job.id);
}

#[test]
fn current_classifier_epoch_accepts_only_the_deterministic_or_bound_model_generation() {
    let deterministic = CurrentClassifierEpoch::parse(resume_classifier::CLASSIFIER_EPOCH).unwrap();
    assert_eq!(deterministic.as_str(), resume_classifier::CLASSIFIER_EPOCH);
    assert_eq!(deterministic.source(), ClassifierEpochSource::Deterministic);

    let promoted = format!("{}0123456789ab", resume_classifier::PROMOTED_EPOCH_PREFIX);
    assert_eq!(
        CurrentClassifierEpoch::parse(&promoted).unwrap().source(),
        ClassifierEpochSource::LocalLinearPromotion
    );
    assert!(CurrentClassifierEpoch::parse("precision_first_v27_replay").is_none());
    assert!(CurrentClassifierEpoch::parse(&format!(
        "{}0123456789AB",
        resume_classifier::PROMOTED_EPOCH_PREFIX
    ))
    .is_none());
}

#[test]
fn projection_rejects_version_without_current_resume_candidate_classification() {
    let (_directory, store) = owned_store();
    let migration_barrier =
        support::acquire_migration_rebuild_barrier_owned(&store, now(1_799_999_999));
    let document = document("unclassified-projection");
    let revision = revision(&document, b"unclassified source");
    let version = version(&document, &revision, "unclassified normalized text");
    let mut staged_document = document.clone();
    staged_document.content_hash = Some(revision.content_hash.as_str().to_string());
    staged_document.status = DocumentStatus::FieldsExtracted;
    store.upsert_document(&staged_document).unwrap();
    store.insert_source_revision(&revision).unwrap();
    store.insert_resume_version(&version).unwrap();

    let generation = "unclassified-generation";
    let projection = ActiveSearchProjection {
        document_id: document.id.clone(),
        resume_version_id: version.id,
    };
    let session = prepare_publication(
        &store,
        generation,
        None,
        0,
        resume_classifier::CLASSIFIER_EPOCH,
        std::slice::from_ref(&projection),
    );
    let terminal_documents = terminal_updates_for(&store, std::slice::from_ref(&projection));
    assert_eq!(
        session
            .commit_migration_rebuild_search_publication(
                &SearchPublicationCommit {
                    generation,
                    terminal_documents: &terminal_documents,
                    projections: std::slice::from_ref(&projection),
                    projected_documents: &support::projected_documents_for_commit(
                        &store,
                        std::slice::from_ref(&projection),
                        &terminal_documents,
                        now(1_800_000_133),
                    ),
                    vector_coverage: &[],
                    now: now(1_800_000_133),
                },
                &migration_barrier,
            )
            .unwrap_err()
            .search_publication_failure(),
        Some(SearchPublicationFailure::ExactClassificationMissing)
    );
    let projection_state = store.search_projection_state().unwrap();
    assert_eq!(
        projection_state.service_state,
        SearchProjectionServiceState::Repairing
    );
    assert_eq!(projection_state.generation, None);
}

#[test]
fn selection_stays_current_across_unrelated_epoch_and_stales_on_target_switch() {
    let (_directory, store) = owned_store();
    let document_a = document("selection-a");
    let revision_a = revision(&document_a, b"source A1");
    let version_a = version(&document_a, &revision_a, "normalized A1");
    seed_version(&store, &document_a, &revision_a, &version_a);
    let selection = SearchSelection {
        document_id: document_a.id.clone(),
        resume_version_id: version_a.id.clone(),
        visible_epoch: 0,
    };
    assert!(matches!(
        store.with_search_metadata_snapshot(|snapshot| {
            Ok::<_, ()>(snapshot.resolve_search_selection(&selection).unwrap())
        }),
        Err(SearchMetadataTransactionError::Unavailable(
            SearchMetadataUnavailable::Repairing(_)
        ))
    ));
    let projection_a = ActiveSearchProjection {
        document_id: document_a.id.clone(),
        resume_version_id: version_a.id.clone(),
    };
    assert_eq!(
        publish(
            &store,
            "generation-1",
            None,
            0,
            std::slice::from_ref(&document_a),
            std::slice::from_ref(&projection_a),
        ),
        SearchPublicationOutcome::Applied
    );
    let selection = SearchSelection {
        visible_epoch: 1,
        ..selection
    };
    assert!(matches!(
        resolve_in_snapshot(&store, &selection),
        SearchSelectionResolution::Current { .. }
    ));
    let future_selection = SearchSelection {
        visible_epoch: 2,
        ..selection.clone()
    };
    assert_eq!(
        store
            .with_search_metadata_snapshot(|snapshot| {
                Ok::<_, ()>(
                    snapshot
                        .resolve_search_selection(&future_selection)
                        .unwrap(),
                )
            })
            .unwrap(),
        SearchSelectionResolution::NotFound
    );

    let document_b = document("selection-b");
    let revision_b = revision(&document_b, b"source B1");
    let version_b = version(&document_b, &revision_b, "normalized B1");
    seed_version(&store, &document_b, &revision_b, &version_b);
    let projection_b = ActiveSearchProjection {
        document_id: document_b.id.clone(),
        resume_version_id: version_b.id,
    };
    assert_eq!(
        publish(
            &store,
            "generation-2",
            Some("generation-1"),
            1,
            &[document_a.clone(), document_b.clone()],
            &[projection_a.clone(), projection_b],
        ),
        SearchPublicationOutcome::Applied
    );
    assert!(matches!(
        resolve_in_snapshot(&store, &selection),
        SearchSelectionResolution::Current { .. }
    ));
    assert!(matches!(
        store
            .with_search_metadata_snapshot(|snapshot| {
                Ok::<_, ()>(snapshot.resolve_search_selection(&selection).unwrap())
            })
            .unwrap(),
        SearchSelectionResolution::Current { .. }
    ));

    let revision_a2 = revision(&document_a, b"source A2");
    let version_a2 = version(&document_a, &revision_a2, "normalized A2");
    let mut staged_document_a2 = document_a.clone();
    staged_document_a2.content_hash = Some(revision_a2.content_hash.as_str().to_string());
    staged_document_a2.byte_size = revision_a2.byte_size;
    staged_document_a2.status = DocumentStatus::FieldsExtracted;
    store.upsert_document(&staged_document_a2).unwrap();
    store.insert_source_revision(&revision_a2).unwrap();
    store.insert_resume_version(&version_a2).unwrap();
    insert_resume_candidate_classification(&store, &version_a2);
    assert!(matches!(
        store
            .with_search_metadata_snapshot(|snapshot| {
                Ok::<_, ()>(snapshot.resolve_search_selection(&selection).unwrap())
            })
            .unwrap(),
        SearchSelectionResolution::Current { .. }
    ));
    let projection_a2 = ActiveSearchProjection {
        document_id: document_a.id.clone(),
        resume_version_id: version_a2.id,
    };
    let session = prepare_publication(
        &store,
        "generation-3",
        Some("generation-2"),
        2,
        resume_classifier::CLASSIFIER_EPOCH,
        std::slice::from_ref(&projection_a2),
    );
    let terminal_a2 = terminal_updates_for(&store, std::slice::from_ref(&projection_a2));
    let removal_b = TerminalDocumentUpdate {
        document_id: document_b.id,
        expected_status: DocumentStatus::Searchable,
        expected_is_deleted: false,
        expected_content_hash: revision_b.content_hash,
        terminal_status: DocumentStatus::FailedPermanent,
        terminal_is_deleted: false,
    };
    let terminal_updates = [terminal_a2[0].clone(), removal_b];
    assert_eq!(
        session
            .commit_search_publication(&SearchPublicationCommit {
                generation: "generation-3",
                terminal_documents: &terminal_updates,
                projections: std::slice::from_ref(&projection_a2),
                projected_documents: &support::projected_documents_for_commit(
                    &store,
                    std::slice::from_ref(&projection_a2),
                    &terminal_updates,
                    now(1_800_000_163),
                ),
                vector_coverage: &[],
                now: now(1_800_000_163),
            })
            .unwrap(),
        SearchPublicationOutcome::Applied
    );
    assert_eq!(
        resolve_in_snapshot(&store, &selection),
        SearchSelectionResolution::Stale
    );
    assert_eq!(
        store
            .with_search_metadata_snapshot(|snapshot| {
                Ok::<_, ()>(snapshot.resolve_search_selection(&selection).unwrap())
            })
            .unwrap(),
        SearchSelectionResolution::Stale
    );
}

#[test]
fn projection_failure_and_wrong_expected_head_preserve_old_snapshot() {
    let (_directory, store) = owned_store();
    let document = document("cas");
    let revision = revision(&document, b"cas source");
    let version = version(&document, &revision, "cas normalized");
    seed_version(&store, &document, &revision, &version);
    let projection = ActiveSearchProjection {
        document_id: document.id.clone(),
        resume_version_id: version.id.clone(),
    };
    publish(
        &store,
        "generation-1",
        None,
        0,
        std::slice::from_ref(&document),
        std::slice::from_ref(&projection),
    );

    let invalid_projection = ActiveSearchProjection {
        document_id: document.id.clone(),
        resume_version_id: ResumeVersionId::from_non_secret_parts(&[
            "s807",
            "missing-publication-version",
        ]),
    };
    let session = prepare_publication(
        &store,
        "generation-invalid-projection",
        Some("generation-1"),
        1,
        resume_classifier::CLASSIFIER_EPOCH,
        std::slice::from_ref(&invalid_projection),
    );
    assert_eq!(
        session
            .commit_search_publication(&SearchPublicationCommit {
                generation: "generation-invalid-projection",
                terminal_documents: &[],
                projections: std::slice::from_ref(&invalid_projection),
                projected_documents: &support::projected_documents_for_commit(
                    &store,
                    std::slice::from_ref(&invalid_projection),
                    &[],
                    now(1_800_000_193),
                ),
                vector_coverage: &[],
                now: now(1_800_000_193),
            })
            .unwrap_err()
            .search_publication_failure(),
        Some(SearchPublicationFailure::InvalidProjectionTransition)
    );
    let preserved = store.search_projection_state().unwrap();
    assert_eq!(preserved.generation.as_deref(), Some("generation-1"));
    assert_eq!(preserved.visible_epoch, 1);
    session
        .abandon_search_publication("generation-invalid-projection", now(1_800_000_194))
        .unwrap();
    prepare_publication_in_session(
        &session,
        "generation-stale-cas",
        Some("generation-1"),
        1,
        resume_classifier::CLASSIFIER_EPOCH,
        std::slice::from_ref(&projection),
    );
    prepare_publication_in_session(
        &session,
        "generation-2",
        Some("generation-1"),
        1,
        resume_classifier::CLASSIFIER_EPOCH,
        std::slice::from_ref(&projection),
    );
    assert_eq!(
        session
            .commit_search_publication(&SearchPublicationCommit {
                generation: "generation-2",
                terminal_documents: &[],
                projections: std::slice::from_ref(&projection),
                projected_documents: &support::projected_documents_for_commit(
                    &store,
                    std::slice::from_ref(&projection),
                    &[],
                    now(1_800_000_202),
                ),
                vector_coverage: &[],
                now: now(1_800_000_202),
            })
            .unwrap(),
        SearchPublicationOutcome::Applied
    );
    assert_eq!(
        session
            .commit_search_publication(&SearchPublicationCommit {
                generation: "generation-stale-cas",
                terminal_documents: &[],
                projections: std::slice::from_ref(&projection),
                projected_documents: &support::projected_documents_for_commit(
                    &store,
                    std::slice::from_ref(&projection),
                    &[],
                    now(1_800_000_203),
                ),
                vector_coverage: &[],
                now: now(1_800_000_203),
            })
            .unwrap(),
        SearchPublicationOutcome::Superseded
    );
    let projection_state = store.search_projection_state().unwrap();
    assert_eq!(projection_state.generation.as_deref(), Some("generation-2"));
    assert_eq!(projection_state.visible_epoch, 2);
}

#[test]
fn publications_are_abandoned_exactly_idempotently_and_pruned_with_a_bound() {
    let (_directory, store) = owned_store();
    let barrier = support::acquire_migration_rebuild_barrier_owned(&store, now(1_800_000_209));
    let mut session = store.wait_for_search_publication_session().unwrap();
    let MigrationRebuildPublicationAttemptAcquire::Started(attempt) = session
        .acquire_migration_rebuild_publication_attempt(&barrier, now(1_800_000_209))
        .unwrap()
    else {
        panic!("expected the synthetic migration attempt to start");
    };
    let empty_digest = SearchProjectionDigest::from_pairs::<_, &str, &str>([]).unwrap();
    let preparing = SearchPublicationDraft {
        generation: "interrupted-preparing".to_string(),
        base_generation: None,
        expected_visible_epoch: 0,
        classifier_epoch: resume_classifier::CLASSIFIER_EPOCH.to_string(),
        projection_digest: empty_digest,
        now: now(1_800_000_210),
    };
    assert_eq!(
        session.begin_search_publication(&preparing).unwrap(),
        SearchPublicationOutcome::Applied
    );
    prepare_publication_in_session(
        &session,
        "interrupted-validated",
        None,
        0,
        resume_classifier::CLASSIFIER_EPOCH,
        &[],
    );

    for generation in ["interrupted-preparing", "interrupted-validated"] {
        session
            .abandon_search_publication(generation, now(1_800_000_220))
            .unwrap();
        session
            .abandon_search_publication(generation, now(1_800_000_221))
            .unwrap();
        assert_eq!(
            store.search_publication(generation).unwrap().unwrap().state,
            SearchPublicationState::Abandoned
        );
    }
    assert_eq!(
        session
            .abandon_search_publication("missing-generation", now(1_800_000_222))
            .unwrap_err()
            .search_publication_failure(),
        Some(SearchPublicationFailure::InvalidState)
    );
    session
        .abandon_migration_rebuild_publication_attempt(&attempt)
        .unwrap();
    drop(session);
    publish(&store, "ready-cannot-abandon", None, 0, &[], &[]);
    let session = store.wait_for_search_publication_session().unwrap();
    assert_eq!(
        session
            .abandon_search_publication("ready-cannot-abandon", now(1_800_000_223))
            .unwrap_err()
            .search_publication_failure(),
        Some(SearchPublicationFailure::InvalidState)
    );
    assert_eq!(
        session
            .prune_search_publication_history(SearchPublicationPrunePolicy {
                retain_ready: 1,
                abandoned_updated_before: now(1_800_000_220),
                max_delete: 1,
            })
            .unwrap(),
        1
    );
    assert_eq!(
        session
            .prune_search_publication_history(SearchPublicationPrunePolicy {
                retain_ready: 1,
                abandoned_updated_before: now(1_800_000_220),
                max_delete: 1,
            })
            .unwrap(),
        1
    );
}

#[test]
fn failed_publication_retirement_blocks_the_exact_initial_migration_head() {
    let (_directory, store) = owned_store();
    let before = store.search_projection_state().unwrap();
    assert_eq!(
        before.service_state,
        SearchProjectionServiceState::Repairing
    );
    assert_eq!(
        before.repair_reason,
        Some(meta_store::SearchRepairReason::MigrationRebuild)
    );
    assert_eq!(before.generation, None);

    let session = prepare_publication(
        &store,
        "retirement-failure-initial",
        None,
        before.visible_epoch,
        resume_classifier::CLASSIFIER_EPOCH,
        &[],
    );
    session
        .begin_search_publication_retirement(
            "retirement-failure-initial",
            now(1_800_000_224),
            SearchPublicationRetirementPlan {
                fulltext: SearchArtifactExpectation::MayExist,
                vector: SearchArtifactExpectation::MayExist,
            },
        )
        .unwrap();

    assert_eq!(
        session
            .block_search_head_after_publication_retirement_failure(
                "retirement-failure-initial",
                now(1_800_000_225),
            )
            .unwrap(),
        SearchPublicationRetirementFailureOutcome::HeadBlocked
    );
    let blocked = store.search_projection_state().unwrap();
    assert_eq!(
        blocked.service_state,
        SearchProjectionServiceState::RepairBlocked
    );
    assert_eq!(
        blocked.repair_reason,
        Some(meta_store::SearchRepairReason::RuntimeInvariant)
    );
    assert_eq!(blocked.generation, before.generation);
    assert_eq!(blocked.visible_epoch, before.visible_epoch);
}

#[test]
fn failed_publication_retirement_blocks_the_exact_ready_head() {
    let (_directory, store) = owned_store();
    publish(&store, "retirement-ready-base", None, 0, &[], &[]);
    let before = store.search_projection_state().unwrap();
    let session = prepare_publication(
        &store,
        "retirement-failure-ready",
        before.generation.as_deref(),
        before.visible_epoch,
        resume_classifier::CLASSIFIER_EPOCH,
        &[],
    );
    session
        .begin_search_publication_retirement(
            "retirement-failure-ready",
            now(1_800_000_226),
            SearchPublicationRetirementPlan {
                fulltext: SearchArtifactExpectation::MayExist,
                vector: SearchArtifactExpectation::MayExist,
            },
        )
        .unwrap();

    assert_eq!(
        session
            .block_search_head_after_publication_retirement_failure(
                "retirement-failure-ready",
                now(1_800_000_227),
            )
            .unwrap(),
        SearchPublicationRetirementFailureOutcome::HeadBlocked
    );
    let blocked = store.search_projection_state().unwrap();
    assert_eq!(
        blocked.service_state,
        SearchProjectionServiceState::RepairBlocked
    );
    assert_eq!(
        blocked.repair_reason,
        Some(meta_store::SearchRepairReason::RuntimeInvariant)
    );
    assert_eq!(blocked.generation, before.generation);
    assert_eq!(blocked.visible_epoch, before.visible_epoch);
    assert_eq!(
        session
            .block_search_head_after_publication_retirement_failure(
                "retirement-failure-ready",
                now(1_800_000_228),
            )
            .unwrap(),
        SearchPublicationRetirementFailureOutcome::ExactHeadAlreadyBlocked
    );
    assert_eq!(store.search_projection_state().unwrap(), blocked);
}

#[test]
fn failed_publication_retirement_blocks_the_exact_artifact_repair_head() {
    let (_directory, store) = owned_store();
    publish(&store, "retirement-repair-base", None, 0, &[], &[]);
    let ready = store.search_projection_state().unwrap();
    assert_eq!(
        store
            .begin_artifact_repair(
                ready.generation.as_deref().unwrap(),
                ready.visible_epoch,
                now(1_800_000_228),
            )
            .unwrap(),
        meta_store::SearchProjectionTransitionOutcome::Applied
    );
    let before = store.search_projection_state().unwrap();
    let session = prepare_publication(
        &store,
        "retirement-failure-repair",
        before.generation.as_deref(),
        before.visible_epoch,
        resume_classifier::CLASSIFIER_EPOCH,
        &[],
    );
    session
        .begin_search_publication_retirement(
            "retirement-failure-repair",
            now(1_800_000_229),
            SearchPublicationRetirementPlan {
                fulltext: SearchArtifactExpectation::MayExist,
                vector: SearchArtifactExpectation::MayExist,
            },
        )
        .unwrap();

    assert_eq!(
        session
            .block_search_head_after_publication_retirement_failure(
                "retirement-failure-repair",
                now(1_800_000_230),
            )
            .unwrap(),
        SearchPublicationRetirementFailureOutcome::HeadBlocked
    );
    let blocked = store.search_projection_state().unwrap();
    assert_eq!(
        blocked.service_state,
        SearchProjectionServiceState::RepairBlocked
    );
    assert_eq!(
        blocked.repair_reason,
        Some(meta_store::SearchRepairReason::RuntimeInvariant)
    );
    assert_eq!(blocked.generation, before.generation);
    assert_eq!(blocked.visible_epoch, before.visible_epoch);
    assert!(store.artifact_repair_context().unwrap().is_some());
}

#[test]
fn failed_publication_retirement_cannot_block_a_superseding_head() {
    let (_directory, store) = owned_store();
    publish(&store, "retirement-superseded-base", None, 0, &[], &[]);
    let base = store.search_projection_state().unwrap();
    let session = prepare_publication(
        &store,
        "retirement-failure-stale",
        base.generation.as_deref(),
        base.visible_epoch,
        resume_classifier::CLASSIFIER_EPOCH,
        &[],
    );
    drop(session);
    publish(
        &store,
        "retirement-newer-head",
        base.generation.as_deref(),
        base.visible_epoch,
        &[],
        &[],
    );
    let newer = store.search_projection_state().unwrap();
    let session = store.wait_for_search_publication_session().unwrap();
    session
        .begin_search_publication_retirement(
            "retirement-failure-stale",
            now(1_800_000_231),
            SearchPublicationRetirementPlan {
                fulltext: SearchArtifactExpectation::MayExist,
                vector: SearchArtifactExpectation::MayExist,
            },
        )
        .unwrap();

    assert_eq!(
        session
            .block_search_head_after_publication_retirement_failure(
                "retirement-failure-stale",
                now(1_800_000_232),
            )
            .unwrap(),
        SearchPublicationRetirementFailureOutcome::HeadSuperseded
    );
    assert_eq!(store.search_projection_state().unwrap(), newer);
}

#[test]
fn artifact_retention_uses_only_one_typed_metadata_snapshot() {
    let (_directory, store) = owned_store();
    publish(&store, "retention-ready-1", None, 0, &[], &[]);
    publish(
        &store,
        "retention-ready-2",
        Some("retention-ready-1"),
        1,
        &[],
        &[],
    );
    publish(
        &store,
        "retention-ready-3",
        Some("retention-ready-2"),
        2,
        &[],
        &[],
    );
    let empty_digest = SearchProjectionDigest::from_pairs::<_, &str, &str>([]).unwrap();
    let session = store.wait_for_search_publication_session().unwrap();
    assert_eq!(
        session
            .begin_search_publication(&SearchPublicationDraft {
                generation: "retention-preparing".to_string(),
                base_generation: Some("retention-ready-3".to_string()),
                expected_visible_epoch: 3,
                classifier_epoch: resume_classifier::CLASSIFIER_EPOCH.to_string(),
                projection_digest: empty_digest.clone(),
                now: now(1_800_000_230),
            })
            .unwrap(),
        SearchPublicationOutcome::Applied
    );
    prepare_publication_in_session(
        &session,
        "retention-validated",
        Some("retention-ready-3"),
        3,
        resume_classifier::CLASSIFIER_EPOCH,
        &[],
    );
    assert_eq!(
        session
            .begin_search_publication(&SearchPublicationDraft {
                generation: "retention-abandoned".to_string(),
                base_generation: Some("retention-ready-1".to_string()),
                expected_visible_epoch: 1,
                classifier_epoch: resume_classifier::CLASSIFIER_EPOCH.to_string(),
                projection_digest: empty_digest,
                now: now(1_800_000_231),
            })
            .unwrap(),
        SearchPublicationOutcome::Superseded
    );

    assert_eq!(
        store.search_artifact_retention_generations(1).unwrap(),
        std::collections::BTreeSet::from([
            "retention-preparing".to_string(),
            "retention-ready-3".to_string(),
            "retention-validated".to_string(),
        ])
    );
    assert_eq!(
        store.search_artifact_retention_generations(2).unwrap(),
        std::collections::BTreeSet::from([
            "retention-preparing".to_string(),
            "retention-ready-2".to_string(),
            "retention-ready-3".to_string(),
            "retention-validated".to_string(),
        ])
    );
    for invalid_limit in [0, 257] {
        assert_eq!(
            store
                .search_artifact_retention_generations(invalid_limit)
                .unwrap_err()
                .search_publication_failure(),
            Some(SearchPublicationFailure::InvalidDescriptor)
        );
    }
    session
        .abandon_search_publication("retention-preparing", now(1_800_000_232))
        .unwrap();
    session
        .abandon_search_publication("retention-validated", now(1_800_000_232))
        .unwrap();
}

#[test]
fn candidate_assignment_is_immutable_and_only_active_projection_counts() {
    let (_directory, store) = owned_store();
    let document = document("candidate");
    let revision = revision(&document, b"candidate source");
    let version = version(&document, &revision, "candidate normalized");
    seed_version(&store, &document, &revision, &version);
    let contact = ContactHash::from_keyed_digest("a".repeat(64)).unwrap();
    let candidate = Candidate {
        id: CandidateId::from_non_secret_parts(&["s807", "candidate-a"]),
        primary_name: None,
        phone_hash: None,
        email_hash: Some(contact.clone()),
        dedupe_key: None,
        merge_confidence: Some(1.0),
        version_count: 0,
    };
    store.upsert_candidate(&candidate).unwrap();
    assert_eq!(
        store
            .insert_candidate_assignment(&version.id, &candidate.id)
            .unwrap(),
        IdentityInsertOutcome::Inserted
    );
    assert_eq!(
        store
            .insert_candidate_assignment(&version.id, &candidate.id)
            .unwrap(),
        IdentityInsertOutcome::AlreadyPresent
    );
    let other = Candidate {
        id: CandidateId::from_non_secret_parts(&["s807", "candidate-b"]),
        email_hash: Some(ContactHash::from_keyed_digest("b".repeat(64)).unwrap()),
        ..candidate.clone()
    };
    store.upsert_candidate(&other).unwrap();
    assert_eq!(
        store
            .insert_candidate_assignment(&version.id, &other.id)
            .unwrap_err()
            .class(),
        MetaStoreErrorClass::ImmutableIdentityConflict
    );
    assert!(store
        .searchable_document_ids_with_contact_hashes(std::slice::from_ref(&contact))
        .unwrap()
        .is_empty());
    assert!(store.searchable_document_ids().unwrap().is_empty());
    assert_eq!(store.status_summary().unwrap().searchable_documents, 0);
    let projection = ActiveSearchProjection {
        document_id: document.id.clone(),
        resume_version_id: version.id,
    };
    publish(
        &store,
        "candidate-generation",
        None,
        0,
        std::slice::from_ref(&document),
        std::slice::from_ref(&projection),
    );
    assert_eq!(
        store
            .searchable_document_ids_with_contact_hashes(&[contact])
            .unwrap(),
        vec![document.id.clone()]
    );
    assert_eq!(
        store.searchable_document_ids().unwrap(),
        vec![document.id.clone()]
    );
    let status = store.status_summary().unwrap();
    assert_eq!(status.indexed_documents, 1);
    assert_eq!(status.searchable_documents, 1);
    assert_eq!(
        store
            .candidate_by_id(&candidate.id)
            .unwrap()
            .unwrap()
            .version_count,
        1
    );
}

#[test]
fn publication_seals_version_bound_derived_data() {
    let (_directory, store) = owned_store();
    let document = document("sealed-derived");
    let revision = revision(&document, b"sealed source");
    let version = version(&document, &revision, "sealed normalized");
    seed_version(&store, &document, &revision, &version);
    let first_mention = EntityMention {
        id: EntityMentionId::from_non_secret_parts(&[version.id.as_str(), "first"]),
        resume_version_id: version.id.clone(),
        section_id: None,
        entity_type: EntityType::Skill,
        raw_value: "Rust".to_string(),
        normalized_value: Some("rust".to_string()),
        span_start: Some(0),
        span_end: Some(4),
        confidence: 0.9,
        extractor: "synthetic-v1".to_string(),
    };
    store
        .insert_entity_mentions(&version.id, std::slice::from_ref(&first_mention))
        .unwrap();
    let projection = ActiveSearchProjection {
        document_id: document.id.clone(),
        resume_version_id: version.id.clone(),
    };
    publish(
        &store,
        "sealed-generation",
        None,
        0,
        std::slice::from_ref(&document),
        std::slice::from_ref(&projection),
    );

    assert_eq!(
        store
            .insert_entity_mentions(&version.id, std::slice::from_ref(&first_mention))
            .unwrap(),
        IdentityInsertOutcome::AlreadyPresent
    );
    let mut second_mention = first_mention;
    second_mention.id = EntityMentionId::from_non_secret_parts(&[version.id.as_str(), "second"]);
    assert_eq!(
        store
            .insert_entity_mentions(&version.id, &[second_mention])
            .unwrap_err()
            .class(),
        MetaStoreErrorClass::InvalidTransition
    );

    let candidate = Candidate {
        id: CandidateId::from_non_secret_parts(&["s807", "late-candidate"]),
        primary_name: None,
        phone_hash: None,
        email_hash: None,
        dedupe_key: None,
        merge_confidence: None,
        version_count: 0,
    };
    store.upsert_candidate(&candidate).unwrap();
    assert_eq!(
        store
            .insert_candidate_assignment(&version.id, &candidate.id)
            .unwrap_err()
            .class(),
        MetaStoreErrorClass::InvalidTransition
    );
}

#[test]
fn snapshot_filter_is_generation_bound_bounded_and_returns_exact_pairs() {
    let (_directory, store) = owned_store();
    let rust_document = document("snapshot-filter-rust");
    let rust_revision = revision(&rust_document, b"snapshot filter rust source");
    let rust_version = version(&rust_document, &rust_revision, "Rust systems engineer");
    seed_version(&store, &rust_document, &rust_revision, &rust_version);
    let rust_mention = EntityMention {
        id: EntityMentionId::from_non_secret_parts(&[rust_version.id.as_str(), "rust"]),
        resume_version_id: rust_version.id.clone(),
        section_id: None,
        entity_type: EntityType::Skill,
        raw_value: "Rust".to_string(),
        normalized_value: Some("rust".to_string()),
        span_start: Some(0),
        span_end: Some(4),
        confidence: 0.95,
        extractor: "synthetic-v1".to_string(),
    };
    store
        .insert_entity_mentions(&rust_version.id, &[rust_mention])
        .unwrap();

    let go_document = document("snapshot-filter-go");
    let go_revision = revision(&go_document, b"snapshot filter go source");
    let go_version = version(&go_document, &go_revision, "Go backend engineer");
    seed_version(&store, &go_document, &go_revision, &go_version);
    let go_mention = EntityMention {
        id: EntityMentionId::from_non_secret_parts(&[go_version.id.as_str(), "go"]),
        resume_version_id: go_version.id.clone(),
        section_id: None,
        entity_type: EntityType::Skill,
        raw_value: "Go".to_string(),
        normalized_value: Some("go".to_string()),
        span_start: Some(0),
        span_end: Some(2),
        confidence: 0.95,
        extractor: "synthetic-v1".to_string(),
    };
    store
        .insert_entity_mentions(&go_version.id, &[go_mention])
        .unwrap();

    let rust_projection = ActiveSearchProjection {
        document_id: rust_document.id.clone(),
        resume_version_id: rust_version.id.clone(),
    };
    let go_projection = ActiveSearchProjection {
        document_id: go_document.id.clone(),
        resume_version_id: go_version.id.clone(),
    };
    let mut projections = vec![rust_projection.clone(), go_projection];
    projections.sort_by(|left, right| left.document_id.cmp(&right.document_id));
    publish(
        &store,
        "snapshot-filter-generation",
        None,
        0,
        &[],
        &projections,
    );

    let filter = SearchProjectionFilter::new(vec![SearchProjectionPredicate::EntityValuesAny {
        entity_type: EntityType::Skill,
        normalized_values: vec!["RUST".to_string()],
        min_confidence: 0.75,
        case: SearchFilterCase::AsciiInsensitive,
    }])
    .unwrap();
    store
        .with_search_metadata_snapshot(|snapshot| {
            assert_eq!(
                snapshot
                    .bounded_filter_selection(&filter, NonZeroUsize::new(1).unwrap())
                    .unwrap(),
                BoundedFilterSelection::Selected(vec![rust_projection])
            );
            assert_eq!(
                snapshot
                    .bounded_filter_selection(
                        &SearchProjectionFilter::default(),
                        NonZeroUsize::new(1).unwrap(),
                    )
                    .unwrap(),
                BoundedFilterSelection::TooLarge { cap: 1 }
            );
            assert_eq!(
                snapshot
                    .bounded_filter_selection(
                        &SearchProjectionFilter::default(),
                        NonZeroUsize::new(MAX_BOUNDED_FILTER_SELECTION + 1).unwrap(),
                    )
                    .unwrap_err()
                    .class(),
                MetaStoreErrorClass::InvalidValue
            );
            Ok::<_, ()>(())
        })
        .unwrap();
}

#[test]
fn exact_hit_hydration_preserves_order_and_rejects_non_current_pairs() {
    let (_directory, store) = owned_store();
    let first_document = document("hydrate-exact-first");
    let first_revision = revision(&first_document, b"hydrate exact first");
    let first_version = version(&first_document, &first_revision, "first exact body");
    seed_version(&store, &first_document, &first_revision, &first_version);
    let second_document = document("hydrate-exact-second");
    let second_revision = revision(&second_document, b"hydrate exact second");
    let second_version = version(&second_document, &second_revision, "second exact body");
    seed_version(&store, &second_document, &second_revision, &second_version);
    let first_projection = ActiveSearchProjection {
        document_id: first_document.id.clone(),
        resume_version_id: first_version.id.clone(),
    };
    let second_projection = ActiveSearchProjection {
        document_id: second_document.id.clone(),
        resume_version_id: second_version.id.clone(),
    };
    let mut publication = vec![first_projection.clone(), second_projection.clone()];
    publication.sort_by(|left, right| left.document_id.cmp(&right.document_id));
    publish(
        &store,
        "hydrate-exact-generation",
        None,
        0,
        &[],
        &publication,
    );

    store
        .with_search_metadata_snapshot(|snapshot| {
            let requested = [second_projection.clone(), first_projection.clone()];
            let ExactHitHydration::Hydrated(hydrated) = snapshot
                .hydrate_exact_hits(&requested, NonZeroUsize::new(2).unwrap())
                .unwrap()
            else {
                panic!("exact active identities must hydrate");
            };
            assert_eq!(
                hydrated
                    .iter()
                    .map(|hit| hit.projection.clone())
                    .collect::<Vec<_>>(),
                requested
            );

            let missing = ActiveSearchProjection {
                document_id: first_document.id.clone(),
                resume_version_id: ResumeVersionId::from_non_secret_parts(&[
                    "s807",
                    "missing-exact-version",
                ]),
            };
            let ExactHitHydration::Failed(failure) = snapshot
                .hydrate_exact_hits(&[missing], NonZeroUsize::new(1).unwrap())
                .unwrap()
            else {
                panic!("unknown exact identity must fail closed");
            };
            assert_eq!(failure.position, Some(0));
            assert_eq!(failure.kind, ExactHitHydrationFailureKind::NotFound);
            Ok::<_, ()>(())
        })
        .unwrap();
}

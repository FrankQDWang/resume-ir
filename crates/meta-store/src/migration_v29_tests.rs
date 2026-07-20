use std::{fs, path::Path, str::FromStr};

use rusqlite::{params, TransactionBehavior};
use tempfile::{tempdir, TempDir};

use super::descriptor_validation::{
    legacy_fingerprint, publication_descriptor_records, validate_active_head,
    CURRENT_FULLTEXT_MANIFEST, CURRENT_VECTOR_INDEX, CURRENT_VECTOR_MANIFEST,
    LEGACY_FULLTEXT_INDEX, LEGACY_FULLTEXT_MANIFEST, LEGACY_VECTOR_INDEX, LEGACY_VECTOR_MANIFEST,
};
use super::*;
use crate::active_store_manifest::read_manifest;
use crate::{
    ActiveSearchProjection, ArtifactRepairAttemptAcquire, ArtifactRepairKey, ClassificationStatus,
    ContentDigest, DataDirectoryOwnerAcquisition, DataDirectoryOwnerLease, Document, DocumentId,
    DocumentStatus, FileExtension, FullTextSnapshotDescriptor, ImmutableIngestStage,
    ImportProcessingContract, MetaStoreErrorClass, MetadataEncryptionState, OwnedMetaStore,
    ProjectedDocumentSnapshot, ReadMetaStore, ReasonCode, ResumeVersion,
    ResumeVersionClassification, ResumeVersionId, ReviewDisposition, SearchArtifactExpectation,
    SearchProjectionDigest, SearchProjectionServiceState, SearchProjectionTransitionOutcome,
    SearchPublicationCommit, SearchPublicationDraft, SearchPublicationOutcome,
    SearchPublicationRetirementPhase, SearchPublicationSession, SearchPublicationState,
    SearchPublicationValidation, SearchRepairReason, SourceRevision, TerminalDocumentUpdate,
    UnixTimestamp, VectorSnapshotDescriptor, CLASSIFIER_EPOCH,
};

#[derive(Clone, Copy, Debug)]
enum FixtureHead {
    Ready,
    Repairing,
    Blocked,
}

#[derive(Clone, Copy, Debug)]
enum FixtureDescriptor {
    Legacy,
    Current,
}

#[test]
fn optional_read_open_distinguishes_absent_storage_from_legacy_storage() {
    let empty = tempdir().unwrap();
    let absent = empty.path().join("not-created");
    assert!(ReadMetaStore::open_data_dir_if_published(&absent)
        .unwrap()
        .is_none());
    assert!(!absent.exists());
    assert_eq!(fs::read_dir(empty.path()).unwrap().count(), 0);

    let legacy = seed_v28_fixture(FixtureDescriptor::Legacy, FixtureHead::Ready);
    let source_manifest = read_manifest(&legacy.data_dir().join(MANIFEST_FILE)).unwrap();
    let Err(error) = ReadMetaStore::open_data_dir_if_published(legacy.data_dir()) else {
        panic!("legacy metadata must require the copy-on-write owner");
    };
    assert_eq!(
        error.class(),
        MetaStoreErrorClass::MigrationOwnershipRequired
    );
    assert_eq!(
        read_manifest(&legacy.data_dir().join(MANIFEST_FILE)).unwrap(),
        source_manifest
    );
}

struct V28Fixture {
    _directory: TempDir,
    owner: DataDirectoryOwnerLease,
    key: [u8; METADATA_ENCRYPTION_KEY_LEN],
    source_manifest: ActiveStoreManifest,
    projection: ActiveSearchProjection,
}

impl V28Fixture {
    fn data_dir(&self) -> &Path {
        self.owner.canonical_data_dir()
    }

    fn source_path(&self) -> std::path::PathBuf {
        self.data_dir().join(&self.source_manifest.file_name)
    }

    fn target_path(&self) -> std::path::PathBuf {
        self.data_dir().join(format!(
            "metadata-v29-{}.sqlite3",
            &self.source_manifest.store_id_digest[..16]
        ))
    }
}

#[test]
fn descriptor_contract_rejects_mixed_versions() {
    assert!(descriptor_contract(
        Some(LEGACY_FULLTEXT_MANIFEST),
        Some(LEGACY_FULLTEXT_INDEX),
        Some(CURRENT_VECTOR_MANIFEST),
        Some(CURRENT_VECTOR_INDEX),
    )
    .is_err());
}

#[test]
fn legacy_fixture_fingerprint_has_a_fixed_known_answer() {
    let fixture = seed_v28_fixture(FixtureDescriptor::Legacy, FixtureHead::Ready);
    let connection = open_encrypted_connection(&fixture.source_path(), &fixture.key).unwrap();
    let fingerprint = connection
        .query_row(
            "SELECT publication_fingerprint FROM search_publication_journal
             WHERE generation = 'v28-legacy-generation'",
            [],
            |row| row.get::<_, String>(0),
        )
        .unwrap();
    assert_eq!(
        fingerprint,
        "sha256:0393f1d2daece59f92547b08e5726a2976985a7364d423eadc2e4ee81997896c"
    );
}

#[test]
fn legacy_ready_repairing_and_blocked_heads_migrate_without_faking_current_descriptors() {
    for head in [
        FixtureHead::Ready,
        FixtureHead::Repairing,
        FixtureHead::Blocked,
    ] {
        let fixture = seed_v28_fixture(FixtureDescriptor::Legacy, head);
        let store = fixture.owner.open_store().unwrap();

        assert_eq!(store.schema_version().unwrap(), schema_v29::VERSION);
        let state = store.search_projection_state().unwrap();
        let expected_state = if matches!(head, FixtureHead::Blocked) {
            SearchProjectionServiceState::RepairBlocked
        } else {
            SearchProjectionServiceState::Repairing
        };
        assert_eq!(state.service_state, expected_state, "head={head:?}");
        assert_eq!(
            state.repair_reason,
            Some(if matches!(head, FixtureHead::Blocked) {
                SearchRepairReason::RuntimeInvariant
            } else {
                SearchRepairReason::ArtifactUnavailable
            })
        );
        assert_eq!(state.generation.as_deref(), Some("v28-legacy-generation"));
        assert_eq!(state.visible_epoch, 1);
        assert!(state.publication.is_none());

        let context = store.artifact_repair_context().unwrap().unwrap();
        assert_eq!(context.generation, "v28-legacy-generation");
        assert_eq!(context.visible_epoch, 1);
        assert_eq!(context.projection_count, 1);
        assert_eq!(
            context.projection_digest,
            projection_digest(std::slice::from_ref(&fixture.projection))
        );
        assert_eq!(
            store
                .active_search_projection_for_document(&fixture.projection.document_id)
                .unwrap(),
            Some(fixture.projection.clone())
        );
        if matches!(head, FixtureHead::Blocked) {
            assert_eq!(
                store
                    .begin_artifact_repair("v28-legacy-generation", 1, timestamp(30),)
                    .unwrap(),
                SearchProjectionTransitionOutcome::Superseded
            );
            assert_eq!(
                store.search_projection_state().unwrap().service_state,
                SearchProjectionServiceState::RepairBlocked
            );
        } else {
            assert_eq!(
                store
                    .begin_artifact_repair("v28-legacy-generation", 1, timestamp(30))
                    .unwrap(),
                SearchProjectionTransitionOutcome::Applied
            );
        }

        let isolated = store
            .connection
            .borrow()
            .query_row(
                "SELECT state, publication_fingerprint, fulltext_manifest_schema,
                        vector_manifest_schema
                 FROM search_publication_journal WHERE generation = ?1",
                params!["v28-legacy-generation"],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, Option<String>>(2)?,
                        row.get::<_, Option<String>>(3)?,
                    ))
                },
            )
            .unwrap();
        assert_eq!(isolated, ("abandoned".to_string(), None, None, None));
        assert!(!fixture.source_path().exists());
    }
}

#[test]
fn current_v28_in_progress_head_gets_typed_context_without_descriptor_isolation() {
    let fixture = seed_v28_fixture(FixtureDescriptor::Current, FixtureHead::Repairing);
    let store = fixture.owner.open_store().unwrap();

    let state = store.search_projection_state().unwrap();
    assert_eq!(state.service_state, SearchProjectionServiceState::Repairing);
    assert_eq!(
        state.repair_reason,
        Some(SearchRepairReason::ArtifactUnavailable)
    );
    assert!(state.publication.is_none());
    assert!(store.artifact_repair_context().unwrap().is_some());
    let persisted = store
        .connection
        .borrow()
        .query_row(
            "SELECT state, fulltext_manifest_schema, vector_manifest_schema
             FROM search_publication_journal WHERE generation = ?1",
            params!["v28-current-generation"],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            },
        )
        .unwrap();
    assert_eq!(
        persisted,
        (
            "ready".to_string(),
            CURRENT_FULLTEXT_MANIFEST.to_string(),
            CURRENT_VECTOR_MANIFEST.to_string(),
        )
    );
}

#[test]
fn interrupted_v28_publications_migrate_to_pending_exact_retirement() {
    for validated in [false, true] {
        let fixture = seed_v28_fixture(FixtureDescriptor::Current, FixtureHead::Ready);
        let connection = open_encrypted_connection(&fixture.source_path(), &fixture.key).unwrap();
        let store = OwnedMetaStore::from_owned_connection(
            connection,
            MetadataEncryptionState::SqlCipher,
            fixture.owner.shared_guard(),
        )
        .unwrap();
        let generation = if validated {
            "v28-interrupted-validated"
        } else {
            "v28-interrupted-preparing"
        };
        let digest = projection_digest(std::slice::from_ref(&fixture.projection));
        let session = store
            .into_search_publication_session_without_prepare_for_test()
            .unwrap();
        assert_eq!(
            session
                .begin_legacy_v28_search_publication_for_test(&SearchPublicationDraft {
                    generation: generation.to_string(),
                    base_generation: Some("v28-current-generation".to_string()),
                    expected_visible_epoch: 1,
                    classifier_epoch: CLASSIFIER_EPOCH.to_string(),
                    projection_digest: digest.clone(),
                    now: timestamp(23),
                })
                .unwrap(),
            SearchPublicationOutcome::Applied
        );
        if validated {
            let fulltext = FullTextSnapshotDescriptor::new(
                generation.to_string(),
                1,
                digest.clone(),
                ContentDigest::from_bytes(b"v28-interrupted-fulltext"),
            );
            let vector = VectorSnapshotDescriptor::disabled(
                generation.to_string(),
                1,
                digest,
                SearchProjectionDigest::from_pairs::<_, &str, &str>([]).unwrap(),
                ContentDigest::from_bytes(b"v28-interrupted-vector"),
            );
            session
                .validate_search_publication(&SearchPublicationValidation {
                    generation,
                    fulltext: &fulltext,
                    vector: &vector,
                    now: timestamp(24),
                })
                .unwrap();
        }
        drop(session);

        let store = fixture.owner.open_store().unwrap();
        assert_eq!(
            store.search_publication(generation).unwrap().unwrap().state,
            SearchPublicationState::Abandoned
        );
        let retirement = store
            .search_publication_retirement(generation)
            .unwrap()
            .unwrap();
        assert_eq!(retirement.phase, SearchPublicationRetirementPhase::Pending);
        assert_eq!(
            retirement.plan.fulltext,
            SearchArtifactExpectation::MayExist
        );
        assert_eq!(retirement.plan.vector, SearchArtifactExpectation::MayExist);
        assert!(!retirement.fulltext_complete);
        assert!(!retirement.vector_complete);
    }
}

#[test]
fn migrated_legacy_context_can_publish_a_current_successor_and_clear_repair_state() {
    let fixture = seed_v28_fixture(FixtureDescriptor::Legacy, FixtureHead::Ready);
    let store = fixture.owner.open_store().unwrap();
    let repairing = store.search_projection_state().unwrap();
    assert_eq!(
        repairing.service_state,
        SearchProjectionServiceState::Repairing
    );
    assert!(store.artifact_repair_context().unwrap().is_some());

    let context = store.artifact_repair_context().unwrap().unwrap();
    let key = ArtifactRepairKey::new(
        context.generation,
        context.publication_fingerprint,
        context.visible_epoch,
    );
    let mut session = store.wait_for_search_publication_session().unwrap();
    assert!(matches!(
        session
            .acquire_artifact_repair_attempt(&key, timestamp(39))
            .unwrap(),
        ArtifactRepairAttemptAcquire::Started(_)
    ));
    publish_successor_projection(
        &session,
        "v29-rebuilt-generation",
        repairing.generation.as_deref().unwrap(),
        repairing.visible_epoch,
        std::slice::from_ref(&fixture.projection),
    );
    drop(session);

    let ready = store.search_projection_state().unwrap();
    assert_eq!(ready.service_state, SearchProjectionServiceState::Ready);
    assert_eq!(ready.generation.as_deref(), Some("v29-rebuilt-generation"));
    assert_eq!(ready.visible_epoch, 2);
    assert!(ready.publication.is_some());
    assert_eq!(store.artifact_repair_context().unwrap(), None);
    assert_eq!(store.artifact_repair_attempt_state().unwrap(), None);
    assert_eq!(
        store
            .active_search_projection_for_document(&fixture.projection.document_id)
            .unwrap(),
        Some(fixture.projection)
    );
}

#[test]
fn pre_manifest_target_is_rebuilt_from_the_latest_source_and_post_manifest_predecessor_is_cleaned()
{
    let fixture = seed_v28_fixture(FixtureDescriptor::Legacy, FixtureHead::Ready);
    migrate_with_failpoint(&fixture, MigrationFailpoint::AfterTargetValidation).unwrap_err();

    assert_eq!(
        read_manifest(&fixture.data_dir().join(MANIFEST_FILE)).unwrap(),
        fixture.source_manifest
    );
    assert_eq!(
        active_store_path(fixture.data_dir()).unwrap(),
        fixture.source_path()
    );
    assert!(fixture.target_path().exists());
    migration_v28::validate_current_v28_store(
        &fixture.source_path(),
        &fixture.key,
        &fixture.source_manifest.store_id_digest,
    )
    .unwrap();
    assert!(ReadMetaStore::open_data_dir(fixture.data_dir()).is_err());

    let (document, revision, version, classification) = classified_fixture_named("after-crash");
    let source_store = OwnedMetaStore::from_owned_connection(
        open_encrypted_connection(&fixture.source_path(), &fixture.key).unwrap(),
        MetadataEncryptionState::SqlCipher,
        fixture.owner.shared_guard(),
    )
    .unwrap();
    source_store
        .stage_immutable_ingest(ImmutableIngestStage::ClassifiedResume {
            document: &document,
            source_revision: &revision,
            version: &version,
            classification: &classification,
            mentions: &[],
            email_hash: None,
            phone_hash: None,
        })
        .unwrap();
    drop(source_store);

    migrate_with_failpoint(&fixture, MigrationFailpoint::None).unwrap();
    let active = read_manifest(&fixture.data_dir().join(MANIFEST_FILE)).unwrap();
    assert_eq!(active.schema_version, schema_v29::VERSION);
    assert_eq!(
        active_store_path(fixture.data_dir()).unwrap(),
        fixture.target_path()
    );
    assert!(!fixture.source_path().exists());
    let migrated = ReadMetaStore::open_data_dir(fixture.data_dir()).unwrap();
    assert_eq!(
        migrated.resume_version_by_id(&version.id).unwrap(),
        Some(version)
    );

    let fixture = seed_v28_fixture(FixtureDescriptor::Legacy, FixtureHead::Ready);
    migrate_with_failpoint(&fixture, MigrationFailpoint::AfterManifest).unwrap_err();
    let active = read_manifest(&fixture.data_dir().join(MANIFEST_FILE)).unwrap();
    assert_eq!(active.schema_version, schema_v29::VERSION);
    assert!(fixture.source_path().exists());
    fixture.owner.open_store().unwrap();
    assert!(!fixture.source_path().exists());
}

#[test]
fn invalid_orphan_target_is_removed_and_rebuilt_from_the_active_v28_source() {
    let fixture = seed_v28_fixture(FixtureDescriptor::Legacy, FixtureHead::Ready);
    fs::write(fixture.target_path(), b"not a sqlite store").unwrap();
    restrict_private_file_permissions(&fixture.target_path()).unwrap();
    let journal = sidecar_path(&fixture.target_path(), "-journal");
    let wal = sidecar_path(&fixture.target_path(), "-wal");
    let shm = sidecar_path(&fixture.target_path(), "-shm");
    for sidecar in [&journal, &wal, &shm] {
        fs::write(sidecar, b"invalid orphan sidecar").unwrap();
        restrict_private_file_permissions(sidecar).unwrap();
    }

    migrate_with_failpoint(&fixture, MigrationFailpoint::None).unwrap();

    let active = read_manifest(&fixture.data_dir().join(MANIFEST_FILE)).unwrap();
    assert_eq!(active.schema_version, schema_v29::VERSION);
    assert_eq!(
        active.file_name,
        fixture.target_path().file_name().unwrap().to_string_lossy()
    );
    let store = fixture.owner.open_store().unwrap();
    assert_eq!(store.schema_version().unwrap(), schema_v29::VERSION);
    assert!(!journal.exists());
    assert!(!wal.exists());
    assert!(!shm.exists());
    assert_eq!(
        store
            .active_search_projection_for_document(&fixture.projection.document_id)
            .unwrap(),
        Some(fixture.projection)
    );
}

#[test]
fn legacy_fingerprint_epoch_projection_and_classification_corruption_fail_closed() {
    for corruption in [
        Corruption::Fingerprint,
        Corruption::Epoch,
        Corruption::ProjectionCount,
        Corruption::ProjectionDigest,
        Corruption::Classification,
        Corruption::MixedDescriptor,
    ] {
        let fixture = seed_v28_fixture(FixtureDescriptor::Legacy, FixtureHead::Ready);
        corrupt_v28_source(&fixture, corruption);

        migrate_with_failpoint(&fixture, MigrationFailpoint::None).unwrap_err();

        assert_eq!(
            read_manifest(&fixture.data_dir().join(MANIFEST_FILE)).unwrap(),
            fixture.source_manifest,
            "corruption={corruption:?}"
        );
        assert!(fixture.source_path().exists(), "corruption={corruption:?}");
    }
}

#[test]
fn current_repairing_fingerprint_tamper_fails_closed_before_context_binding() {
    let fixture = seed_v28_fixture(FixtureDescriptor::Current, FixtureHead::Repairing);
    corrupt_v28_source(&fixture, Corruption::Fingerprint);

    migrate_with_failpoint(&fixture, MigrationFailpoint::None).unwrap_err();

    assert_eq!(
        read_manifest(&fixture.data_dir().join(MANIFEST_FILE)).unwrap(),
        fixture.source_manifest
    );
    assert!(fixture.source_path().exists());
}

#[test]
fn reopened_v29_context_revalidates_exact_source_projection_semantics() {
    let fixture = seed_v28_fixture(FixtureDescriptor::Legacy, FixtureHead::Ready);
    let store = fixture.owner.open_store().unwrap();
    assert!(store.artifact_repair_context().unwrap().is_some());
    {
        let connection = store.connection.borrow();
        let restore =
            trigger_restore_sql(&connection, &["active_search_projection_immutable_update"]);
        connection
            .execute_batch("DROP TRIGGER active_search_projection_immutable_update;")
            .unwrap();
        connection
            .execute(
                "UPDATE active_search_projection SET content_hash = ?1",
                params![ContentDigest::from_bytes(b"tampered active snapshot").as_str()],
            )
            .unwrap();
        connection.execute_batch(&restore).unwrap();
    }
    drop(store);

    assert!(fixture.owner.open_store().is_err());
    assert_eq!(
        read_manifest(&fixture.data_dir().join(MANIFEST_FILE))
            .unwrap()
            .schema_version,
        schema_v29::VERSION
    );
}

#[derive(Clone, Copy, Debug)]
enum Corruption {
    Fingerprint,
    Epoch,
    ProjectionCount,
    ProjectionDigest,
    Classification,
    MixedDescriptor,
}

fn seed_v28_fixture(descriptor: FixtureDescriptor, head: FixtureHead) -> V28Fixture {
    let directory = tempdir().unwrap();
    let data_dir = directory.path().join("data");
    let owner = acquire_owner(&data_dir);
    let owner_guard = owner.shared_guard();
    let (source_path, key) = migration_v28::prepare_active_v28_store(&owner_guard).unwrap();
    let source_manifest = read_manifest(&data_dir.join(MANIFEST_FILE)).unwrap();
    let connection = open_encrypted_connection(&source_path, &key).unwrap();
    let store = OwnedMetaStore::from_owned_connection(
        connection,
        MetadataEncryptionState::SqlCipher,
        owner_guard,
    )
    .unwrap();
    assert_eq!(store.schema_version().unwrap(), schema_v28::VERSION);

    let (document, revision, version, classification) = classified_fixture();
    store
        .stage_immutable_ingest(ImmutableIngestStage::ClassifiedResume {
            document: &document,
            source_revision: &revision,
            version: &version,
            classification: &classification,
            mentions: &[],
            email_hash: None,
            phone_hash: None,
        })
        .unwrap();
    let contract = ImportProcessingContract::new(
        "v29-fixture-parser",
        "v29-fixture-ocr",
        "schema-v28",
        CLASSIFIER_EPOCH,
    )
    .unwrap();
    store
        .activate_migration_rebuild_contract(&contract, timestamp(10))
        .unwrap();
    let barrier = store
        .acquire_migration_rebuild_barrier_token(contract.id())
        .unwrap()
        .unwrap();
    let projection = ActiveSearchProjection {
        document_id: document.id.clone(),
        resume_version_id: version.id,
    };
    let mut session = store
        .into_search_publication_session_without_prepare_for_test()
        .unwrap();
    let _attempt = match session
        .acquire_migration_rebuild_publication_attempt(&barrier, timestamp(20))
        .unwrap()
    {
        crate::MigrationRebuildPublicationAttemptAcquire::Started(attempt) => attempt,
        other => panic!("expected migration attempt, got {other:?}"),
    };
    let generation = match descriptor {
        FixtureDescriptor::Legacy => "v28-legacy-generation",
        FixtureDescriptor::Current => "v28-current-generation",
    };
    publish_initial_projection(
        &session,
        generation,
        &document,
        std::slice::from_ref(&projection),
        &barrier,
    );
    set_descriptor_and_head(&session, generation, descriptor, head);
    drop(session);

    V28Fixture {
        _directory: directory,
        owner,
        key,
        source_manifest,
        projection,
    }
}

fn classified_fixture() -> (
    Document,
    SourceRevision,
    ResumeVersion,
    ResumeVersionClassification,
) {
    classified_fixture_named("document")
}

fn classified_fixture_named(
    label: &str,
) -> (
    Document,
    SourceRevision,
    ResumeVersion,
    ResumeVersionClassification,
) {
    let source = if label == "document" {
        "synthetic v28 legacy resume".to_string()
    } else {
        format!("synthetic v28 legacy resume {label}")
    };
    let now = timestamp(1);
    let mut document = Document {
        id: DocumentId::from_non_secret_parts(&["v29-cow", label]),
        source_uri: format!("synthetic://v29-cow/{label}"),
        normalized_path: format!("synthetic/v29-cow/{label}.txt"),
        file_name: format!("{label}.txt"),
        extension: FileExtension::Txt,
        byte_size: source.len() as u64,
        mtime: now,
        content_hash: None,
        text_hash: None,
        is_deleted: false,
        created_at: now,
        updated_at: now,
        status: DocumentStatus::FieldsExtracted,
    };
    let revision = SourceRevision::for_content(
        document.id.clone(),
        ContentDigest::from_bytes(source.as_bytes()),
        source.len() as u64,
    );
    document.content_hash = Some(revision.content_hash.as_str().to_string());
    let clean_text = if label == "document" {
        "synthetic normalized v28 resume".to_string()
    } else {
        format!("synthetic normalized v28 resume {label}")
    };
    let normalized_text_hash = ContentDigest::from_bytes(clean_text.as_bytes());
    let version = ResumeVersion {
        id: ResumeVersionId::from_content_identity(
            &document.id,
            &revision.id,
            &normalized_text_hash,
            "v29-fixture-parser",
            "schema-v28",
        ),
        document_id: document.id.clone(),
        source_revision_id: revision.id.clone(),
        normalized_text_hash,
        parse_version: "v29-fixture-parser".to_string(),
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
        classified_at: timestamp(2),
        review_disposition: ReviewDisposition::NotRequired,
    };
    (document, revision, version, classification)
}

fn publish_initial_projection(
    session: &SearchPublicationSession,
    generation: &str,
    document: &Document,
    projections: &[ActiveSearchProjection],
    barrier: &crate::MigrationRebuildBarrierToken,
) {
    let digest = projection_digest(projections);
    session
        .begin_legacy_v28_search_publication_for_test(&SearchPublicationDraft {
            generation: generation.to_string(),
            base_generation: None,
            expected_visible_epoch: 0,
            classifier_epoch: CLASSIFIER_EPOCH.to_string(),
            projection_digest: digest.clone(),
            now: timestamp(20),
        })
        .unwrap();
    let fulltext = FullTextSnapshotDescriptor::new(
        generation.to_string(),
        projections.len() as u64,
        digest.clone(),
        ContentDigest::from_bytes(b"v28-fulltext-logical"),
    );
    let vector = VectorSnapshotDescriptor::disabled(
        generation.to_string(),
        projections.len() as u64,
        digest,
        SearchProjectionDigest::from_pairs::<_, &str, &str>([]).unwrap(),
        ContentDigest::from_bytes(b"v28-vector-logical"),
    );
    session
        .validate_search_publication(&SearchPublicationValidation {
            generation,
            fulltext: &fulltext,
            vector: &vector,
            now: timestamp(21),
        })
        .unwrap();
    let terminal = [TerminalDocumentUpdate {
        document_id: document.id.clone(),
        expected_status: DocumentStatus::FieldsExtracted,
        expected_is_deleted: false,
        expected_content_hash: ContentDigest::from_str(document.content_hash.as_deref().unwrap())
            .unwrap(),
        terminal_status: DocumentStatus::Searchable,
        terminal_is_deleted: false,
    }];
    let mut searchable = document.clone();
    searchable.status = DocumentStatus::Searchable;
    searchable.updated_at = timestamp(22);
    let projected = [ProjectedDocumentSnapshot::Replacement {
        projection: projections[0].clone(),
        document: searchable,
    }];
    assert_eq!(
        session
            .commit_migration_rebuild_search_publication(
                &SearchPublicationCommit {
                    generation,
                    terminal_documents: &terminal,
                    projections,
                    projected_documents: &projected,
                    vector_coverage: &[],
                    now: timestamp(22),
                },
                barrier,
            )
            .unwrap(),
        SearchPublicationOutcome::Applied
    );
}

fn publish_successor_projection(
    session: &SearchPublicationSession,
    generation: &str,
    base_generation: &str,
    expected_visible_epoch: u64,
    projections: &[ActiveSearchProjection],
) {
    let digest = projection_digest(projections);
    assert_eq!(
        session
            .begin_search_publication(&SearchPublicationDraft {
                generation: generation.to_string(),
                base_generation: Some(base_generation.to_string()),
                expected_visible_epoch,
                classifier_epoch: CLASSIFIER_EPOCH.to_string(),
                projection_digest: digest.clone(),
                now: timestamp(40),
            })
            .unwrap(),
        SearchPublicationOutcome::Applied
    );
    let fulltext = FullTextSnapshotDescriptor::new(
        generation.to_string(),
        projections.len() as u64,
        digest.clone(),
        ContentDigest::from_bytes(b"v29-rebuilt-fulltext"),
    );
    let vector = VectorSnapshotDescriptor::disabled(
        generation.to_string(),
        projections.len() as u64,
        digest,
        SearchProjectionDigest::from_pairs::<_, &str, &str>([]).unwrap(),
        ContentDigest::from_bytes(b"v29-rebuilt-vector"),
    );
    session
        .validate_search_publication(&SearchPublicationValidation {
            generation,
            fulltext: &fulltext,
            vector: &vector,
            now: timestamp(41),
        })
        .unwrap();
    let projected = projections
        .iter()
        .cloned()
        .map(|projection| ProjectedDocumentSnapshot::RetainedUnchanged { projection })
        .collect::<Vec<_>>();
    assert_eq!(
        session
            .commit_search_publication(&SearchPublicationCommit {
                generation,
                terminal_documents: &[],
                projections,
                projected_documents: &projected,
                vector_coverage: &[],
                now: timestamp(42),
            })
            .unwrap(),
        SearchPublicationOutcome::Applied
    );
}

fn set_descriptor_and_head(
    session: &SearchPublicationSession,
    generation: &str,
    descriptor: FixtureDescriptor,
    head: FixtureHead,
) {
    let store = session.owned_store();
    let mut connection = store.connection.borrow_mut();
    if matches!(descriptor, FixtureDescriptor::Legacy) {
        let records = publication_descriptor_records(&connection).unwrap();
        let publication = records.first().unwrap();
        let fingerprint = legacy_fingerprint(publication).unwrap();
        let restore_triggers = publication_trigger_restore_sql(&connection);
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .unwrap();
        transaction
            .execute_batch(schema_v29::DROP_LEGACY_ISOLATION_TRIGGERS)
            .unwrap();
        transaction
            .execute(
                "UPDATE search_publication_journal
                 SET publication_fingerprint = ?1,
                     fulltext_manifest_schema = ?2, fulltext_index_schema = ?3,
                     vector_manifest_schema = ?4, vector_index_schema = ?5
                 WHERE generation = ?6",
                params![
                    fingerprint.as_str(),
                    LEGACY_FULLTEXT_MANIFEST,
                    LEGACY_FULLTEXT_INDEX,
                    LEGACY_VECTOR_MANIFEST,
                    LEGACY_VECTOR_INDEX,
                    generation,
                ],
            )
            .unwrap();
        transaction.execute_batch(&restore_triggers).unwrap();
        transaction.commit().unwrap();
    }
    let (service_state, repair_reason) = match head {
        FixtureHead::Ready => ("ready", None),
        FixtureHead::Repairing => ("repairing", Some("artifact_unavailable")),
        FixtureHead::Blocked => ("repair_blocked", Some("runtime_invariant")),
    };
    connection
        .execute(
            "UPDATE search_projection_state
             SET service_state = ?1, repair_reason = ?2
             WHERE state_key = 'default' AND generation = ?3 AND visible_epoch = 1",
            params![service_state, repair_reason, generation],
        )
        .unwrap();
    let records = publication_descriptor_records(&connection).unwrap();
    let raw_head = connection
        .query_row(
            "SELECT service_state, generation, visible_epoch, repair_reason
             FROM search_projection_state WHERE state_key = 'default'",
            [],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, Option<String>>(3)?,
                ))
            },
        )
        .unwrap();
    validate_active_head(&connection, &raw_head, records.first().unwrap()).unwrap();
}

fn publication_trigger_restore_sql(connection: &rusqlite::Connection) -> String {
    let mut statement = connection
        .prepare(
            "SELECT sql FROM sqlite_master
             WHERE type = 'trigger' AND name IN (
                 'search_publication_payload_immutable_after_validation',
                 'search_publication_same_state_immutable',
                 'search_publication_transition',
                 'ready_search_publication_immutable_update'
             ) ORDER BY name",
        )
        .unwrap();
    statement
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .collect::<std::result::Result<Vec<_>, _>>()
        .unwrap()
        .into_iter()
        .map(|sql| format!("{sql};\n"))
        .collect()
}

fn corrupt_v28_source(fixture: &V28Fixture, corruption: Corruption) {
    let mut connection = open_encrypted_connection(&fixture.source_path(), &fixture.key).unwrap();
    match corruption {
        Corruption::Fingerprint => mutate_publication(&mut connection, |transaction| {
            transaction.execute(
                "UPDATE search_publication_journal SET publication_fingerprint = ?1",
                params![ContentDigest::from_bytes(b"tampered-fingerprint").as_str()],
            )
        }),
        Corruption::Epoch => {
            let restore_triggers = trigger_restore_sql(
                &connection,
                &[
                    "ready_projection_head_matches_journal",
                    "search_projection_head_change_requires_commit_guard",
                ],
            );
            let transaction = connection
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .unwrap();
            transaction
                .execute_batch(
                    "DROP TRIGGER ready_projection_head_matches_journal;
                     DROP TRIGGER search_projection_head_change_requires_commit_guard;",
                )
                .unwrap();
            transaction
                .execute(
                    "UPDATE search_projection_state SET visible_epoch = 2
                     WHERE state_key = 'default'",
                    [],
                )
                .unwrap();
            transaction.execute_batch(&restore_triggers).unwrap();
            transaction.commit().unwrap();
        }
        Corruption::ProjectionCount => mutate_publication(&mut connection, |transaction| {
            transaction.execute(
                "UPDATE search_publication_journal
                 SET fulltext_document_count = 2, vector_projection_count = 2",
                [],
            )
        }),
        Corruption::ProjectionDigest => mutate_publication(&mut connection, |transaction| {
            let document = DocumentId::from_non_secret_parts(&["v29-cow", "other-document"]);
            let version = ResumeVersionId::from_non_secret_parts(&["v29-cow", "other-version"]);
            let digest =
                SearchProjectionDigest::from_pairs([(document.as_str(), version.as_str())])
                    .unwrap();
            transaction.execute(
                "UPDATE search_publication_journal
                 SET projection_digest = ?1, fulltext_projection_digest = ?1,
                     vector_projection_digest = ?1",
                params![digest.as_str()],
            )
        }),
        Corruption::Classification => {
            connection
                .execute("DELETE FROM resume_version_classification", [])
                .unwrap();
        }
        Corruption::MixedDescriptor => mutate_publication(&mut connection, |transaction| {
            transaction.execute(
                "UPDATE search_publication_journal
                 SET vector_manifest_schema = ?1, vector_index_schema = ?2",
                params![CURRENT_VECTOR_MANIFEST, CURRENT_VECTOR_INDEX],
            )
        }),
    }
}

fn mutate_publication(
    connection: &mut rusqlite::Connection,
    mutate: impl FnOnce(&rusqlite::Transaction<'_>) -> rusqlite::Result<usize>,
) {
    let restore_triggers = publication_trigger_restore_sql(connection);
    let restore_static = trigger_restore_sql(
        connection,
        &["search_publication_static_identity_immutable"],
    );
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .unwrap();
    transaction
        .execute_batch(schema_v29::DROP_LEGACY_ISOLATION_TRIGGERS)
        .unwrap();
    transaction
        .execute_batch("DROP TRIGGER search_publication_static_identity_immutable;")
        .unwrap();
    mutate(&transaction).unwrap();
    transaction.execute_batch(&restore_triggers).unwrap();
    transaction.execute_batch(&restore_static).unwrap();
    transaction.commit().unwrap();
}

fn trigger_restore_sql(connection: &rusqlite::Connection, names: &[&str]) -> String {
    names
        .iter()
        .map(|name| {
            connection
                .query_row(
                    "SELECT sql FROM sqlite_master WHERE type = 'trigger' AND name = ?1",
                    params![name],
                    |row| row.get::<_, String>(0),
                )
                .unwrap()
        })
        .map(|sql| format!("{sql};\n"))
        .collect()
}

fn migrate_with_failpoint(fixture: &V28Fixture, failpoint: MigrationFailpoint) -> Result<PathBuf> {
    with_migration_lock(fixture.data_dir(), || {
        ensure_active_v29_store_locked(&fixture.owner.shared_guard(), &fixture.key, failpoint)
    })
}

fn acquire_owner(data_dir: &Path) -> DataDirectoryOwnerLease {
    match DataDirectoryOwnerLease::try_acquire(data_dir).unwrap() {
        DataDirectoryOwnerAcquisition::Acquired(owner) => owner,
        DataDirectoryOwnerAcquisition::Contended => panic!("synthetic data dir is owned"),
    }
}

fn projection_digest(projections: &[ActiveSearchProjection]) -> SearchProjectionDigest {
    SearchProjectionDigest::from_pairs(projections.iter().map(|projection| {
        (
            projection.document_id.as_str(),
            projection.resume_version_id.as_str(),
        )
    }))
    .unwrap()
}

fn timestamp(seconds: i64) -> UnixTimestamp {
    UnixTimestamp::from_unix_seconds(seconds)
}

fn sidecar_path(path: &Path, suffix: &str) -> std::path::PathBuf {
    let mut value = path.as_os_str().to_owned();
    value.push(suffix);
    value.into()
}

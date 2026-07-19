//! Synthetic-only fixture builders for cross-crate migration integration tests.
//!
//! This module is compiled only through the non-default
//! `migration-test-support` feature. It is not a legacy reader or a production
//! compatibility surface.

#![cfg(feature = "migration-test-support")]

use std::{
    fmt, fs,
    path::{Component, Path},
};

use rusqlite::{params, Connection, TransactionBehavior};

use crate::{
    active_store_manifest::{publish_new_active_store, ActiveStoreManifest},
    apply_sqlcipher_key, load_or_create_metadata_encryption_key,
    migration_v27::{create_private_empty_file, sync_validated_store},
    schema_v27, ActiveSearchProjection, ClassificationStatus, ContentDigest,
    DataDirectoryOwnerAcquisition, DataDirectoryOwnerLease, Document, DocumentId, DocumentStatus,
    FileExtension, ImportProcessingContract, ImportProcessingContractId, ImportRootKind,
    ImportScanProfile, ImportScanScope, ImportSourceDispositionKind, ImportTask, ImportTaskId,
    ImportTaskSourceDisposition, ImportTaskStatus, MetaStoreError, MetadataEncryptionState,
    MigrationRebuildContractActivation, OwnedMetaStore, ReadMetaStore, ReasonCode, Result,
    ResumeVersion, ResumeVersionClassification, ResumeVersionId, ReviewDisposition,
    SearchProjectionTransitionOutcome, SearchRepairReason, SourceRevision, SourceRevisionId,
    UnixTimestamp,
};

const STORE_ID_DIGEST: &str = "7171717171717171717171717171717171717171717171717171717171717171";
const STORE_FILE: &str = "metadata-v27-7171717171717171.sqlite3";
const MAX_SYNTHETIC_ROOT_BYTES: usize = 4_096;
const SYNTHETIC_ROOT_PREFIX: &str = "resume-ir-synthetic-";
const CORRUPT_IMPORT_TASK_ID: &str = "diagnostic-materialization-task";

/// Closed synthetic fault vocabulary for cross-process diagnostics tests.
///
/// The enum deliberately carries no SQL, path, key, or connection capability.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OwnedStoreFault {
    MissingImportTaskTable,
    CorruptImportTaskId { canonical_root: String },
    BlockStatusUpdate,
    DeleteAfterStatusUpdate,
}

/// Opaque acknowledgement that one exact synthetic fault was installed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OwnedStoreFaultOutcome {
    Applied,
}

/// Closed writer-race setup used to prove lock-after-read behavior.
///
/// Each variant performs one fixed synthetic mutation under an immediate
/// transaction. Callers cannot supply SQL, a connection, a path, or a key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OwnedStoreWriteRace {
    StageSyntheticProjectionCommit {
        projection: ActiveSearchProjection,
        generation: String,
    },
    RestoreSyntheticDocument {
        document_id: DocumentId,
    },
}

/// Opaque owner-bound write transaction retained until a test releases it.
///
/// Dropping without [`commit`](Self::commit) rolls the fixed synthetic
/// mutation back.
#[must_use = "the synthetic writer race must be committed or explicitly dropped"]
pub struct HeldOwnedStoreWrite {
    store: OwnedMetaStore,
    active: bool,
}

impl fmt::Debug for HeldOwnedStoreWrite {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("HeldOwnedStoreWrite(<redacted>)")
    }
}

impl HeldOwnedStoreWrite {
    /// Commits the exact fixed mutation and releases the SQLite writer lock.
    pub fn commit(mut self) -> Result<()> {
        self.store
            .connection
            .borrow()
            .execute_batch("COMMIT;")
            .map_err(MetaStoreError::storage)?;
        self.active = false;
        Ok(())
    }
}

impl Drop for HeldOwnedStoreWrite {
    fn drop(&mut self) {
        if self.active {
            let _ = self.store.connection.borrow().execute_batch("ROLLBACK;");
        }
    }
}

/// Starts one fixed owner-bound write and retains its SQLite writer lock.
pub fn begin_owned_store_write_race(
    store: &OwnedMetaStore,
    race: OwnedStoreWriteRace,
) -> Result<HeldOwnedStoreWrite> {
    let race_store = store.open_sibling()?;
    let connection = race_store.connection.borrow();
    connection
        .execute_batch("BEGIN IMMEDIATE;")
        .map_err(MetaStoreError::storage)?;
    let result = match race {
        OwnedStoreWriteRace::StageSyntheticProjectionCommit {
            projection,
            generation,
        } => {
            validate_synthetic_generation(&generation)?;
            connection
                .execute(
                    "UPDATE document SET status = 'searchable'
                     WHERE id = ?1 AND is_deleted = 0",
                    [projection.document_id.as_str()],
                )
                .and_then(|changed| {
                    if changed == 1 {
                        Ok(changed)
                    } else {
                        Err(rusqlite::Error::QueryReturnedNoRows)
                    }
                })
                .and_then(|_| {
                    connection.execute(
                        "INSERT INTO search_publication_commit_guard (state_key, generation)
                         VALUES ('default', ?1)",
                        [&generation],
                    )
                })
                .and_then(|_| {
                    connection.execute(
                        "INSERT INTO active_search_projection (
                             document_id, resume_version_id, generation,
                             source_uri, normalized_path, file_name, extension,
                             byte_size, mtime_seconds, content_hash, text_hash,
                             is_deleted, created_at_seconds, updated_at_seconds, status
                         )
                         SELECT ?1, ?2, ?3, source_uri, normalized_path, file_name,
                                extension, byte_size, mtime_seconds, content_hash,
                                text_hash, is_deleted, created_at_seconds,
                                updated_at_seconds, status
                         FROM document WHERE id = ?1",
                        params![
                            projection.document_id.as_str(),
                            projection.resume_version_id.as_str(),
                            generation,
                        ],
                    )
                })
                .map(|_| ())
                .map_err(MetaStoreError::storage)
        }
        OwnedStoreWriteRace::RestoreSyntheticDocument { document_id } => connection
            .execute(
                "UPDATE document
                 SET is_deleted = 0, status = 'discovered'
                 WHERE id = ?1",
                [document_id.as_str()],
            )
            .map_err(MetaStoreError::storage)
            .and_then(|changed| {
                if changed == 1 {
                    Ok(())
                } else {
                    Err(MetaStoreError::storage_invariant())
                }
            }),
    };
    drop(connection);
    if let Err(error) = result {
        let _ = race_store.connection.borrow().execute_batch("ROLLBACK;");
        return Err(error);
    }
    Ok(HeldOwnedStoreWrite {
        store: race_store,
        active: true,
    })
}

/// Installs one bounded synthetic fault through an already owner-bound store.
///
/// This seam exists only under `migration-test-support`; it cannot open a
/// path, acquire ownership, return raw storage handles, or execute caller
/// supplied SQL.
pub fn apply_owned_store_fault(
    store: &OwnedMetaStore,
    fault: OwnedStoreFault,
) -> Result<OwnedStoreFaultOutcome> {
    let mut connection = store.connection.borrow_mut();
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(MetaStoreError::storage)?;
    match fault {
        OwnedStoreFault::MissingImportTaskTable => {
            transaction
                .execute_batch("ALTER TABLE import_task RENAME TO import_task_missing;")
                .map_err(MetaStoreError::storage)?;
        }
        OwnedStoreFault::CorruptImportTaskId { canonical_root } => {
            validate_synthetic_canonical_root(&canonical_root)?;
            transaction
                .execute(
                    "INSERT INTO import_task (
                         id, root_path, status, queued_at_seconds, updated_at_seconds
                     ) VALUES (?1, ?2, 'queued', 1, 1)",
                    params![CORRUPT_IMPORT_TASK_ID, canonical_root],
                )
                .map_err(MetaStoreError::storage)?;
            let changed = transaction
                .execute(
                    "UPDATE import_task SET id = zeroblob(16) WHERE id = ?1",
                    [CORRUPT_IMPORT_TASK_ID],
                )
                .map_err(MetaStoreError::storage)?;
            if changed != 1 {
                return Err(MetaStoreError::storage_invariant());
            }
        }
        OwnedStoreFault::BlockStatusUpdate => {
            transaction
                .execute_batch(
                    "CREATE TRIGGER import_task_block_status_update
                     BEFORE UPDATE OF status ON import_task
                     BEGIN
                         SELECT RAISE(FAIL, 'synthetic status update blocked');
                     END;",
                )
                .map_err(MetaStoreError::storage)?;
        }
        OwnedStoreFault::DeleteAfterStatusUpdate => {
            transaction
                .execute_batch(
                    "CREATE TRIGGER import_task_delete_after_status_update
                     AFTER UPDATE OF status ON import_task
                     BEGIN
                         DELETE FROM import_task WHERE id = NEW.id;
                     END;",
                )
                .map_err(MetaStoreError::storage)?;
        }
    }
    transaction.commit().map_err(MetaStoreError::storage)?;
    Ok(OwnedStoreFaultOutcome::Applied)
}

fn validate_synthetic_canonical_root(value: &str) -> Result<()> {
    if value.is_empty() || value.len() > MAX_SYNTHETIC_ROOT_BYTES || value.contains('\0') {
        return Err(MetaStoreError::invalid_value(
            "migration_test_support.canonical_root",
        ));
    }
    let path = Path::new(value);
    let components_are_canonical = path.is_absolute()
        && path.components().all(|component| {
            matches!(
                component,
                Component::Prefix(_) | Component::RootDir | Component::Normal(_)
            )
        });
    let has_synthetic_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.starts_with(SYNTHETIC_ROOT_PREFIX));
    let resolves_to_itself = fs::canonicalize(path)
        .map(|canonical| canonical == path)
        .unwrap_or(false);
    if !components_are_canonical || !has_synthetic_name || !resolves_to_itself {
        return Err(MetaStoreError::invalid_value(
            "migration_test_support.canonical_root",
        ));
    }
    Ok(())
}

fn validate_synthetic_generation(value: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > 256
        || value.contains('\0')
        || !value.starts_with("synthetic-")
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err(MetaStoreError::invalid_value(
            "migration_test_support.generation",
        ));
    }
    Ok(())
}

/// Opaque, non-sensitive facts needed by a cross-crate migration assertion.
pub struct V27RepairingFixtureFacts {
    legacy_task_id: ImportTaskId,
    inherited_visible_epoch: u64,
}

/// Opaque synthetic identities retained across a v28 processing-contract cut.
pub struct V28BlockedProcessingContractFixtureFacts {
    legacy_task_id: ImportTaskId,
    immutable_document_id: DocumentId,
    immutable_source_revision_id: SourceRevisionId,
    immutable_resume_version_id: ResumeVersionId,
    inherited_visible_epoch: u64,
}

impl V28BlockedProcessingContractFixtureFacts {
    pub fn legacy_task_id(&self) -> &ImportTaskId {
        &self.legacy_task_id
    }

    pub fn immutable_document_id(&self) -> &DocumentId {
        &self.immutable_document_id
    }

    pub fn immutable_source_revision_id(&self) -> &SourceRevisionId {
        &self.immutable_source_revision_id
    }

    pub fn immutable_resume_version_id(&self) -> &ResumeVersionId {
        &self.immutable_resume_version_id
    }

    pub fn inherited_visible_epoch(&self) -> u64 {
        self.inherited_visible_epoch
    }
}

impl fmt::Debug for V28BlockedProcessingContractFixtureFacts {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("V28BlockedProcessingContractFixtureFacts")
            .field("legacy_task_id", &"<synthetic>")
            .field("immutable_document_id", &"<synthetic>")
            .field("immutable_source_revision_id", &"<synthetic>")
            .field("immutable_resume_version_id", &"<synthetic>")
            .field("inherited_visible_epoch", &self.inherited_visible_epoch)
            .finish()
    }
}

/// Seeds an already-v28 store blocked before first publication under one exact
/// processing contract. Immutable rows are intentionally left unprojected,
/// while the completed task, completion manifest, and durable publication
/// attempt are all bound to the supplied contract.
pub fn seed_v28_blocked_processing_contract_fixture(
    data_dir: &Path,
    canonical_root: &Path,
    inherited_visible_epoch: u64,
    contract: &ImportProcessingContract,
) -> Result<V28BlockedProcessingContractFixtureFacts> {
    let canonical_root = canonical_root
        .to_str()
        .ok_or_else(|| MetaStoreError::invalid_value("migration_fixture.canonical_root"))?;
    validate_synthetic_canonical_root(canonical_root)?;
    let visible_epoch = i64::try_from(inherited_visible_epoch)
        .map_err(|_| MetaStoreError::invalid_value("migration_fixture.visible_epoch"))?;
    let owner = match DataDirectoryOwnerLease::try_acquire(data_dir)
        .map_err(|_| MetaStoreError::storage_invariant())?
    {
        DataDirectoryOwnerAcquisition::Acquired(owner) => owner,
        DataDirectoryOwnerAcquisition::Contended => {
            return Err(MetaStoreError::migration_ownership_required());
        }
    };
    let store = owner.open_store()?;
    if store.schema_version()? != 28
        || store.activate_migration_rebuild_contract(
            contract,
            UnixTimestamp::from_unix_seconds(1_700_000_000),
        )? != MigrationRebuildContractActivation::Activated
    {
        return Err(MetaStoreError::storage_invariant());
    }
    set_unpublished_visible_epoch(&store.connection.borrow(), visible_epoch)?;

    let source = b"synthetic retained immutable source";
    let mut document = Document {
        id: DocumentId::from_non_secret_parts(&[
            "migration-test-support",
            "blocked-processing-contract",
            contract.id().as_str(),
        ]),
        source_uri: "synthetic://migration-test-support/retained.txt".to_string(),
        normalized_path: "synthetic/retained.txt".to_string(),
        file_name: "retained.txt".to_string(),
        extension: FileExtension::Txt,
        byte_size: source.len() as u64,
        mtime: UnixTimestamp::from_unix_seconds(1_700_000_001),
        content_hash: None,
        text_hash: None,
        is_deleted: false,
        created_at: UnixTimestamp::from_unix_seconds(1_700_000_001),
        updated_at: UnixTimestamp::from_unix_seconds(1_700_000_001),
        status: DocumentStatus::FieldsExtracted,
    };
    let revision = SourceRevision::for_content(
        document.id.clone(),
        ContentDigest::from_bytes(source),
        source.len() as u64,
    );
    document.content_hash = Some(revision.content_hash.as_str().to_string());
    let normalized_text = "synthetic retained immutable resume";
    let normalized_text_hash = ContentDigest::from_bytes(normalized_text.as_bytes());
    let version = ResumeVersion {
        id: ResumeVersionId::from_content_identity(
            &document.id,
            &revision.id,
            &normalized_text_hash,
            contract.primary_parse_version(),
            contract.derived_schema_version(),
        ),
        document_id: document.id.clone(),
        source_revision_id: revision.id.clone(),
        normalized_text_hash,
        parse_version: contract.primary_parse_version().to_string(),
        schema_version: contract.derived_schema_version().to_string(),
        language_set: vec!["en".to_string()],
        page_count: Some(1),
        raw_text: None,
        clean_text: Some(normalized_text.to_string()),
        quality_score: Some(0.9),
    };
    let classification = ResumeVersionClassification {
        resume_version_id: version.id.clone(),
        status: ClassificationStatus::ResumeCandidate,
        classifier_epoch: contract.classifier_epoch().to_string(),
        reason_codes: vec![ReasonCode::CorroboratedResumeSignals],
        classified_at: UnixTimestamp::from_unix_seconds(1_700_000_002),
        review_disposition: ReviewDisposition::NotRequired,
    };
    store.upsert_document(&document)?;
    store.insert_source_revision(&revision)?;
    store.insert_resume_version(&version)?;
    store.insert_resume_version_classification(&classification)?;

    let task = ImportTask {
        id: ImportTaskId::from_non_secret_parts(&[
            "migration-test-support",
            "blocked-processing-contract-task",
            contract.id().as_str(),
        ]),
        root_path: canonical_root.to_string(),
        status: ImportTaskStatus::Queued,
        queued_at: UnixTimestamp::from_unix_seconds(1_700_000_003),
        started_at: None,
        finished_at: None,
        updated_at: UnixTimestamp::from_unix_seconds(1_700_000_003),
    };
    let mut scope = ImportScanScope {
        import_task_id: task.id.clone(),
        root_kind: ImportRootKind::Explicit,
        root_preset: None,
        scan_profile: ImportScanProfile::Explicit,
        requested_root_path: canonical_root.to_string(),
        canonical_root_path: canonical_root.to_string(),
        files_discovered: 1,
        ignored_entries: 0,
        scan_errors: 0,
        searchable_documents: 1,
        ocr_required_documents: 0,
        ocr_jobs_queued: 0,
        failed_documents: 0,
        deleted_documents: 0,
        scan_budget_kind: None,
        scan_budget_limit: None,
        scan_budget_observed: None,
        scan_budget_exhausted: false,
        updated_at: task.queued_at,
    };
    store.insert_import_task_with_scan_scope(&task, &scope, contract)?;
    store
        .connection
        .borrow()
        .execute(
            "INSERT INTO migration_rebuild_full_corpus_task (
             import_task_id, processing_contract_id
         ) VALUES (?1, ?2)",
            params![task.id.as_str(), contract.id().as_str()],
        )
        .map_err(MetaStoreError::storage)?;
    let running = store
        .claim_observed_import_task_for_worker(
            &task,
            UnixTimestamp::from_unix_seconds(1_700_000_004),
        )?
        .ok_or_else(MetaStoreError::storage_invariant)?;
    store.stage_import_task_source_dispositions(
        &running.id,
        contract.id(),
        &[ImportTaskSourceDisposition {
            source_ordinal: 0,
            document_id: document.id.clone(),
            source_revision_id: revision.id.clone(),
            resume_version_id: Some(version.id.clone()),
            kind: ImportSourceDispositionKind::Searchable,
        }],
    )?;
    scope.updated_at = UnixTimestamp::from_unix_seconds(1_700_000_005);
    store.complete_import_task(&running.id, contract.id(), &scope, scope.updated_at)?;
    seed_publication_attempt(&store, contract)?;
    if store.block_migration_rebuild(
        SearchRepairReason::RuntimeInvariant,
        UnixTimestamp::from_unix_seconds(1_700_000_006),
    )? != SearchProjectionTransitionOutcome::Applied
    {
        return Err(MetaStoreError::storage_invariant());
    }

    Ok(V28BlockedProcessingContractFixtureFacts {
        legacy_task_id: task.id,
        immutable_document_id: document.id,
        immutable_source_revision_id: revision.id,
        immutable_resume_version_id: version.id,
        inherited_visible_epoch,
    })
}

impl V27RepairingFixtureFacts {
    pub fn legacy_task_id(&self) -> &ImportTaskId {
        &self.legacy_task_id
    }

    pub fn inherited_visible_epoch(&self) -> u64 {
        self.inherited_visible_epoch
    }
}

impl fmt::Debug for V27RepairingFixtureFacts {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("V27RepairingFixtureFacts")
            .field("legacy_task_id", &"<synthetic>")
            .field("inherited_visible_epoch", &self.inherited_visible_epoch)
            .finish()
    }
}

/// Creates an encrypted v27 store that represents the failed installed shape:
/// a repairing search head, one authorized budgeted root, and a recoverable,
/// budget-exhausted task head. The fixture contains synthetic state only.
pub fn seed_v27_repairing_fixture(
    data_dir: &Path,
    canonical_root: &Path,
    inherited_visible_epoch: u64,
) -> Result<V27RepairingFixtureFacts> {
    fs::create_dir_all(data_dir).map_err(MetaStoreError::io_storage)?;
    let owner = match DataDirectoryOwnerLease::try_acquire(data_dir)
        .map_err(|_| MetaStoreError::storage_invariant())?
    {
        DataDirectoryOwnerAcquisition::Acquired(owner) => owner,
        DataDirectoryOwnerAcquisition::Contended => {
            return Err(MetaStoreError::migration_ownership_required());
        }
    };
    let synthetic_root_name = canonical_root
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| name.starts_with("resume-ir-synthetic-"))
        .ok_or_else(|| MetaStoreError::invalid_value("migration_fixture.synthetic_root"))?;
    if synthetic_root_name.len() > 96 {
        return Err(MetaStoreError::invalid_value(
            "migration_fixture.synthetic_root",
        ));
    }
    let canonical_root = canonical_root
        .to_str()
        .ok_or_else(|| MetaStoreError::invalid_value("migration_fixture.canonical_root"))?;
    let visible_epoch = i64::try_from(inherited_visible_epoch)
        .map_err(|_| MetaStoreError::invalid_value("migration_fixture.visible_epoch"))?;
    let legacy_task_id = ImportTaskId::from_non_secret_parts(&[
        "migration-test-support",
        "legacy-budget-exhausted-retryable",
    ]);
    let key = load_or_create_metadata_encryption_key(data_dir)?;
    let store_path = data_dir.join(STORE_FILE);
    create_private_empty_file(&store_path)?;
    let connection = Connection::open(&store_path).map_err(MetaStoreError::storage)?;
    apply_sqlcipher_key(&connection, &key)?;
    let store = OwnedMetaStore::from_owned_connection(
        connection,
        MetadataEncryptionState::SqlCipher,
        owner.shared_guard(),
    )?;
    store.migrate_staging_store_to_v27(STORE_ID_DIGEST)?;

    {
        let mut connection = store.connection.borrow_mut();
        let transaction = connection.transaction().map_err(MetaStoreError::storage)?;
        transaction
            .execute(
                "INSERT INTO authorized_import_root (
                    canonical_root_path, requested_root_path, root_kind, root_preset,
                    scan_profile, scan_budget_kind, scan_budget_limit, paused,
                    updated_at_seconds
                 ) VALUES (?1, ?1, 'explicit', NULL, 'explicit', 'files', 1, 0, 10)",
                [canonical_root],
            )
            .map_err(MetaStoreError::storage)?;
        transaction
            .execute(
                "INSERT INTO import_task (
                    id, root_path, status, queued_at_seconds, started_at_seconds,
                    finished_at_seconds, updated_at_seconds
                 ) VALUES (?1, ?2, 'failed_retryable', 10, 11, 12, 12)",
                params![legacy_task_id.as_str(), canonical_root],
            )
            .map_err(MetaStoreError::storage)?;
        transaction
            .execute(
                "INSERT INTO import_scan_scope (
                    import_task_id, root_kind, root_preset, scan_profile,
                    requested_root_path, canonical_root_path, files_discovered,
                    ignored_entries, scan_errors, searchable_documents,
                    ocr_required_documents, ocr_jobs_queued, failed_documents,
                    deleted_documents, scan_budget_kind, scan_budget_limit,
                    scan_budget_observed, scan_budget_exhausted, updated_at_seconds
                 ) VALUES (
                    ?1, 'explicit', NULL, 'explicit', ?2, ?2, 1, 0, 0, 0,
                    0, 0, 1, 0, 'files', 1, 1, 1, 12
                 )",
                params![legacy_task_id.as_str(), canonical_root],
            )
            .map_err(MetaStoreError::storage)?;
        set_repairing_epoch(&transaction, visible_epoch)?;
        transaction.commit().map_err(MetaStoreError::storage)?;
    }
    drop(store);
    sync_validated_store(&store_path)?;
    publish_new_active_store(
        data_dir,
        &ActiveStoreManifest {
            file_name: STORE_FILE.to_string(),
            schema_version: schema_v27::VERSION,
            store_id_digest: STORE_ID_DIGEST.to_string(),
        },
        || Ok(()),
    )?;

    Ok(V27RepairingFixtureFacts {
        legacy_task_id,
        inherited_visible_epoch,
    })
}

/// Counts the exact v28 completed full-corpus tasks without broadening the
/// production metadata API for a migration-only assertion.
pub fn completed_migration_rebuild_task_count(store: &ReadMetaStore) -> Result<u64> {
    store
        .connection
        .borrow()
        .query_row(
            "SELECT COUNT(*)
             FROM import_task AS task
             JOIN migration_rebuild_full_corpus_task AS rebuild
               ON rebuild.import_task_id = task.id
             WHERE task.status = 'completed'",
            [],
            |row| row.get::<_, i64>(0),
        )
        .map_err(MetaStoreError::storage)
        .and_then(|count| u64::try_from(count).map_err(|_| MetaStoreError::storage_invariant()))
}

/// Counts sealed completed tasks bound to one exact processing contract.
pub fn completed_import_task_count_for_processing_contract(
    store: &ReadMetaStore,
    contract_id: &ImportProcessingContractId,
) -> Result<u64> {
    bounded_count(
        store,
        "SELECT COUNT(*) FROM import_task_completion WHERE processing_contract_id = ?1",
        contract_id.as_str(),
    )
}

/// Counts active projections whose immutable version matches one exact parse,
/// derived-schema, and classifier contract.
pub fn active_projection_count_for_processing_contract(
    store: &ReadMetaStore,
    contract_id: &ImportProcessingContractId,
) -> Result<u64> {
    bounded_count(
        store,
        "SELECT COUNT(*)
         FROM active_search_projection AS projection
         JOIN resume_version AS version ON version.id = projection.resume_version_id
         JOIN import_processing_contract AS contract ON contract.id = ?1
         JOIN resume_version_classification AS classification
           ON classification.resume_version_id = version.id
          AND classification.classifier_epoch = contract.classifier_epoch
         WHERE version.schema_version = contract.derived_schema_version
           AND version.parse_version IN (
               contract.primary_parse_version, contract.ocr_parse_version
           )",
        contract_id.as_str(),
    )
}

fn bounded_count(store: &ReadMetaStore, sql: &str, parameter: &str) -> Result<u64> {
    store
        .connection
        .borrow()
        .query_row(sql, [parameter], |row| row.get::<_, i64>(0))
        .map_err(MetaStoreError::storage)
        .and_then(|count| u64::try_from(count).map_err(|_| MetaStoreError::storage_invariant()))
}

fn seed_publication_attempt(
    store: &OwnedMetaStore,
    contract: &ImportProcessingContract,
) -> Result<()> {
    let barrier_digest = ContentDigest::from_bytes(b"synthetic-blocked-contract-barrier");
    let attempt_id = ContentDigest::from_bytes(b"synthetic-blocked-contract-attempt");
    store
        .connection
        .borrow()
        .execute(
            "INSERT INTO migration_rebuild_publication_attempt (
                 state_key, processing_contract_id, barrier_digest, attempt_id,
                 attempt_count, phase, started_at_seconds, next_retry_at_seconds,
                 last_error_class, updated_at_seconds
             ) VALUES ('default', ?1, ?2, ?3, 1, 'running', ?4, NULL, NULL, ?4)",
            params![
                contract.id().as_str(),
                barrier_digest.as_str(),
                attempt_id.as_str(),
                1_700_000_005_i64,
            ],
        )
        .map_err(MetaStoreError::storage)?;
    Ok(())
}

fn set_unpublished_visible_epoch(connection: &Connection, visible_epoch: i64) -> Result<()> {
    connection
        .execute(
            "INSERT INTO metadata_cow_staging_authority (
                 state_key, target_visible_epoch
             ) VALUES ('default', ?1)",
            [visible_epoch],
        )
        .map_err(MetaStoreError::storage)?;
    connection
        .execute(
            "UPDATE search_projection_state SET visible_epoch = ?1
             WHERE state_key = 'default'",
            [visible_epoch],
        )
        .map_err(MetaStoreError::storage)?;
    Ok(())
}

fn set_repairing_epoch(connection: &Connection, visible_epoch: i64) -> Result<()> {
    let trigger_names = [
        "ready_projection_head_matches_journal",
        "search_projection_head_change_requires_commit_guard",
    ];
    let trigger_sql = trigger_names
        .iter()
        .map(|name| {
            connection
                .query_row(
                    "SELECT sql FROM sqlite_master WHERE type = 'trigger' AND name = ?1",
                    [name],
                    |row| row.get::<_, String>(0),
                )
                .map_err(MetaStoreError::storage)
        })
        .collect::<Result<Vec<_>>>()?;
    for name in trigger_names {
        connection
            .execute_batch(&format!("DROP TRIGGER {name};"))
            .map_err(MetaStoreError::storage)?;
    }
    connection
        .execute(
            "UPDATE search_projection_state
             SET service_state = 'repairing', generation = NULL,
                 visible_epoch = ?1, repair_reason = 'migration_rebuild',
                 updated_at_seconds = 13
             WHERE state_key = 'default'",
            [visible_epoch],
        )
        .map_err(MetaStoreError::storage)?;
    for sql in trigger_sql {
        connection
            .execute_batch(&sql)
            .map_err(MetaStoreError::storage)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use super::*;
    use crate::PendingImportTaskByRootDiagnostic;

    #[test]
    fn corrupt_import_task_fault_is_owner_bound_and_materializes_one_synthetic_row() {
        let directory = tempdir().unwrap();
        let data_dir = directory.path().join("data");
        let root = directory.path().join("resume-ir-synthetic-corrupt-row");
        fs::create_dir_all(&data_dir).unwrap();
        fs::create_dir_all(&root).unwrap();
        let canonical_root = fs::canonicalize(&root).unwrap();
        let owner = match DataDirectoryOwnerLease::try_acquire(&data_dir).unwrap() {
            DataDirectoryOwnerAcquisition::Acquired(owner) => owner,
            DataDirectoryOwnerAcquisition::Contended => panic!("synthetic owner contended"),
        };
        let store = owner.open_store().unwrap();

        assert_eq!(
            apply_owned_store_fault(
                &store,
                OwnedStoreFault::CorruptImportTaskId {
                    canonical_root: canonical_root.to_string_lossy().into_owned(),
                },
            )
            .unwrap(),
            OwnedStoreFaultOutcome::Applied
        );
        assert_eq!(
            store.diagnose_pending_import_task_by_root(&canonical_root.to_string_lossy()),
            Err(PendingImportTaskByRootDiagnostic::RowMaterializationFailure)
        );
    }

    #[test]
    fn corrupt_import_task_fault_rejects_noncanonical_or_nonsynthetic_roots() {
        assert!(validate_synthetic_canonical_root("relative/resume-ir-synthetic-root").is_err());

        let directory = tempdir().unwrap();
        let canonical = fs::canonicalize(directory.path()).unwrap();
        assert!(validate_synthetic_canonical_root(&canonical.to_string_lossy()).is_err());
    }
}

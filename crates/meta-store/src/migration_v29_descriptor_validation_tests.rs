use std::path::Path;

use rusqlite::{params, TransactionBehavior};
use tempfile::{tempdir, TempDir};

use super::*;
use crate::{
    ContentDigest, DataDirectoryOwnerAcquisition, DataDirectoryOwnerLease,
    FullTextSnapshotDescriptor, ImportProcessingContract, MetaStoreErrorClass, OwnedMetaStore,
    SearchProjectionDigest, SearchPublicationCommit, SearchPublicationDraft,
    SearchPublicationOutcome, SearchPublicationValidation, UnixTimestamp, VectorSnapshotDescriptor,
    CLASSIFIER_EPOCH,
};

const GENERATION: &str = "v29-artifact-authority";

#[test]
fn permanent_authority_rejects_a_forged_legacy_descriptor_context() {
    let (_directory, _owner, store) = ready_file_store();
    rewrite_publication_as_legacy(&store);

    assert!(insert_context_from_head(&store.connection.borrow()).is_err());
    assert_eq!(context_count(&store), 0);
}

#[test]
fn permanent_authority_accepts_an_exact_current_descriptor_context() {
    let (_directory, _owner, store) = ready_file_store();

    assert_eq!(
        insert_context_from_head(&store.connection.borrow()).unwrap(),
        1
    );
    assert_eq!(context_count(&store), 1);
    validate_current_repair_authority_trigger(&store.connection.borrow()).unwrap();
}

#[test]
fn migration_authority_accepts_legacy_only_until_current_authority_is_restored() {
    let (_directory, _owner, store) = ready_file_store();
    rewrite_publication_as_legacy(&store);
    let mut connection = store.connection.borrow_mut();
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .unwrap();

    let permanent_authority = install_migration_repair_authority(&transaction).unwrap();
    assert_eq!(insert_context_from_head(&transaction).unwrap(), 1);
    restore_current_repair_authority(&transaction, &permanent_authority).unwrap();
    transaction.commit().unwrap();

    validate_current_repair_authority_trigger(&connection).unwrap();
}

#[test]
fn current_authority_validation_rejects_missing_or_legacy_definitions() {
    let (_directory, _owner, store) = ready_file_store();
    let connection = store.connection.borrow();
    connection
        .execute_batch("DROP TRIGGER artifact_repair_context_insert_authority;")
        .unwrap();
    assert_eq!(
        validate_current_repair_authority_trigger(&connection)
            .unwrap_err()
            .class(),
        MetaStoreErrorClass::StorageInvariant
    );
    connection
        .execute_batch(schema_v29::CURRENT_REPAIR_CONTEXT_AUTHORITY)
        .unwrap();
    replace_current_authority_with_legacy_definition(&connection);
    assert_eq!(
        validate_current_repair_authority_trigger(&connection)
            .unwrap_err()
            .class(),
        MetaStoreErrorClass::StorageInvariant
    );
}

#[test]
fn reopen_rejects_a_missing_current_authority_trigger() {
    let (directory, owner, store) = ready_file_store();
    store
        .connection
        .borrow()
        .execute_batch("DROP TRIGGER artifact_repair_context_insert_authority;")
        .unwrap();
    drop(store);
    drop(owner);

    assert_reopen_storage_invariant(&directory);
}

#[test]
fn reopen_rejects_the_old_dual_authority_definition() {
    let (directory, owner, store) = ready_file_store();
    replace_current_authority_with_legacy_definition(&store.connection.borrow());
    drop(store);
    drop(owner);

    assert_reopen_storage_invariant(&directory);
}

fn ready_file_store() -> (TempDir, DataDirectoryOwnerLease, OwnedMetaStore) {
    let directory = tempdir().unwrap();
    let data_dir = directory.path().join("data");
    let owner = acquire_owner(&data_dir);
    let store = owner.open_store().unwrap();
    store.run_migrations().unwrap();
    let contract = ImportProcessingContract::new(
        "artifact-authority-parser-v1",
        "artifact-authority-ocr-v1",
        "artifact-authority-schema-v29",
        CLASSIFIER_EPOCH,
    )
    .unwrap();
    store
        .activate_migration_rebuild_contract(&contract, timestamp(1))
        .unwrap();
    let barrier = store
        .acquire_migration_rebuild_barrier_token(contract.id())
        .unwrap()
        .unwrap();
    let mut session = store.wait_for_search_publication_session().unwrap();
    let _attempt = match session
        .acquire_migration_rebuild_publication_attempt(&barrier, timestamp(2))
        .unwrap()
    {
        crate::MigrationRebuildPublicationAttemptAcquire::Started(attempt) => attempt,
        other => panic!("expected migration attempt, got {other:?}"),
    };
    let projection_digest = SearchProjectionDigest::from_pairs::<_, &str, &str>([]).unwrap();
    assert_eq!(
        session
            .begin_search_publication(&SearchPublicationDraft {
                generation: GENERATION.to_string(),
                base_generation: None,
                expected_visible_epoch: 0,
                classifier_epoch: CLASSIFIER_EPOCH.to_string(),
                projection_digest: projection_digest.clone(),
                now: timestamp(2),
            })
            .unwrap(),
        SearchPublicationOutcome::Applied
    );
    let fulltext = FullTextSnapshotDescriptor::new(
        GENERATION.to_string(),
        0,
        projection_digest.clone(),
        ContentDigest::from_bytes(b"artifact-authority-fulltext"),
    );
    let vector = VectorSnapshotDescriptor::disabled(
        GENERATION.to_string(),
        0,
        projection_digest,
        SearchProjectionDigest::from_pairs::<_, &str, &str>([]).unwrap(),
        ContentDigest::from_bytes(b"artifact-authority-vector"),
    );
    session
        .validate_search_publication(&SearchPublicationValidation {
            generation: GENERATION,
            fulltext: &fulltext,
            vector: &vector,
            now: timestamp(3),
        })
        .unwrap();
    assert_eq!(
        session
            .commit_migration_rebuild_search_publication(
                &SearchPublicationCommit {
                    generation: GENERATION,
                    terminal_documents: &[],
                    projections: &[],
                    projected_documents: &[],
                    vector_coverage: &[],
                    now: timestamp(4),
                },
                &barrier,
            )
            .unwrap(),
        SearchPublicationOutcome::Applied
    );
    drop(session);
    (directory, owner, store)
}

fn acquire_owner(data_dir: &Path) -> DataDirectoryOwnerLease {
    match DataDirectoryOwnerLease::try_acquire(data_dir).unwrap() {
        DataDirectoryOwnerAcquisition::Acquired(owner) => owner,
        DataDirectoryOwnerAcquisition::Contended => panic!("synthetic data dir is owned"),
    }
}

fn rewrite_publication_as_legacy(store: &OwnedMetaStore) {
    let mut connection = store.connection.borrow_mut();
    let records = publication_descriptor_records(&connection).unwrap();
    assert_eq!(records.len(), 1);
    let fingerprint = legacy_fingerprint(&records[0]).unwrap();
    let restore = publication_trigger_restore_sql(&connection);
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .unwrap();
    transaction
        .execute_batch(schema_v29::DROP_LEGACY_ISOLATION_TRIGGERS)
        .unwrap();
    assert_eq!(
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
                    GENERATION,
                ],
            )
            .unwrap(),
        1
    );
    transaction.execute_batch(&restore).unwrap();
    transaction.commit().unwrap();
}

fn insert_context_from_head(connection: &rusqlite::Connection) -> rusqlite::Result<usize> {
    connection.execute(
        "INSERT INTO artifact_repair_context (
             state_key, generation, publication_fingerprint, visible_epoch,
             classifier_epoch, projection_digest, projection_count,
             vector_mode, vector_model_id, vector_dimension,
             created_at_seconds, updated_at_seconds
         )
         SELECT 'default', publication.generation,
                publication.publication_fingerprint, head.visible_epoch,
                publication.classifier_epoch, publication.projection_digest,
                publication.fulltext_document_count, publication.vector_mode,
                publication.vector_model_id, publication.vector_dimension, 5, 5
         FROM search_projection_state AS head
         JOIN search_publication_journal AS publication
           ON publication.generation = head.generation
         WHERE head.state_key = 'default'",
        [],
    )
}

fn context_count(store: &OwnedMetaStore) -> i64 {
    store
        .connection
        .borrow()
        .query_row("SELECT COUNT(*) FROM artifact_repair_context", [], |row| {
            row.get(0)
        })
        .unwrap()
}

fn replace_current_authority_with_legacy_definition(connection: &rusqlite::Connection) {
    connection
        .execute_batch(schema_v29::INSTALL_MIGRATION_REPAIR_CONTEXT_AUTHORITY)
        .unwrap();
    let legacy_definition = connection
        .query_row(
            "SELECT sql FROM sqlite_master
             WHERE type = 'trigger' AND name = ?1",
            params![MIGRATION_REPAIR_AUTHORITY_TRIGGER],
            |row| row.get::<_, String>(0),
        )
        .unwrap()
        .replacen(
            MIGRATION_REPAIR_AUTHORITY_TRIGGER,
            CURRENT_REPAIR_AUTHORITY_TRIGGER,
            1,
        )
        .replacen(
            "artifact repair context lacks exact migration authority",
            "artifact repair context lacks exact head authority",
            1,
        );
    connection
        .execute_batch(&format!(
            "DROP TRIGGER {MIGRATION_REPAIR_AUTHORITY_TRIGGER};\n{legacy_definition};"
        ))
        .unwrap();
}

fn assert_reopen_storage_invariant(directory: &TempDir) {
    let owner = acquire_owner(&directory.path().join("data"));
    let error = match owner.open_store() {
        Ok(_) => panic!("corrupt v29 authority unexpectedly reopened"),
        Err(error) => error,
    };
    assert_eq!(error.class(), MetaStoreErrorClass::StorageInvariant);
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
    let sql = statement
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .collect::<std::result::Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(sql.len(), 4);
    sql.into_iter().map(|sql| format!("{sql};\n")).collect()
}

fn timestamp(seconds: i64) -> UnixTimestamp {
    UnixTimestamp::from_unix_seconds(seconds)
}

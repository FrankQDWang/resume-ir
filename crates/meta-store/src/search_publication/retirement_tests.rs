use rusqlite::params;
use tempfile::{tempdir, TempDir};

use crate::{
    ArtifactRepairKey, ContentDigest, DataDirectoryOwnerAcquisition, DataDirectoryOwnerLease,
    FullTextSnapshotDescriptor, ImportProcessingContract,
    MigrationRebuildPublicationAttemptAcquire, OwnedMetaStore, SearchArtifactExpectation,
    SearchProjectionDigest, SearchPublicationCommit, SearchPublicationDraft,
    SearchPublicationFailure, SearchPublicationOutcome, SearchPublicationPrunePolicy,
    SearchPublicationRetirementArtifact, SearchPublicationRetirementPhase,
    SearchPublicationRetirementPlan, SearchPublicationValidation, UnixTimestamp,
    VectorSnapshotDescriptor, CLASSIFIER_EPOCH,
};

use super::SEARCH_PUBLICATION_RETIREMENT_REPLAY_LIMIT;

#[test]
fn excessive_pending_retirements_fail_closed_before_publication_or_migration_attempt() {
    let (_directory, store) = repairing_store("pending-overflow-migration");
    let contract = contract("pending-overflow-migration");
    store
        .activate_migration_rebuild_contract(&contract, timestamp(10))
        .unwrap();
    let barrier = store
        .acquire_migration_rebuild_barrier_token(contract.id())
        .unwrap()
        .unwrap();
    let mut session = store.wait_for_search_publication_session().unwrap();
    seed_pending_retirements(&store, SEARCH_PUBLICATION_RETIREMENT_REPLAY_LIMIT + 1);

    assert_eq!(
        store
            .pending_search_publication_retirements()
            .unwrap_err()
            .search_publication_failure(),
        Some(SearchPublicationFailure::InvalidPersistedState)
    );
    assert_eq!(
        session
            .acquire_migration_rebuild_publication_attempt(&barrier, timestamp(20))
            .unwrap_err()
            .search_publication_failure(),
        Some(SearchPublicationFailure::InvalidPersistedState)
    );
    assert_eq!(
        session
            .begin_search_publication(&empty_draft("blocked-publication", None, 0, timestamp(20)))
            .unwrap_err()
            .search_publication_failure(),
        Some(SearchPublicationFailure::InvalidPersistedState)
    );
    assert_eq!(
        store.migration_rebuild_publication_attempt_state().unwrap(),
        None
    );
}

#[test]
fn excessive_pending_retirements_fail_closed_before_artifact_attempt() {
    let (_directory, store) = ready_store("pending-overflow-artifact");
    let ready = store.search_projection_state().unwrap();
    store
        .begin_artifact_repair(
            ready.generation.as_deref().unwrap(),
            ready.visible_epoch,
            timestamp(30),
        )
        .unwrap();
    let context = store.artifact_repair_context().unwrap().unwrap();
    let key = ArtifactRepairKey::new(
        context.generation,
        context.publication_fingerprint,
        context.visible_epoch,
    );
    let mut session = store.wait_for_search_publication_session().unwrap();
    seed_pending_retirements(&store, SEARCH_PUBLICATION_RETIREMENT_REPLAY_LIMIT + 1);

    assert_eq!(
        session
            .acquire_artifact_repair_attempt(&key, timestamp(31))
            .unwrap_err()
            .search_publication_failure(),
        Some(SearchPublicationFailure::InvalidPersistedState)
    );
    assert_eq!(store.artifact_repair_attempt_state().unwrap(), None);
}

#[test]
fn pending_retirement_cannot_be_pruned_or_deleted() {
    let (_directory, store) = ready_store("pending-prune");
    let head = store.search_projection_state().unwrap();
    let session = store.wait_for_search_publication_session().unwrap();
    let generation = "pending-prune-candidate";
    assert_eq!(
        session
            .begin_search_publication(&empty_draft(
                generation,
                head.generation.as_deref(),
                head.visible_epoch,
                timestamp(40),
            ))
            .unwrap(),
        SearchPublicationOutcome::Applied
    );
    session
        .begin_search_publication_retirement(
            generation,
            timestamp(41),
            SearchPublicationRetirementPlan {
                fulltext: crate::SearchArtifactExpectation::MayExist,
                vector: crate::SearchArtifactExpectation::None,
            },
        )
        .unwrap();

    assert_eq!(
        session
            .prune_search_publication_history(SearchPublicationPrunePolicy {
                retain_ready: 1,
                abandoned_updated_before: timestamp(42),
                max_delete: 256,
            })
            .unwrap(),
        0
    );
    assert!(store.search_publication(generation).unwrap().is_some());
    assert!(store
        .search_publication_retirement(generation)
        .unwrap()
        .is_some());
    assert!(store
        .connection
        .borrow()
        .execute(
            "DELETE FROM search_publication_journal WHERE generation = ?1",
            params![generation],
        )
        .is_err());
    assert!(store
        .connection
        .borrow()
        .execute(
            "DELETE FROM search_publication_retirement WHERE generation = ?1",
            params![generation],
        )
        .is_err());
    assert!(store
        .connection
        .borrow()
        .execute(
            "INSERT OR REPLACE INTO search_publication_retirement (
                generation, phase, fulltext_expectation, vector_expectation,
                fulltext_complete, vector_complete, created_at_seconds,
                updated_at_seconds
             ) VALUES (?1, 'complete', 'may_exist', 'none', 1, 1, 40, 42)",
            params![generation],
        )
        .is_err());
    assert_eq!(
        store
            .search_publication_retirement(generation)
            .unwrap()
            .unwrap()
            .phase,
        SearchPublicationRetirementPhase::Pending
    );
}

#[test]
fn direct_sql_cannot_forge_retirement_completion_but_guarded_api_can_complete() {
    let (_directory, store) = ready_store("guarded-retirement-completion");
    let head = store.search_projection_state().unwrap();
    let session = store.wait_for_search_publication_session().unwrap();
    let generation = "guarded-retirement-completion-candidate";
    assert_eq!(
        session
            .begin_search_publication(&empty_draft(
                generation,
                head.generation.as_deref(),
                head.visible_epoch,
                timestamp(50),
            ))
            .unwrap(),
        SearchPublicationOutcome::Applied
    );
    session
        .begin_search_publication_retirement(
            generation,
            timestamp(51),
            SearchPublicationRetirementPlan {
                fulltext: SearchArtifactExpectation::MayExist,
                vector: SearchArtifactExpectation::MayExist,
            },
        )
        .unwrap();

    assert!(store
        .connection
        .borrow()
        .execute(
            "UPDATE search_publication_retirement
             SET phase = 'complete', fulltext_complete = 1,
                 vector_complete = 1, updated_at_seconds = 52
             WHERE generation = ?1",
            params![generation],
        )
        .is_err());
    let unchanged = store
        .search_publication_retirement(generation)
        .unwrap()
        .unwrap();
    assert_eq!(unchanged.phase, SearchPublicationRetirementPhase::Pending);
    assert!(!unchanged.fulltext_complete);
    assert!(!unchanged.vector_complete);

    session
        .complete_search_publication_retirement_artifact(
            generation,
            SearchPublicationRetirementArtifact::FullText,
            timestamp(52),
        )
        .unwrap();
    let partial = store
        .search_publication_retirement(generation)
        .unwrap()
        .unwrap();
    assert_eq!(partial.phase, SearchPublicationRetirementPhase::Pending);
    assert!(partial.fulltext_complete);
    assert!(!partial.vector_complete);
    session
        .complete_search_publication_retirement_artifact(
            generation,
            SearchPublicationRetirementArtifact::Vector,
            timestamp(53),
        )
        .unwrap();
    assert_eq!(
        store
            .search_publication_retirement(generation)
            .unwrap()
            .unwrap()
            .phase,
        SearchPublicationRetirementPhase::Complete
    );
    assert_eq!(
        store
            .connection
            .borrow()
            .query_row(
                "SELECT COUNT(*) FROM search_publication_retirement_completion_guard",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        0
    );
    drop(session);
    assert!(store.open_sibling().is_ok());
}

#[test]
fn v29_reopen_validation_rejects_a_tampered_retirement_trigger() {
    let (_directory, store) = ready_store("retirement-trigger-validation");
    crate::schema_v29_publication_retirement::validate(&store.connection.borrow()).unwrap();
    store
        .connection
        .borrow()
        .execute_batch(
            "DROP TRIGGER pending_search_publication_cannot_be_deleted;
             CREATE TRIGGER pending_search_publication_cannot_be_deleted
             BEFORE DELETE ON search_publication_journal
             WHEN 0
             BEGIN
                 SELECT 1;
             END;",
        )
        .unwrap();
    assert!(
        crate::schema_v29_publication_retirement::validate(&store.connection.borrow()).is_err()
    );
    assert!(store.open_sibling().is_err());
}

#[test]
fn v29_reopen_validation_rejects_an_unfinished_completion_guard() {
    let (_directory, store) = ready_store("retirement-guard-validation");
    let head = store.search_projection_state().unwrap();
    let session = store.wait_for_search_publication_session().unwrap();
    let generation = "unfinished-retirement-guard";
    session
        .begin_search_publication(&empty_draft(
            generation,
            head.generation.as_deref(),
            head.visible_epoch,
            timestamp(60),
        ))
        .unwrap();
    session
        .begin_search_publication_retirement(
            generation,
            timestamp(61),
            SearchPublicationRetirementPlan {
                fulltext: SearchArtifactExpectation::MayExist,
                vector: SearchArtifactExpectation::None,
            },
        )
        .unwrap();
    drop(session);
    store
        .connection
        .borrow()
        .execute(
            "INSERT INTO search_publication_retirement_completion_guard (
                generation, artifact, completed_at_seconds
             ) VALUES (?1, 'fulltext', 62)",
            params![generation],
        )
        .unwrap();

    assert!(
        crate::schema_v29_publication_retirement::validate(&store.connection.borrow()).is_err()
    );
    assert!(store.open_sibling().is_err());
}

fn repairing_store(label: &str) -> (TempDir, OwnedMetaStore) {
    let directory = tempdir().unwrap();
    let data_dir = directory.path().join(label);
    let owner = match DataDirectoryOwnerLease::try_acquire(&data_dir).unwrap() {
        DataDirectoryOwnerAcquisition::Acquired(owner) => owner,
        DataDirectoryOwnerAcquisition::Contended => panic!("synthetic data directory contended"),
    };
    let store = owner.open_store().unwrap();
    (directory, store)
}

fn ready_store(label: &str) -> (TempDir, OwnedMetaStore) {
    let (directory, store) = repairing_store(label);
    let contract = contract(label);
    store
        .activate_migration_rebuild_contract(&contract, timestamp(1))
        .unwrap();
    let barrier = store
        .acquire_migration_rebuild_barrier_token(contract.id())
        .unwrap()
        .unwrap();
    let mut session = store.wait_for_search_publication_session().unwrap();
    assert!(matches!(
        session
            .acquire_migration_rebuild_publication_attempt(&barrier, timestamp(2))
            .unwrap(),
        MigrationRebuildPublicationAttemptAcquire::Started(_)
    ));
    let generation = format!("{label}-ready");
    let draft = empty_draft(&generation, None, 0, timestamp(3));
    assert_eq!(
        session.begin_search_publication(&draft).unwrap(),
        SearchPublicationOutcome::Applied
    );
    let digest = SearchProjectionDigest::from_pairs::<_, &str, &str>([]).unwrap();
    let fulltext = FullTextSnapshotDescriptor::new(
        generation.clone(),
        0,
        digest.clone(),
        ContentDigest::from_bytes(format!("fulltext:{label}").as_bytes()),
    );
    let vector = VectorSnapshotDescriptor::disabled(
        generation.clone(),
        0,
        digest,
        SearchProjectionDigest::from_pairs::<_, &str, &str>([]).unwrap(),
        ContentDigest::from_bytes(format!("vector:{label}").as_bytes()),
    );
    session
        .validate_search_publication(&SearchPublicationValidation {
            generation: &generation,
            fulltext: &fulltext,
            vector: &vector,
            now: timestamp(4),
        })
        .unwrap();
    assert_eq!(
        session
            .commit_migration_rebuild_search_publication(
                &SearchPublicationCommit {
                    generation: &generation,
                    terminal_documents: &[],
                    projections: &[],
                    projected_documents: &[],
                    vector_coverage: &[],
                    now: timestamp(5),
                },
                &barrier,
            )
            .unwrap(),
        SearchPublicationOutcome::Applied
    );
    drop(session);
    (directory, store)
}

fn seed_pending_retirements(store: &OwnedMetaStore, count: usize) {
    let digest = ContentDigest::from_bytes(b"pending retirement overflow");
    let connection = store.connection.borrow();
    let transaction = connection.unchecked_transaction().unwrap();
    for index in 0..count {
        let generation = format!("pending-overflow-{index:03}");
        transaction
            .execute(
                "INSERT INTO search_publication_journal (
                    generation, base_generation, expected_visible_epoch,
                    classifier_epoch, projection_digest, state,
                    created_at_seconds, updated_at_seconds, authority_kind
                 ) VALUES (?1, NULL, 0, ?2, ?3, 'abandoned', 100, 100, 'current_head')",
                params![generation, CLASSIFIER_EPOCH, digest.as_str()],
            )
            .unwrap();
        transaction
            .execute(
                "INSERT INTO search_publication_retirement (
                    generation, phase, fulltext_expectation, vector_expectation,
                    fulltext_complete, vector_complete, created_at_seconds,
                    updated_at_seconds
                 ) VALUES (?1, 'pending', 'may_exist', 'none', 0, 1, 100, 100)",
                params![generation],
            )
            .unwrap();
    }
    transaction.commit().unwrap();
}

fn empty_draft(
    generation: &str,
    base_generation: Option<&str>,
    expected_visible_epoch: u64,
    now: UnixTimestamp,
) -> SearchPublicationDraft {
    SearchPublicationDraft {
        generation: generation.to_string(),
        base_generation: base_generation.map(str::to_string),
        expected_visible_epoch,
        classifier_epoch: CLASSIFIER_EPOCH.to_string(),
        projection_digest: SearchProjectionDigest::from_pairs::<_, &str, &str>([]).unwrap(),
        now,
    }
}

fn contract(label: &str) -> ImportProcessingContract {
    ImportProcessingContract::new(
        format!("{label}-parser"),
        format!("{label}-ocr"),
        format!("{label}-schema"),
        CLASSIFIER_EPOCH,
    )
    .unwrap()
}

fn timestamp(seconds: i64) -> UnixTimestamp {
    UnixTimestamp::from_unix_seconds(seconds)
}

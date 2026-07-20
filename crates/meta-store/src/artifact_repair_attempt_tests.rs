use std::path::Path;

use tempfile::{tempdir, TempDir};

use super::*;
use crate::{
    ContentDigest, DataDirectoryOwnerAcquisition, DataDirectoryOwnerLease,
    FullTextSnapshotDescriptor, ImportProcessingContract, OwnedMetaStore,
    SearchArtifactExpectation, SearchProjectionDigest, SearchProjectionServiceState,
    SearchProjectionTransitionOutcome, SearchPublicationCommit, SearchPublicationDraft,
    SearchPublicationOutcome, SearchPublicationRetirementFailureOutcome,
    SearchPublicationRetirementPlan, SearchPublicationSession, SearchPublicationValidation,
    SearchRepairReason, VectorSnapshotDescriptor, CLASSIFIER_EPOCH,
};

#[test]
fn retry_schedule_is_fixed_and_bounded() {
    let now = UnixTimestamp::from_unix_seconds(100);
    assert_eq!(retry_at(now, 1), UnixTimestamp::from_unix_seconds(101));
    assert_eq!(retry_at(now, 2), UnixTimestamp::from_unix_seconds(104));
    assert_eq!(retry_at(now, 3), UnixTimestamp::from_unix_seconds(115));
    assert_eq!(retry_at(now, 4), UnixTimestamp::from_unix_seconds(130));
    assert_eq!(retry_at(now, 5), UnixTimestamp::from_unix_seconds(160));
}

#[test]
fn v29_attempt_storage_rejects_collapsed_legacy_error_values() {
    let (_directory, _owner, store) = ready_file_store();
    let key = begin_repair(&store, timestamp(100));
    let mut session = store.wait_for_search_publication_session().unwrap();
    let _attempt = started(
        session
            .acquire_artifact_repair_attempt(&key, timestamp(100))
            .unwrap(),
    );
    drop(session);

    let connection = store.connection.borrow();
    let columns = connection
        .prepare("PRAGMA table_info(artifact_repair_attempt)")
        .unwrap()
        .query_map([], |row| row.get::<_, String>(1))
        .unwrap()
        .collect::<std::result::Result<Vec<_>, _>>()
        .unwrap();
    assert!(columns.iter().any(|column| column == "last_error_kind"));
    assert!(!columns.iter().any(|column| column == "last_error_class"));
    for legacy_value in ["fulltext", "vector", "metadata"] {
        assert!(connection
            .execute(
                "UPDATE artifact_repair_attempt
                 SET phase = 'retry_wait', next_retry_at_seconds = 101,
                     last_error_kind = ?1
                 WHERE state_key = 'default'",
                [legacy_value],
            )
            .is_err());
    }
    connection
        .execute(
            "UPDATE artifact_repair_attempt
             SET phase = 'retry_wait', next_retry_at_seconds = 101,
                 last_error_kind = 'fulltext_publication_busy'
             WHERE state_key = 'default'",
            [],
        )
        .unwrap();
    drop(connection);

    assert_eq!(
        store
            .artifact_repair_attempt_state()
            .unwrap()
            .unwrap()
            .last_error_kind,
        Some(ArtifactRepairAttemptErrorKind::FullTextPublicationBusy)
    );
}

#[test]
fn wall_clock_rollback_rebases_one_retry_without_resetting_its_attempt_count() {
    let (_directory, _owner, store) = ready_file_store();
    let key = begin_repair(&store, timestamp(10_000));
    let mut session = store.wait_for_search_publication_session().unwrap();
    let first = started(
        session
            .acquire_artifact_repair_attempt(&key, timestamp(10_000))
            .unwrap(),
    );
    session
        .finish_artifact_repair_attempt_failure(
            &first,
            ArtifactRepairAttemptFailure::Retryable(
                ArtifactRepairAttemptErrorKind::FullTextFailure,
            ),
            timestamp(10_000),
        )
        .unwrap();
    assert_eq!(
        store
            .artifact_repair_attempt_state()
            .unwrap()
            .unwrap()
            .next_retry_at,
        Some(timestamp(10_001))
    );

    assert_eq!(
        session
            .acquire_artifact_repair_attempt(&key, timestamp(100))
            .unwrap(),
        ArtifactRepairAttemptAcquire::NotDue
    );
    let rebased = store.artifact_repair_attempt_state().unwrap().unwrap();
    assert_eq!(rebased.attempt_count, 1);
    assert_eq!(rebased.next_retry_at, Some(timestamp(101)));
    assert_eq!(
        session
            .acquire_artifact_repair_attempt(&key, timestamp(100))
            .unwrap(),
        ArtifactRepairAttemptAcquire::NotDue
    );
    let second = started(
        session
            .acquire_artifact_repair_attempt(&key, timestamp(101))
            .unwrap(),
    );
    assert_eq!(second.attempt_count, 2);
}

#[test]
fn retry_count_and_deadline_survive_reopen_and_fifth_failure_blocks_exact_head() {
    let (directory, owner, store) = ready_file_store();
    let key = begin_repair(&store, timestamp(100));
    let mut session = store.wait_for_search_publication_session().unwrap();
    let first = started(
        session
            .acquire_artifact_repair_attempt(&key, timestamp(100))
            .unwrap(),
    );
    assert_eq!(
        session
            .finish_artifact_repair_attempt_failure(
                &first,
                ArtifactRepairAttemptFailure::Retryable(
                    ArtifactRepairAttemptErrorKind::FullTextFailure,
                ),
                timestamp(100),
            )
            .unwrap(),
        ArtifactRepairAttemptFailureOutcome::RetryScheduled
    );
    drop(session);
    drop(store);
    drop(owner);

    let data_dir = directory.path().join("data");
    let owner = acquire_owner(&data_dir);
    let store = owner.open_store().unwrap();
    assert_eq!(
        store.artifact_repair_attempt_state().unwrap(),
        Some(ArtifactRepairAttemptState {
            attempt_count: 1,
            phase: ArtifactRepairAttemptPhase::RetryWait,
            started_at: timestamp(100),
            next_retry_at: Some(timestamp(101)),
            last_error_kind: Some(ArtifactRepairAttemptErrorKind::FullTextFailure),
        })
    );
    let mut session = store.wait_for_search_publication_session().unwrap();
    assert_eq!(
        session
            .acquire_artifact_repair_attempt(&key, timestamp(100))
            .unwrap(),
        ArtifactRepairAttemptAcquire::NotDue
    );

    let mut due = timestamp(101);
    for expected_count in 2..=5 {
        let attempt = started(session.acquire_artifact_repair_attempt(&key, due).unwrap());
        let outcome = session
            .finish_artifact_repair_attempt_failure(
                &attempt,
                ArtifactRepairAttemptFailure::Retryable(
                    ArtifactRepairAttemptErrorKind::VectorFailure,
                ),
                due,
            )
            .unwrap();
        assert_eq!(
            outcome,
            if expected_count == 5 {
                ArtifactRepairAttemptFailureOutcome::RepairBlocked
            } else {
                ArtifactRepairAttemptFailureOutcome::RetryScheduled
            }
        );
        if expected_count < 5 {
            let state = store.artifact_repair_attempt_state().unwrap().unwrap();
            assert_eq!(state.attempt_count, expected_count);
            due = state.next_retry_at.unwrap();
        }
    }

    let blocked = store.search_projection_state().unwrap();
    assert_eq!(
        blocked.service_state,
        SearchProjectionServiceState::RepairBlocked
    );
    assert_eq!(
        blocked.repair_reason,
        Some(SearchRepairReason::RuntimeInvariant)
    );
    assert_eq!(blocked.generation.as_deref(), Some(key.generation()));
    assert_eq!(blocked.visible_epoch, key.visible_epoch());
}

#[test]
fn terminal_retirement_failure_settles_the_exact_already_blocked_attempt_after_reopen() {
    let (directory, owner, store) = ready_file_store();
    let key = begin_repair(&store, timestamp(220));
    let mut session = store.wait_for_search_publication_session().unwrap();
    let attempt = started(
        session
            .acquire_artifact_repair_attempt(&key, timestamp(220))
            .unwrap(),
    );
    let projection_digest = SearchProjectionDigest::from_pairs::<_, &str, &str>([]).unwrap();
    assert_eq!(
        session
            .begin_search_publication(&SearchPublicationDraft {
                generation: "artifact-repair-retirement-terminal".to_string(),
                base_generation: Some(key.generation().to_string()),
                expected_visible_epoch: key.visible_epoch(),
                classifier_epoch: CLASSIFIER_EPOCH.to_string(),
                projection_digest,
                now: timestamp(221),
            })
            .unwrap(),
        SearchPublicationOutcome::Applied
    );
    session
        .begin_search_publication_retirement(
            "artifact-repair-retirement-terminal",
            timestamp(222),
            SearchPublicationRetirementPlan {
                fulltext: SearchArtifactExpectation::MayExist,
                vector: SearchArtifactExpectation::MayExist,
            },
        )
        .unwrap();
    assert!(session
        .fail_search_publication_retirement_settlement_before_commit_for_test(
            "artifact-repair-retirement-terminal",
            timestamp(223),
        )
        .is_err());
    let rolled_back_head = store.search_projection_state().unwrap();
    assert_eq!(
        rolled_back_head.service_state,
        SearchProjectionServiceState::Repairing
    );
    assert_eq!(
        rolled_back_head.repair_reason,
        Some(SearchRepairReason::ArtifactUnavailable)
    );
    let rolled_back_attempt = store.artifact_repair_attempt_state().unwrap().unwrap();
    assert_eq!(
        rolled_back_attempt.phase,
        ArtifactRepairAttemptPhase::Running
    );
    assert_eq!(rolled_back_attempt.last_error_kind, None);
    assert_eq!(
        session
            .block_search_head_after_publication_retirement_failure(
                "artifact-repair-retirement-terminal",
                timestamp(223),
            )
            .unwrap(),
        SearchPublicationRetirementFailureOutcome::HeadBlocked
    );

    drop(session);
    drop(store);
    drop(owner);

    let data_dir = directory.path().join("data");
    let owner = acquire_owner(&data_dir);
    let store = owner.open_store().unwrap();
    let blocked = store.search_projection_state().unwrap();
    assert_eq!(
        blocked.service_state,
        SearchProjectionServiceState::RepairBlocked
    );
    assert_eq!(
        blocked.repair_reason,
        Some(SearchRepairReason::RuntimeInvariant)
    );
    assert_eq!(blocked.generation.as_deref(), Some(key.generation()));
    assert_eq!(blocked.visible_epoch, key.visible_epoch());
    let settled = store.artifact_repair_attempt_state().unwrap().unwrap();
    assert_eq!(settled.attempt_count, 1);
    assert_eq!(settled.phase, ArtifactRepairAttemptPhase::Terminal);
    assert_eq!(
        settled.last_error_kind,
        Some(ArtifactRepairAttemptErrorKind::Cleanup)
    );
    let mut session = store.wait_for_search_publication_session().unwrap();
    assert_eq!(
        session
            .finish_artifact_repair_attempt_failure(
                &attempt,
                ArtifactRepairAttemptFailure::Terminal(
                    ArtifactRepairAttemptErrorKind::FullTextFailure,
                ),
                timestamp(224),
            )
            .unwrap(),
        ArtifactRepairAttemptFailureOutcome::RepairBlocked
    );
    assert_eq!(
        store.artifact_repair_attempt_state().unwrap(),
        Some(settled.clone())
    );
    assert!(session
        .acquire_artifact_repair_attempt(&key, timestamp(1_000))
        .is_err());
    assert_eq!(
        store.artifact_repair_attempt_state().unwrap(),
        Some(settled.clone())
    );
    assert_eq!(
        session
            .block_search_head_after_publication_retirement_failure(
                "artifact-repair-retirement-terminal",
                timestamp(225),
            )
            .unwrap(),
        SearchPublicationRetirementFailureOutcome::ExactHeadAlreadyBlocked
    );
    assert_eq!(store.search_projection_state().unwrap(), blocked);
    assert_eq!(
        store.artifact_repair_attempt_state().unwrap(),
        Some(settled.clone())
    );

    let mismatched_attempt_id = ContentDigest::from_bytes(b"mismatched artifact cleanup attempt");
    store
        .connection
        .borrow()
        .execute(
            "UPDATE artifact_repair_attempt SET attempt_id = ?1 WHERE state_key = 'default'",
            [mismatched_attempt_id.as_str()],
        )
        .unwrap();
    assert_eq!(
        session
            .block_search_head_after_publication_retirement_failure(
                "artifact-repair-retirement-terminal",
                timestamp(226),
            )
            .unwrap(),
        SearchPublicationRetirementFailureOutcome::HeadSuperseded
    );
    assert_eq!(store.search_projection_state().unwrap(), blocked);
    assert_eq!(
        store.artifact_repair_attempt_state().unwrap(),
        Some(settled)
    );
}

#[test]
fn one_orphaned_running_attempt_is_recorded_as_interrupted_once() {
    let (directory, owner, store) = ready_file_store();
    let key = begin_repair(&store, timestamp(200));
    let mut session = store.wait_for_search_publication_session().unwrap();
    let _orphan = started(
        session
            .acquire_artifact_repair_attempt(&key, timestamp(200))
            .unwrap(),
    );
    drop(session);
    drop(store);
    drop(owner);

    let data_dir = directory.path().join("data");
    let owner = acquire_owner(&data_dir);
    let store = owner.open_store().unwrap();
    let mut session = store.wait_for_search_publication_session().unwrap();
    assert_eq!(
        session
            .acquire_artifact_repair_attempt(&key, timestamp(205))
            .unwrap(),
        ArtifactRepairAttemptAcquire::NotDue
    );
    let interrupted = store.artifact_repair_attempt_state().unwrap().unwrap();
    assert_eq!(interrupted.attempt_count, 1);
    assert_eq!(interrupted.phase, ArtifactRepairAttemptPhase::RetryWait);
    assert_eq!(interrupted.next_retry_at, Some(timestamp(206)));
    assert_eq!(
        interrupted.last_error_kind,
        Some(ArtifactRepairAttemptErrorKind::Interrupted)
    );
    assert_eq!(
        session
            .acquire_artifact_repair_attempt(&key, timestamp(205))
            .unwrap(),
        ArtifactRepairAttemptAcquire::NotDue
    );
    assert_eq!(
        store.artifact_repair_attempt_state().unwrap().unwrap(),
        interrupted
    );
    let second = started(
        session
            .acquire_artifact_repair_attempt(&key, timestamp(206))
            .unwrap(),
    );
    assert_eq!(second.attempt_count, 2);
}

#[test]
fn lifecycle_cancellation_does_not_consume_the_retry_budget() {
    let (_directory, _owner, store) = ready_file_store();
    let key = begin_repair(&store, timestamp(300));
    let mut session = store.wait_for_search_publication_session().unwrap();

    let first = started(
        session
            .acquire_artifact_repair_attempt(&key, timestamp(300))
            .unwrap(),
    );
    assert_eq!(
        session.cancel_artifact_repair_attempt(&first).unwrap(),
        ArtifactRepairAttemptCancellationOutcome::Restored
    );
    assert_eq!(store.artifact_repair_attempt_state().unwrap(), None);

    let first = started(
        session
            .acquire_artifact_repair_attempt(&key, timestamp(301))
            .unwrap(),
    );
    session
        .finish_artifact_repair_attempt_failure(
            &first,
            ArtifactRepairAttemptFailure::Retryable(
                ArtifactRepairAttemptErrorKind::MetadataFailure,
            ),
            timestamp(301),
        )
        .unwrap();
    let before = store.artifact_repair_attempt_state().unwrap().unwrap();
    let second = started(
        session
            .acquire_artifact_repair_attempt(&key, before.next_retry_at.unwrap())
            .unwrap(),
    );
    assert_eq!(second.attempt_count, 2);
    assert_eq!(
        session.cancel_artifact_repair_attempt(&second).unwrap(),
        ArtifactRepairAttemptCancellationOutcome::Restored
    );
    assert_eq!(
        store.artifact_repair_attempt_state().unwrap().unwrap(),
        before
    );
}

#[test]
fn successful_successor_clears_context_and_ledger_and_stale_finish_is_superseded() {
    let (_directory, _owner, store) = ready_file_store();
    let original = store.search_projection_state().unwrap();
    let key = begin_repair(&store, timestamp(400));
    let mut session = store.wait_for_search_publication_session().unwrap();
    let stale = started(
        session
            .acquire_artifact_repair_attempt(&key, timestamp(400))
            .unwrap(),
    );

    publish_empty_generation(
        &session,
        "artifact-repair-successor",
        original.generation.as_deref(),
        original.visible_epoch,
        None,
        timestamp(401),
    );
    assert_eq!(store.artifact_repair_context().unwrap(), None);
    assert_eq!(store.artifact_repair_attempt_state().unwrap(), None);
    assert_eq!(
        session
            .finish_artifact_repair_attempt_failure(
                &stale,
                ArtifactRepairAttemptFailure::Terminal(
                    ArtifactRepairAttemptErrorKind::MetadataFailure,
                ),
                timestamp(402),
            )
            .unwrap(),
        ArtifactRepairAttemptFailureOutcome::Superseded
    );

    let ready = store.search_projection_state().unwrap();
    assert_eq!(ready.service_state, SearchProjectionServiceState::Ready);
    assert_eq!(
        ready.generation.as_deref(),
        Some("artifact-repair-successor")
    );
    assert_eq!(ready.visible_epoch, original.visible_epoch + 1);
    assert_eq!(store.artifact_repair_context().unwrap(), None);
    assert_eq!(store.artifact_repair_attempt_state().unwrap(), None);
}

#[test]
fn artifact_repair_refuses_a_tampered_current_publication_fingerprint() {
    let (_directory, _owner, store) = ready_file_store();
    let ready = store.search_projection_state().unwrap();
    let generation = ready.generation.as_deref().unwrap();
    let connection = store.connection.borrow();
    let restore = publication_trigger_restore_sql(&connection);
    connection
        .execute_batch(crate::schema_v29::DROP_LEGACY_ISOLATION_TRIGGERS)
        .unwrap();
    connection
        .execute(
            "UPDATE search_publication_journal SET publication_fingerprint = ?1
             WHERE generation = ?2",
            rusqlite::params![
                ContentDigest::from_bytes(b"tampered runtime fingerprint").as_str(),
                generation,
            ],
        )
        .unwrap();
    connection.execute_batch(&restore).unwrap();
    drop(connection);

    assert!(store
        .begin_artifact_repair(generation, ready.visible_epoch, timestamp(500))
        .is_err());
    let unchanged = store
        .connection
        .borrow()
        .query_row(
            "SELECT service_state, repair_reason FROM search_projection_state
             WHERE state_key = 'default'",
            [],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?)),
        )
        .unwrap();
    assert_eq!(unchanged, ("ready".to_string(), None));
    assert_eq!(store.artifact_repair_context().unwrap(), None);
}

fn ready_file_store() -> (TempDir, DataDirectoryOwnerLease, OwnedMetaStore) {
    let directory = tempdir().unwrap();
    let data_dir = directory.path().join("data");
    let owner = acquire_owner(&data_dir);
    let store = owner.open_store().unwrap();
    store.run_migrations().unwrap();
    let contract = ImportProcessingContract::new(
        "artifact-repair-parser-v1",
        "artifact-repair-ocr-v1",
        "artifact-repair-schema-v29",
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
    publish_empty_generation(
        &session,
        "artifact-repair-original",
        None,
        0,
        Some(&barrier),
        timestamp(2),
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

fn begin_repair(store: &OwnedMetaStore, now: UnixTimestamp) -> ArtifactRepairKey {
    let ready = store.search_projection_state().unwrap();
    let generation = ready.generation.as_deref().unwrap();
    assert_eq!(
        store
            .begin_artifact_repair(generation, ready.visible_epoch, now)
            .unwrap(),
        SearchProjectionTransitionOutcome::Applied
    );
    let context = store.artifact_repair_context().unwrap().unwrap();
    ArtifactRepairKey::new(
        context.generation,
        context.publication_fingerprint,
        context.visible_epoch,
    )
}

fn publish_empty_generation(
    session: &SearchPublicationSession,
    generation: &str,
    base_generation: Option<&str>,
    expected_visible_epoch: u64,
    migration_barrier: Option<&crate::MigrationRebuildBarrierToken>,
    now: UnixTimestamp,
) {
    let projection_digest = SearchProjectionDigest::from_pairs::<_, &str, &str>([]).unwrap();
    assert_eq!(
        session
            .begin_search_publication(&SearchPublicationDraft {
                generation: generation.to_string(),
                base_generation: base_generation.map(str::to_string),
                expected_visible_epoch,
                classifier_epoch: CLASSIFIER_EPOCH.to_string(),
                projection_digest: projection_digest.clone(),
                now,
            })
            .unwrap(),
        SearchPublicationOutcome::Applied
    );
    let fulltext = FullTextSnapshotDescriptor::new(
        generation.to_string(),
        0,
        projection_digest.clone(),
        ContentDigest::from_bytes(format!("fulltext:{generation}").as_bytes()),
    );
    let vector = VectorSnapshotDescriptor::disabled(
        generation.to_string(),
        0,
        projection_digest,
        SearchProjectionDigest::from_pairs::<_, &str, &str>([]).unwrap(),
        ContentDigest::from_bytes(format!("vector:{generation}").as_bytes()),
    );
    session
        .validate_search_publication(&SearchPublicationValidation {
            generation,
            fulltext: &fulltext,
            vector: &vector,
            now,
        })
        .unwrap();
    let commit = SearchPublicationCommit {
        generation,
        terminal_documents: &[],
        projections: &[],
        projected_documents: &[],
        vector_coverage: &[],
        now,
    };
    let outcome = if let Some(barrier) = migration_barrier {
        session
            .commit_migration_rebuild_search_publication(&commit, barrier)
            .unwrap()
    } else {
        session.commit_search_publication(&commit).unwrap()
    };
    assert_eq!(outcome, SearchPublicationOutcome::Applied);
}

fn started(outcome: ArtifactRepairAttemptAcquire) -> ArtifactRepairAttempt {
    match outcome {
        ArtifactRepairAttemptAcquire::Started(attempt) => attempt,
        other => panic!("expected a started attempt, got {other:?}"),
    }
}

fn timestamp(seconds: i64) -> UnixTimestamp {
    UnixTimestamp::from_unix_seconds(seconds)
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

use std::collections::BTreeSet;
use std::fs;
#[cfg(unix)]
use std::os::unix::fs::symlink;
use std::sync::mpsc;
use std::thread;

use index_fulltext::{FullTextIndex, IndexDocument, SnapshotReadLease};
use index_vector::{VectorDocument, VectorModelContract, VectorSnapshotRoot, VectorSnapshotStore};
use meta_store::{
    ActiveSearchProjection, ArtifactRepairAttempt, ArtifactRepairAttemptAcquire,
    ArtifactRepairAttemptErrorKind, ArtifactRepairAttemptPhase, ArtifactRepairKey, ContentDigest,
    DataDirectoryOwnerAcquisition, DataDirectoryOwnerLease, FullTextSnapshotDescriptor,
    ImportProcessingContract, OwnedMetaStore, SearchProjectionDigest, SearchProjectionServiceState,
    SearchProjectionState, SearchProjectionTransitionOutcome, SearchPublicationCommit,
    SearchPublicationDraft, SearchPublicationOutcome, SearchPublicationSession,
    SearchPublicationState, SearchPublicationValidation, SearchRepairReason, UnixTimestamp,
    VectorSnapshotDescriptor, CLASSIFIER_EPOCH,
};
use tempfile::tempdir;

use super::{
    reconcile_search_artifacts, reconcile_search_artifacts_for_offline_mutation,
    settle_artifact_rebuild_failure,
};
use crate::search_artifact_cache::CurrentImportCacheMode;
use crate::search_artifacts::write_incremental_search_artifacts_for_test;
use crate::{
    finalize_migration_rebuild, ImportPipelineError, ImportPipelineErrorClass,
    ImportResourcePolicy, PipelineRunControl, SearchPublicationVectorization,
};

fn ready_empty_store(data_dir: &std::path::Path) -> OwnedMetaStore {
    let owner = match DataDirectoryOwnerLease::try_acquire(data_dir).unwrap() {
        DataDirectoryOwnerAcquisition::Acquired(owner) => owner,
        DataDirectoryOwnerAcquisition::Contended => panic!("synthetic data dir is owned"),
    };
    let store = owner.open_store().unwrap();
    store.run_migrations().unwrap();
    let contract = ImportProcessingContract::new(
        "artifact-repair-parser-v1",
        "artifact-repair-ocr-v1",
        "artifact-repair-schema-v28",
        CLASSIFIER_EPOCH,
    )
    .unwrap();
    let now = UnixTimestamp::from_unix_seconds(1_700_000_000);
    store
        .activate_migration_rebuild_contract(&contract, now)
        .unwrap();
    finalize_migration_rebuild(
        &store,
        now,
        &contract,
        &SearchPublicationVectorization::default(),
        &PipelineRunControl::default(),
    )
    .unwrap();
    assert_eq!(
        store.search_projection_state().unwrap().service_state,
        SearchProjectionServiceState::Ready
    );
    store
}

#[test]
fn maintenance_defers_when_a_foreground_publication_owns_the_session() {
    let directory = tempdir().unwrap();
    let data_dir = directory.path().join("data");
    let owner = match DataDirectoryOwnerLease::try_acquire(&data_dir).unwrap() {
        DataDirectoryOwnerAcquisition::Acquired(owner) => owner,
        DataDirectoryOwnerAcquisition::Contended => panic!("synthetic data dir is owned"),
    };
    let store = owner.open_store().unwrap();
    store.run_migrations().unwrap();
    let holder_store = store.open_sibling().unwrap();
    let (acquired_sender, acquired_receiver) = mpsc::sync_channel(1);
    let (release_sender, release_receiver) = mpsc::sync_channel(1);

    thread::scope(|scope| {
        scope.spawn(move || {
            let _publication_session = holder_store.wait_for_search_publication_session().unwrap();
            acquired_sender.send(()).unwrap();
            release_receiver.recv().unwrap();
        });
        acquired_receiver.recv().unwrap();

        let summary = reconcile_search_artifacts(
            &store,
            UnixTimestamp::from_unix_seconds(1_700_000_000),
            &SearchPublicationVectorization::default(),
            &crate::PipelineRunControl::default(),
        )
        .unwrap();

        assert_eq!(summary, Default::default());
        release_sender.send(()).unwrap();
    });
}

#[test]
fn restart_reconciliation_retires_exact_interrupted_fulltext_and_vector_before_gc() {
    let directory = tempdir().unwrap();
    let data_dir = directory.path().join("data");
    let store = ready_empty_store(&data_dir);
    let base = store.search_projection_state().unwrap();
    let generation = leave_validated_interrupted_publication(
        &store,
        UnixTimestamp::from_unix_seconds(1_700_000_010),
    );
    let obsolete_generation = "restart-obsolete-after-exact-cleanup";
    publish_empty_artifact_pair(&data_dir, obsolete_generation);
    assert_exact_generation_present(&data_dir, &generation);
    assert_exact_generation_present(&data_dir, obsolete_generation);

    let summary = reconcile_search_artifacts(
        &store,
        UnixTimestamp::from_unix_seconds(1_700_000_011),
        &SearchPublicationVectorization::default(),
        &PipelineRunControl::default(),
    )
    .unwrap();

    assert_eq!(summary.interrupted_publications_abandoned, 1);
    assert_eq!(
        store
            .search_publication(&generation)
            .unwrap()
            .unwrap()
            .state,
        SearchPublicationState::Abandoned
    );
    assert_exact_generation_absent(&data_dir, &generation);
    assert_exact_generation_absent(&data_dir, obsolete_generation);
    assert_eq!(store.search_projection_state().unwrap(), base);
}

#[test]
fn restart_reconciliation_retires_preparing_fulltext_and_accepts_missing_vector() {
    let directory = tempdir().unwrap();
    let data_dir = directory.path().join("data");
    let store = ready_empty_store(&data_dir);
    let base = store.search_projection_state().unwrap();
    let generation = "restart-preparing-fulltext-only";
    let session = store.wait_for_search_publication_session().unwrap();
    assert_eq!(
        session
            .begin_search_publication(&SearchPublicationDraft {
                generation: generation.to_string(),
                base_generation: base.generation.clone(),
                expected_visible_epoch: base.visible_epoch,
                classifier_epoch: CLASSIFIER_EPOCH.to_string(),
                projection_digest: SearchProjectionDigest::from_pairs::<_, &str, &str>([]).unwrap(),
                now: UnixTimestamp::from_unix_seconds(1_700_000_012),
            })
            .unwrap(),
        SearchPublicationOutcome::Applied
    );
    index_fulltext::publish_snapshot(
        &data_dir.join("search-index"),
        generation,
        std::iter::empty::<IndexDocument>(),
    )
    .unwrap();
    drop(session);
    assert!(data_dir
        .join("search-index/snapshots")
        .join(generation)
        .is_dir());
    assert!(!data_dir
        .join("vector-index/snapshots")
        .join(generation)
        .exists());

    let summary = reconcile_search_artifacts(
        &store,
        UnixTimestamp::from_unix_seconds(1_700_000_013),
        &SearchPublicationVectorization::default(),
        &PipelineRunControl::default(),
    )
    .unwrap();

    assert_eq!(summary.interrupted_publications_abandoned, 1);
    assert_eq!(
        store.search_publication(generation).unwrap().unwrap().state,
        SearchPublicationState::Abandoned
    );
    assert_exact_generation_absent(&data_dir, generation);
    assert_eq!(store.search_projection_state().unwrap(), base);
}

#[test]
fn restart_reconciliation_deferred_fulltext_retirement_blocks_current_base() {
    let directory = tempdir().unwrap();
    let data_dir = directory.path().join("data");
    let store = ready_empty_store(&data_dir);
    let base = store.search_projection_state().unwrap();
    let generation = leave_validated_interrupted_publication(
        &store,
        UnixTimestamp::from_unix_seconds(1_700_000_014),
    );
    let unrelated_generation = "restart-unrelated-before-failed-exact-cleanup";
    publish_empty_artifact_pair(&data_dir, unrelated_generation);
    let fulltext_root = data_dir.join("search-index");
    let lease = SnapshotReadLease::acquire(&fulltext_root)
        .unwrap()
        .expect("interrupted full-text root must be readable");
    let reader = FullTextIndex::open_snapshot_with_lease(&fulltext_root, &generation, lease)
        .unwrap()
        .expect("interrupted generation must be readable");

    let error = reconcile_search_artifacts(
        &store,
        UnixTimestamp::from_unix_seconds(1_700_000_015),
        &SearchPublicationVectorization::default(),
        &PipelineRunControl::default(),
    )
    .unwrap_err();

    assert_eq!(error.class(), ImportPipelineErrorClass::ArtifactRetirement);
    assert!(!error.is_retryable());
    assert_abandoned_and_blocked(&store, &generation, &base);
    assert_exact_generation_present(&data_dir, &generation);
    assert_exact_generation_present(&data_dir, unrelated_generation);
    drop(reader);

    let replay = reconcile_search_artifacts(
        &store,
        UnixTimestamp::from_unix_seconds(1_700_000_016),
        &SearchPublicationVectorization::default(),
        &PipelineRunControl::default(),
    )
    .unwrap();

    assert_eq!(replay.interrupted_publications_abandoned, 1);
    assert!(store
        .pending_search_publication_retirements()
        .unwrap()
        .is_empty());
    assert_exact_generation_absent(&data_dir, &generation);
    assert_exact_generation_present(&data_dir, unrelated_generation);
    assert_abandoned_and_blocked(&store, &generation, &base);
}

#[test]
fn restart_reconciliation_partial_vector_retirement_blocks_current_base() {
    let directory = tempdir().unwrap();
    let data_dir = directory.path().join("data");
    let store = ready_empty_store(&data_dir);
    let base = store.search_projection_state().unwrap();
    let generation = leave_validated_interrupted_publication(
        &store,
        UnixTimestamp::from_unix_seconds(1_700_000_016),
    );
    let vector_root = VectorSnapshotRoot::new(data_dir.join("vector-index")).unwrap();
    let lease = vector_root.acquire_read_lease().unwrap();
    let reader = vector_root
        .open_generation_with_lease(&generation, &VectorModelContract::Disabled, lease)
        .unwrap();

    let error = reconcile_search_artifacts(
        &store,
        UnixTimestamp::from_unix_seconds(1_700_000_017),
        &SearchPublicationVectorization::default(),
        &PipelineRunControl::default(),
    )
    .unwrap_err();

    assert_eq!(error.class(), ImportPipelineErrorClass::ArtifactRetirement);
    assert!(!error.is_retryable());
    assert_abandoned_and_blocked(&store, &generation, &base);
    assert!(!data_dir
        .join("search-index/snapshots")
        .join(&generation)
        .exists());
    assert!(data_dir
        .join("vector-index/snapshots")
        .join(&generation)
        .is_dir());
    drop(reader);
}

#[test]
fn restart_reconciliation_missing_generation_pin_blocks_current_base() {
    let directory = tempdir().unwrap();
    let data_dir = directory.path().join("data");
    let store = ready_empty_store(&data_dir);
    let base = store.search_projection_state().unwrap();
    let generation = leave_validated_interrupted_publication(
        &store,
        UnixTimestamp::from_unix_seconds(1_700_000_018),
    );
    fs::remove_file(
        data_dir
            .join("search-index/generation-pins")
            .join(format!("{generation}.lock")),
    )
    .unwrap();

    let error = reconcile_search_artifacts(
        &store,
        UnixTimestamp::from_unix_seconds(1_700_000_019),
        &SearchPublicationVectorization::default(),
        &PipelineRunControl::default(),
    )
    .unwrap_err();

    assert_eq!(error.class(), ImportPipelineErrorClass::ArtifactRetirement);
    assert!(!error.is_retryable());
    assert_abandoned_and_blocked(&store, &generation, &base);
    assert!(data_dir
        .join("search-index/snapshots")
        .join(generation)
        .is_dir());
}

#[cfg(unix)]
#[test]
fn unsafe_fulltext_root_blocks_the_exact_artifact_repair_without_worker_failure() {
    let directory = tempdir().unwrap();
    let data_dir = directory.path().join("data");
    let store = ready_empty_store(&data_dir);
    let before = store.search_projection_state().unwrap();
    let snapshots = data_dir.join("search-index/snapshots");
    let real_snapshots = data_dir.join("search-index/snapshots-real");
    fs::rename(&snapshots, &real_snapshots).unwrap();
    symlink(&real_snapshots, &snapshots).unwrap();

    let summary = reconcile_search_artifacts(
        &store,
        UnixTimestamp::from_unix_seconds(1_700_000_001),
        &SearchPublicationVectorization::default(),
        &PipelineRunControl::default(),
    )
    .unwrap();

    assert!(!summary.active_generation_rebuilt);
    let blocked = store.search_projection_state().unwrap();
    assert_eq!(
        blocked.service_state,
        SearchProjectionServiceState::RepairBlocked
    );
    assert_eq!(
        blocked.repair_reason,
        Some(SearchRepairReason::RuntimeInvariant)
    );
    assert_eq!(blocked.generation, before.generation);
    assert_eq!(blocked.visible_epoch, before.visible_epoch);

    let next_tick = reconcile_search_artifacts(
        &store,
        UnixTimestamp::from_unix_seconds(1_700_000_002),
        &SearchPublicationVectorization::default(),
        &PipelineRunControl::default(),
    )
    .unwrap();
    assert!(!next_tick.active_generation_rebuilt);
    assert_eq!(store.search_projection_state().unwrap(), blocked);
}

#[cfg(unix)]
#[test]
fn offline_mutation_fails_closed_after_artifact_repair_blocks() {
    let directory = tempdir().unwrap();
    let data_dir = directory.path().join("data");
    let store = ready_empty_store(&data_dir);
    let before = store.search_projection_state().unwrap();
    let snapshots = data_dir.join("search-index/snapshots");
    let real_snapshots = data_dir.join("search-index/snapshots-real");
    fs::rename(&snapshots, &real_snapshots).unwrap();
    symlink(&real_snapshots, &snapshots).unwrap();

    let error = reconcile_search_artifacts_for_offline_mutation(
        &store,
        UnixTimestamp::from_unix_seconds(1_700_000_001),
        &SearchPublicationVectorization::default(),
        &PipelineRunControl::default(),
    )
    .unwrap_err();

    assert_eq!(error.class(), ImportPipelineErrorClass::Repairing);
    let blocked = store.search_projection_state().unwrap();
    assert_eq!(
        blocked.service_state,
        SearchProjectionServiceState::RepairBlocked
    );
    assert_eq!(
        blocked.repair_reason,
        Some(SearchRepairReason::RuntimeInvariant)
    );
    assert_eq!(blocked.generation, before.generation);
    assert_eq!(blocked.visible_epoch, before.visible_epoch);

    let replay_error = reconcile_search_artifacts_for_offline_mutation(
        &store,
        UnixTimestamp::from_unix_seconds(1_700_000_002),
        &SearchPublicationVectorization::default(),
        &PipelineRunControl::default(),
    )
    .unwrap_err();
    assert_eq!(
        replay_error.class(),
        ImportPipelineErrorClass::ArtifactRetirement
    );
}

#[test]
fn stale_nonretryable_rebuild_failure_cannot_block_a_newer_ready_head() {
    let directory = tempdir().unwrap();
    let data_dir = directory.path().join("data");
    let store = ready_empty_store(&data_dir);
    let ready = store.search_projection_state().unwrap();
    let (mut session, stale_attempt) =
        begin_artifact_attempt(&store, UnixTimestamp::from_unix_seconds(1_700_000_001));
    publish_empty_successor(
        &session,
        "newer-ready-generation",
        ready.generation.as_deref().unwrap(),
        ready.visible_epoch,
        UnixTimestamp::from_unix_seconds(1_700_000_002),
    );

    settle_artifact_rebuild_failure(
        &mut session,
        &stale_attempt,
        UnixTimestamp::from_unix_seconds(1_700_000_003),
        ImportPipelineError::vector(index_vector::VectorIndexError::StorageLayoutInvalid),
    )
    .unwrap();

    let current = store.search_projection_state().unwrap();
    assert_eq!(current.service_state, SearchProjectionServiceState::Ready);
    assert_eq!(
        current.generation.as_deref(),
        Some("newer-ready-generation")
    );
    assert_eq!(current.visible_epoch, ready.visible_epoch + 1);
    assert_eq!(store.artifact_repair_context().unwrap(), None);
    assert_eq!(store.artifact_repair_attempt_state().unwrap(), None);
}

#[test]
fn retryable_failure_is_scheduled_without_worker_failure_and_lifecycle_errors_restore_budget() {
    for error in [
        ImportPipelineError::vector(index_vector::VectorIndexError::Storage),
        ImportPipelineError::cancelled(),
        ImportPipelineError::interrupted(),
    ] {
        let directory = tempdir().unwrap();
        let data_dir = directory.path().join("data");
        let store = ready_empty_store(&data_dir);
        let ready = store.search_projection_state().unwrap();
        let (mut session, attempt) =
            begin_artifact_attempt(&store, UnixTimestamp::from_unix_seconds(1_700_000_001));
        let expected_class = error.class();

        let returned = settle_artifact_rebuild_failure(
            &mut session,
            &attempt,
            UnixTimestamp::from_unix_seconds(1_700_000_002),
            error,
        );
        if expected_class == ImportPipelineErrorClass::VectorStorage {
            returned.unwrap();
            let retry = store.artifact_repair_attempt_state().unwrap().unwrap();
            assert_eq!(retry.attempt_count, 1);
            assert_eq!(retry.phase, ArtifactRepairAttemptPhase::RetryWait);
            assert_eq!(
                retry.next_retry_at,
                Some(UnixTimestamp::from_unix_seconds(1_700_000_003))
            );
            let context = store.artifact_repair_context().unwrap().unwrap();
            let key = ArtifactRepairKey::new(
                context.generation,
                context.publication_fingerprint,
                context.visible_epoch,
            );
            assert_eq!(
                session
                    .acquire_artifact_repair_attempt(
                        &key,
                        UnixTimestamp::from_unix_seconds(1_700_000_002),
                    )
                    .unwrap(),
                ArtifactRepairAttemptAcquire::NotDue
            );
            let second = match session
                .acquire_artifact_repair_attempt(
                    &key,
                    UnixTimestamp::from_unix_seconds(1_700_000_003),
                )
                .unwrap()
            {
                ArtifactRepairAttemptAcquire::Started(attempt) => attempt,
                other => panic!("expected due second attempt, got {other:?}"),
            };
            assert_eq!(
                store
                    .artifact_repair_attempt_state()
                    .unwrap()
                    .unwrap()
                    .attempt_count,
                2
            );
            session.cancel_artifact_repair_attempt(&second).unwrap();
        } else {
            assert_eq!(returned.unwrap_err().class(), expected_class);
            assert_eq!(store.artifact_repair_attempt_state().unwrap(), None);
        }
        let repairing = store.search_projection_state().unwrap();
        assert_eq!(
            repairing.service_state,
            SearchProjectionServiceState::Repairing
        );
        assert_eq!(
            repairing.repair_reason,
            Some(SearchRepairReason::ArtifactUnavailable)
        );
        assert_eq!(repairing.generation, ready.generation);
        assert_eq!(repairing.visible_epoch, ready.visible_epoch);
    }
}

#[test]
fn repair_attempt_records_the_exact_publication_contention_cause() {
    let cases = [
        (
            ImportPipelineError::index(index_fulltext::FullTextError::PublicationBusy),
            ArtifactRepairAttemptErrorKind::FullTextPublicationBusy,
            ImportPipelineErrorClass::FullText,
        ),
        (
            ImportPipelineError::vector(index_vector::VectorIndexError::PublicationBusy),
            ArtifactRepairAttemptErrorKind::VectorPublicationBusy,
            ImportPipelineErrorClass::VectorStorage,
        ),
    ];

    for (error, expected_kind, expected_public_class) in cases {
        let directory = tempdir().unwrap();
        let data_dir = directory.path().join("data");
        let store = ready_empty_store(&data_dir);
        let (mut session, attempt) =
            begin_artifact_attempt(&store, UnixTimestamp::from_unix_seconds(1_700_000_001));
        assert_eq!(error.class(), expected_public_class);

        settle_artifact_rebuild_failure(
            &mut session,
            &attempt,
            UnixTimestamp::from_unix_seconds(1_700_000_002),
            error,
        )
        .unwrap();

        let retry = store.artifact_repair_attempt_state().unwrap().unwrap();
        assert_eq!(retry.phase, ArtifactRepairAttemptPhase::RetryWait);
        assert_eq!(retry.last_error_kind, Some(expected_kind));
    }
}

#[test]
fn failed_generation_cleanup_blocks_the_first_attempt_without_becoming_retryable() {
    let cases = [
        (
            ImportPipelineError::fulltext_artifact_retirement(),
            ArtifactRepairAttemptErrorKind::FullTextFailure,
        ),
        (
            ImportPipelineError::vector_artifact_retirement(),
            ArtifactRepairAttemptErrorKind::VectorFailure,
        ),
    ];

    for (error, expected_kind) in cases {
        let directory = tempdir().unwrap();
        let data_dir = directory.path().join("data");
        let store = ready_empty_store(&data_dir);
        let (mut session, attempt) =
            begin_artifact_attempt(&store, UnixTimestamp::from_unix_seconds(1_700_000_001));
        assert_eq!(error.class(), ImportPipelineErrorClass::ArtifactRetirement);
        assert!(!error.is_retryable());

        settle_artifact_rebuild_failure(
            &mut session,
            &attempt,
            UnixTimestamp::from_unix_seconds(1_700_000_002),
            error,
        )
        .unwrap();

        let blocked = store.search_projection_state().unwrap();
        assert_eq!(
            blocked.service_state,
            SearchProjectionServiceState::RepairBlocked
        );
        assert_eq!(
            blocked.repair_reason,
            Some(SearchRepairReason::RuntimeInvariant)
        );
        let attempt_state = store.artifact_repair_attempt_state().unwrap().unwrap();
        assert_eq!(attempt_state.attempt_count, 1);
        assert_eq!(attempt_state.last_error_kind, Some(expected_kind));
        let context = store.artifact_repair_context().unwrap().unwrap();
        let key = ArtifactRepairKey::new(
            context.generation,
            context.publication_fingerprint,
            context.visible_epoch,
        );
        assert_eq!(
            session
                .acquire_artifact_repair_attempt(
                    &key,
                    UnixTimestamp::from_unix_seconds(1_700_000_100),
                )
                .unwrap(),
            ArtifactRepairAttemptAcquire::Superseded
        );
        assert_eq!(
            store
                .artifact_repair_attempt_state()
                .unwrap()
                .unwrap()
                .attempt_count,
            1
        );
    }
}

#[test]
fn retry_backoff_starts_when_the_failed_attempt_finishes() {
    let directory = tempdir().unwrap();
    let data_dir = directory.path().join("data");
    let store = ready_empty_store(&data_dir);
    let started_at = UnixTimestamp::from_unix_seconds(1_700_000_001);
    let finished_at = UnixTimestamp::from_unix_seconds(1_700_000_061);
    let (mut session, attempt) = begin_artifact_attempt(&store, started_at);

    settle_artifact_rebuild_failure(
        &mut session,
        &attempt,
        finished_at,
        ImportPipelineError::vector(index_vector::VectorIndexError::Storage),
    )
    .unwrap();

    let retry = store.artifact_repair_attempt_state().unwrap().unwrap();
    assert_eq!(retry.attempt_count, 1);
    assert_eq!(retry.phase, ArtifactRepairAttemptPhase::RetryWait);
    assert_eq!(
        retry.next_retry_at,
        Some(UnixTimestamp::from_unix_seconds(1_700_000_062))
    );
    let context = store.artifact_repair_context().unwrap().unwrap();
    let key = ArtifactRepairKey::new(
        context.generation,
        context.publication_fingerprint,
        context.visible_epoch,
    );
    assert_eq!(
        session
            .acquire_artifact_repair_attempt(&key, finished_at)
            .unwrap(),
        ArtifactRepairAttemptAcquire::NotDue
    );
}

fn begin_artifact_attempt(
    store: &OwnedMetaStore,
    now: UnixTimestamp,
) -> (SearchPublicationSession, ArtifactRepairAttempt) {
    let ready = store.search_projection_state().unwrap();
    assert_eq!(
        store
            .begin_artifact_repair(
                ready.generation.as_deref().unwrap(),
                ready.visible_epoch,
                now,
            )
            .unwrap(),
        SearchProjectionTransitionOutcome::Applied
    );
    let context = store.artifact_repair_context().unwrap().unwrap();
    let key = ArtifactRepairKey::new(
        context.generation,
        context.publication_fingerprint,
        context.visible_epoch,
    );
    let mut session = store.wait_for_search_publication_session().unwrap();
    let attempt = match session.acquire_artifact_repair_attempt(&key, now).unwrap() {
        ArtifactRepairAttemptAcquire::Started(attempt) => attempt,
        other => panic!("expected a started attempt, got {other:?}"),
    };
    (session, attempt)
}

fn publish_empty_successor(
    session: &SearchPublicationSession,
    generation: &str,
    base_generation: &str,
    expected_visible_epoch: u64,
    now: UnixTimestamp,
) {
    let projection_digest = SearchProjectionDigest::from_pairs::<_, &str, &str>([]).unwrap();
    assert_eq!(
        session
            .begin_search_publication(&SearchPublicationDraft {
                generation: generation.to_string(),
                base_generation: Some(base_generation.to_string()),
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
    assert_eq!(
        session
            .commit_search_publication(&SearchPublicationCommit {
                generation,
                terminal_documents: &[],
                projections: &[],
                projected_documents: &[],
                vector_coverage: &[],
                now,
            })
            .unwrap(),
        SearchPublicationOutcome::Applied
    );
}

fn leave_validated_interrupted_publication(store: &OwnedMetaStore, now: UnixTimestamp) -> String {
    let session = store.wait_for_search_publication_session().unwrap();
    let publication = write_incremental_search_artifacts_for_test(
        &session,
        now,
        CLASSIFIER_EPOCH,
        Vec::new(),
        &BTreeSet::new(),
        0,
        0,
        None,
        CurrentImportCacheMode::Retain,
        None,
        None,
        None,
        ImportResourcePolicy::detect().index_writer_heap_bytes,
        &SearchPublicationVectorization::default(),
    )
    .unwrap();
    let generation = publication.generation().to_string();
    drop(publication);
    drop(session);
    generation
}

fn publish_empty_artifact_pair(data_dir: &std::path::Path, generation: &str) {
    index_fulltext::publish_snapshot(
        &data_dir.join("search-index"),
        generation,
        std::iter::empty::<IndexDocument>(),
    )
    .unwrap();
    VectorSnapshotStore::new(data_dir.join("vector-index"), VectorModelContract::Disabled)
        .unwrap()
        .publish_generation(
            generation,
            std::iter::empty::<ActiveSearchProjection>(),
            std::iter::empty::<VectorDocument>(),
        )
        .unwrap();
}

fn assert_abandoned_and_blocked(
    store: &OwnedMetaStore,
    generation: &str,
    expected_head: &SearchProjectionState,
) {
    assert_eq!(
        store.search_publication(generation).unwrap().unwrap().state,
        SearchPublicationState::Abandoned
    );
    let blocked = store.search_projection_state().unwrap();
    assert_eq!(
        blocked.service_state,
        SearchProjectionServiceState::RepairBlocked
    );
    assert_eq!(
        blocked.repair_reason,
        Some(SearchRepairReason::RuntimeInvariant)
    );
    assert_eq!(blocked.generation, expected_head.generation);
    assert_eq!(blocked.visible_epoch, expected_head.visible_epoch);
}

fn assert_exact_generation_present(data_dir: &std::path::Path, generation: &str) {
    for relative in ["search-index", "vector-index"] {
        let root = data_dir.join(relative);
        assert!(root.join("snapshots").join(generation).is_dir());
        assert!(root
            .join("generation-pins")
            .join(format!("{generation}.lock"))
            .is_file());
    }
}

fn assert_exact_generation_absent(data_dir: &std::path::Path, generation: &str) {
    for relative in ["search-index", "vector-index"] {
        let root = data_dir.join(relative);
        assert!(!root.join("snapshots").join(generation).exists());
        assert!(!root
            .join("generation-pins")
            .join(format!("{generation}.lock"))
            .exists());
        assert_no_generation_entry(&root.join("snapshots"), generation);
        assert_no_generation_entry(&root.join("staging"), generation);
    }
}

fn assert_no_generation_entry(root: &std::path::Path, generation: &str) {
    let entries = match fs::read_dir(root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return,
        Err(error) => panic!("failed to inspect artifact root: {error}"),
    };
    for entry in entries {
        let name = entry.unwrap().file_name();
        assert!(
            !name.to_string_lossy().contains(generation),
            "exact generation artifact remains"
        );
    }
}

//! Concurrency contract tests for migration-rebuild publication ownership.

use std::sync::Barrier;
use std::thread;

use meta_store::{
    DataDirectoryOwnerAcquisition, DataDirectoryOwnerLease, ImportProcessingContract,
    MigrationRebuildPublicationAttemptPhase, MigrationRebuildPublicationErrorClass,
    SearchProjectionServiceState, SearchPublicationState, SearchRepairReason, UnixTimestamp,
    CLASSIFIER_EPOCH,
};
use tempfile::tempdir;

use super::{
    finalize_migration_rebuild_with_fault, MigrationPublicationCommitObserver,
    MigrationPublicationFault, MigrationPublicationTestGate,
};
use crate::{ImportPipelineErrorClass, PipelineRunControl, SearchPublicationVectorization};

#[test]
fn competing_publisher_fails_closed_without_advancing_the_active_attempt() {
    let directory = tempdir().unwrap();
    let data_dir = directory.path().join("data");
    let owner = match DataDirectoryOwnerLease::try_acquire(&data_dir).unwrap() {
        DataDirectoryOwnerAcquisition::Acquired(owner) => owner,
        DataDirectoryOwnerAcquisition::Contended => panic!("synthetic data dir is owned"),
    };
    let setup = owner.open_store().unwrap();
    setup.run_migrations().unwrap();
    let contract = ImportProcessingContract::new(
        "lock-order-parser-v1",
        "lock-order-ocr-v1",
        "lock-order-schema-v28",
        CLASSIFIER_EPOCH,
    )
    .unwrap();
    setup
        .activate_migration_rebuild_contract(&contract, UnixTimestamp::from_unix_seconds(100))
        .unwrap();

    let first_store = setup.open_sibling().unwrap();
    let second_store = setup.open_sibling().unwrap();
    let observer = setup.open_sibling().unwrap();
    drop(setup);
    let first_gate: &'static MigrationPublicationTestGate =
        Box::leak(Box::new(MigrationPublicationTestGate {
            entered: Barrier::new(2),
            release: Barrier::new(2),
        }));
    let second_before_session: &'static Barrier = Box::leak(Box::new(Barrier::new(2)));

    let first_contract = contract.clone();
    let first = thread::spawn(move || {
        finalize_migration_rebuild_with_fault(
            &first_store,
            UnixTimestamp::from_unix_seconds(100),
            &first_contract,
            &SearchPublicationVectorization::default(),
            &crate::PipelineRunControl::default(),
            MigrationPublicationFault::HoldBeforeFullText(first_gate),
        )
    });
    first_gate.entered.wait();
    let running = observer
        .migration_rebuild_publication_attempt_state()
        .unwrap()
        .unwrap();
    assert_eq!(running.attempt_count, 1);
    assert_eq!(
        running.phase,
        MigrationRebuildPublicationAttemptPhase::Running
    );

    let second_contract = contract.clone();
    let second = thread::spawn(move || {
        finalize_migration_rebuild_with_fault(
            &second_store,
            UnixTimestamp::from_unix_seconds(200),
            &second_contract,
            &SearchPublicationVectorization::default(),
            &crate::PipelineRunControl::default(),
            MigrationPublicationFault::SignalBeforePublicationSession(second_before_session),
        )
    });
    second_before_session.wait();
    let second_error = second.join().unwrap().unwrap_err();
    assert_eq!(
        second_error.metadata_class_label(),
        Some("migration_ownership_required")
    );

    let still_running = observer
        .migration_rebuild_publication_attempt_state()
        .unwrap()
        .unwrap();
    assert_eq!(still_running.attempt_count, 1);
    assert_eq!(
        still_running.phase,
        MigrationRebuildPublicationAttemptPhase::Running
    );

    first_gate.release.wait();
    assert!(first.join().unwrap().is_err());
    assert_eq!(
        observer
            .migration_rebuild_publication_attempt_state()
            .unwrap()
            .unwrap()
            .attempt_count,
        1
    );
}

#[test]
fn lifecycle_cancellation_abandons_attempt_without_consuming_retry_budget() {
    let directory = tempdir().unwrap();
    let data_dir = directory.path().join("data");
    let owner = match DataDirectoryOwnerLease::try_acquire(&data_dir).unwrap() {
        DataDirectoryOwnerAcquisition::Acquired(owner) => owner,
        DataDirectoryOwnerAcquisition::Contended => panic!("synthetic data dir is owned"),
    };
    let setup = owner.open_store().unwrap();
    setup.run_migrations().unwrap();
    let contract = ImportProcessingContract::new(
        "cancel-parser-v1",
        "cancel-ocr-v1",
        "cancel-schema-v28",
        CLASSIFIER_EPOCH,
    )
    .unwrap();
    setup
        .activate_migration_rebuild_contract(&contract, UnixTimestamp::from_unix_seconds(100))
        .unwrap();

    let worker_store = setup.open_sibling().unwrap();
    let observer = setup.open_sibling().unwrap();
    let gate: &'static MigrationPublicationTestGate =
        Box::leak(Box::new(MigrationPublicationTestGate {
            entered: Barrier::new(2),
            release: Barrier::new(2),
        }));
    let control = PipelineRunControl::default();
    let worker_control = control.clone();
    let worker_contract = contract.clone();
    let worker = thread::spawn(move || {
        finalize_migration_rebuild_with_fault(
            &worker_store,
            UnixTimestamp::from_unix_seconds(100),
            &worker_contract,
            &SearchPublicationVectorization::default(),
            &worker_control,
            MigrationPublicationFault::HoldBeforeFullTextForCancellation(gate),
        )
    });

    gate.entered.wait();
    assert_eq!(
        observer
            .migration_rebuild_publication_attempt_state()
            .unwrap()
            .unwrap()
            .attempt_count,
        1
    );
    control.request_shutdown();
    gate.release.wait();
    let error = worker.join().unwrap().unwrap_err();
    assert_eq!(error.class(), ImportPipelineErrorClass::Interrupted);
    assert!(observer
        .migration_rebuild_publication_attempt_state()
        .unwrap()
        .is_none());

    let retry_error = finalize_migration_rebuild_with_fault(
        &observer,
        UnixTimestamp::from_unix_seconds(101),
        &contract,
        &SearchPublicationVectorization::default(),
        &PipelineRunControl::default(),
        MigrationPublicationFault::RetryableFullText,
    )
    .unwrap_err();
    assert_eq!(retry_error.class(), ImportPipelineErrorClass::FullText);
    assert_eq!(
        observer
            .migration_rebuild_publication_attempt_state()
            .unwrap()
            .unwrap()
            .attempt_count,
        1
    );
}

#[test]
fn publication_cancelled_class_abandons_attempt_without_consuming_retry_budget() {
    let directory = tempdir().unwrap();
    let data_dir = directory.path().join("data");
    let owner = match DataDirectoryOwnerLease::try_acquire(&data_dir).unwrap() {
        DataDirectoryOwnerAcquisition::Acquired(owner) => owner,
        DataDirectoryOwnerAcquisition::Contended => panic!("synthetic data dir is owned"),
    };
    let store = owner.open_store().unwrap();
    store.run_migrations().unwrap();
    let contract = ImportProcessingContract::new(
        "cancelled-class-parser-v1",
        "cancelled-class-ocr-v1",
        "cancelled-class-schema-v28",
        CLASSIFIER_EPOCH,
    )
    .unwrap();
    store
        .activate_migration_rebuild_contract(&contract, UnixTimestamp::from_unix_seconds(100))
        .unwrap();

    let error = finalize_migration_rebuild_with_fault(
        &store,
        UnixTimestamp::from_unix_seconds(100),
        &contract,
        &SearchPublicationVectorization::default(),
        &PipelineRunControl::default(),
        MigrationPublicationFault::CancelledPublication,
    )
    .unwrap_err();
    assert_eq!(error.class(), ImportPipelineErrorClass::Cancelled);
    assert!(store
        .migration_rebuild_publication_attempt_state()
        .unwrap()
        .is_none());

    let retry_error = finalize_migration_rebuild_with_fault(
        &store,
        UnixTimestamp::from_unix_seconds(101),
        &contract,
        &SearchPublicationVectorization::default(),
        &PipelineRunControl::default(),
        MigrationPublicationFault::RetryableFullText,
    )
    .unwrap_err();
    assert_eq!(retry_error.class(), ImportPipelineErrorClass::FullText);
    assert_eq!(
        store
            .migration_rebuild_publication_attempt_state()
            .unwrap()
            .unwrap()
            .attempt_count,
        1
    );
}

#[test]
fn migration_commit_supersession_retires_only_its_exact_generation() {
    let directory = tempdir().unwrap();
    let data_dir = directory.path().join("data");
    let unrelated_generation = "unrelated-obsolete-generation";
    let (store, contract) = migration_store(&data_dir, "migration-superseded");
    publish_unrelated_fulltext_generation(&data_dir, unrelated_generation);
    let observer: &'static MigrationPublicationCommitObserver =
        Box::leak(Box::new(MigrationPublicationCommitObserver {
            generation: std::sync::Mutex::new(None),
            fulltext_reader: std::sync::Mutex::new(None),
        }));

    let summary = finalize_migration_rebuild_with_fault(
        &store,
        UnixTimestamp::from_unix_seconds(300),
        &contract,
        &SearchPublicationVectorization::default(),
        &PipelineRunControl::default(),
        MigrationPublicationFault::CommitSuperseded(observer),
    )
    .unwrap();

    let generation = observer.generation.lock().unwrap().clone().unwrap();
    assert_eq!(
        store
            .search_publication(&generation)
            .unwrap()
            .unwrap()
            .state,
        SearchPublicationState::Abandoned
    );
    assert_exact_generation_absent(&data_dir, &generation);
    assert_fulltext_generation_present(&data_dir, unrelated_generation);
    assert_eq!(summary.fulltext_generations_removed, 0);
    assert_eq!(summary.vector_generations_removed, 0);
    assert!(store
        .migration_rebuild_publication_attempt_state()
        .unwrap()
        .is_none());
    let state = store.search_projection_state().unwrap();
    assert_eq!(state.service_state, SearchProjectionServiceState::Repairing);
    assert_eq!(
        state.repair_reason,
        Some(SearchRepairReason::MigrationRebuild)
    );
}

#[test]
fn migration_deferred_retirement_blocks_after_the_first_attempt() {
    let directory = tempdir().unwrap();
    let data_dir = directory.path().join("data");
    let (store, contract) = migration_store(&data_dir, "migration-retirement-deferred");
    let observer: &'static MigrationPublicationCommitObserver =
        Box::leak(Box::new(MigrationPublicationCommitObserver {
            generation: std::sync::Mutex::new(None),
            fulltext_reader: std::sync::Mutex::new(None),
        }));

    let error = finalize_migration_rebuild_with_fault(
        &store,
        UnixTimestamp::from_unix_seconds(350),
        &contract,
        &SearchPublicationVectorization::default(),
        &PipelineRunControl::default(),
        MigrationPublicationFault::CommitSupersededWithRetirementLease(observer),
    )
    .unwrap_err();

    assert_eq!(error.class(), ImportPipelineErrorClass::ArtifactRetirement);
    assert!(!error.is_retryable());
    let generation = observer.generation.lock().unwrap().clone().unwrap();
    assert_eq!(
        store
            .search_publication(&generation)
            .unwrap()
            .unwrap()
            .state,
        SearchPublicationState::Abandoned
    );
    assert_fulltext_generation_present(&data_dir, &generation);
    let attempt = store
        .migration_rebuild_publication_attempt_state()
        .unwrap()
        .unwrap();
    assert_eq!(attempt.attempt_count, 1);
    assert_eq!(
        attempt.phase,
        MigrationRebuildPublicationAttemptPhase::Terminal
    );
    assert_eq!(
        attempt.last_error_class,
        Some(MigrationRebuildPublicationErrorClass::Cleanup)
    );
    let projection = store.search_projection_state().unwrap();
    assert_eq!(
        projection.service_state,
        SearchProjectionServiceState::RepairBlocked
    );
    assert_eq!(
        projection.repair_reason,
        Some(SearchRepairReason::RuntimeInvariant)
    );

    drop(observer.fulltext_reader.lock().unwrap().take());
    let next = finalize_migration_rebuild_with_fault(
        &store,
        UnixTimestamp::from_unix_seconds(1_000),
        &contract,
        &SearchPublicationVectorization::default(),
        &PipelineRunControl::default(),
        MigrationPublicationFault::None,
    )
    .unwrap();
    assert_eq!(next.interrupted_publications_abandoned, 1);
    assert!(store
        .pending_search_publication_retirements()
        .unwrap()
        .is_empty());
    assert_exact_generation_absent(&data_dir, &generation);
    assert_eq!(
        store
            .migration_rebuild_publication_attempt_state()
            .unwrap()
            .unwrap()
            .attempt_count,
        1
    );
}

#[test]
fn retryable_migration_failure_preserves_unrelated_obsolete_generation() {
    let directory = tempdir().unwrap();
    let data_dir = directory.path().join("data");
    let unrelated_generation = "unrelated-retry-generation";
    let (store, contract) = migration_store(&data_dir, "migration-retry");
    publish_unrelated_fulltext_generation(&data_dir, unrelated_generation);

    let error = finalize_migration_rebuild_with_fault(
        &store,
        UnixTimestamp::from_unix_seconds(400),
        &contract,
        &SearchPublicationVectorization::default(),
        &PipelineRunControl::default(),
        MigrationPublicationFault::RetryableFullText,
    )
    .unwrap_err();

    assert_eq!(error.class(), ImportPipelineErrorClass::FullText);
    assert!(error.is_retryable());
    assert_fulltext_generation_present(&data_dir, unrelated_generation);
    let attempt = store
        .migration_rebuild_publication_attempt_state()
        .unwrap()
        .unwrap();
    assert_eq!(attempt.attempt_count, 1);
    assert_eq!(
        attempt.phase,
        MigrationRebuildPublicationAttemptPhase::RetryWait
    );
}

fn migration_store(
    data_dir: &std::path::Path,
    contract_prefix: &str,
) -> (meta_store::OwnedMetaStore, ImportProcessingContract) {
    let owner = match DataDirectoryOwnerLease::try_acquire(data_dir).unwrap() {
        DataDirectoryOwnerAcquisition::Acquired(owner) => owner,
        DataDirectoryOwnerAcquisition::Contended => panic!("synthetic data dir is owned"),
    };
    let store = owner.open_store().unwrap();
    store.run_migrations().unwrap();
    let contract = ImportProcessingContract::new(
        format!("{contract_prefix}-parser-v1"),
        format!("{contract_prefix}-ocr-v1"),
        format!("{contract_prefix}-schema-v29"),
        CLASSIFIER_EPOCH,
    )
    .unwrap();
    store
        .activate_migration_rebuild_contract(&contract, UnixTimestamp::from_unix_seconds(299))
        .unwrap();
    (store, contract)
}

fn publish_unrelated_fulltext_generation(data_dir: &std::path::Path, generation: &str) {
    std::fs::create_dir_all(data_dir).unwrap();
    index_fulltext::publish_trusted_redacted_snapshot_with_control(
        &data_dir.join("search-index"),
        generation,
        Vec::<index_fulltext::IndexDocument>::new(),
        index_fulltext::SnapshotPublishControl::disabled(),
    )
    .unwrap();
}

fn assert_fulltext_generation_present(data_dir: &std::path::Path, generation: &str) {
    let root = data_dir.join("search-index");
    assert!(root.join("snapshots").join(generation).is_dir());
    assert!(root
        .join("generation-pins")
        .join(format!("{generation}.lock"))
        .is_file());
}

fn assert_exact_generation_absent(data_dir: &std::path::Path, generation: &str) {
    for relative in ["search-index", "vector-index"] {
        let root = data_dir.join(relative);
        assert!(!root.join("snapshots").join(generation).exists());
        assert!(!root
            .join("generation-pins")
            .join(format!("{generation}.lock"))
            .exists());
        for candidate_root in [root.join("snapshots"), root.join("staging")] {
            let Ok(entries) = std::fs::read_dir(candidate_root) else {
                continue;
            };
            for entry in entries {
                assert!(!entry
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .contains(generation));
            }
        }
    }
}

//! Concurrency contract tests for migration-rebuild publication ownership.

use std::sync::Barrier;
use std::thread;

use meta_store::{
    DataDirectoryOwnerAcquisition, DataDirectoryOwnerLease, ImportProcessingContract,
    MigrationRebuildPublicationAttemptPhase, UnixTimestamp, CLASSIFIER_EPOCH,
};
use tempfile::tempdir;

use super::{
    finalize_migration_rebuild_with_fault, MigrationPublicationFault, MigrationPublicationTestGate,
};
use crate::SearchPublicationVectorization;

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

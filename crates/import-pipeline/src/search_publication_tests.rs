use std::collections::BTreeSet;
use std::fs;
use std::sync::atomic::{AtomicUsize, Ordering};

use index_fulltext::{FullTextGenerationRetirement, FullTextIndex, SnapshotReadLease};
use index_vector::{VectorGenerationRetirement, VectorModelContract, VectorSnapshotRoot};
use meta_store::{
    DataDirectoryOwnerAcquisition, DataDirectoryOwnerLease, ImportProcessingContract,
    OwnedMetaStore, SearchArtifactExpectation, SearchProjectionServiceState, SearchProjectionState,
    SearchPublicationOutcome, SearchPublicationRetirementPhase, SearchPublicationRetirementPlan,
    SearchPublicationSession, SearchPublicationState, SearchRepairReason, UnixTimestamp,
    CLASSIFIER_EPOCH,
};
use tempfile::tempdir;

use super::{
    replay_pending_search_publication_retirements, retire_abandoned_search_publication_generation,
};
use crate::search_artifact_cache::CurrentImportCacheMode;
use crate::search_artifacts::write_incremental_search_artifacts_for_test;
use crate::search_publication::{
    SearchPublicationDecision, SearchPublicationTransactionOutcome, SearchPublicationView,
};
use crate::search_publication_commit::{
    decide_search_publication_cancellable, decide_search_publication_with_for_test,
};
use crate::{
    ImportPipelineError, ImportPipelineErrorClass, ImportPipelineErrorKind, ImportResourcePolicy,
    PipelineRunControl, SearchPublicationVectorization,
};

#[test]
fn prepared_publication_owner_does_not_cross_production_module_boundaries() {
    for (module, source) in [
        ("search_artifacts", include_str!("search_artifacts.rs")),
        (
            "search_publication_commit",
            include_str!("search_publication_commit.rs"),
        ),
        (
            "publication_coordinator",
            include_str!("publication_coordinator.rs"),
        ),
        (
            "search_publication_ocr",
            include_str!("search_publication_ocr.rs"),
        ),
        ("ocr_publication", include_str!("ocr_publication.rs")),
        (
            "migration_publication",
            include_str!("index_recovery/migration_publication.rs"),
        ),
        (
            "reconciliation",
            include_str!("index_recovery/reconciliation.rs"),
        ),
    ] {
        let production_owner_mentions = source
            .match_indices("PreparedSearchPublication")
            .filter(|(offset, _)| {
                !source[*offset..].starts_with("PreparedSearchPublicationForTest")
            })
            .count();
        assert_eq!(
            production_owner_mentions, 0,
            "{module} must receive only a borrowed publication view"
        );
    }
}

#[test]
fn exact_physical_retirement_has_one_production_owner() {
    for (module, source) in [
        ("search_artifacts", include_str!("search_artifacts.rs")),
        (
            "search_publication_commit",
            include_str!("search_publication_commit.rs"),
        ),
        (
            "publication_coordinator",
            include_str!("publication_coordinator.rs"),
        ),
        (
            "migration_publication",
            include_str!("index_recovery/migration_publication.rs"),
        ),
        (
            "reconciliation",
            include_str!("index_recovery/reconciliation.rs"),
        ),
        (
            "artifact_maintenance",
            include_str!("index_recovery/artifact_maintenance.rs"),
        ),
    ] {
        for forbidden in [
            "try_retire_unpublished_generation",
            "begin_search_publication_retirement",
            "complete_search_publication_retirement_artifact",
        ] {
            assert!(
                !source.contains(forbidden),
                "{module} bypasses the durable retirement owner via {forbidden}"
            );
        }
    }
}

#[test]
fn metadata_commit_fault_abandons_and_retires_the_validated_generation() {
    let directory = tempdir().unwrap();
    let data_dir = directory.path().join("data");
    let store = ready_empty_store(&data_dir);
    let session = store.wait_for_search_publication_session().unwrap();
    let mut generation = None;
    let commit_calls = AtomicUsize::new(0);

    let error = match publish_empty_successor(
        &session,
        UnixTimestamp::from_unix_seconds(1_700_100_001),
        |publication| {
            generation = Some(publication.generation().to_string());
            assert_prepared_generation(&data_dir, &store, generation.as_deref().unwrap());
            decide_search_publication_with_for_test(
                publication,
                UnixTimestamp::from_unix_seconds(1_700_100_002),
                &[],
                |_, _| {
                    commit_calls.fetch_add(1, Ordering::SeqCst);
                    Err(metadata_commit_fault())
                },
            )
        },
    ) {
        Err(error) => error,
        Ok(_) => panic!("metadata commit fault unexpectedly succeeded"),
    };

    assert_eq!(commit_calls.load(Ordering::SeqCst), 1);
    assert_eq!(error.class(), ImportPipelineErrorClass::Metadata);
    assert!(error.is_retryable());
    assert_abandoned_and_retired(&data_dir, &store, generation.as_deref().unwrap());
}

#[test]
fn commit_supersession_abandons_and_retires_the_validated_generation() {
    let directory = tempdir().unwrap();
    let data_dir = directory.path().join("data");
    let store = ready_empty_store(&data_dir);
    let session = store.wait_for_search_publication_session().unwrap();
    let mut generation = None;

    let committed = publish_empty_successor(
        &session,
        UnixTimestamp::from_unix_seconds(1_700_100_011),
        |publication| {
            generation = Some(publication.generation().to_string());
            assert_prepared_generation(&data_dir, &store, generation.as_deref().unwrap());
            decide_search_publication_with_for_test(
                publication,
                UnixTimestamp::from_unix_seconds(1_700_100_012),
                &[],
                |_, _| Ok(SearchPublicationOutcome::Superseded),
            )
        },
    )
    .unwrap();

    assert!(matches!(
        committed,
        SearchPublicationTransactionOutcome::NotApplied
    ));
    assert_abandoned_and_retired(&data_dir, &store, generation.as_deref().unwrap());
}

#[test]
fn precommit_cancellation_abandons_and_retires_the_validated_generation() {
    let directory = tempdir().unwrap();
    let data_dir = directory.path().join("data");
    let store = ready_empty_store(&data_dir);
    let session = store.wait_for_search_publication_session().unwrap();
    let mut generation = None;
    let cancellation_checks = AtomicUsize::new(0);

    let error = match publish_empty_successor(
        &session,
        UnixTimestamp::from_unix_seconds(1_700_100_016),
        |publication| {
            generation = Some(publication.generation().to_string());
            assert_prepared_generation(&data_dir, &store, generation.as_deref().unwrap());
            decide_search_publication_cancellable(
                publication,
                UnixTimestamp::from_unix_seconds(1_700_100_017),
                &[],
                &|| {
                    cancellation_checks.fetch_add(1, Ordering::SeqCst);
                    Err(ImportPipelineError::cancelled())
                },
            )
        },
    ) {
        Err(error) => error,
        Ok(_) => panic!("precommit cancellation unexpectedly succeeded"),
    };

    assert_eq!(cancellation_checks.load(Ordering::SeqCst), 1);
    assert_eq!(error.class(), ImportPipelineErrorClass::Cancelled);
    assert_abandoned_and_retired(&data_dir, &store, generation.as_deref().unwrap());
}

#[test]
fn deferred_retirement_overrides_the_retryable_commit_fault_and_does_not_retry() {
    let directory = tempdir().unwrap();
    let data_dir = directory.path().join("data");
    let store = ready_empty_store(&data_dir);
    let base_head = store.search_projection_state().unwrap();
    let session = store.wait_for_search_publication_session().unwrap();
    let mut generation = None;
    let mut reader = None;
    let commit_calls = AtomicUsize::new(0);

    let error = match publish_empty_successor(
        &session,
        UnixTimestamp::from_unix_seconds(1_700_100_021),
        |publication| {
            generation = Some(publication.generation().to_string());
            let fulltext_root = data_dir.join("search-index");
            let lease = SnapshotReadLease::acquire(&fulltext_root)
                .unwrap()
                .expect("prepared full-text root must be readable");
            reader = Some(
                FullTextIndex::open_snapshot_with_lease(
                    &fulltext_root,
                    publication.generation(),
                    lease,
                )
                .unwrap()
                .expect("prepared generation must be readable"),
            );
            decide_search_publication_with_for_test(
                publication,
                UnixTimestamp::from_unix_seconds(1_700_100_022),
                &[],
                |_, _| {
                    commit_calls.fetch_add(1, Ordering::SeqCst);
                    Err(metadata_commit_fault())
                },
            )
        },
    ) {
        Err(error) => error,
        Ok(_) => panic!("metadata commit fault unexpectedly succeeded"),
    };

    assert_eq!(commit_calls.load(Ordering::SeqCst), 1);
    assert_eq!(error.class(), ImportPipelineErrorClass::ArtifactRetirement);
    assert!(!error.is_retryable());
    assert_blocked_head(&store, &base_head);
    assert_eq!(
        store
            .search_publication(generation.as_deref().unwrap())
            .unwrap()
            .unwrap()
            .state,
        SearchPublicationState::Abandoned
    );
    let generation = generation.unwrap();
    assert_exact_generation_present(&data_dir, &generation);

    drop(reader);
    retire_abandoned_search_publication_generation(
        &session,
        &generation,
        UnixTimestamp::from_unix_seconds(1_700_100_023),
    )
    .unwrap();
    assert_exact_generation_absent(&data_dir, &generation);
}

#[test]
fn partial_retirement_overrides_the_retryable_commit_fault() {
    let directory = tempdir().unwrap();
    let data_dir = directory.path().join("data");
    let store = ready_empty_store(&data_dir);
    let base_head = store.search_projection_state().unwrap();
    let session = store.wait_for_search_publication_session().unwrap();
    let mut generation = None;
    let mut reader = None;
    let commit_calls = AtomicUsize::new(0);

    let error = match publish_empty_successor(
        &session,
        UnixTimestamp::from_unix_seconds(1_700_100_031),
        |publication| {
            generation = Some(publication.generation().to_string());
            let vector_root = VectorSnapshotRoot::new(data_dir.join("vector-index")).unwrap();
            let lease = vector_root.acquire_read_lease().unwrap();
            reader = Some(
                vector_root
                    .open_generation_with_lease(
                        publication.generation(),
                        &VectorModelContract::Disabled,
                        lease,
                    )
                    .unwrap(),
            );
            decide_search_publication_with_for_test(
                publication,
                UnixTimestamp::from_unix_seconds(1_700_100_032),
                &[],
                |_, _| {
                    commit_calls.fetch_add(1, Ordering::SeqCst);
                    Err(metadata_commit_fault())
                },
            )
        },
    ) {
        Err(error) => error,
        Ok(_) => panic!("metadata commit fault unexpectedly succeeded"),
    };

    assert_eq!(commit_calls.load(Ordering::SeqCst), 1);
    assert_eq!(error.class(), ImportPipelineErrorClass::ArtifactRetirement);
    assert!(!error.is_retryable());
    assert_blocked_head(&store, &base_head);
    assert_eq!(
        store
            .search_publication(generation.as_deref().unwrap())
            .unwrap()
            .unwrap()
            .state,
        SearchPublicationState::Abandoned
    );
    assert!(!data_dir
        .join("search-index/snapshots")
        .join(generation.as_deref().unwrap())
        .exists());
    assert!(data_dir
        .join("vector-index/snapshots")
        .join(generation.as_deref().unwrap())
        .is_dir());
    drop(reader);
}

#[test]
fn retirement_failure_overrides_the_retryable_commit_fault() {
    let directory = tempdir().unwrap();
    let data_dir = directory.path().join("data");
    let store = ready_empty_store(&data_dir);
    let base_head = store.search_projection_state().unwrap();
    let session = store.wait_for_search_publication_session().unwrap();
    let mut generation = None;
    let commit_calls = AtomicUsize::new(0);

    let error = match publish_empty_successor(
        &session,
        UnixTimestamp::from_unix_seconds(1_700_100_041),
        |publication| {
            generation = Some(publication.generation().to_string());
            fs::remove_file(
                data_dir
                    .join("search-index/generation-pins")
                    .join(format!("{}.lock", publication.generation())),
            )
            .unwrap();
            decide_search_publication_with_for_test(
                publication,
                UnixTimestamp::from_unix_seconds(1_700_100_042),
                &[],
                |_, _| {
                    commit_calls.fetch_add(1, Ordering::SeqCst);
                    Err(metadata_commit_fault())
                },
            )
        },
    ) {
        Err(error) => error,
        Ok(_) => panic!("metadata commit fault unexpectedly succeeded"),
    };

    assert_eq!(commit_calls.load(Ordering::SeqCst), 1);
    assert_eq!(error.class(), ImportPipelineErrorClass::ArtifactRetirement);
    assert!(!error.is_retryable());
    assert_blocked_head(&store, &base_head);
    assert_eq!(
        store
            .search_publication(generation.as_deref().unwrap())
            .unwrap()
            .unwrap()
            .state,
        SearchPublicationState::Abandoned
    );
    assert!(data_dir
        .join("search-index/snapshots")
        .join(generation.unwrap())
        .is_dir());
}

#[derive(Clone, Copy, Debug)]
enum RetirementCrashCut {
    IntentOnly,
    FullTextDeleted,
    BothDeleted,
}

#[test]
fn pending_retirement_replay_covers_every_physical_delete_crash_cut() {
    for crash_cut in [
        RetirementCrashCut::IntentOnly,
        RetirementCrashCut::FullTextDeleted,
        RetirementCrashCut::BothDeleted,
    ] {
        let directory = tempdir().unwrap();
        let data_dir = directory.path().join(format!("data-{crash_cut:?}"));
        let store = ready_empty_store(&data_dir);
        let original_head = store.search_projection_state().unwrap();
        let session = store.wait_for_search_publication_session().unwrap();
        let generation = prepare_empty_validated_generation(
            &session,
            UnixTimestamp::from_unix_seconds(1_700_100_100 + crash_cut as i64),
        );
        session
            .begin_search_publication_retirement(
                &generation,
                UnixTimestamp::from_unix_seconds(1_700_100_110),
                SearchPublicationRetirementPlan {
                    fulltext: SearchArtifactExpectation::Published,
                    vector: SearchArtifactExpectation::Published,
                },
            )
            .unwrap();
        let retained = store.search_artifact_retention_generations(256).unwrap();
        if matches!(
            crash_cut,
            RetirementCrashCut::FullTextDeleted | RetirementCrashCut::BothDeleted
        ) {
            assert!(matches!(
                index_fulltext::try_retire_unpublished_generation(
                    &data_dir.join("search-index"),
                    &generation,
                    &retained,
                )
                .unwrap(),
                FullTextGenerationRetirement::Retired(_)
            ));
        }
        if matches!(crash_cut, RetirementCrashCut::BothDeleted) {
            assert!(matches!(
                index_vector::try_retire_unpublished_generation(
                    &data_dir.join("vector-index"),
                    &generation,
                    &retained,
                )
                .unwrap(),
                VectorGenerationRetirement::Retired(_)
            ));
        }
        drop(session);
        drop(store);

        let store = reopen_store(&data_dir);
        let session = store.wait_for_search_publication_session().unwrap();
        assert_eq!(
            replay_pending_search_publication_retirements(
                &session,
                UnixTimestamp::from_unix_seconds(1_700_100_120),
            )
            .unwrap(),
            1
        );
        assert_exact_generation_absent(&data_dir, &generation);
        assert_eq!(store.search_projection_state().unwrap(), original_head);
        let retirement = store
            .search_publication_retirement(&generation)
            .unwrap()
            .unwrap();
        assert_eq!(retirement.phase, SearchPublicationRetirementPhase::Complete);
        assert!(retirement.fulltext_complete);
        assert!(retirement.vector_complete);
    }
}

#[test]
fn preparing_single_artifact_retirement_replays_without_inventing_a_vector() {
    let directory = tempdir().unwrap();
    let data_dir = directory.path().join("data-preparing-single-artifact");
    let store = ready_empty_store(&data_dir);
    let original_head = store.search_projection_state().unwrap();
    let session = store.wait_for_search_publication_session().unwrap();
    let generation = "preparing-single-fulltext";
    assert_eq!(
        session
            .begin_search_publication(&meta_store::SearchPublicationDraft {
                generation: generation.to_string(),
                base_generation: original_head.generation.clone(),
                expected_visible_epoch: original_head.visible_epoch,
                classifier_epoch: CLASSIFIER_EPOCH.to_string(),
                projection_digest: meta_store::SearchProjectionDigest::from_pairs::<_, &str, &str>(
                    []
                )
                .unwrap(),
                now: UnixTimestamp::from_unix_seconds(1_700_100_130),
            })
            .unwrap(),
        SearchPublicationOutcome::Applied
    );
    index_fulltext::publish_snapshot(
        &data_dir.join("search-index"),
        generation,
        std::iter::empty::<index_fulltext::IndexDocument>(),
    )
    .unwrap();
    session
        .begin_search_publication_retirement(
            generation,
            UnixTimestamp::from_unix_seconds(1_700_100_131),
            SearchPublicationRetirementPlan {
                fulltext: SearchArtifactExpectation::Published,
                vector: SearchArtifactExpectation::None,
            },
        )
        .unwrap();
    drop(session);
    drop(store);

    let store = reopen_store(&data_dir);
    let session = store.wait_for_search_publication_session().unwrap();
    assert_eq!(
        replay_pending_search_publication_retirements(
            &session,
            UnixTimestamp::from_unix_seconds(1_700_100_132),
        )
        .unwrap(),
        1
    );
    assert!(!data_dir
        .join("search-index/snapshots")
        .join(generation)
        .exists());
    assert!(!data_dir
        .join("vector-index/snapshots")
        .join(generation)
        .exists());
    assert_eq!(store.search_projection_state().unwrap(), original_head);
}

fn ready_empty_store(data_dir: &std::path::Path) -> OwnedMetaStore {
    let owner = match DataDirectoryOwnerLease::try_acquire(data_dir).unwrap() {
        DataDirectoryOwnerAcquisition::Acquired(owner) => owner,
        DataDirectoryOwnerAcquisition::Contended => panic!("synthetic data dir is owned"),
    };
    let store = owner.open_store().unwrap();
    store.run_migrations().unwrap();
    let contract = ImportProcessingContract::new(
        "publication-retirement-parser-v1",
        "publication-retirement-ocr-v1",
        "publication-retirement-schema-v29",
        CLASSIFIER_EPOCH,
    )
    .unwrap();
    let now = UnixTimestamp::from_unix_seconds(1_700_100_000);
    store
        .activate_migration_rebuild_contract(&contract, now)
        .unwrap();
    let summary = crate::index_recovery::finalize_migration_rebuild(
        &store,
        now,
        &contract,
        &SearchPublicationVectorization::default(),
        &PipelineRunControl::default(),
    )
    .unwrap();
    assert!(summary.active_generation_rebuilt);
    store
}

fn publish_empty_successor(
    session: &SearchPublicationSession,
    now: UnixTimestamp,
    decide: impl FnOnce(&SearchPublicationView<'_>) -> crate::Result<SearchPublicationDecision>,
) -> crate::Result<SearchPublicationTransactionOutcome> {
    crate::search_artifacts::publish_rebuilt_search_artifacts(
        session,
        now,
        CLASSIFIER_EPOCH,
        &BTreeSet::new(),
        Vec::new(),
        &SearchPublicationVectorization::default(),
        decide,
    )
}

fn prepare_empty_validated_generation(
    session: &SearchPublicationSession,
    now: UnixTimestamp,
) -> String {
    let publication = write_incremental_search_artifacts_for_test(
        session,
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
    generation
}

fn reopen_store(data_dir: &std::path::Path) -> OwnedMetaStore {
    let owner = match DataDirectoryOwnerLease::try_acquire(data_dir).unwrap() {
        DataDirectoryOwnerAcquisition::Acquired(owner) => owner,
        DataDirectoryOwnerAcquisition::Contended => panic!("synthetic data dir is owned"),
    };
    owner.open_store().unwrap()
}

fn metadata_commit_fault() -> ImportPipelineError {
    ImportPipelineError {
        kind: ImportPipelineErrorKind::Store,
        retryable: true,
    }
}

fn assert_prepared_generation(
    data_dir: &std::path::Path,
    store: &OwnedMetaStore,
    generation: &str,
) {
    assert_eq!(
        store.search_publication(generation).unwrap().unwrap().state,
        SearchPublicationState::Validated
    );
    assert_exact_generation_present(data_dir, generation);
}

fn assert_abandoned_and_retired(
    data_dir: &std::path::Path,
    store: &OwnedMetaStore,
    generation: &str,
) {
    assert_eq!(
        store.search_publication(generation).unwrap().unwrap().state,
        SearchPublicationState::Abandoned
    );
    assert_exact_generation_absent(data_dir, generation);
}

fn assert_blocked_head(store: &OwnedMetaStore, expected: &SearchProjectionState) {
    let blocked = store.search_projection_state().unwrap();
    assert_eq!(
        blocked.service_state,
        SearchProjectionServiceState::RepairBlocked
    );
    assert_eq!(
        blocked.repair_reason,
        Some(SearchRepairReason::RuntimeInvariant)
    );
    assert_eq!(blocked.generation, expected.generation);
    assert_eq!(blocked.visible_epoch, expected.visible_epoch);
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

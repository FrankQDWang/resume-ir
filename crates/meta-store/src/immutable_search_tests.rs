use std::path::Path;

use tempfile::{tempdir, TempDir};

use crate::{
    ContentDigest, DataDirectoryOwnerAcquisition, DataDirectoryOwnerLease,
    FullTextSnapshotDescriptor, ImportProcessingContract, OwnedMetaStore, SearchProjectionDigest,
    SearchProjectionTransitionOutcome, SearchPublicationCommit, SearchPublicationDraft,
    SearchPublicationOutcome, SearchPublicationValidation, SearchRepairReason, UnixTimestamp,
    VectorSnapshotDescriptor, CLASSIFIER_EPOCH,
};

fn store_with_projection_state(
    service_state: &str,
    generation: Option<&str>,
    visible_epoch: i64,
    repair_reason: Option<&str>,
) -> (TempDir, DataDirectoryOwnerLease, OwnedMetaStore) {
    let directory = tempdir().unwrap();
    let data_dir = directory.path().join("data");
    let owner = acquire_owner(&data_dir);
    let store = owner.open_store().unwrap();
    match (service_state, generation, repair_reason) {
        ("repairing", None, Some("migration_rebuild")) => {
            store
                .connection
                .borrow()
                .execute(
                    "INSERT INTO metadata_cow_staging_authority (
                         state_key, target_visible_epoch
                     ) VALUES ('default', ?1)",
                    [visible_epoch],
                )
                .unwrap();
            store
                .connection
                .borrow()
                .execute(
                    "UPDATE search_projection_state SET visible_epoch = ?1
                     WHERE state_key = 'default'",
                    [visible_epoch],
                )
                .unwrap();
        }
        ("ready", Some(generation), None) => {
            assert_eq!(visible_epoch, 1);
            publish_empty_generation(&store, generation);
        }
        ("repairing", Some(generation), Some("artifact_unavailable")) => {
            assert_eq!(visible_epoch, 1);
            publish_empty_generation(&store, generation);
            assert_eq!(
                store
                    .begin_artifact_repair(generation, 1, timestamp(10))
                    .unwrap(),
                SearchProjectionTransitionOutcome::Applied
            );
        }
        _ => panic!("unsupported synthetic projection state"),
    }
    (directory, owner, store)
}

fn publish_empty_generation(store: &OwnedMetaStore, generation: &str) {
    let contract = ImportProcessingContract::new(
        "immutable-search-parser-v1",
        "immutable-search-ocr-v1",
        "immutable-search-schema-v29",
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
                generation: generation.to_string(),
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
        generation.to_string(),
        0,
        projection_digest.clone(),
        ContentDigest::from_bytes(b"immutable-search-fulltext"),
    );
    let vector = VectorSnapshotDescriptor::disabled(
        generation.to_string(),
        0,
        projection_digest,
        SearchProjectionDigest::from_pairs::<_, &str, &str>([]).unwrap(),
        ContentDigest::from_bytes(b"immutable-search-vector"),
    );
    session
        .validate_search_publication(&SearchPublicationValidation {
            generation,
            fulltext: &fulltext,
            vector: &vector,
            now: timestamp(3),
        })
        .unwrap();
    assert_eq!(
        session
            .commit_migration_rebuild_search_publication(
                &SearchPublicationCommit {
                    generation,
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
}

fn acquire_owner(data_dir: &Path) -> DataDirectoryOwnerLease {
    match DataDirectoryOwnerLease::try_acquire(data_dir).unwrap() {
        DataDirectoryOwnerAcquisition::Acquired(owner) => owner,
        DataDirectoryOwnerAcquisition::Contended => panic!("synthetic data dir is owned"),
    }
}

fn timestamp(seconds: i64) -> UnixTimestamp {
    UnixTimestamp::from_unix_seconds(seconds)
}

#[test]
fn migration_block_preserves_an_inherited_visible_epoch_and_is_sticky() {
    let (_directory, _owner, store) =
        store_with_projection_state("repairing", None, 9, Some("migration_rebuild"));

    assert_eq!(
        store
            .block_migration_rebuild(
                SearchRepairReason::SourceUnavailable,
                UnixTimestamp::from_unix_seconds(11),
            )
            .unwrap(),
        SearchProjectionTransitionOutcome::Applied
    );
    assert_eq!(
        store
            .begin_artifact_repair("stale-generation", 9, UnixTimestamp::from_unix_seconds(12))
            .unwrap(),
        SearchProjectionTransitionOutcome::Superseded
    );
    assert_eq!(
        store
            .block_migration_rebuild(
                SearchRepairReason::RuntimeInvariant,
                UnixTimestamp::from_unix_seconds(13),
            )
            .unwrap(),
        SearchProjectionTransitionOutcome::Superseded
    );

    let observed = store
        .connection
        .borrow()
        .query_row(
            "SELECT service_state, generation, visible_epoch, repair_reason,
                    updated_at_seconds
             FROM search_projection_state WHERE state_key = 'default'",
            [],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, i64>(4)?,
                ))
            },
        )
        .unwrap();
    assert_eq!(
        observed,
        (
            "repair_blocked".to_string(),
            None,
            9,
            Some("source_unavailable".to_string()),
            11,
        )
    );
}

#[test]
fn artifact_repair_requires_the_exact_ready_head() {
    let (_directory, _owner, store) =
        store_with_projection_state("ready", Some("generation-1"), 1, None);

    assert_eq!(
        store
            .begin_artifact_repair("generation-1", 0, UnixTimestamp::from_unix_seconds(11))
            .unwrap(),
        SearchProjectionTransitionOutcome::Superseded
    );
    assert_eq!(
        store
            .begin_artifact_repair("generation-1", 1, UnixTimestamp::from_unix_seconds(12))
            .unwrap(),
        SearchProjectionTransitionOutcome::Applied
    );
    assert_eq!(
        store
            .begin_artifact_repair("generation-1", 1, UnixTimestamp::from_unix_seconds(13))
            .unwrap(),
        SearchProjectionTransitionOutcome::Applied
    );
}

#[test]
fn artifact_repair_block_is_exact_preserves_head_and_is_sticky() {
    let (_directory, _owner, store) = store_with_projection_state(
        "repairing",
        Some("generation-1"),
        1,
        Some("artifact_unavailable"),
    );
    let fingerprint = store
        .artifact_repair_context()
        .unwrap()
        .unwrap()
        .publication_fingerprint;
    let wrong_fingerprint = ContentDigest::from_bytes(b"different-publication");

    assert_eq!(
        store
            .block_artifact_repair(
                "generation-1",
                &fingerprint,
                0,
                UnixTimestamp::from_unix_seconds(11),
            )
            .unwrap(),
        SearchProjectionTransitionOutcome::Superseded
    );
    assert_eq!(
        store
            .block_artifact_repair(
                "generation-1",
                &wrong_fingerprint,
                1,
                UnixTimestamp::from_unix_seconds(12),
            )
            .unwrap(),
        SearchProjectionTransitionOutcome::Superseded
    );
    assert_eq!(
        store
            .block_artifact_repair(
                "generation-1",
                &fingerprint,
                1,
                UnixTimestamp::from_unix_seconds(13),
            )
            .unwrap(),
        SearchProjectionTransitionOutcome::Applied
    );
    assert_eq!(
        store
            .block_artifact_repair(
                "generation-1",
                &fingerprint,
                1,
                UnixTimestamp::from_unix_seconds(14),
            )
            .unwrap(),
        SearchProjectionTransitionOutcome::Superseded
    );
    assert_eq!(
        store
            .begin_artifact_repair("generation-1", 1, UnixTimestamp::from_unix_seconds(14))
            .unwrap(),
        SearchProjectionTransitionOutcome::Superseded
    );

    let observed = store
        .connection
        .borrow()
        .query_row(
            "SELECT service_state, generation, visible_epoch, repair_reason,
                    updated_at_seconds
             FROM search_projection_state WHERE state_key = 'default'",
            [],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, i64>(4)?,
                ))
            },
        )
        .unwrap();
    assert_eq!(
        observed,
        (
            "repair_blocked".to_string(),
            Some("generation-1".to_string()),
            1,
            Some("runtime_invariant".to_string()),
            13,
        )
    );
}

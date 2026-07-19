use rusqlite::params;

use crate::{
    EphemeralMetaStore, SearchProjectionTransitionOutcome, SearchRepairReason, UnixTimestamp,
};

fn store_with_projection_state(
    service_state: &str,
    generation: Option<&str>,
    visible_epoch: i64,
    repair_reason: Option<&str>,
) -> EphemeralMetaStore {
    let store = EphemeralMetaStore::open_in_memory().unwrap();
    store
        .connection
        .borrow()
        .execute_batch(
            "CREATE TABLE search_projection_state (
                state_key TEXT PRIMARY KEY NOT NULL,
                service_state TEXT NOT NULL,
                generation TEXT,
                visible_epoch INTEGER NOT NULL,
                repair_reason TEXT,
                updated_at_seconds INTEGER NOT NULL
             );",
        )
        .unwrap();
    store
        .connection
        .borrow()
        .execute(
            "INSERT INTO search_projection_state (
                state_key, service_state, generation, visible_epoch,
                repair_reason, updated_at_seconds
             ) VALUES ('default', ?1, ?2, ?3, ?4, 10)",
            params![service_state, generation, visible_epoch, repair_reason],
        )
        .unwrap();
    store
}

#[test]
fn migration_block_preserves_an_inherited_visible_epoch_and_is_sticky() {
    let store = store_with_projection_state("repairing", None, 9, Some("migration_rebuild"));

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
    let store = store_with_projection_state("ready", Some("generation-1"), 4, None);

    assert_eq!(
        store
            .begin_artifact_repair("generation-1", 3, UnixTimestamp::from_unix_seconds(11))
            .unwrap(),
        SearchProjectionTransitionOutcome::Superseded
    );
    assert_eq!(
        store
            .begin_artifact_repair("generation-1", 4, UnixTimestamp::from_unix_seconds(12))
            .unwrap(),
        SearchProjectionTransitionOutcome::Applied
    );
    assert_eq!(
        store
            .begin_artifact_repair("generation-1", 4, UnixTimestamp::from_unix_seconds(13))
            .unwrap(),
        SearchProjectionTransitionOutcome::Applied
    );
}

use crate::{EphemeralMetaStore, UnixTimestamp};

#[test]
fn deletion_attempt_evidence_rejects_unproduced_error_codes() {
    let store = EphemeralMetaStore::open_in_memory().unwrap();
    store.run_migrations().unwrap();
    let now = UnixTimestamp::from_unix_seconds(1_800_301_000);
    let root = store
        .register_source_root(
            "/synthetic/error-vocabulary",
            "/synthetic/error-vocabulary",
            "Synthetic error vocabulary",
            now,
        )
        .unwrap();
    store.begin_source_root_deletion(&root.id, now).unwrap();
    store
        .begin_source_root_deletion_attempt(&root.id, now)
        .unwrap();

    for retired_code in ["receipt_unavailable", "service_unavailable"] {
        let result = store.connection.borrow().execute(
            "UPDATE source_root_deletion_attempt_evidence
             SET last_error_phase = 'requested',
                 last_error_code = ?2,
                 last_error_at_seconds = ?3
             WHERE root_id = ?1",
            rusqlite::params![root.id.as_str(), retired_code, now.as_unix_seconds()],
        );
        assert!(
            result.is_err(),
            "retired error code {retired_code} persisted"
        );
    }
}

#[test]
fn saturated_attempt_count_does_not_block_the_next_deletion_attempt() {
    let store = EphemeralMetaStore::open_in_memory().unwrap();
    store.run_migrations().unwrap();
    let requested_at = UnixTimestamp::from_unix_seconds(1_800_302_000);
    let next_attempt_at = UnixTimestamp::from_unix_seconds(1_800_302_001);
    let root = store
        .register_source_root(
            "/synthetic/saturated-attempt",
            "/synthetic/saturated-attempt",
            "Synthetic saturated attempt",
            requested_at,
        )
        .unwrap();
    store
        .begin_source_root_deletion(&root.id, requested_at)
        .unwrap();
    store
        .connection
        .borrow()
        .execute(
            "UPDATE source_root_deletion_attempt_evidence
             SET attempt_count = 9007199254740991,
                 last_attempt_at_seconds = ?2,
                 last_error_phase = 'requested',
                 last_error_code = 'internal',
                 last_error_at_seconds = ?2
             WHERE root_id = ?1",
            rusqlite::params![root.id.as_str(), requested_at.as_unix_seconds()],
        )
        .unwrap();

    let saturated = store
        .begin_source_root_deletion_attempt(&root.id, next_attempt_at)
        .unwrap();

    assert_eq!(saturated.attempt_count, 9_007_199_254_740_991);
    assert_eq!(saturated.last_attempt_at, Some(next_attempt_at));
    assert_eq!(saturated.last_error_phase, None);
    assert_eq!(saturated.last_error_code, None);
    assert_eq!(saturated.last_error_at, None);
}

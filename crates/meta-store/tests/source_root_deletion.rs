use meta_store::{SourceRootDeletionErrorCode, SourceRootDeletionPhase, UnixTimestamp};

mod support;

#[test]
fn deletion_receipts_release_the_connection_after_commit() {
    const ROOT: &str = "/synthetic/source-a";
    let (_directory, store) = support::owned_store();
    let requested_at = UnixTimestamp::from_unix_seconds(1_800_300_200);
    let completed_at = UnixTimestamp::from_unix_seconds(1_800_300_201);
    let root = store
        .register_source_root(ROOT, ROOT, "Synthetic source A", requested_at)
        .unwrap();

    let requested = store
        .begin_source_root_deletion(&root.id, requested_at)
        .unwrap();
    assert_eq!(requested.phase, SourceRootDeletionPhase::Requested);
    assert_eq!(
        store.source_root_deletion(&root.id).unwrap(),
        Some(requested)
    );

    for phase in [
        SourceRootDeletionPhase::Quiescing,
        SourceRootDeletionPhase::Publishing,
        SourceRootDeletionPhase::Purging,
    ] {
        store
            .set_source_root_deletion_phase(&root.id, phase, requested_at)
            .unwrap();
    }
    store
        .purge_source_root_data(&root.id, completed_at)
        .unwrap();
    let completed = store
        .complete_source_root_deletion(&root.id, completed_at)
        .unwrap();
    assert_eq!(completed.phase, SourceRootDeletionPhase::Complete);
    assert_eq!(completed.completed_at, Some(completed_at));
    assert_eq!(
        store.source_root_deletion(&root.id).unwrap(),
        Some(completed)
    );
    assert!(store.source_root(&root.id).unwrap().is_none());
}

#[test]
fn deletion_attempt_evidence_survives_reopen_and_resets_for_the_next_attempt() {
    const ROOT: &str = "/synthetic/source-attempt-evidence";
    let (_directory, store) = support::owned_store();
    let requested_at = UnixTimestamp::from_unix_seconds(1_800_300_300);
    let attempted_at = UnixTimestamp::from_unix_seconds(1_800_300_301);
    let failed_at = UnixTimestamp::from_unix_seconds(1_800_300_302);
    let root = store
        .register_source_root(ROOT, ROOT, "Synthetic attempt evidence", requested_at)
        .unwrap();
    let requested = store
        .begin_source_root_deletion(&root.id, requested_at)
        .unwrap();
    assert_eq!(requested.attempt_count, 0);
    assert_eq!(requested.last_attempt_at, None);
    assert_eq!(requested.last_error_code, None);
    store
        .set_source_root_deletion_phase(&root.id, SourceRootDeletionPhase::Quiescing, attempted_at)
        .unwrap();

    let started = store
        .begin_source_root_deletion_attempt(&root.id, attempted_at)
        .unwrap();
    assert_eq!(started.attempt_count, 1);
    assert_eq!(started.last_attempt_at, Some(attempted_at));
    let failed = store
        .record_source_root_deletion_attempt_failure(
            &root.id,
            SourceRootDeletionPhase::Quiescing,
            SourceRootDeletionErrorCode::OcrQuiescenceTimeout,
            failed_at,
        )
        .unwrap();
    assert_eq!(
        failed.last_error_phase,
        Some(SourceRootDeletionPhase::Quiescing)
    );
    assert_eq!(
        failed.last_error_code,
        Some(SourceRootDeletionErrorCode::OcrQuiescenceTimeout)
    );
    assert_eq!(failed.last_error_at, Some(failed_at));

    let reopened = store.open_sibling().unwrap();
    assert_eq!(
        reopened.source_root_deletion(&root.id).unwrap(),
        Some(failed)
    );
    let second = reopened
        .begin_source_root_deletion_attempt(&root.id, failed_at)
        .unwrap();
    assert_eq!(second.attempt_count, 2);
    assert_eq!(second.last_attempt_at, Some(failed_at));
    assert_eq!(second.last_error_phase, None);
    assert_eq!(second.last_error_code, None);
    assert_eq!(second.last_error_at, None);
}

#[test]
fn recent_deletion_attempt_evidence_is_bounded_to_sixteen_records() {
    let (_directory, store) = support::owned_store();
    let mut completed_root_id = None;
    for index in 0..17 {
        let root_path = format!("/synthetic/bounded-attempt-{index:02}");
        let now = UnixTimestamp::from_unix_seconds(1_800_301_000 + index);
        let root = store
            .register_source_root(&root_path, &root_path, "Synthetic bounded attempt", now)
            .unwrap();
        store.begin_source_root_deletion(&root.id, now).unwrap();
        store
            .begin_source_root_deletion_attempt(&root.id, now)
            .unwrap();
        if index == 0 {
            completed_root_id = Some(root.id.clone());
            for phase in [
                SourceRootDeletionPhase::Quiescing,
                SourceRootDeletionPhase::Publishing,
                SourceRootDeletionPhase::Purging,
            ] {
                store
                    .set_source_root_deletion_phase(&root.id, phase, now)
                    .unwrap();
            }
            store.purge_source_root_data(&root.id, now).unwrap();
            store.complete_source_root_deletion(&root.id, now).unwrap();
        }
    }

    let attempts = store.recent_source_root_deletion_attempts().unwrap();
    assert_eq!(attempts.len(), 16);
    assert!(attempts.iter().all(|attempt| attempt.attempt_count == 1));
    assert!(attempts
        .iter()
        .all(|attempt| Some(&attempt.root_id) != completed_root_id.as_ref()));
}

use meta_store::{SourceRootDeletionPhase, UnixTimestamp};

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

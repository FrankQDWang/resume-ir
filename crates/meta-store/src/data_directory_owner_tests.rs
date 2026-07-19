use std::fs::{self, OpenOptions};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Arc};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

use super::{
    DataDirectoryOwnerAcquireError, DataDirectoryOwnerAcquisition, DataDirectoryOwnerLease,
    ImportProcessingOrphanNormalizationError, ImportTaskOwnerLock, DATA_DIRECTORY_OWNER_LOCK_FILE,
    LEGACY_DAEMON_OWNER_LOCK_FILE, SEARCH_PUBLICATION_LOCK_FILE,
};
use crate::{
    ImportProcessingContract, ImportRootKind, ImportRootTaskHeadOutcome, ImportScanProfile,
    ImportScanScope, ImportTask, ImportTaskId, ImportTaskPurpose, ImportTaskStatus,
    MetaStoreErrorClass, MigrationRebuildContractActivation, OwnedMetaStore, UnixTimestamp,
    CLASSIFIER_EPOCH,
};

static NEXT_TEST_DIR_ID: AtomicU64 = AtomicU64::new(0);

#[test]
fn one_data_directory_has_exactly_one_live_storage_import_owner() {
    let temp = TestDir::new("single-owner");
    let first = acquired(&temp.0);
    assert!(matches!(
        DataDirectoryOwnerLease::try_acquire(&temp.0).unwrap(),
        DataDirectoryOwnerAcquisition::Contended
    ));

    drop(first);
    let _replacement = acquired(&temp.0);
}

#[test]
fn derived_owner_guard_retains_exclusion_after_the_public_lease_drops() {
    let temp = TestDir::new("derived-owner-guard");
    let lease = acquired(&temp.0);
    let guard = lease.shared_guard();

    drop(lease);
    assert!(matches!(
        DataDirectoryOwnerLease::try_acquire(&temp.0).unwrap(),
        DataDirectoryOwnerAcquisition::Contended
    ));

    drop(guard);
    let _replacement = acquired(&temp.0);
}

#[test]
fn owned_store_retains_exclusion_after_the_public_lease_drops() {
    let temp = TestDir::new("owned-store-retains-owner");
    let lease = acquired(&temp.0);
    let store = lease.open_store().unwrap();

    drop(lease);
    assert!(matches!(
        DataDirectoryOwnerLease::try_acquire(&temp.0).unwrap(),
        DataDirectoryOwnerAcquisition::Contended
    ));

    drop(store);
    let _replacement = acquired(&temp.0);
}

#[test]
fn search_publication_waiters_are_fifo_and_a_hot_contender_cannot_starve_them() {
    const ORDERED_WAITERS: usize = 8;
    const HOT_ACQUISITIONS: usize = 32;

    let temp = TestDir::new("publication-fifo");
    let owner = acquired(&temp.0);
    let store = owner.open_store().unwrap();
    let guard = owner.shared_guard();
    let first = store.wait_for_search_publication_session().unwrap();
    let (order_tx, order_rx) = mpsc::channel();
    let mut waiters = Vec::new();

    for label in 0..ORDERED_WAITERS {
        let waiting_store = store.open_sibling().unwrap();
        let acquired_tx = order_tx.clone();
        waiters.push(thread::spawn(move || {
            let session = waiting_store.wait_for_search_publication_session().unwrap();
            acquired_tx.send(label).unwrap();
            drop(session);
        }));
        guard
            .wait_for_search_publication_waiters(label + 1)
            .unwrap();
    }

    let hot_store = store.open_sibling().unwrap();
    let hot_tx = order_tx.clone();
    let hot = thread::spawn(move || {
        for sequence in 0..HOT_ACQUISITIONS {
            let session = hot_store.wait_for_search_publication_session().unwrap();
            hot_tx.send(ORDERED_WAITERS + sequence).unwrap();
            drop(session);
        }
    });
    guard
        .wait_for_search_publication_waiters(ORDERED_WAITERS + 1)
        .unwrap();
    drop(order_tx);
    drop(first);

    let observed = order_rx.iter().collect::<Vec<_>>();
    assert_eq!(
        observed,
        (0..ORDERED_WAITERS + HOT_ACQUISITIONS).collect::<Vec<_>>()
    );
    for waiter in waiters {
        waiter.join().unwrap();
    }
    hot.join().unwrap();
}

#[test]
fn nonblocking_publication_acquisition_fails_fast_without_issuing_a_fifo_ticket() {
    let temp = TestDir::new("publication-nonblocking-contention");
    let owner = acquired(&temp.0);
    let store = owner.open_store().unwrap();
    let first = store.wait_for_search_publication_session().unwrap();
    let tickets_before = {
        let state = owner.guard.search_publication_arbiter.state.lock().unwrap();
        (state.next_ticket, state.serving_ticket)
    };

    assert_eq!(
        store
            .try_acquire_search_publication_session()
            .unwrap_err()
            .class(),
        MetaStoreErrorClass::StorageInvariant
    );
    let competing_store = store.open_sibling().unwrap();
    let (result_tx, result_rx) = mpsc::channel();
    let contender = thread::spawn(move || {
        result_tx
            .send(
                competing_store
                    .try_acquire_search_publication_session()
                    .map(|_| ())
                    .map_err(|error| error.class()),
            )
            .unwrap();
    });
    assert_eq!(
        result_rx
            .recv_timeout(std::time::Duration::from_millis(250))
            .unwrap()
            .unwrap_err(),
        MetaStoreErrorClass::MigrationOwnershipRequired
    );
    contender.join().unwrap();
    let tickets_after = {
        let state = owner.guard.search_publication_arbiter.state.lock().unwrap();
        (state.next_ticket, state.serving_ticket)
    };
    assert_eq!(tickets_after, tickets_before);

    drop(first);
    let replacement = store.try_acquire_search_publication_session().unwrap();
    drop(replacement);
}

#[test]
fn panicking_publication_holder_releases_the_next_waiter() {
    let temp = TestDir::new("publication-panic-release");
    let owner = acquired(&temp.0);
    let store = owner.open_store().unwrap();
    let guard = owner.shared_guard();
    let holder_store = store.open_sibling().unwrap();
    let (holder_ready_tx, holder_ready_rx) = mpsc::channel();
    let (panic_tx, panic_rx) = mpsc::channel();
    let holder = thread::spawn(move || {
        let _session = holder_store.wait_for_search_publication_session().unwrap();
        holder_ready_tx.send(()).unwrap();
        panic_rx.recv().unwrap();
        panic!("synthetic publication holder panic");
    });
    holder_ready_rx.recv().unwrap();

    let waiting_store = store.open_sibling().unwrap();
    let (acquired_tx, acquired_rx) = mpsc::channel();
    let waiter = thread::spawn(move || {
        let _session = waiting_store.wait_for_search_publication_session().unwrap();
        acquired_tx.send(()).unwrap();
    });
    guard.wait_for_search_publication_waiters(1).unwrap();
    panic_tx.send(()).unwrap();

    assert!(holder.join().is_err());
    acquired_rx.recv().unwrap();
    waiter.join().unwrap();
}

#[test]
fn poisoned_publication_arbiter_fails_with_a_typed_invariant_error() {
    let temp = TestDir::new("publication-poison");
    let owner = acquired(&temp.0);
    let store = owner.open_store().unwrap();
    let arbiter = Arc::clone(&owner.guard.search_publication_arbiter);
    assert!(thread::spawn(move || {
        let _state = arbiter.state.lock().unwrap();
        panic!("synthetic arbiter poison");
    })
    .join()
    .is_err());

    assert_eq!(
        store
            .wait_for_search_publication_session()
            .unwrap_err()
            .class(),
        MetaStoreErrorClass::StorageInvariant
    );
}

#[test]
fn exhausted_publication_ticket_space_fails_with_a_typed_invariant_error() {
    let temp = TestDir::new("publication-ticket-exhaustion");
    let owner = acquired(&temp.0);
    let store = owner.open_store().unwrap();
    {
        let mut state = owner.guard.search_publication_arbiter.state.lock().unwrap();
        state.next_ticket = u64::MAX;
        state.serving_ticket = u64::MAX;
    }

    assert_eq!(
        store
            .wait_for_search_publication_session()
            .unwrap_err()
            .class(),
        MetaStoreErrorClass::StorageInvariant
    );
}

#[test]
fn independent_publication_lock_owner_is_rejected_without_waiting() {
    let temp = TestDir::new("external-publication-owner");
    let owner = acquired(&temp.0);
    let store = owner.open_store().unwrap();
    let lock_path = temp.0.join(SEARCH_PUBLICATION_LOCK_FILE);
    let external_owner = super::private_lock_options().open(&lock_path).unwrap();
    fs4::fs_std::FileExt::lock_exclusive(&external_owner).unwrap();
    let nonblocking_store = store.open_sibling().unwrap();
    let (waiting_result_tx, waiting_result_rx) = mpsc::channel();
    let waiting_contender = thread::spawn(move || {
        waiting_result_tx
            .send(
                store
                    .wait_for_search_publication_session()
                    .map(|_| ())
                    .map_err(|error| error.class()),
            )
            .unwrap();
    });
    let (nonblocking_result_tx, nonblocking_result_rx) = mpsc::channel();
    let nonblocking_contender = thread::spawn(move || {
        nonblocking_result_tx
            .send(
                nonblocking_store
                    .try_acquire_search_publication_session()
                    .map(|_| ())
                    .map_err(|error| error.class()),
            )
            .unwrap();
    });

    let waiting_result = waiting_result_rx.recv_timeout(std::time::Duration::from_millis(250));
    let nonblocking_result =
        nonblocking_result_rx.recv_timeout(std::time::Duration::from_millis(250));
    fs4::fs_std::FileExt::unlock(&external_owner).unwrap();
    waiting_contender.join().unwrap();
    nonblocking_contender.join().unwrap();
    assert_eq!(
        waiting_result.unwrap().unwrap_err(),
        MetaStoreErrorClass::MigrationOwnershipRequired
    );
    assert_eq!(
        nonblocking_result.unwrap().unwrap_err(),
        MetaStoreErrorClass::MigrationOwnershipRequired
    );
}

#[test]
fn live_legacy_daemon_namespace_blocks_the_new_global_owner() {
    let temp = TestDir::new("legacy-daemon-owner");
    let path = temp.0.join(LEGACY_DAEMON_OWNER_LOCK_FILE);
    let mut options = OpenOptions::new();
    options.create(true).read(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        options.mode(0o600);
        let legacy_owner = options.open(&path).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        fs4::fs_std::FileExt::lock_exclusive(&legacy_owner).unwrap();
        assert!(matches!(
            DataDirectoryOwnerLease::try_acquire(&temp.0).unwrap(),
            DataDirectoryOwnerAcquisition::Contended
        ));
        drop(legacy_owner);
    }
    #[cfg(not(unix))]
    {
        let legacy_owner = options.open(&path).unwrap();
        fs4::fs_std::FileExt::lock_exclusive(&legacy_owner).unwrap();
        assert!(matches!(
            DataDirectoryOwnerLease::try_acquire(&temp.0).unwrap(),
            DataDirectoryOwnerAcquisition::Contended
        ));
        drop(legacy_owner);
    }

    let _replacement = acquired(&temp.0);
}

#[test]
fn owner_capability_creates_and_reopens_only_its_bound_store() {
    let first = TestDir::new("first-store");
    let second = TestDir::new("second-store");
    let first_owner = acquired(&first.0);
    let second_owner = acquired(&second.0);

    let first_store = first_owner.open_store().unwrap();
    let second_store = second_owner.open_store().unwrap();

    assert_eq!(first_store.schema_version().unwrap(), 28);
    assert_eq!(second_store.schema_version().unwrap(), 28);
    assert!(first_store.open_sibling().is_ok());
    assert!(second_store.open_sibling().is_ok());
}

#[test]
fn exclusive_owner_normalizes_running_tasks_including_cancelled_attempts() {
    let temp = TestDir::new("normalize-running");
    let owner = acquired(&temp.0);
    let store = owner.open_store().unwrap();
    let contract = contract();
    store
        .activate_migration_rebuild_contract(&contract, timestamp(1))
        .unwrap();
    let first = running_task(&store, &contract, "first", timestamp(10));
    let cancelled = running_task(&store, &contract, "cancelled", timestamp(20));
    store
        .cancel_import_task(&cancelled.id, timestamp(22))
        .unwrap();

    assert_eq!(
        store
            .normalize_orphaned_running_tasks(timestamp(30))
            .unwrap(),
        2
    );
    let first = store.import_task_by_id(&first.id).unwrap().unwrap();
    assert_eq!(first.status, ImportTaskStatus::Queued);
    assert_eq!(first.started_at, None);
    assert_eq!(first.finished_at, None);
    let cancelled = store.import_task_by_id(&cancelled.id).unwrap().unwrap();
    assert_eq!(cancelled.status, ImportTaskStatus::FailedRetryable);
    assert_eq!(cancelled.finished_at, Some(timestamp(30)));
    let replacement = ImportProcessingContract::new(
        "synthetic-primary-v28-replacement",
        "synthetic-ocr-v28",
        "synthetic-derived-v28",
        CLASSIFIER_EPOCH,
    )
    .unwrap();
    assert_eq!(
        store
            .activate_migration_rebuild_contract(&replacement, timestamp(31))
            .unwrap(),
        MigrationRebuildContractActivation::Activated
    );
}

#[test]
fn exclusive_owner_fails_closed_when_a_legacy_task_owner_is_still_live() {
    let temp = TestDir::new("legacy-task-owner");
    let owner = acquired(&temp.0);
    let store = owner.open_store().unwrap();
    let contract = contract();
    store
        .activate_migration_rebuild_contract(&contract, timestamp(1))
        .unwrap();
    let task = running_task(&store, &contract, "contended", timestamp(10));
    let _legacy_owner = ImportTaskOwnerLock::acquire(&temp.0, &task.id).unwrap();

    assert!(matches!(
        store.normalize_orphaned_running_tasks(timestamp(30)),
        Err(ImportProcessingOrphanNormalizationError::TaskOwnerLockContended)
    ));
    assert_eq!(
        store.import_task_by_id(&task.id).unwrap().unwrap().status,
        ImportTaskStatus::Running
    );
}

#[cfg(unix)]
#[test]
fn data_directory_owner_lock_rejects_a_symlink() {
    use std::os::unix::fs::symlink;

    let temp = TestDir::new("symlink");
    let target = temp.0.join("target");
    fs::write(&target, []).unwrap();
    symlink(&target, temp.0.join(DATA_DIRECTORY_OWNER_LOCK_FILE)).unwrap();

    assert_eq!(
        DataDirectoryOwnerLease::try_acquire(&temp.0).unwrap_err(),
        DataDirectoryOwnerAcquireError::RuntimeIntegrity
    );
}

fn acquired(data_dir: &Path) -> DataDirectoryOwnerLease {
    match DataDirectoryOwnerLease::try_acquire(data_dir).unwrap() {
        DataDirectoryOwnerAcquisition::Acquired(lease) => lease,
        DataDirectoryOwnerAcquisition::Contended => panic!("data-directory owner was contended"),
    }
}

fn contract() -> ImportProcessingContract {
    ImportProcessingContract::new(
        "synthetic-primary-v28",
        "synthetic-ocr-v28",
        "synthetic-derived-v28",
        CLASSIFIER_EPOCH,
    )
    .unwrap()
}

fn running_task(
    store: &OwnedMetaStore,
    contract: &ImportProcessingContract,
    label: &str,
    now: UnixTimestamp,
) -> ImportTask {
    let seed = ImportTask {
        id: ImportTaskId::from_non_secret_parts(&["processing-owner", label, "seed"]),
        root_path: format!("/synthetic/{label}"),
        status: ImportTaskStatus::Queued,
        queued_at: now,
        started_at: None,
        finished_at: None,
        updated_at: now,
    };
    let scope = ImportScanScope {
        import_task_id: seed.id.clone(),
        root_kind: ImportRootKind::Explicit,
        root_preset: None,
        scan_profile: ImportScanProfile::Explicit,
        requested_root_path: seed.root_path.clone(),
        canonical_root_path: seed.root_path.clone(),
        files_discovered: 0,
        ignored_entries: 0,
        scan_errors: 0,
        searchable_documents: 0,
        ocr_required_documents: 0,
        ocr_jobs_queued: 0,
        failed_documents: 0,
        deleted_documents: 0,
        scan_budget_kind: None,
        scan_budget_limit: None,
        scan_budget_observed: None,
        scan_budget_exhausted: false,
        updated_at: now,
    };
    store
        .insert_import_task_with_scan_scope(&seed, &scope, contract)
        .unwrap();
    store.cancel_import_task(&seed.id, now).unwrap();
    let task_id = ImportTaskId::from_non_secret_parts(&["processing-owner", label, "running"]);
    assert!(matches!(
        store
            .enqueue_full_corpus_migration_rebuild_root(&seed.root_path, &task_id, contract, now)
            .unwrap(),
        ImportRootTaskHeadOutcome::HeadInserted {
            purpose: ImportTaskPurpose::MigrationRebuildFullCorpus,
            ..
        }
    ));
    let task = store.import_task_by_id(&task_id).unwrap().unwrap();
    store
        .claim_observed_import_task_for_worker(&task, timestamp(now.as_unix_seconds() + 1))
        .unwrap()
        .unwrap()
}

fn timestamp(seconds: i64) -> UnixTimestamp {
    UnixTimestamp::from_unix_seconds(seconds)
}

struct TestDir(PathBuf);

impl TestDir {
    fn new(label: &str) -> Self {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "resume-ir-storage-owner-{label}-{}-{suffix}-{}",
            std::process::id(),
            NEXT_TEST_DIR_ID.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

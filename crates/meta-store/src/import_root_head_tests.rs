use std::ops::Deref;
use std::sync::{Arc, Barrier};
use std::thread;

use tempfile::{tempdir, TempDir};

use super::{
    ImportRootTaskHeadBatchOutcome, ImportRootTaskHeadBatchRejection, ImportRootTaskHeadOutcome,
    ImportRootTaskHeadRequest,
};
use crate::{
    import_task_purpose::insert_migration_rebuild_full_corpus_task_marker_in_connection,
    ContentDigest, DataDirectoryOwnerAcquisition, DataDirectoryOwnerLease, EphemeralMetaStore,
    FullTextSnapshotDescriptor, ImportProcessingContract, ImportRootKind, ImportScanBudgetKind,
    ImportScanProfile, ImportScanScope, ImportTask, ImportTaskId, ImportTaskPurpose,
    ImportTaskStatus, OwnedMetaStore, SearchProjectionDigest, SearchPublicationCommit,
    SearchPublicationDraft, SearchPublicationOutcome, SearchPublicationValidation, UnixTimestamp,
    VectorSnapshotDescriptor, CLASSIFIER_EPOCH,
};

const ROOT: &str = "/synthetic/root-head";

#[test]
fn configured_and_migration_connections_converge_on_one_claimable_head() {
    let directory = tempdir().unwrap();
    let owner = match DataDirectoryOwnerLease::try_acquire(directory.path()).unwrap() {
        DataDirectoryOwnerAcquisition::Acquired(owner) => owner,
        DataDirectoryOwnerAcquisition::Contended => panic!("test data directory was contended"),
    };
    let store = owner.open_store().unwrap();
    let contract = processing_contract();
    store
        .activate_migration_rebuild_contract(&contract, timestamp(1))
        .unwrap();
    let (seed_task, seed_scope) = configured_request("seed", timestamp(2));
    let inserted = store
        .coordinate_import_root_task_head(ImportRootTaskHeadRequest::Configured {
            task: &seed_task,
            scope: &seed_scope,
            processing_contract: &contract,
        })
        .unwrap();
    assert!(matches!(
        inserted,
        ImportRootTaskHeadOutcome::HeadInserted {
            purpose: ImportTaskPurpose::MigrationRebuildFullCorpus,
            ..
        }
    ));
    store
        .cancel_import_task(&seed_task.id, timestamp(3))
        .unwrap();

    let configured_store = store.open_sibling().unwrap();
    let migration_store = store.open_sibling().unwrap();
    let start = Arc::new(Barrier::new(3));
    let configured_start = Arc::clone(&start);
    let configured_contract = contract.clone();
    let configured_thread = thread::spawn(move || {
        let (task, scope) = configured_request("configured", timestamp(4));
        configured_start.wait();
        configured_store
            .coordinate_import_root_task_head(ImportRootTaskHeadRequest::Configured {
                task: &task,
                scope: &scope,
                processing_contract: &configured_contract,
            })
            .unwrap()
    });
    let migration_start = Arc::clone(&start);
    let migration_contract = contract.clone();
    let migration_thread = thread::spawn(move || {
        let task_id = ImportTaskId::from_non_secret_parts(&["migration"]);
        migration_start.wait();
        migration_store
            .coordinate_import_root_task_head(ImportRootTaskHeadRequest::MigrationRebuild {
                canonical_root_path: ROOT,
                task_id: &task_id,
                processing_contract: &migration_contract,
                queued_at: timestamp(4),
            })
            .unwrap()
    });
    start.wait();
    let configured_outcome = configured_thread.join().unwrap();
    let migration_outcome = migration_thread.join().unwrap();
    assert!(matches!(
        configured_outcome,
        ImportRootTaskHeadOutcome::HeadInserted {
            purpose: ImportTaskPurpose::MigrationRebuildFullCorpus,
            ..
        } | ImportRootTaskHeadOutcome::HeadRetained {
            purpose: ImportTaskPurpose::MigrationRebuildFullCorpus,
            ..
        }
    ));
    assert!(matches!(
        migration_outcome,
        ImportRootTaskHeadOutcome::HeadInserted {
            purpose: ImportTaskPurpose::MigrationRebuildFullCorpus,
            ..
        } | ImportRootTaskHeadOutcome::HeadRetained {
            purpose: ImportTaskPurpose::MigrationRebuildFullCorpus,
            ..
        }
    ));

    let head = store.latest_import_task_by_root(ROOT).unwrap().unwrap();
    let pending = store.pending_import_task_by_root(ROOT).unwrap().unwrap();
    assert_eq!(head.id, pending.id);
    assert_eq!(
        store.import_task_purpose(&head.id).unwrap(),
        ImportTaskPurpose::MigrationRebuildFullCorpus
    );
    let pending_count = store
        .connection
        .borrow()
        .query_row(
            "SELECT COUNT(*) FROM import_task AS task
             WHERE root_path = ?1 AND status IN ('queued', 'running', 'failed_retryable')
               AND NOT EXISTS (
                   SELECT 1 FROM import_task_cancellation AS cancellation
                   WHERE cancellation.import_task_id = task.id
               )",
            [ROOT],
            |row| row.get::<_, i64>(0),
        )
        .unwrap();
    assert_eq!(pending_count, 1);
}

#[test]
fn rowid_head_is_stable_across_identical_timestamps_and_older_heartbeat() {
    let store = EphemeralMetaStore::open_in_memory().unwrap();
    store.run_migrations().unwrap();
    let contract = processing_contract();
    store
        .activate_migration_rebuild_contract(&contract, timestamp(1))
        .unwrap();
    let (first, first_scope) = migration_task("first", timestamp(2));
    let (second, second_scope) = migration_task("second", timestamp(2));
    insert_migration_task(&store, &first, &first_scope, &contract);
    insert_migration_task(&store, &second, &second_scope, &contract);
    assert_eq!(
        store.latest_import_task_by_root(ROOT).unwrap().unwrap().id,
        second.id
    );

    let running = store
        .claim_observed_import_task_for_worker(&first, timestamp(3))
        .unwrap()
        .unwrap();
    assert!(store
        .heartbeat_running_import_task(&running.id, timestamp(99))
        .unwrap());
    assert_eq!(
        store.latest_import_task_by_root(ROOT).unwrap().unwrap().id,
        second.id
    );
}

#[test]
fn configured_budget_changes_replace_queued_and_retryable_heads() {
    let store = ready_store();
    let contract = processing_contract();
    let (finite, finite_scope) =
        configured_request_for_root("finite", ROOT, Some(10), timestamp(2));
    assert!(matches!(
        store
            .coordinate_import_root_task_head(ImportRootTaskHeadRequest::Configured {
                task: &finite,
                scope: &finite_scope,
                processing_contract: &contract,
            })
            .unwrap(),
        ImportRootTaskHeadOutcome::HeadInserted { .. }
    ));

    let (unbounded, unbounded_scope) =
        configured_request_for_root("unbounded", ROOT, None, timestamp(3));
    let unbounded_outcome = store
        .coordinate_import_root_task_head(ImportRootTaskHeadRequest::Configured {
            task: &unbounded,
            scope: &unbounded_scope,
            processing_contract: &contract,
        })
        .unwrap();
    assert!(matches!(
        unbounded_outcome,
        ImportRootTaskHeadOutcome::HeadInserted {
            ref task,
            ref scope,
            ..
        } if task.id == unbounded.id
            && scope.scan_budget_kind.is_none()
            && scope.scan_budget_limit.is_none()
    ));
    assert!(store.is_import_task_cancelled(&finite.id).unwrap());

    let running = store
        .claim_observed_import_task_for_worker(&unbounded, timestamp(4))
        .unwrap()
        .unwrap();
    store
        .update_import_task_status(&running.id, ImportTaskStatus::FailedRetryable, timestamp(5))
        .unwrap();
    let (finite_retry, finite_retry_scope) =
        configured_request_for_root("finite-retry", ROOT, Some(5), timestamp(6));
    let finite_retry_outcome = store
        .coordinate_import_root_task_head(ImportRootTaskHeadRequest::Configured {
            task: &finite_retry,
            scope: &finite_retry_scope,
            processing_contract: &contract,
        })
        .unwrap();
    assert!(matches!(
        finite_retry_outcome,
        ImportRootTaskHeadOutcome::HeadInserted {
            ref task,
            ref scope,
            ..
        } if task.id == finite_retry.id
            && scope.scan_budget_kind == Some(ImportScanBudgetKind::Files)
            && scope.scan_budget_limit == Some(5)
    ));
    assert!(store.is_import_task_cancelled(&unbounded.id).unwrap());
    let running_finite_retry = store
        .claim_observed_import_task_for_worker(&finite_retry, timestamp(7))
        .unwrap()
        .unwrap();
    store
        .update_import_task_status(
            &running_finite_retry.id,
            ImportTaskStatus::FailedRetryable,
            timestamp(8),
        )
        .unwrap();

    let (same_budget, same_budget_scope) =
        configured_request_for_root("same-budget", ROOT, Some(5), timestamp(9));
    assert!(matches!(
        store
            .coordinate_import_root_task_head(ImportRootTaskHeadRequest::Configured {
                task: &same_budget,
                scope: &same_budget_scope,
                processing_contract: &contract,
            })
            .unwrap(),
        ImportRootTaskHeadOutcome::HeadRetained {
            task,
            scope,
            ..
        } if task.id == finite_retry.id
            && task.status == ImportTaskStatus::FailedRetryable
            && scope.scan_budget_limit == Some(5)
    ));
}

#[test]
fn configured_batch_rolls_back_every_root_when_a_later_root_is_running() {
    const FIRST_ROOT: &str = "/synthetic/a-first-root";
    const RUNNING_ROOT: &str = "/synthetic/z-running-root";
    let store = ready_store();
    let contract = processing_contract();
    let (running, running_scope) =
        configured_request_for_root("running", RUNNING_ROOT, None, timestamp(2));
    store
        .coordinate_import_root_task_head(ImportRootTaskHeadRequest::Configured {
            task: &running,
            scope: &running_scope,
            processing_contract: &contract,
        })
        .unwrap();
    store
        .claim_observed_import_task_for_worker(&running, timestamp(3))
        .unwrap()
        .unwrap();

    let (first, first_scope) = configured_request_for_root("first", FIRST_ROOT, None, timestamp(4));
    let (replacement, replacement_scope) =
        configured_request_for_root("replacement", RUNNING_ROOT, None, timestamp(4));
    let outcome = store
        .coordinate_import_root_task_heads(&[
            ImportRootTaskHeadRequest::Configured {
                task: &first,
                scope: &first_scope,
                processing_contract: &contract,
            },
            ImportRootTaskHeadRequest::Configured {
                task: &replacement,
                scope: &replacement_scope,
                processing_contract: &contract,
            },
        ])
        .unwrap();
    assert_eq!(
        outcome,
        ImportRootTaskHeadBatchOutcome::Rejected(
            ImportRootTaskHeadBatchRejection::RunningTaskConflict
        )
    );
    assert!(store
        .latest_import_task_by_root(FIRST_ROOT)
        .unwrap()
        .is_none());
    assert!(!store
        .active_authorized_import_roots()
        .unwrap()
        .iter()
        .any(|root| root == FIRST_ROOT));
    assert_eq!(
        store
            .latest_import_task_by_root(RUNNING_ROOT)
            .unwrap()
            .unwrap()
            .id,
        running.id
    );
    assert!(!store.is_import_task_cancelled(&running.id).unwrap());
}

#[test]
fn configured_batch_rolls_back_every_root_when_a_later_root_is_paused() {
    const FIRST_ROOT: &str = "/synthetic/a-paused-batch-first";
    const PAUSED_ROOT: &str = "/synthetic/z-paused-batch-root";
    let store = ready_store();
    let contract = processing_contract();
    let (paused, paused_scope) =
        configured_request_for_root("paused", PAUSED_ROOT, None, timestamp(2));
    store
        .coordinate_import_root_task_head(ImportRootTaskHeadRequest::Configured {
            task: &paused,
            scope: &paused_scope,
            processing_contract: &contract,
        })
        .unwrap();
    store.pause_import_root(PAUSED_ROOT, timestamp(3)).unwrap();

    let (first, first_scope) =
        configured_request_for_root("paused-first", FIRST_ROOT, None, timestamp(4));
    let (replacement, replacement_scope) =
        configured_request_for_root("paused-replacement", PAUSED_ROOT, None, timestamp(4));
    assert_eq!(
        store
            .coordinate_import_root_task_heads(&[
                ImportRootTaskHeadRequest::Configured {
                    task: &first,
                    scope: &first_scope,
                    processing_contract: &contract,
                },
                ImportRootTaskHeadRequest::Configured {
                    task: &replacement,
                    scope: &replacement_scope,
                    processing_contract: &contract,
                },
            ])
            .unwrap(),
        ImportRootTaskHeadBatchOutcome::Rejected(ImportRootTaskHeadBatchRejection::RootPaused)
    );
    assert!(store
        .latest_import_task_by_root(FIRST_ROOT)
        .unwrap()
        .is_none());
    assert!(!store
        .active_authorized_import_roots()
        .unwrap()
        .iter()
        .any(|root| root == FIRST_ROOT));
}

#[test]
fn configured_batch_rolls_back_every_root_on_a_repair_contract_mismatch() {
    const FIRST_ROOT: &str = "/synthetic/a-repair-batch-first";
    const REJECTED_ROOT: &str = "/synthetic/z-repair-batch-rejected";
    let store = EphemeralMetaStore::open_in_memory().unwrap();
    store.run_migrations().unwrap();
    let active_contract = processing_contract();
    let stale_contract = ImportProcessingContract::new(
        "stale-primary",
        "stale-ocr",
        "stale-derived",
        CLASSIFIER_EPOCH,
    )
    .unwrap();
    store
        .activate_migration_rebuild_contract(&active_contract, timestamp(1))
        .unwrap();
    let (first, first_scope) =
        configured_request_for_root("repair-first", FIRST_ROOT, None, timestamp(2));
    let (rejected, rejected_scope) =
        configured_request_for_root("repair-rejected", REJECTED_ROOT, None, timestamp(2));
    assert_eq!(
        store
            .coordinate_import_root_task_heads(&[
                ImportRootTaskHeadRequest::Configured {
                    task: &first,
                    scope: &first_scope,
                    processing_contract: &active_contract,
                },
                ImportRootTaskHeadRequest::Configured {
                    task: &rejected,
                    scope: &rejected_scope,
                    processing_contract: &stale_contract,
                },
            ])
            .unwrap(),
        ImportRootTaskHeadBatchOutcome::Rejected(
            ImportRootTaskHeadBatchRejection::MigrationRebuildSuperseded
        )
    );
    assert!(store
        .latest_import_task_by_root(FIRST_ROOT)
        .unwrap()
        .is_none());
    assert!(store
        .latest_import_task_by_root(REJECTED_ROOT)
        .unwrap()
        .is_none());
    assert!(store.active_authorized_import_roots().unwrap().is_empty());
}

#[test]
fn configured_source_change_supersedes_running_and_completed_migration_heads() {
    let store = owned_store();
    let contract = processing_contract();
    store
        .activate_migration_rebuild_contract(&contract, timestamp(1))
        .unwrap();
    let (first, first_scope) = configured_request("first-source", timestamp(2));
    store
        .coordinate_import_root_task_head(ImportRootTaskHeadRequest::Configured {
            task: &first,
            scope: &first_scope,
            processing_contract: &contract,
        })
        .unwrap();
    store
        .claim_observed_import_task_for_worker(&first, timestamp(3))
        .unwrap()
        .unwrap();

    let (second, second_scope) = configured_request("second-source", timestamp(4));
    let persisted_second_scope = match store
        .coordinate_import_root_task_head(ImportRootTaskHeadRequest::Configured {
            task: &second,
            scope: &second_scope,
            processing_contract: &contract,
        })
        .unwrap()
    {
        ImportRootTaskHeadOutcome::HeadInserted {
            task,
            scope,
            purpose: ImportTaskPurpose::MigrationRebuildFullCorpus,
            ..
        } if task.id == second.id && scope.scan_budget_limit.is_none() => scope,
        other => panic!("unexpected source-change outcome: {other:?}"),
    };
    assert!(store.is_import_task_cancelled(&first.id).unwrap());

    let running_second = store
        .claim_observed_import_task_for_worker(&second, timestamp(5))
        .unwrap()
        .unwrap();
    store
        .complete_import_task(
            &running_second.id,
            contract.id(),
            &persisted_second_scope,
            timestamp(6),
        )
        .unwrap();
    let barrier = store
        .acquire_migration_rebuild_barrier_token(contract.id())
        .unwrap()
        .unwrap();
    let generation = "source-change-superseded-publication".to_string();
    let projection_digest = SearchProjectionDigest::from_pairs::<_, &str, &str>([]).unwrap();
    let mut session = store.wait_for_search_publication_session().unwrap();
    let _attempt = match session
        .acquire_migration_rebuild_publication_attempt(&barrier, timestamp(6))
        .unwrap()
    {
        crate::MigrationRebuildPublicationAttemptAcquire::Started(attempt) => attempt,
        other => panic!("expected migration attempt, got {other:?}"),
    };
    assert_eq!(
        session
            .begin_search_publication(&SearchPublicationDraft {
                generation: generation.clone(),
                base_generation: None,
                expected_visible_epoch: 0,
                classifier_epoch: CLASSIFIER_EPOCH.to_string(),
                projection_digest: projection_digest.clone(),
                now: timestamp(6),
            })
            .unwrap(),
        SearchPublicationOutcome::Applied
    );
    let fulltext = FullTextSnapshotDescriptor::new(
        generation.clone(),
        0,
        projection_digest.clone(),
        ContentDigest::from_bytes(b"source-change-fulltext"),
    );
    let vector = VectorSnapshotDescriptor::disabled(
        generation.clone(),
        0,
        projection_digest,
        SearchProjectionDigest::from_pairs::<_, &str, &str>([]).unwrap(),
        ContentDigest::from_bytes(b"source-change-vector"),
    );
    session
        .validate_search_publication(&SearchPublicationValidation {
            generation: &generation,
            fulltext: &fulltext,
            vector: &vector,
            now: timestamp(6),
        })
        .unwrap();

    let (third, third_scope) = configured_request("third-source", timestamp(7));
    assert!(matches!(
        store
            .coordinate_import_root_task_head(ImportRootTaskHeadRequest::Configured {
                task: &third,
                scope: &third_scope,
                processing_contract: &contract,
            })
            .unwrap(),
        ImportRootTaskHeadOutcome::HeadInserted {
            task,
            purpose: ImportTaskPurpose::MigrationRebuildFullCorpus,
            ..
        } if task.id == third.id
    ));
    assert_eq!(
        store.latest_import_task_by_root(ROOT).unwrap().unwrap().id,
        third.id
    );
    assert!(store.is_import_task_cancelled(&second.id).unwrap());
    assert_eq!(
        session
            .commit_migration_rebuild_search_publication(
                &SearchPublicationCommit {
                    generation: &generation,
                    terminal_documents: &[],
                    projections: &[],
                    projected_documents: &[],
                    vector_coverage: &[],
                    now: timestamp(7),
                },
                &barrier,
            )
            .unwrap(),
        SearchPublicationOutcome::Superseded
    );
}

#[test]
fn migration_pending_head_reuse_includes_the_configured_budget_identity() {
    let store = EphemeralMetaStore::open_in_memory().unwrap();
    store.run_migrations().unwrap();
    let contract = processing_contract();
    store
        .activate_migration_rebuild_contract(&contract, timestamp(1))
        .unwrap();
    let (finite, finite_scope) = configured_request("migration-finite", timestamp(2));
    store
        .coordinate_import_root_task_head(ImportRootTaskHeadRequest::Configured {
            task: &finite,
            scope: &finite_scope,
            processing_contract: &contract,
        })
        .unwrap();

    let (unbounded, unbounded_scope) =
        configured_request_for_root("migration-unbounded", ROOT, None, timestamp(3));
    assert!(matches!(
        store
            .coordinate_import_root_task_head(ImportRootTaskHeadRequest::Configured {
                task: &unbounded,
                scope: &unbounded_scope,
                processing_contract: &contract,
            })
            .unwrap(),
        ImportRootTaskHeadOutcome::HeadInserted {
            task,
            scope,
            purpose: ImportTaskPurpose::MigrationRebuildFullCorpus,
            ..
        } if task.id == unbounded.id && scope.scan_budget_limit.is_none()
    ));
    assert!(store.is_import_task_cancelled(&finite.id).unwrap());

    let (same_unbounded, same_unbounded_scope) =
        configured_request_for_root("migration-same-unbounded", ROOT, None, timestamp(4));
    assert!(matches!(
        store
            .coordinate_import_root_task_head(ImportRootTaskHeadRequest::Configured {
                task: &same_unbounded,
                scope: &same_unbounded_scope,
                processing_contract: &contract,
            })
            .unwrap(),
        ImportRootTaskHeadOutcome::HeadRetained { task, scope, .. }
            if task.id == unbounded.id && scope.import_task_id == unbounded.id
    ));
}

fn insert_migration_task(
    store: &EphemeralMetaStore,
    task: &ImportTask,
    scope: &ImportScanScope,
    contract: &ImportProcessingContract,
) {
    store
        .insert_import_task_with_scan_scope(task, scope, contract)
        .unwrap();
    let connection = store.connection.borrow();
    insert_migration_rebuild_full_corpus_task_marker_in_connection(
        &connection,
        &task.id,
        contract.id(),
    )
    .unwrap();
}

fn configured_request(label: &str, now: UnixTimestamp) -> (ImportTask, ImportScanScope) {
    configured_request_for_root(label, ROOT, Some(10), now)
}

fn configured_request_for_root(
    label: &str,
    root: &str,
    scan_budget_limit: Option<u64>,
    now: UnixTimestamp,
) -> (ImportTask, ImportScanScope) {
    let task = ImportTask {
        id: ImportTaskId::from_non_secret_parts(&[label]),
        root_path: root.to_string(),
        status: ImportTaskStatus::Queued,
        queued_at: now,
        started_at: None,
        finished_at: None,
        updated_at: now,
    };
    let scope = ImportScanScope {
        import_task_id: task.id.clone(),
        root_kind: ImportRootKind::Explicit,
        root_preset: None,
        scan_profile: ImportScanProfile::Explicit,
        requested_root_path: root.to_string(),
        canonical_root_path: root.to_string(),
        files_discovered: 0,
        ignored_entries: 0,
        scan_errors: 0,
        searchable_documents: 0,
        ocr_required_documents: 0,
        ocr_jobs_queued: 0,
        failed_documents: 0,
        deleted_documents: 0,
        scan_budget_kind: scan_budget_limit.map(|_| ImportScanBudgetKind::Files),
        scan_budget_limit,
        scan_budget_observed: scan_budget_limit.map(|_| 0),
        scan_budget_exhausted: false,
        updated_at: now,
    };
    (task, scope)
}

fn migration_task(label: &str, now: UnixTimestamp) -> (ImportTask, ImportScanScope) {
    let task = ImportTask {
        id: ImportTaskId::from_non_secret_parts(&[label]),
        root_path: ROOT.to_string(),
        status: ImportTaskStatus::Queued,
        queued_at: now,
        started_at: None,
        finished_at: None,
        updated_at: now,
    };
    let scope = ImportScanScope {
        import_task_id: task.id.clone(),
        root_kind: ImportRootKind::Explicit,
        root_preset: None,
        scan_profile: ImportScanProfile::Explicit,
        requested_root_path: ROOT.to_string(),
        canonical_root_path: ROOT.to_string(),
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
    (task, scope)
}

fn processing_contract() -> ImportProcessingContract {
    ImportProcessingContract::new("primary-v28", "ocr-v28", "derived-v28", CLASSIFIER_EPOCH)
        .unwrap()
}

struct OwnedTestStore {
    store: OwnedMetaStore,
    _owner: DataDirectoryOwnerLease,
    _directory: TempDir,
}

impl Deref for OwnedTestStore {
    type Target = OwnedMetaStore;

    fn deref(&self) -> &Self::Target {
        &self.store
    }
}

fn owned_store() -> OwnedTestStore {
    let directory = tempdir().unwrap();
    let owner = match DataDirectoryOwnerLease::try_acquire(directory.path()).unwrap() {
        DataDirectoryOwnerAcquisition::Acquired(owner) => owner,
        DataDirectoryOwnerAcquisition::Contended => panic!("test data directory was contended"),
    };
    let store = owner.open_store().unwrap();
    OwnedTestStore {
        store,
        _owner: owner,
        _directory: directory,
    }
}

fn ready_store() -> OwnedTestStore {
    let store = owned_store();
    let contract = processing_contract();
    store
        .activate_migration_rebuild_contract(&contract, timestamp(1))
        .unwrap();
    let barrier = store
        .acquire_migration_rebuild_barrier_token(contract.id())
        .unwrap()
        .unwrap();
    let generation = "root-head-ready".to_string();
    let projection_digest = SearchProjectionDigest::from_pairs::<_, &str, &str>([]).unwrap();
    let draft = SearchPublicationDraft {
        generation: generation.clone(),
        base_generation: None,
        expected_visible_epoch: 0,
        classifier_epoch: CLASSIFIER_EPOCH.to_string(),
        projection_digest: projection_digest.clone(),
        now: timestamp(1),
    };
    let mut session = store.wait_for_search_publication_session().unwrap();
    let _attempt = match session
        .acquire_migration_rebuild_publication_attempt(&barrier, timestamp(1))
        .unwrap()
    {
        crate::MigrationRebuildPublicationAttemptAcquire::Started(attempt) => attempt,
        other => panic!("expected migration attempt, got {other:?}"),
    };
    assert_eq!(
        session.begin_search_publication(&draft).unwrap(),
        SearchPublicationOutcome::Applied
    );
    let fulltext = FullTextSnapshotDescriptor::new(
        generation.clone(),
        0,
        projection_digest.clone(),
        ContentDigest::from_bytes(generation.as_bytes()),
    );
    let vector = VectorSnapshotDescriptor::disabled(
        generation.clone(),
        0,
        projection_digest,
        SearchProjectionDigest::from_pairs::<_, &str, &str>([]).unwrap(),
        ContentDigest::from_bytes(b"root-head-ready-vector"),
    );
    session
        .validate_search_publication(&SearchPublicationValidation {
            generation: &generation,
            fulltext: &fulltext,
            vector: &vector,
            now: timestamp(1),
        })
        .unwrap();
    assert_eq!(
        session
            .commit_migration_rebuild_search_publication(
                &SearchPublicationCommit {
                    generation: &generation,
                    terminal_documents: &[],
                    projections: &[],
                    projected_documents: &[],
                    vector_coverage: &[],
                    now: timestamp(1),
                },
                &barrier,
            )
            .unwrap(),
        SearchPublicationOutcome::Applied
    );
    store
}

fn timestamp(value: i64) -> UnixTimestamp {
    UnixTimestamp::from_unix_seconds(value)
}

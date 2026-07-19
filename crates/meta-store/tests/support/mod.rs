#![allow(dead_code)]

use core_domain::{ContentDigest, SearchProjectionDigest};
use meta_store::{
    ActiveSearchProjection, DataDirectoryOwnerAcquisition, DataDirectoryOwnerLease,
    EphemeralMetaStore, FullTextSnapshotDescriptor, ImportProcessingContract, ImportRootKind,
    ImportScanProfile, ImportScanScope, ImportTask, ImportTaskStatus, MigrationRebuildBarrierToken,
    OwnedMetaStore, ProjectedDocumentSnapshot, SearchProjectionServiceState,
    SearchPublicationCommit, SearchPublicationDraft, SearchPublicationOutcome,
    SearchPublicationValidation, TerminalDocumentUpdate, UnixTimestamp, VectorSnapshotDescriptor,
    CLASSIFIER_EPOCH,
};
use tempfile::TempDir;

pub fn owned_store() -> (TempDir, OwnedMetaStore) {
    let directory = tempfile::tempdir().unwrap();
    let data_dir = directory.path().join("data");
    std::fs::create_dir_all(&data_dir).unwrap();
    let owner = match DataDirectoryOwnerLease::try_acquire(&data_dir).unwrap() {
        DataDirectoryOwnerAcquisition::Acquired(owner) => owner,
        DataDirectoryOwnerAcquisition::Contended => panic!("synthetic data directory contended"),
    };
    let store = owner.open_store().unwrap();
    (directory, store)
}

pub fn processing_contract() -> ImportProcessingContract {
    ImportProcessingContract::new("parser-v1", "ocr-parser-v1", "schema-v28", CLASSIFIER_EPOCH)
        .unwrap()
}

pub fn activate_processing_contract(
    store: &EphemeralMetaStore,
    activated_at: UnixTimestamp,
) -> ImportProcessingContract {
    let contract = processing_contract();
    store
        .activate_migration_rebuild_contract(&contract, activated_at)
        .unwrap();
    contract
}

pub fn activate_processing_contract_owned(
    store: &OwnedMetaStore,
    activated_at: UnixTimestamp,
) -> ImportProcessingContract {
    let contract = processing_contract();
    store
        .activate_migration_rebuild_contract(&contract, activated_at)
        .unwrap();
    contract
}

pub fn complete_import_task_with_empty_manifest(
    store: &EphemeralMetaStore,
    observed: &ImportTask,
    started_at: UnixTimestamp,
    completed_at: UnixTimestamp,
) {
    let final_scope = store
        .import_scan_scope_by_task_id(&observed.id)
        .unwrap()
        .unwrap();
    complete_import_task_with_final_scope(store, observed, &final_scope, started_at, completed_at);
}

pub fn complete_import_task_with_final_scope(
    store: &EphemeralMetaStore,
    observed: &ImportTask,
    final_scope: &ImportScanScope,
    started_at: UnixTimestamp,
    completed_at: UnixTimestamp,
) {
    let contract = activate_processing_contract(store, observed.queued_at);
    let running = store
        .claim_observed_import_task_for_worker(observed, started_at)
        .unwrap()
        .unwrap();
    assert_eq!(final_scope.import_task_id, running.id);
    assert_eq!(final_scope.files_discovered, 0);
    let mut final_scope = final_scope.clone();
    final_scope.updated_at = completed_at;
    store
        .complete_import_task(&running.id, contract.id(), &final_scope, completed_at)
        .unwrap();
}

pub fn complete_import_task_with_empty_manifest_owned(
    store: &OwnedMetaStore,
    observed: &ImportTask,
    started_at: UnixTimestamp,
    completed_at: UnixTimestamp,
) {
    let final_scope = store
        .import_scan_scope_by_task_id(&observed.id)
        .unwrap()
        .unwrap();
    complete_import_task_with_final_scope_owned(
        store,
        observed,
        &final_scope,
        started_at,
        completed_at,
    );
}

pub fn complete_import_task_with_final_scope_owned(
    store: &OwnedMetaStore,
    observed: &ImportTask,
    final_scope: &ImportScanScope,
    started_at: UnixTimestamp,
    completed_at: UnixTimestamp,
) {
    let contract = activate_processing_contract_owned(store, observed.queued_at);
    let running = store
        .claim_observed_import_task_for_worker(observed, started_at)
        .unwrap()
        .unwrap();
    assert_eq!(final_scope.import_task_id, running.id);
    assert_eq!(final_scope.files_discovered, 0);
    let mut final_scope = final_scope.clone();
    final_scope.updated_at = completed_at;
    store
        .complete_import_task(&running.id, contract.id(), &final_scope, completed_at)
        .unwrap();
}

pub fn import_scan_scope(task: &ImportTask) -> ImportScanScope {
    ImportScanScope {
        import_task_id: task.id.clone(),
        root_kind: ImportRootKind::Explicit,
        root_preset: None,
        scan_profile: ImportScanProfile::Explicit,
        requested_root_path: task.root_path.clone(),
        canonical_root_path: task.root_path.clone(),
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
        updated_at: task.updated_at,
    }
}

pub fn insert_import_task(store: &EphemeralMetaStore, task: &ImportTask) {
    insert_import_task_with_scan_scope(store, task, &import_scan_scope(task));
}

pub fn insert_import_task_with_scan_scope(
    store: &EphemeralMetaStore,
    task: &ImportTask,
    scope: &ImportScanScope,
) {
    let queued = ImportTask {
        id: task.id.clone(),
        root_path: task.root_path.clone(),
        status: ImportTaskStatus::Queued,
        queued_at: task.queued_at,
        started_at: None,
        finished_at: None,
        updated_at: task.queued_at,
    };
    let mut initial_scope = scope.clone();
    initial_scope.updated_at = task.queued_at;
    let contract = activate_processing_contract(store, task.queued_at);
    store
        .insert_import_task_with_scan_scope(&queued, &initial_scope, &contract)
        .unwrap();

    if task.status == ImportTaskStatus::Queued {
        if scope.updated_at != initial_scope.updated_at {
            store.upsert_import_scan_scope(scope).unwrap();
        }
        return;
    }

    let started_at = task.started_at.unwrap_or(task.updated_at);
    let claimed = store
        .claim_observed_import_task_for_worker(&queued, started_at)
        .unwrap()
        .unwrap();
    if task.status == ImportTaskStatus::Running {
        if task.updated_at != claimed.updated_at {
            assert!(store
                .heartbeat_running_import_task(&task.id, task.updated_at)
                .unwrap());
        }
        store.upsert_import_scan_scope(scope).unwrap();
        return;
    }

    let finished_at = task.finished_at.unwrap_or(task.updated_at);
    match task.status {
        ImportTaskStatus::FailedRetryable | ImportTaskStatus::FailedPermanent => {
            store
                .update_import_task_status(&task.id, task.status, finished_at)
                .unwrap();
            store.upsert_import_scan_scope(scope).unwrap();
        }
        ImportTaskStatus::Completed => {
            assert_eq!(scope.files_discovered, 0);
            let mut final_scope = scope.clone();
            final_scope.updated_at = finished_at;
            store
                .complete_import_task(&task.id, contract.id(), &final_scope, finished_at)
                .unwrap();
        }
        ImportTaskStatus::Queued | ImportTaskStatus::Running => unreachable!(),
    }
}

pub fn insert_import_task_owned(store: &OwnedMetaStore, task: &ImportTask) {
    insert_import_task_with_scan_scope_owned(store, task, &import_scan_scope(task));
}

pub fn insert_import_task_with_scan_scope_owned(
    store: &OwnedMetaStore,
    task: &ImportTask,
    scope: &ImportScanScope,
) {
    ensure_ready_empty_search_owned(store, task.queued_at);
    let queued = ImportTask {
        id: task.id.clone(),
        root_path: task.root_path.clone(),
        status: ImportTaskStatus::Queued,
        queued_at: task.queued_at,
        started_at: None,
        finished_at: None,
        updated_at: task.queued_at,
    };
    let mut initial_scope = scope.clone();
    initial_scope.updated_at = task.queued_at;
    let contract = activate_processing_contract_owned(store, task.queued_at);
    store
        .insert_import_task_with_scan_scope(&queued, &initial_scope, &contract)
        .unwrap();

    if task.status == ImportTaskStatus::Queued {
        if scope.updated_at != initial_scope.updated_at {
            store.upsert_import_scan_scope(scope).unwrap();
        }
        return;
    }

    let started_at = task.started_at.unwrap_or(task.updated_at);
    let claimed = store
        .claim_observed_import_task_for_worker(&queued, started_at)
        .unwrap()
        .unwrap();
    if task.status == ImportTaskStatus::Running {
        if task.updated_at != claimed.updated_at {
            assert!(store
                .heartbeat_running_import_task(&task.id, task.updated_at)
                .unwrap());
        }
        store.upsert_import_scan_scope(scope).unwrap();
        return;
    }

    let finished_at = task.finished_at.unwrap_or(task.updated_at);
    match task.status {
        ImportTaskStatus::FailedRetryable | ImportTaskStatus::FailedPermanent => {
            store
                .update_import_task_status(&task.id, task.status, finished_at)
                .unwrap();
            store.upsert_import_scan_scope(scope).unwrap();
        }
        ImportTaskStatus::Completed => {
            assert_eq!(scope.files_discovered, 0);
            let mut final_scope = scope.clone();
            final_scope.updated_at = finished_at;
            store
                .complete_import_task(&task.id, contract.id(), &final_scope, finished_at)
                .unwrap();
        }
        ImportTaskStatus::Queued | ImportTaskStatus::Running => unreachable!(),
    }
}

pub fn insert_migration_rebuild_import_task_with_scan_scope(
    store: &EphemeralMetaStore,
    task: &ImportTask,
    scope: &ImportScanScope,
) {
    assert_eq!(scope.import_task_id, task.id);
    assert_eq!(scope.canonical_root_path, task.root_path);
    let contract = activate_processing_contract(store, task.queued_at);
    let queued_task = ImportTask {
        id: task.id.clone(),
        root_path: task.root_path.clone(),
        status: ImportTaskStatus::Queued,
        queued_at: task.queued_at,
        started_at: None,
        finished_at: None,
        updated_at: task.queued_at,
    };
    assert!(matches!(
        store
            .coordinate_import_root_task_head(meta_store::ImportRootTaskHeadRequest::Configured {
                task: &queued_task,
                scope,
                processing_contract: &contract,
            })
            .unwrap(),
        meta_store::ImportRootTaskHeadOutcome::HeadInserted { .. }
    ));
    let queued = store.import_task_by_id(&task.id).unwrap().unwrap();
    if task.status == ImportTaskStatus::Queued {
        return;
    }

    let started_at = task.started_at.unwrap_or(task.updated_at);
    let claimed = store
        .claim_observed_import_task_for_worker(&queued, started_at)
        .unwrap()
        .unwrap();
    if task.status == ImportTaskStatus::Running {
        if task.updated_at != claimed.updated_at {
            assert!(store
                .heartbeat_running_import_task(&task.id, task.updated_at)
                .unwrap());
        }
        return;
    }

    let finished_at = task.finished_at.unwrap_or(task.updated_at);
    match task.status {
        ImportTaskStatus::FailedRetryable | ImportTaskStatus::FailedPermanent => {
            store
                .update_import_task_status(&task.id, task.status, finished_at)
                .unwrap();
        }
        ImportTaskStatus::Completed => {
            assert_eq!(scope.files_discovered, 0);
            let mut final_scope = scope.clone();
            final_scope.updated_at = finished_at;
            store
                .complete_import_task(&task.id, contract.id(), &final_scope, finished_at)
                .unwrap();
        }
        ImportTaskStatus::Queued | ImportTaskStatus::Running => unreachable!(),
    }
}

pub fn insert_migration_rebuild_import_task_with_scan_scope_owned(
    store: &OwnedMetaStore,
    task: &ImportTask,
    scope: &ImportScanScope,
) {
    assert_eq!(scope.import_task_id, task.id);
    assert_eq!(scope.canonical_root_path, task.root_path);
    let contract = activate_processing_contract_owned(store, task.queued_at);
    let queued_task = ImportTask {
        id: task.id.clone(),
        root_path: task.root_path.clone(),
        status: ImportTaskStatus::Queued,
        queued_at: task.queued_at,
        started_at: None,
        finished_at: None,
        updated_at: task.queued_at,
    };
    assert!(matches!(
        store
            .coordinate_import_root_task_head(meta_store::ImportRootTaskHeadRequest::Configured {
                task: &queued_task,
                scope,
                processing_contract: &contract,
            })
            .unwrap(),
        meta_store::ImportRootTaskHeadOutcome::HeadInserted { .. }
    ));
    let queued = store.import_task_by_id(&task.id).unwrap().unwrap();
    if task.status == ImportTaskStatus::Queued {
        return;
    }

    let started_at = task.started_at.unwrap_or(task.updated_at);
    let claimed = store
        .claim_observed_import_task_for_worker(&queued, started_at)
        .unwrap()
        .unwrap();
    if task.status == ImportTaskStatus::Running {
        if task.updated_at != claimed.updated_at {
            assert!(store
                .heartbeat_running_import_task(&task.id, task.updated_at)
                .unwrap());
        }
        return;
    }

    let finished_at = task.finished_at.unwrap_or(task.updated_at);
    match task.status {
        ImportTaskStatus::FailedRetryable | ImportTaskStatus::FailedPermanent => {
            store
                .update_import_task_status(&task.id, task.status, finished_at)
                .unwrap();
        }
        ImportTaskStatus::Completed => {
            assert_eq!(scope.files_discovered, 0);
            let mut final_scope = scope.clone();
            final_scope.updated_at = finished_at;
            store
                .complete_import_task(&task.id, contract.id(), &final_scope, finished_at)
                .unwrap();
        }
        ImportTaskStatus::Queued | ImportTaskStatus::Running => unreachable!(),
    }
}

pub fn ensure_ready_empty_search_owned(store: &OwnedMetaStore, now: UnixTimestamp) {
    if store.search_projection_state().unwrap().service_state == SearchProjectionServiceState::Ready
    {
        return;
    }
    let barrier = acquire_migration_rebuild_barrier_owned(store, now);
    let generation = format!("meta-test-empty-ready-{}", now.as_unix_seconds());
    let projection_digest = SearchProjectionDigest::from_pairs::<_, &str, &str>([]).unwrap();
    let draft = SearchPublicationDraft {
        generation: generation.clone(),
        base_generation: None,
        expected_visible_epoch: 0,
        classifier_epoch: CLASSIFIER_EPOCH.to_string(),
        projection_digest: projection_digest.clone(),
        now,
    };
    let session = store.wait_for_search_publication_session().unwrap();
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
        ContentDigest::from_bytes(format!("vector:{generation}").as_bytes()),
    );
    session
        .validate_search_publication(&SearchPublicationValidation {
            generation: &generation,
            fulltext: &fulltext,
            vector: &vector,
            now,
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
                    now,
                },
                &barrier,
            )
            .unwrap(),
        SearchPublicationOutcome::Applied
    );
}

pub fn projected_documents_for_commit(
    store: &OwnedMetaStore,
    projections: &[ActiveSearchProjection],
    terminal_documents: &[TerminalDocumentUpdate],
    now: UnixTimestamp,
) -> Vec<ProjectedDocumentSnapshot> {
    projections
        .iter()
        .map(|projection| {
            let retained = store
                .active_search_projection_for_document(&projection.document_id)
                .unwrap()
                .as_ref()
                == Some(projection);
            if retained {
                return ProjectedDocumentSnapshot::RetainedUnchanged {
                    projection: projection.clone(),
                };
            }
            let mut document = store
                .document_by_id(&projection.document_id)
                .unwrap()
                .expect("replacement projection document must exist");
            if let Some(terminal) = terminal_documents
                .iter()
                .find(|terminal| terminal.document_id == projection.document_id)
            {
                document.status = terminal.terminal_status;
                document.is_deleted = terminal.terminal_is_deleted;
                document.updated_at = now;
            }
            ProjectedDocumentSnapshot::Replacement {
                projection: projection.clone(),
                document,
            }
        })
        .collect()
}

pub fn acquire_migration_rebuild_barrier_owned(
    store: &OwnedMetaStore,
    activated_at: UnixTimestamp,
) -> MigrationRebuildBarrierToken {
    let contract = activate_processing_contract_owned(store, activated_at);
    store
        .acquire_migration_rebuild_barrier_token(contract.id())
        .unwrap()
        .unwrap()
}

pub fn acquire_migration_rebuild_barrier(
    store: &EphemeralMetaStore,
    activated_at: UnixTimestamp,
) -> MigrationRebuildBarrierToken {
    let contract = activate_processing_contract(store, activated_at);
    store
        .acquire_migration_rebuild_barrier_token(contract.id())
        .unwrap()
        .unwrap()
}

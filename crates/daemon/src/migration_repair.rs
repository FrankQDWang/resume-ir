use meta_store::{
    ImportProcessingContract, ImportRootTaskHeadOutcome, ImportRootTaskHeadRequest, ImportTaskId,
    OwnedMetaStore, SearchProjectionServiceState, SearchProjectionState, SearchRepairReason,
    UnixTimestamp,
};

use super::{DaemonError, Result};

pub(super) fn enqueue_authorized_roots(
    store: &OwnedMetaStore,
    processing_contract: &ImportProcessingContract,
    now: UnixTimestamp,
) -> Result<usize> {
    reconcile_authorized_roots(store, processing_contract, now)
}

pub(super) fn reconcile_authorized_roots(
    store: &OwnedMetaStore,
    processing_contract: &ImportProcessingContract,
    now: UnixTimestamp,
) -> Result<usize> {
    let state = store
        .search_projection_state()
        .map_err(DaemonError::store)?;
    if !is_unpublished_migration_rebuild(&state) {
        return Ok(0);
    }

    let roots = store
        .active_authorized_import_roots()
        .map_err(DaemonError::store)?;
    let mut queued = 0;
    for (index, root) in roots.into_iter().enumerate() {
        let task_id: ImportTaskId = crate::import_command::new_task_id(index)
            .map_err(|_| DaemonError::user("system clock is before unix epoch"))?;
        match store
            .coordinate_import_root_task_head(ImportRootTaskHeadRequest::MigrationRebuild {
                canonical_root_path: &root,
                task_id: &task_id,
                processing_contract,
                queued_at: now,
            })
            .map_err(DaemonError::store)?
        {
            ImportRootTaskHeadOutcome::HeadInserted { .. } => queued += 1,
            ImportRootTaskHeadOutcome::HeadPromoted { .. }
            | ImportRootTaskHeadOutcome::HeadRetained { .. }
            | ImportRootTaskHeadOutcome::RunningTaskConflict
            | ImportRootTaskHeadOutcome::RootPaused
            | ImportRootTaskHeadOutcome::MigrationRebuildSuperseded => {}
        }
    }
    Ok(queued)
}

fn is_unpublished_migration_rebuild(state: &SearchProjectionState) -> bool {
    state.service_state == SearchProjectionServiceState::Repairing
        && state.repair_reason == Some(SearchRepairReason::MigrationRebuild)
        && state.generation.is_none()
        && state.publication.is_none()
}

#[cfg(test)]
mod tests {
    use std::ops::Deref;

    use import_pipeline::{
        current_import_processing_contract, DataDirectoryOwnerAcquisition, DataDirectoryOwnerLease,
        ImportOptions,
    };
    use meta_store::{
        ImportProcessingContract, ImportRootControlStatus, ImportRootKind, ImportScanBudgetKind,
        ImportScanProfile, ImportScanScope, ImportTask, ImportTaskId, ImportTaskPurpose,
        ImportTaskStatus, OwnedMetaStore, SearchRepairReason, UnixTimestamp,
    };
    use tempfile::TempDir;

    use super::{enqueue_authorized_roots, reconcile_authorized_roots};

    #[test]
    fn first_tick_replaces_a_configured_head_with_exact_full_corpus_purpose() {
        let (store, contract) = store_with_authorized_root();
        let now = UnixTimestamp::from_unix_seconds(4);

        assert_eq!(enqueue_authorized_roots(&store, &contract, now).unwrap(), 1);
        assert_eq!(enqueue_authorized_roots(&store, &contract, now).unwrap(), 0);
        assert_eq!(
            store
                .pending_import_task_by_root("/synthetic/authorized")
                .unwrap()
                .unwrap()
                .status,
            ImportTaskStatus::Queued
        );
        let task = store
            .latest_import_task_by_root("/synthetic/authorized")
            .unwrap()
            .unwrap();
        assert_eq!(
            store.import_task_purpose(&task.id).unwrap(),
            ImportTaskPurpose::MigrationRebuildFullCorpus
        );
    }

    #[test]
    fn budgeted_root_rebuild_uses_unbounded_scope_and_closes_barrier() {
        let (store, contract) = store_with_authorized_root_and_budget(Some(1), true);
        let queued_at = UnixTimestamp::from_unix_seconds(4);
        assert_eq!(
            enqueue_authorized_roots(&store, &contract, queued_at).unwrap(),
            1
        );
        let task = store
            .latest_import_task_by_root("/synthetic/authorized")
            .unwrap()
            .unwrap();
        let scope = store
            .import_scan_scope_by_task_id(&task.id)
            .unwrap()
            .unwrap();
        assert_eq!(scope.scan_budget_kind, None);
        assert_eq!(scope.scan_budget_limit, None);
        assert_eq!(scope.scan_budget_observed, None);
        assert_eq!(
            store.import_task_purpose(&task.id).unwrap(),
            ImportTaskPurpose::MigrationRebuildFullCorpus
        );

        let running = store
            .claim_observed_import_task_for_worker(&task, UnixTimestamp::from_unix_seconds(5))
            .unwrap()
            .unwrap();
        let mut final_scope = scope;
        final_scope.updated_at = UnixTimestamp::from_unix_seconds(6);
        store
            .complete_import_task(
                &running.id,
                contract.id(),
                &final_scope,
                UnixTimestamp::from_unix_seconds(6),
            )
            .unwrap();
        assert!(store
            .acquire_migration_rebuild_barrier_token(contract.id())
            .unwrap()
            .is_some());
    }

    #[test]
    fn non_migration_repair_never_replays_authorized_roots() {
        let (store, contract) = store_with_authorized_root();
        store
            .block_migration_rebuild(
                SearchRepairReason::SourceUnavailable,
                UnixTimestamp::from_unix_seconds(4),
            )
            .unwrap();

        assert_eq!(
            enqueue_authorized_roots(&store, &contract, UnixTimestamp::from_unix_seconds(5),)
                .unwrap(),
            0
        );
        assert_eq!(
            reconcile_authorized_roots(&store, &contract, UnixTimestamp::from_unix_seconds(5),)
                .unwrap(),
            0
        );
        assert!(store
            .pending_import_task_by_root("/synthetic/authorized")
            .unwrap()
            .is_none());
    }

    #[test]
    fn migration_rebuild_reconciles_a_cancelled_latest_root_exactly_once() {
        let (store, contract) = store_with_authorized_root();
        let first_rebuild_at = UnixTimestamp::from_unix_seconds(4);
        assert_eq!(
            enqueue_authorized_roots(&store, &contract, first_rebuild_at).unwrap(),
            1
        );
        let cancelled = store
            .latest_import_task_by_root("/synthetic/authorized")
            .unwrap()
            .unwrap();
        store
            .cancel_import_task(&cancelled.id, UnixTimestamp::from_unix_seconds(5))
            .unwrap();

        assert_eq!(
            reconcile_authorized_roots(&store, &contract, UnixTimestamp::from_unix_seconds(6),)
                .unwrap(),
            1
        );
        assert_eq!(
            reconcile_authorized_roots(&store, &contract, UnixTimestamp::from_unix_seconds(7),)
                .unwrap(),
            0
        );
        let replacement = store
            .latest_import_task_by_root("/synthetic/authorized")
            .unwrap()
            .unwrap();
        assert_ne!(replacement.id, cancelled.id);
        assert_eq!(replacement.status, ImportTaskStatus::Queued);
        assert!(!store.is_import_task_cancelled(&replacement.id).unwrap());
    }

    #[test]
    fn paused_root_stays_excluded_and_resume_is_reconciled_to_full_corpus() {
        let (store, contract) = store_with_authorized_root();
        let root = "/synthetic/authorized";
        let paused = store
            .pause_import_root(root, UnixTimestamp::from_unix_seconds(3))
            .unwrap();
        assert_eq!(paused.status, ImportRootControlStatus::Paused);
        assert_eq!(
            enqueue_authorized_roots(&store, &contract, UnixTimestamp::from_unix_seconds(4))
                .unwrap(),
            0
        );

        let catch_up_task_id = ImportTaskId::from_non_secret_parts(&["configured-resume"]);
        let resumed = store
            .resume_import_root(
                root,
                &catch_up_task_id,
                &contract,
                UnixTimestamp::from_unix_seconds(5),
            )
            .unwrap();
        assert!(resumed.catch_up_queued);
        assert_eq!(
            store.import_task_purpose(&catch_up_task_id).unwrap(),
            ImportTaskPurpose::MigrationRebuildFullCorpus
        );

        assert_eq!(
            reconcile_authorized_roots(&store, &contract, UnixTimestamp::from_unix_seconds(6),)
                .unwrap(),
            0
        );
        let replacement = store.latest_import_task_by_root(root).unwrap().unwrap();
        assert_eq!(replacement.id, catch_up_task_id);
        assert_eq!(
            store.import_task_purpose(&replacement.id).unwrap(),
            ImportTaskPurpose::MigrationRebuildFullCorpus
        );
    }

    #[test]
    fn migration_rebuild_replaces_a_same_contract_budgeted_head() {
        let (store, contract) = store_with_authorized_root_and_budget(Some(1), false);
        let previous = store
            .latest_import_task_by_root("/synthetic/authorized")
            .unwrap()
            .unwrap();

        assert_eq!(
            reconcile_authorized_roots(&store, &contract, UnixTimestamp::from_unix_seconds(4),)
                .unwrap(),
            1
        );
        let replacement = store
            .latest_import_task_by_root("/synthetic/authorized")
            .unwrap()
            .unwrap();
        assert_ne!(replacement.id, previous.id);
        let replacement_scope = store
            .import_scan_scope_by_task_id(&replacement.id)
            .unwrap()
            .unwrap();
        assert_eq!(replacement_scope.scan_budget_kind, None);
        assert_eq!(replacement_scope.scan_budget_limit, None);
        assert_eq!(replacement_scope.scan_budget_observed, None);
    }

    #[test]
    fn migration_rebuild_replaces_a_terminal_failed_full_corpus_head() {
        let (store, contract) = store_with_authorized_root();
        assert_eq!(
            enqueue_authorized_roots(&store, &contract, UnixTimestamp::from_unix_seconds(4))
                .unwrap(),
            1
        );
        let failed = store
            .latest_import_task_by_root("/synthetic/authorized")
            .unwrap()
            .unwrap();
        store
            .claim_observed_import_task_for_worker(&failed, UnixTimestamp::from_unix_seconds(5))
            .unwrap()
            .unwrap();
        store
            .update_import_task_status(
                &failed.id,
                ImportTaskStatus::FailedPermanent,
                UnixTimestamp::from_unix_seconds(6),
            )
            .unwrap();

        assert_eq!(
            reconcile_authorized_roots(&store, &contract, UnixTimestamp::from_unix_seconds(7),)
                .unwrap(),
            1
        );
        let replacement = store
            .latest_import_task_by_root("/synthetic/authorized")
            .unwrap()
            .unwrap();
        assert_ne!(replacement.id, failed.id);
        assert_eq!(replacement.status, ImportTaskStatus::Queued);
        assert_eq!(
            store.import_task_purpose(&replacement.id).unwrap(),
            ImportTaskPurpose::MigrationRebuildFullCorpus
        );
    }

    #[test]
    fn migration_rebuild_reconciliation_does_not_requeue_a_completed_head() {
        let (store, contract) = store_with_authorized_root();
        assert_eq!(
            enqueue_authorized_roots(&store, &contract, UnixTimestamp::from_unix_seconds(4))
                .unwrap(),
            1
        );
        let task = store
            .latest_import_task_by_root("/synthetic/authorized")
            .unwrap()
            .unwrap();
        let mut final_scope = store
            .import_scan_scope_by_task_id(&task.id)
            .unwrap()
            .unwrap();
        let running = store
            .claim_observed_import_task_for_worker(&task, UnixTimestamp::from_unix_seconds(5))
            .unwrap()
            .unwrap();
        final_scope.updated_at = UnixTimestamp::from_unix_seconds(6);
        store
            .complete_import_task(
                &running.id,
                contract.id(),
                &final_scope,
                UnixTimestamp::from_unix_seconds(6),
            )
            .unwrap();

        assert_eq!(
            reconcile_authorized_roots(&store, &contract, UnixTimestamp::from_unix_seconds(7),)
                .unwrap(),
            0
        );
        assert!(store
            .pending_import_task_by_root("/synthetic/authorized")
            .unwrap()
            .is_none());
    }

    #[test]
    fn migration_contract_switch_discards_old_tasks_and_queues_exact_replacement() {
        let (store, previous_contract) = store_with_authorized_root();
        let previous = store
            .latest_import_task_by_root("/synthetic/authorized")
            .unwrap()
            .unwrap();
        let contract = ImportProcessingContract::new(
            "synthetic-next-primary-v28",
            "synthetic-next-ocr-v28",
            previous_contract.derived_schema_version(),
            previous_contract.classifier_epoch(),
        )
        .unwrap();
        let now = UnixTimestamp::from_unix_seconds(4);
        store
            .activate_migration_rebuild_contract(&contract, now)
            .unwrap();
        assert!(store.import_task_by_id(&previous.id).unwrap().is_none());

        assert_eq!(
            reconcile_authorized_roots(&store, &contract, UnixTimestamp::from_unix_seconds(5),)
                .unwrap(),
            1
        );
        let replacement = store
            .latest_import_task_by_root("/synthetic/authorized")
            .unwrap()
            .unwrap();
        assert_ne!(replacement.id, previous.id);
        assert_eq!(
            store
                .import_task_processing_contract_id(&replacement.id)
                .unwrap()
                .as_ref(),
            Some(contract.id())
        );
    }

    struct OwnedTestStore {
        _directory: TempDir,
        store: OwnedMetaStore,
    }

    impl Deref for OwnedTestStore {
        type Target = OwnedMetaStore;

        fn deref(&self) -> &Self::Target {
            &self.store
        }
    }

    fn store_with_authorized_root() -> (OwnedTestStore, ImportProcessingContract) {
        store_with_authorized_root_and_budget(None, true)
    }

    fn store_with_authorized_root_and_budget(
        scan_budget_limit: Option<u64>,
        cancel_existing_task: bool,
    ) -> (OwnedTestStore, ImportProcessingContract) {
        let directory = TempDir::new().unwrap();
        let owner = match DataDirectoryOwnerLease::try_acquire(directory.path()).unwrap() {
            DataDirectoryOwnerAcquisition::Acquired(owner) => owner,
            DataDirectoryOwnerAcquisition::Contended => panic!("test data directory contended"),
        };
        let store = owner.open_store().unwrap();
        let store = OwnedTestStore {
            _directory: directory,
            store,
        };
        let contract = current_import_processing_contract(&ImportOptions::default()).unwrap();
        store
            .activate_migration_rebuild_contract(&contract, UnixTimestamp::from_unix_seconds(1))
            .unwrap();
        let task_id = ImportTaskId::from_non_secret_parts(&["completed-root"]);
        let queued_at = UnixTimestamp::from_unix_seconds(1);
        let task = ImportTask {
            id: task_id.clone(),
            root_path: "/synthetic/authorized".to_string(),
            status: ImportTaskStatus::Queued,
            queued_at,
            started_at: None,
            finished_at: None,
            updated_at: queued_at,
        };
        let scope = ImportScanScope {
            import_task_id: task_id.clone(),
            root_kind: ImportRootKind::Explicit,
            root_preset: None,
            scan_profile: ImportScanProfile::Explicit,
            requested_root_path: "/synthetic/requested".to_string(),
            canonical_root_path: task.root_path.clone(),
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
            updated_at: queued_at,
        };
        store
            .insert_import_task_with_scan_scope(&task, &scope, &contract)
            .unwrap();
        if cancel_existing_task {
            store
                .cancel_import_task(&task_id, UnixTimestamp::from_unix_seconds(2))
                .unwrap();
        }
        (store, contract)
    }
}

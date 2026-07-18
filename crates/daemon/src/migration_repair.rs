use meta_store::{
    ImportTaskId, MetaStore, SearchProjectionServiceState, SearchRepairReason, UnixTimestamp,
};

use super::{new_import_task_id, DaemonError, Result};

pub(super) fn enqueue_authorized_roots(store: &MetaStore, now: UnixTimestamp) -> Result<usize> {
    let state = store
        .search_projection_state()
        .map_err(DaemonError::store)?;
    if state.service_state != SearchProjectionServiceState::Repairing
        || state.repair_reason != Some(SearchRepairReason::MigrationRebuild)
    {
        return Ok(0);
    }

    let roots = store
        .active_authorized_import_roots()
        .map_err(DaemonError::store)?;
    let mut queued = 0;
    for (index, root) in roots.into_iter().enumerate() {
        let task_id: ImportTaskId = new_import_task_id(index)?;
        if store
            .enqueue_authorized_import_root(&root, &task_id, now)
            .map_err(DaemonError::store)?
        {
            queued += 1;
        }
    }
    Ok(queued)
}

#[cfg(test)]
mod tests {
    use meta_store::{
        ImportRootKind, ImportScanProfile, ImportScanScope, ImportTask, ImportTaskId,
        ImportTaskStatus, MetaStore, SearchRepairReason, UnixTimestamp,
    };

    use super::enqueue_authorized_roots;

    #[test]
    fn migration_rebuild_queues_each_active_authorized_root_exactly_once() {
        let store = store_with_completed_authorized_root();
        let now = UnixTimestamp::from_unix_seconds(4);

        assert_eq!(enqueue_authorized_roots(&store, now).unwrap(), 1);
        assert_eq!(enqueue_authorized_roots(&store, now).unwrap(), 0);
        assert_eq!(
            store
                .pending_import_task_by_root("/synthetic/authorized")
                .unwrap()
                .unwrap()
                .status,
            ImportTaskStatus::Queued
        );
    }

    #[test]
    fn non_migration_repair_never_replays_authorized_roots() {
        let store = store_with_completed_authorized_root();
        store
            .mark_search_repairing(
                SearchRepairReason::ArtifactUnavailable,
                UnixTimestamp::from_unix_seconds(4),
            )
            .unwrap();

        assert_eq!(
            enqueue_authorized_roots(&store, UnixTimestamp::from_unix_seconds(5)).unwrap(),
            0
        );
        assert!(store
            .pending_import_task_by_root("/synthetic/authorized")
            .unwrap()
            .is_none());
    }

    fn store_with_completed_authorized_root() -> MetaStore {
        let store = MetaStore::open_in_memory().unwrap();
        store.run_migrations().unwrap();
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
            scan_budget_kind: None,
            scan_budget_limit: None,
            scan_budget_observed: None,
            scan_budget_exhausted: false,
            updated_at: queued_at,
        };
        store
            .insert_import_task_with_scan_scope(&task, &scope)
            .unwrap();
        store
            .update_import_task_status(
                &task_id,
                ImportTaskStatus::Running,
                UnixTimestamp::from_unix_seconds(2),
            )
            .unwrap();
        store
            .update_import_task_status(
                &task_id,
                ImportTaskStatus::Completed,
                UnixTimestamp::from_unix_seconds(3),
            )
            .unwrap();
        store
    }
}

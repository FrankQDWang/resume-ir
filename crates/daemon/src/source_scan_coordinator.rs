use meta_store::{
    ImportProcessingContract, ImportRootKind, ImportRootTaskHeadOutcome, ImportScanProfile,
    ImportScanScope, ImportTask, ImportTaskStatus, OwnedMetaStore, ScanSnapshot, ScanTrigger,
    SourceRoot, SourceRootScanCoordination,
};

use crate::command_failure::CommandFailure;

pub(crate) fn enqueue(
    store: &OwnedMetaStore,
    processing_contract: &ImportProcessingContract,
    root: &SourceRoot,
    trigger: ScanTrigger,
    now: meta_store::UnixTimestamp,
) -> Result<ScanSnapshot, CommandFailure> {
    if store
        .source_root_deletion_in_progress(&root.id)
        .map_err(|_| CommandFailure::Internal)?
    {
        return Err(CommandFailure::Conflict("source root is being deleted"));
    }
    store
        .activate_source_root_pipeline(&root.id, now)
        .map_err(|_| CommandFailure::Internal)?;
    let task_id = crate::import_command::new_task_id(0).map_err(|_| CommandFailure::Internal)?;
    let task = ImportTask {
        id: task_id.clone(),
        root_path: root.canonical_path.clone(),
        status: ImportTaskStatus::Queued,
        queued_at: now,
        started_at: None,
        finished_at: None,
        updated_at: now,
    };
    let scope = ImportScanScope {
        import_task_id: task_id,
        root_kind: ImportRootKind::Explicit,
        root_preset: None,
        scan_profile: ImportScanProfile::Explicit,
        requested_root_path: root.requested_path.clone(),
        canonical_root_path: root.canonical_path.clone(),
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
    match store
        .coordinate_source_root_scan(&root.id, trigger, &task, &scope, processing_contract, now)
        .map_err(|_| CommandFailure::Internal)?
    {
        SourceRootScanCoordination::Started { snapshot, .. }
        | SourceRootScanCoordination::Coalesced(snapshot) => Ok(snapshot),
        SourceRootScanCoordination::Rejected(ImportRootTaskHeadOutcome::RunningTaskConflict) => {
            Err(CommandFailure::Conflict("import task is already running"))
        }
        SourceRootScanCoordination::Rejected(ImportRootTaskHeadOutcome::RootPaused) => {
            Err(CommandFailure::Conflict("managed root is paused"))
        }
        SourceRootScanCoordination::Rejected(
            ImportRootTaskHeadOutcome::MigrationRebuildSuperseded,
        ) => Err(CommandFailure::ServiceUnavailable("REPAIRING")),
        SourceRootScanCoordination::Rejected(_) => Err(CommandFailure::Internal),
    }
}

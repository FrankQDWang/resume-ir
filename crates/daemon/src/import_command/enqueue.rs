use std::path::Path;

use meta_store::{
    ImportProcessingContract, ImportRootKind, ImportRootTaskHeadBatchOutcome,
    ImportRootTaskHeadBatchRejection, ImportRootTaskHeadOutcome, ImportRootTaskHeadRequest,
    ImportScanBudgetKind, ImportScanProfile, ImportScanScope, ImportTask, ImportTaskId,
    ImportTaskStatus, OwnedMetaStore, UnixTimestamp,
};

use super::parse;
use crate::command_failure::CommandFailure;

pub(crate) fn enqueue(
    store: &OwnedMetaStore,
    processing_contract: &ImportProcessingContract,
    body: &[u8],
) -> Result<String, CommandFailure> {
    let payload = serde_json::from_slice::<serde_json::Value>(body)
        .map_err(|_| CommandFailure::BadRequest("invalid json"))?;
    let roots = parse::roots(&payload)?;
    let root_preset = parse::root_preset(&payload)?;
    let profile = parse::profile(&payload)?;
    let max_files = parse::max_files(&payload)?;
    let canonical_roots = parse::canonical_roots(&roots)?;
    let now = crate::current_timestamp().map_err(|_| CommandFailure::Internal)?;
    let requested_heads = canonical_roots
        .iter()
        .enumerate()
        .map(|(root_index, root)| {
            let canonical_root_path = path_string(&root.canonical);
            let requested_root_path = path_string(&root.requested);
            let requested_task = ImportTask {
                id: super::new_task_id(root_index).map_err(|_| CommandFailure::Internal)?,
                root_path: canonical_root_path.clone(),
                status: ImportTaskStatus::Queued,
                queued_at: now,
                started_at: None,
                finished_at: None,
                updated_at: now,
            };
            let requested_scope = scan_scope(
                &requested_task.id,
                requested_root_path,
                canonical_root_path,
                root_preset,
                profile,
                max_files,
                now,
            )?;
            Ok((requested_task, requested_scope))
        })
        .collect::<Result<Vec<_>, CommandFailure>>()?;
    let requests = requested_heads
        .iter()
        .map(|(task, scope)| ImportRootTaskHeadRequest::Configured {
            task,
            scope,
            processing_contract,
        })
        .collect::<Vec<_>>();
    let outcomes = match store
        .coordinate_import_root_task_heads(&requests)
        .map_err(|_| CommandFailure::Internal)?
    {
        ImportRootTaskHeadBatchOutcome::Committed { outcomes } => outcomes,
        ImportRootTaskHeadBatchOutcome::Rejected(
            ImportRootTaskHeadBatchRejection::RunningTaskConflict,
        ) => return Err(CommandFailure::Conflict("import task is already running")),
        ImportRootTaskHeadBatchOutcome::Rejected(ImportRootTaskHeadBatchRejection::RootPaused) => {
            return Err(CommandFailure::Conflict("managed root is paused"));
        }
        ImportRootTaskHeadBatchOutcome::Rejected(
            ImportRootTaskHeadBatchRejection::MigrationRebuildSuperseded,
        ) => return Err(CommandFailure::ServiceUnavailable("REPAIRING")),
    };
    let new_tasks = outcomes
        .iter()
        .filter(|outcome| matches!(outcome, ImportRootTaskHeadOutcome::HeadInserted { .. }))
        .count();
    let mut task_ids = Vec::with_capacity(outcomes.len());
    let mut persisted_configuration = None;
    for outcome in &outcomes {
        let (task, scope) = match outcome {
            ImportRootTaskHeadOutcome::HeadInserted { task, scope, .. }
            | ImportRootTaskHeadOutcome::HeadPromoted { task, scope, .. }
            | ImportRootTaskHeadOutcome::HeadRetained { task, scope, .. } => (task, scope),
            ImportRootTaskHeadOutcome::RunningTaskConflict
            | ImportRootTaskHeadOutcome::RootPaused
            | ImportRootTaskHeadOutcome::MigrationRebuildSuperseded => {
                return Err(CommandFailure::Internal);
            }
        };
        let configuration = (scope.scan_profile, scope.scan_budget_limit);
        if persisted_configuration.is_some_and(|persisted| persisted != configuration) {
            return Err(CommandFailure::Internal);
        }
        persisted_configuration = Some(configuration);
        task_ids.push(task.id.to_string());
    }
    let (persisted_profile, persisted_file_limit) =
        persisted_configuration.ok_or(CommandFailure::Internal)?;

    Ok(serde_json::json!({
        "schema_version": "daemon.import.v1",
        "status": "accepted",
        "accepted_roots": canonical_roots.len(),
        "new_tasks": new_tasks,
        "task_ids": task_ids,
        "scan_profile": profile_label(persisted_profile),
        "scan_file_limit": persisted_file_limit,
    })
    .to_string())
}

fn scan_scope(
    task_id: &ImportTaskId,
    requested_root_path: String,
    canonical_root_path: String,
    root_preset: Option<meta_store::ImportRootPreset>,
    profile: ImportScanProfile,
    max_files: Option<usize>,
    updated_at: UnixTimestamp,
) -> Result<ImportScanScope, CommandFailure> {
    Ok(ImportScanScope {
        import_task_id: task_id.clone(),
        root_kind: if root_preset.is_some() {
            ImportRootKind::Preset
        } else {
            ImportRootKind::Explicit
        },
        root_preset,
        scan_profile: profile,
        requested_root_path,
        canonical_root_path,
        files_discovered: 0,
        ignored_entries: 0,
        scan_errors: 0,
        searchable_documents: 0,
        ocr_required_documents: 0,
        ocr_jobs_queued: 0,
        failed_documents: 0,
        deleted_documents: 0,
        scan_budget_kind: max_files.map(|_| ImportScanBudgetKind::Files),
        scan_budget_limit: max_files
            .map(u64::try_from)
            .transpose()
            .map_err(|_| CommandFailure::Internal)?,
        scan_budget_observed: max_files.map(|_| 0),
        scan_budget_exhausted: false,
        updated_at,
    })
}

fn path_string(path: &Path) -> String {
    path.as_os_str().to_string_lossy().into_owned()
}

fn profile_label(profile: ImportScanProfile) -> &'static str {
    match profile {
        ImportScanProfile::Explicit => "explicit",
        ImportScanProfile::Discovery => "discovery",
    }
}

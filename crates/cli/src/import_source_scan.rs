use std::str::FromStr;

use meta_store::{
    ImportProcessingContract, ImportRootTaskHeadOutcome, ImportScanScope, ImportTask, ImportTaskId,
    OwnedMetaStore, ScanTrigger, SourceRootRegistration, SourceRootRegistrationAvailability,
    SourceRootScanCoordination, UnixTimestamp,
};

use crate::{CliError, Result};

pub(crate) fn coordinate_direct_import_tasks(
    store: &OwnedMetaStore,
    requested_heads: &[(ImportTask, ImportScanScope)],
    processing_contract: &ImportProcessingContract,
    now: UnixTimestamp,
) -> Result<Vec<ImportTask>> {
    let managed_roots = requested_heads
        .iter()
        .map(|(task, _)| {
            store
                .source_root_by_canonical_path(&task.root_path)
                .map_err(CliError::store)
        })
        .collect::<Result<Vec<_>>>()?;
    let managed_count = managed_roots.iter().filter(|root| root.is_some()).count();
    if managed_count != 0 && managed_count != requested_heads.len() {
        return Err(CliError::user(
            "direct import cannot mix managed and unmanaged source roots",
        ));
    }
    let roots = if managed_count == 0 {
        let registrations = requested_heads
            .iter()
            .map(|(task, _)| SourceRootRegistration {
                canonical_path: task.root_path.clone(),
                requested_path: task.root_path.clone(),
                display_label: "Direct import".to_string(),
                availability: SourceRootRegistrationAvailability::Available,
            })
            .collect::<Vec<_>>();
        store
            .register_source_roots_atomically(&registrations, now)
            .map_err(CliError::store)?
    } else {
        managed_roots
            .into_iter()
            .map(|root| root.expect("managed source-root count was validated"))
            .collect()
    };

    requested_heads
        .iter()
        .zip(roots)
        .map(|((task, scope), root)| {
            store
                .activate_source_root_pipeline(&root.id, now)
                .map_err(CliError::store)?;
            match store
                .coordinate_source_root_scan(
                    &root.id,
                    ScanTrigger::Manual,
                    task,
                    scope,
                    processing_contract,
                    now,
                )
                .map_err(CliError::store)?
            {
                SourceRootScanCoordination::Started { task_head, .. } => {
                    import_task_from_head_outcome(*task_head)
                }
                SourceRootScanCoordination::Coalesced(snapshot) => {
                    let task_id = ImportTaskId::from_str(&snapshot.id)
                        .map_err(|_| CliError::user("source scan task identity is invalid"))?;
                    store
                        .import_task_by_id(&task_id)
                        .map_err(CliError::store)?
                        .ok_or_else(|| CliError::user("source scan task is unavailable"))
                }
                SourceRootScanCoordination::Rejected(outcome) => {
                    import_task_from_head_outcome(outcome)
                }
            }
        })
        .collect()
}

fn import_task_from_head_outcome(outcome: ImportRootTaskHeadOutcome) -> Result<ImportTask> {
    match outcome {
        ImportRootTaskHeadOutcome::HeadInserted { task, .. }
        | ImportRootTaskHeadOutcome::HeadPromoted { task, .. }
        | ImportRootTaskHeadOutcome::HeadRetained { task, .. } => Ok(task),
        ImportRootTaskHeadOutcome::RunningTaskConflict => {
            Err(CliError::user("import task is already running"))
        }
        ImportRootTaskHeadOutcome::RootPaused => Err(CliError::user("managed root is paused")),
        ImportRootTaskHeadOutcome::MigrationRebuildSuperseded => Err(CliError::user(
            "offline import is blocked until migration rebuild completes",
        )),
    }
}

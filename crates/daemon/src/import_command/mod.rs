mod enqueue;
mod parse;
mod root_control;

use std::str::FromStr;
use std::time::{SystemTime, UNIX_EPOCH};

use meta_store::{ImportTaskId, ImportTaskStatus, OwnedMetaStore};

use crate::command_failure::CommandFailure;

pub(crate) use enqueue::enqueue;
pub(crate) use root_control::{
    execute as control_root, RootControlAction, RootControlCommand, RootControlOutput,
};

#[derive(Debug)]
pub(crate) struct TaskIdGenerationError;

pub(crate) fn cancel(store: &OwnedMetaStore, body: &[u8]) -> Result<String, CommandFailure> {
    let payload = serde_json::from_slice::<serde_json::Value>(body)
        .map_err(|_| CommandFailure::BadRequest("invalid json"))?;
    let task_id = parse_cancel_task_id(&payload)?;
    let Some(task) = store
        .import_task_by_id(&task_id)
        .map_err(|_| CommandFailure::Internal)?
    else {
        return Err(CommandFailure::NotFound("import task was not found"));
    };
    if !matches!(
        task.status,
        ImportTaskStatus::Queued | ImportTaskStatus::Running | ImportTaskStatus::FailedRetryable
    ) {
        return Err(CommandFailure::Conflict("import task cannot be cancelled"));
    }
    let now = crate::current_timestamp().map_err(|_| CommandFailure::Internal)?;
    let inserted = store
        .cancel_import_task(&task_id, now)
        .map_err(|_| CommandFailure::Internal)?;
    Ok(serde_json::json!({
        "schema_version": "daemon.import_cancel.v1",
        "status": "cancel_requested",
        "task_id": task_id.to_string(),
        "already_cancelled": !inserted,
    })
    .to_string())
}

fn parse_cancel_task_id(payload: &serde_json::Value) -> Result<ImportTaskId, CommandFailure> {
    let value = payload
        .get("task_id")
        .and_then(serde_json::Value::as_str)
        .ok_or(CommandFailure::BadRequest("task_id is required"))?;
    ImportTaskId::from_str(value).map_err(|_| CommandFailure::BadRequest("task_id is invalid"))
}

pub(crate) fn new_task_id(root_index: usize) -> Result<ImportTaskId, TaskIdGenerationError> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| TaskIdGenerationError)?;
    Ok(ImportTaskId::from_non_secret_parts(&[
        "s46-import-task",
        &duration.as_nanos().to_string(),
        &std::process::id().to_string(),
        &root_index.to_string(),
    ]))
}

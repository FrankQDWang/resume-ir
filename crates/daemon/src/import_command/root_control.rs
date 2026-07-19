use meta_store::{
    ImportProcessingContract, ImportRootControlStatus, ImportRootControlUpdate, OwnedMetaStore,
};

use crate::command_failure::CommandFailure;

#[derive(Clone, Copy)]
pub(crate) enum RootControlAction {
    Inspect,
    Pause,
    Resume,
}

pub(crate) struct RootControlCommand {
    pub(crate) root_path: String,
    pub(crate) action: RootControlAction,
}

pub(crate) struct RootControlOutput {
    pub(crate) status: &'static str,
    pub(crate) changed: bool,
    pub(crate) task_cancel_requested: bool,
    pub(crate) catch_up_queued: bool,
}

pub(crate) fn execute(
    store: &OwnedMetaStore,
    processing_contract: &ImportProcessingContract,
    command: RootControlCommand,
) -> Result<RootControlOutput, CommandFailure> {
    let Some(current_status) = store
        .import_root_control_status(&command.root_path)
        .map_err(|_| CommandFailure::Internal)?
    else {
        return Err(CommandFailure::NotFound("managed root was not found"));
    };
    let now = crate::current_timestamp().map_err(|_| CommandFailure::Internal)?;
    match command.action {
        RootControlAction::Inspect => Ok(RootControlOutput {
            status: status_label(current_status),
            changed: false,
            task_cancel_requested: false,
            catch_up_queued: false,
        }),
        RootControlAction::Pause => store
            .pause_import_root(&command.root_path, now)
            .map(response_from_update)
            .map_err(|_| CommandFailure::Internal),
        RootControlAction::Resume => {
            let task_id = super::new_task_id(0).map_err(|_| CommandFailure::Internal)?;
            store
                .resume_import_root(&command.root_path, &task_id, processing_contract, now)
                .map(response_from_update)
                .map_err(|_| CommandFailure::Internal)
        }
    }
}

fn response_from_update(update: ImportRootControlUpdate) -> RootControlOutput {
    RootControlOutput {
        status: status_label(update.status),
        changed: update.changed,
        task_cancel_requested: update.cancellation_requests > 0,
        catch_up_queued: update.catch_up_queued,
    }
}

fn status_label(status: ImportRootControlStatus) -> &'static str {
    match status {
        ImportRootControlStatus::Active => "active",
        ImportRootControlStatus::Paused => "paused",
    }
}

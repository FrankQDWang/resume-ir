use std::path::{Component, Path};

use meta_store::{ImportRootControlStatus, ImportRootControlUpdate};
use serde::{Deserialize, Serialize};

use super::{
    current_timestamp, ipc_command_authorized, new_import_task_id, open_store, write_http_response,
    write_service_unavailable, DaemonError, IpcCommandError, IpcRequest, Result, TcpStream,
};

const REQUEST_SCHEMA_VERSION: &str = "daemon.import_root_control_request.v1";
const RESPONSE_SCHEMA_VERSION: &str = "daemon.import_root_control.v1";
const ROOT_PATH_MAX_BYTES: usize = 32 * 1024;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ImportRootControlRequest {
    schema_version: String,
    root_path: String,
    action: ImportRootControlAction,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ImportRootControlAction {
    Inspect,
    Pause,
    Resume,
}

#[derive(Serialize)]
struct ImportRootControlResponse {
    schema_version: &'static str,
    status: &'static str,
    changed: bool,
    task_cancel_requested: bool,
    catch_up_queued: bool,
}

pub(super) fn handle_ipc(
    data_dir: &Path,
    auth_token: &str,
    request: &IpcRequest,
    stream: &mut TcpStream,
) -> Result<()> {
    if !ipc_command_authorized(auth_token, &request.headers) {
        return write_http_response(
            stream,
            401,
            "application/json",
            r#"{"schema_version":"daemon.error.v1","status":"unauthorized"}"#,
        );
    }

    match execute(data_dir, &request.body) {
        Ok(body) => write_http_response(stream, 200, "application/json", &body),
        Err(IpcCommandError::BadRequest(message)) => {
            write_error(stream, 400, "bad_request", message)
        }
        Err(IpcCommandError::Conflict(message)) => write_error(stream, 409, "conflict", message),
        Err(IpcCommandError::NotFound(message)) => write_error(stream, 404, "not_found", message),
        Err(IpcCommandError::TooLarge(message)) => write_error(stream, 413, "too_large", message),
        Err(IpcCommandError::ServiceUnavailable(_)) => write_service_unavailable(
            stream,
            super::ipc::ServiceErrorCode::QueryServiceUnavailable,
        ),
        Err(IpcCommandError::Internal(_error)) => {
            write_service_unavailable(stream, super::ipc::ServiceErrorCode::MetadataUnavailable)
        }
    }
}

fn execute(data_dir: &Path, body: &[u8]) -> std::result::Result<String, IpcCommandError> {
    let request = parse_request(body)?;
    let store = open_store(data_dir).map_err(IpcCommandError::Internal)?;
    let Some(current_status) = store
        .import_root_control_status(&request.root_path)
        .map_err(DaemonError::store)
        .map_err(IpcCommandError::Internal)?
    else {
        return Err(IpcCommandError::NotFound("managed root was not found"));
    };
    let now = current_timestamp().map_err(IpcCommandError::Internal)?;
    let response = match request.action {
        ImportRootControlAction::Inspect => ImportRootControlResponse {
            schema_version: RESPONSE_SCHEMA_VERSION,
            status: status_label(current_status),
            changed: false,
            task_cancel_requested: false,
            catch_up_queued: false,
        },
        ImportRootControlAction::Pause => {
            let update = store
                .pause_import_root(&request.root_path, now)
                .map_err(DaemonError::store)
                .map_err(IpcCommandError::Internal)?;
            response_from_update(update)
        }
        ImportRootControlAction::Resume => {
            let task_id = new_import_task_id(0).map_err(IpcCommandError::Internal)?;
            let update = store
                .resume_import_root(&request.root_path, &task_id, now)
                .map_err(DaemonError::store)
                .map_err(IpcCommandError::Internal)?;
            response_from_update(update)
        }
    };
    serde_json::to_string(&response).map_err(|_| {
        IpcCommandError::Internal(DaemonError::user(
            "unable to serialize import root control response",
        ))
    })
}

fn parse_request(body: &[u8]) -> std::result::Result<ImportRootControlRequest, IpcCommandError> {
    let request = serde_json::from_slice::<ImportRootControlRequest>(body)
        .map_err(|_| IpcCommandError::BadRequest("invalid root control request"))?;
    if request.schema_version != REQUEST_SCHEMA_VERSION
        || request.root_path.is_empty()
        || request.root_path.len() > ROOT_PATH_MAX_BYTES
        || request.root_path.contains('\0')
    {
        return Err(IpcCommandError::BadRequest("invalid root control request"));
    }
    let path = Path::new(&request.root_path);
    if !path.is_absolute()
        || path
            .components()
            .any(|component| matches!(component, Component::CurDir | Component::ParentDir))
    {
        return Err(IpcCommandError::BadRequest("invalid root control request"));
    }
    Ok(request)
}

fn response_from_update(update: ImportRootControlUpdate) -> ImportRootControlResponse {
    ImportRootControlResponse {
        schema_version: RESPONSE_SCHEMA_VERSION,
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

fn write_error(
    stream: &mut TcpStream,
    status_code: u16,
    status: &'static str,
    message: &'static str,
) -> Result<()> {
    let body = serde_json::json!({
        "schema_version": "daemon.error.v1",
        "status": status,
        "message": message,
    })
    .to_string();
    write_http_response(stream, status_code, "application/json", &body)
}

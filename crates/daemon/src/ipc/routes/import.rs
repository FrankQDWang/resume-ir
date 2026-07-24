use std::net::TcpStream;
use std::path::{Component, Path};
use std::thread;
use std::time::Duration;

use meta_store::{ImportProcessingContract, OwnedMetaStore, ReadMetaStore};
use serde::Deserialize;

use crate::command_failure::CommandFailure;
use crate::import_command::{RootControlAction, RootControlCommand, RootControlOutput};

use super::super::protocol::Request;
use super::super::{RequestFailure, ServiceErrorCode};
use super::status::import_progress_event_json;
use super::{
    authorized, unauthorized_body, unified_error_body, write, write_service_unavailable,
    RouteResult,
};

const ROOT_CONTROL_REQUEST_SCHEMA_VERSION: &str = "daemon.import_root_control_request.v1";
const ROOT_CONTROL_RESPONSE_SCHEMA_VERSION: &str = "daemon.import_root_control.v1";
const ROOT_PATH_MAX_BYTES: usize = 32 * 1024;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RootControlRequest {
    schema_version: String,
    root_path: String,
    action: RootControlRequestAction,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum RootControlRequestAction {
    Inspect,
    Pause,
    Resume,
}

pub(super) fn enqueue(
    store: &OwnedMetaStore,
    processing_contract: &ImportProcessingContract,
    auth_token: &str,
    request: &Request,
    stream: &mut TcpStream,
) -> RouteResult {
    if !authorized(auth_token, request) {
        return write(stream, 401, "application/json", &unauthorized_body());
    }
    write_command_result(
        stream,
        crate::import_command::enqueue(store, processing_contract, &request.body),
    )
}

pub(super) fn cancel(
    store: &OwnedMetaStore,
    auth_token: &str,
    request: &Request,
    stream: &mut TcpStream,
) -> RouteResult {
    if !authorized(auth_token, request) {
        return write(stream, 401, "application/json", &unauthorized_body());
    }
    write_command_result(stream, crate::import_command::cancel(store, &request.body))
}

fn write_command_result(
    stream: &mut TcpStream,
    result: Result<String, CommandFailure>,
) -> RouteResult {
    match result {
        Ok(body) => write(stream, 202, "application/json", &body),
        Err(error) => write_command_failure(stream, error),
    }
}

fn write_command_failure(stream: &mut TcpStream, error: CommandFailure) -> RouteResult {
    match error {
        CommandFailure::BadRequest(message) => write_error(stream, 400, "bad_request", message),
        CommandFailure::Conflict(message) => write_error(stream, 409, "conflict", message),
        CommandFailure::NotFound(message) => write_error(stream, 404, "not_found", message),
        CommandFailure::TooLarge(message) => write_error(stream, 413, "too_large", message),
        CommandFailure::ServiceUnavailable("REPAIRING") => {
            write_service_unavailable(stream, ServiceErrorCode::Repairing)
        }
        CommandFailure::ServiceUnavailable(_) => {
            write_service_unavailable(stream, ServiceErrorCode::QueryServiceUnavailable)
        }
        CommandFailure::Internal => {
            write_service_unavailable(stream, ServiceErrorCode::MetadataUnavailable)
        }
    }
}

fn write_error(
    stream: &mut TcpStream,
    status_code: u16,
    status: &str,
    message: &str,
) -> RouteResult {
    let _ = message;
    let (code, action) = match status {
        "bad_request" => ("BAD_REQUEST", "correct_request"),
        "conflict" => ("CONFLICT", "retry"),
        "not_found" => ("NOT_FOUND", "refresh_search"),
        "too_large" => ("LIMIT_EXCEEDED", "reduce_page_size"),
        _ => ("INTERNAL", "retry"),
    };
    let body = unified_error_body(None, code, action);
    write(stream, status_code, "application/json", &body)
}

pub(super) fn control(
    store: &OwnedMetaStore,
    processing_contract: &ImportProcessingContract,
    auth_token: &str,
    request: &Request,
    stream: &mut TcpStream,
) -> RouteResult {
    if !authorized(auth_token, request) {
        return write(stream, 401, "application/json", &unauthorized_body());
    }
    let command = match parse_root_control(&request.body) {
        Ok(command) => command,
        Err(message) => return write_error(stream, 400, "bad_request", message),
    };
    match crate::import_command::control_root(store, processing_contract, command) {
        Ok(output) => write_root_control_output(stream, output),
        Err(error) => write_command_failure(stream, error),
    }
}

fn parse_root_control(body: &[u8]) -> Result<RootControlCommand, &'static str> {
    let request = serde_json::from_slice::<RootControlRequest>(body)
        .map_err(|_| "invalid root control request")?;
    if request.schema_version != ROOT_CONTROL_REQUEST_SCHEMA_VERSION
        || request.root_path.is_empty()
        || request.root_path.len() > ROOT_PATH_MAX_BYTES
        || request.root_path.contains('\0')
    {
        return Err("invalid root control request");
    }
    let path = Path::new(&request.root_path);
    if !path.is_absolute()
        || path
            .components()
            .any(|component| matches!(component, Component::CurDir | Component::ParentDir))
    {
        return Err("invalid root control request");
    }
    Ok(RootControlCommand {
        root_path: request.root_path,
        action: match request.action {
            RootControlRequestAction::Inspect => RootControlAction::Inspect,
            RootControlRequestAction::Pause => RootControlAction::Pause,
            RootControlRequestAction::Resume => RootControlAction::Resume,
        },
    })
}

fn write_root_control_output(stream: &mut TcpStream, output: RootControlOutput) -> RouteResult {
    let body = serde_json::json!({
        "schema_version": ROOT_CONTROL_RESPONSE_SCHEMA_VERSION,
        "status": output.status,
        "changed": output.changed,
        "task_cancel_requested": output.task_cancel_requested,
        "catch_up_queued": output.catch_up_queued,
    })
    .to_string();
    write(stream, 200, "application/json", &body)
}

pub(super) fn progress(
    store: &ReadMetaStore,
    auth_token: &str,
    request: &Request,
    stream: &mut TcpStream,
) -> RouteResult {
    if !authorized(auth_token, request) {
        return write(stream, 401, "application/json", &unauthorized_body());
    }
    let first_event = match import_progress_event_json(store) {
        Ok(event) => event,
        Err(_) => return write_service_unavailable(stream, ServiceErrorCode::MetadataUnavailable),
    };
    super::super::response::write_all(
        stream,
        b"HTTP/1.1 200 OK\r\nContent-Type: application/x-ndjson\r\nConnection: close\r\n\r\n",
    )
    .map_err(RequestFailure::ResponseSink)?;
    for event_index in 0..crate::IMPORT_PROGRESS_STREAM_EVENTS {
        let event = if event_index == 0 {
            first_event.clone()
        } else {
            import_progress_event_json(store).map_err(|_| RequestFailure::Handler)?
        };
        super::super::response::write_all(stream, event.as_bytes())
            .and_then(|_| super::super::response::write_all(stream, b"\n"))
            .and_then(|_| super::super::response::flush(stream))
            .map_err(RequestFailure::ResponseSink)?;
        if event_index + 1 < crate::IMPORT_PROGRESS_STREAM_EVENTS {
            thread::sleep(Duration::from_millis(
                crate::IMPORT_PROGRESS_STREAM_INTERVAL_MS,
            ));
        }
    }
    Ok(())
}

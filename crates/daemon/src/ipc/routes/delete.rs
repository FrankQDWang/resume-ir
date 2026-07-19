use std::net::TcpStream;

use meta_store::OwnedMetaStore;

use crate::command_failure::CommandFailure;

use super::super::protocol::Request;
use super::super::ServiceErrorCode;
use super::{authorized, unauthorized_body, write, write_service_unavailable, RouteResult};

pub(super) fn handle(
    store: &OwnedMetaStore,
    auth_token: &str,
    request: &Request,
    stream: &mut TcpStream,
) -> RouteResult {
    if !authorized(auth_token, request) {
        return write(stream, 401, "application/json", &unauthorized_body());
    }

    match crate::delete_command::execute(store, &request.body) {
        Ok(body) => write(stream, 200, "application/json", &body),
        Err(CommandFailure::BadRequest(message)) => {
            write_error(stream, 400, "bad_request", Some(message))
        }
        Err(CommandFailure::Conflict(message)) => {
            write_error(stream, 409, "conflict", Some(message))
        }
        Err(CommandFailure::NotFound(_)) => write_error(stream, 404, "not_found", None),
        Err(CommandFailure::TooLarge(message)) => {
            write_error(stream, 413, "too_large", Some(message))
        }
        Err(CommandFailure::ServiceUnavailable(_)) => {
            write_service_unavailable(stream, ServiceErrorCode::QueryServiceUnavailable)
        }
        Err(CommandFailure::Internal) => {
            write_service_unavailable(stream, ServiceErrorCode::MetadataUnavailable)
        }
    }
}

fn write_error(
    stream: &mut TcpStream,
    status_code: u16,
    status: &str,
    message: Option<&str>,
) -> RouteResult {
    let mut body = serde_json::json!({
        "schema_version": "daemon.error.v1",
        "status": status,
    });
    if let Some(message) = message {
        body["message"] = serde_json::json!(message);
    }
    write(stream, status_code, "application/json", &body.to_string())
}

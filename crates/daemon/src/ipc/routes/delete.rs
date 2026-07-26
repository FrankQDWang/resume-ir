use std::net::TcpStream;

use meta_store::OwnedMetaStore;

use crate::command_failure::CommandFailure;

use super::super::protocol::Request;
use super::super::ServiceErrorCode;
use super::{
    authorized, unauthorized_body, unified_error_body, write, write_service_unavailable,
    RouteResult,
};

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
    let _ = message;
    let (code, action) = match status {
        "bad_request" => ("BAD_REQUEST", "correct_request"),
        "conflict" => ("CONFLICT", "retry"),
        "not_found" => ("NOT_FOUND", "refresh_search"),
        "too_large" => ("LIMIT_EXCEEDED", "reduce_page_size"),
        _ => ("INTERNAL", "retry"),
    };
    write(
        stream,
        status_code,
        "application/json",
        &unified_error_body(None, code, action),
    )
}

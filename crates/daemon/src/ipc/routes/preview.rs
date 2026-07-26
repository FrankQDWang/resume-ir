use std::net::TcpStream;

use serde::Deserialize;

use super::super::protocol::Request;
use super::super::source_file_service::SourceFileService;
use super::super::ConnectionCompletion;
use super::{authorized, unified_error_body, write, RouteResult};
use crate::detail_ipc::{self, WireSearchSelection};

const CREATE_REQUEST_SCHEMA: &str = "resume-ir.source-preview-create-request.v1";
const RANGE_REQUEST_SCHEMA: &str = "resume-ir.source-preview-range-request.v1";
const CLOSE_REQUEST_SCHEMA: &str = "resume-ir.source-preview-close-request.v1";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateRequest {
    schema_version: String,
    request_id: String,
    selection: WireSearchSelection,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RangeRequest {
    schema_version: String,
    request_id: String,
    lease_id: String,
    offset: u64,
    length: usize,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CloseRequest {
    schema_version: String,
    request_id: String,
    lease_id: String,
}

pub(super) fn create(
    service: &SourceFileService,
    auth_token: &str,
    request: &Request,
    mut stream: TcpStream,
    completion: &ConnectionCompletion,
) -> RouteResult {
    if !authorized(auth_token, request) {
        return error(&mut stream, None, 401, "UNAUTHORIZED", "authenticate");
    }
    let Ok(request) = serde_json::from_slice::<CreateRequest>(&request.body) else {
        return error(&mut stream, None, 400, "BAD_REQUEST", "correct_request");
    };
    if request.schema_version != CREATE_REQUEST_SCHEMA {
        return error(
            &mut stream,
            Some(&request.request_id),
            400,
            "BAD_REQUEST",
            "correct_request",
        );
    }
    let context = match detail_ipc::request_context(request.request_id, request.selection) {
        Ok(context) => context,
        Err(_) => return error(&mut stream, None, 400, "BAD_REQUEST", "correct_request"),
    };
    service.create_preview(
        stream,
        completion.defer(),
        context.request_id,
        context.selection,
    );
    Ok(())
}

pub(super) fn read_range(
    service: &SourceFileService,
    auth_token: &str,
    request: &Request,
    mut stream: TcpStream,
    completion: &ConnectionCompletion,
) -> RouteResult {
    if !authorized(auth_token, request) {
        return error(&mut stream, None, 401, "UNAUTHORIZED", "authenticate");
    }
    let Ok(request) = serde_json::from_slice::<RangeRequest>(&request.body) else {
        return error(&mut stream, None, 400, "BAD_REQUEST", "correct_request");
    };
    if request.schema_version != RANGE_REQUEST_SCHEMA
        || !valid_request_id(&request.request_id)
        || !valid_lease_id(&request.lease_id)
    {
        return error(
            &mut stream,
            Some(&request.request_id),
            400,
            "BAD_REQUEST",
            "correct_request",
        );
    }
    service.read_preview_range(
        stream,
        completion.defer(),
        request.request_id,
        request.lease_id,
        request.offset,
        request.length,
    );
    Ok(())
}

pub(super) fn close(
    service: &SourceFileService,
    auth_token: &str,
    request: &Request,
    mut stream: TcpStream,
    completion: &ConnectionCompletion,
) -> RouteResult {
    if !authorized(auth_token, request) {
        return error(&mut stream, None, 401, "UNAUTHORIZED", "authenticate");
    }
    let Ok(request) = serde_json::from_slice::<CloseRequest>(&request.body) else {
        return error(&mut stream, None, 400, "BAD_REQUEST", "correct_request");
    };
    if request.schema_version != CLOSE_REQUEST_SCHEMA
        || !valid_request_id(&request.request_id)
        || !valid_lease_id(&request.lease_id)
    {
        return error(
            &mut stream,
            Some(&request.request_id),
            400,
            "BAD_REQUEST",
            "correct_request",
        );
    }
    service.close_preview(
        stream,
        completion.defer(),
        request.request_id,
        request.lease_id,
    );
    Ok(())
}

fn error(
    stream: &mut TcpStream,
    request_id: Option<&str>,
    status: u16,
    code: &str,
    action: &str,
) -> RouteResult {
    write(
        stream,
        status,
        "application/json",
        &unified_error_body(request_id, code, action),
    )
}

fn valid_request_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

fn valid_lease_id(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

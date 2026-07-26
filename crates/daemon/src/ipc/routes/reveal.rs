use std::net::TcpStream;

use serde::Deserialize;

use super::super::protocol::Request;
use super::super::source_file_service::SourceFileService;
use super::super::ConnectionCompletion;
use super::{authorized, unified_error_body, write, RouteResult};
use crate::detail_ipc::{self, WireSearchSelection};

const REQUEST_SCHEMA: &str = "resume-ir.source-reveal-request.v1";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RevealRequest {
    schema_version: String,
    request_id: String,
    selection: WireSearchSelection,
}

pub(super) fn resolve(
    service: &SourceFileService,
    auth_token: &str,
    request: &Request,
    mut stream: TcpStream,
    completion: &ConnectionCompletion,
) -> RouteResult {
    if !authorized(auth_token, request) {
        return error(&mut stream, None, 401, "UNAUTHORIZED", "authenticate");
    }
    let Ok(request) = serde_json::from_slice::<RevealRequest>(&request.body) else {
        return error(&mut stream, None, 400, "BAD_REQUEST", "correct_request");
    };
    if request.schema_version != REQUEST_SCHEMA {
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
    service.resolve_reveal(
        stream,
        completion.defer(),
        context.request_id,
        context.selection,
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

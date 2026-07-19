use std::net::TcpStream;

use meta_store::{ImportProcessingContract, OwnedMetaStore, ReadMetaStore};

use super::protocol::Request;
use super::search_service::SearchService;
use super::{ConnectionCompletion, RequestFailure};

mod delete;
mod detail;
mod r#import;
mod search;
pub(crate) mod status;

pub(super) type RouteResult = Result<(), RequestFailure>;

pub(super) fn write(
    stream: &mut TcpStream,
    status_code: u16,
    content_type: &str,
    body: &str,
) -> RouteResult {
    super::response::write_http_response(stream, status_code, content_type, body)
        .map_err(RequestFailure::ResponseSink)
}

pub(super) fn authorized(auth_token: &str, request: &Request) -> bool {
    super::protocol::authorized(auth_token, &request.headers)
}

pub(super) fn unauthorized_body() -> String {
    serde_json::json!({
        "schema_version": "daemon.error.v1",
        "status": "unauthorized",
    })
    .to_string()
}

pub(super) fn write_service_unavailable(
    stream: &mut TcpStream,
    code: super::ServiceErrorCode,
) -> RouteResult {
    super::response::write_service_unavailable(stream, code).map_err(RequestFailure::ResponseSink)
}

pub(super) fn unified_error_body(request_id: Option<&str>, code: &str, action: &str) -> String {
    super::response::unified_error_body(request_id, code, action)
}

pub(crate) struct Context<'a> {
    pub(crate) store: &'a ReadMetaStore,
    pub(crate) owned_store: &'a OwnedMetaStore,
    pub(crate) query_service: &'a SearchService,
    pub(crate) processing_contract: &'a ImportProcessingContract,
    pub(crate) auth_token: &'a str,
}

/// Dispatches one parsed request. Route code has no process-fatal error type;
/// every failure is bounded to this connection.
pub(crate) fn dispatch(
    context: Context<'_>,
    request: Request,
    mut stream: TcpStream,
    completion: &ConnectionCompletion,
) -> Result<(), RequestFailure> {
    if request.matches("GET", "/status") {
        return status::status(context.store, &mut stream);
    }
    if request.matches("GET", "/diagnostics") {
        return status::diagnostics(context.store, context.auth_token, &request, &mut stream);
    }
    if request.matches("POST", "/imports") {
        return r#import::enqueue(
            context.owned_store,
            context.processing_contract,
            context.auth_token,
            &request,
            &mut stream,
        );
    }
    if request.matches("POST", "/imports/cancel") {
        return r#import::cancel(
            context.owned_store,
            context.auth_token,
            &request,
            &mut stream,
        );
    }
    if request.matches("POST", "/imports/control") {
        return r#import::control(
            context.owned_store,
            context.processing_contract,
            context.auth_token,
            &request,
            &mut stream,
        );
    }
    if request.matches("GET", "/imports/progress") {
        return r#import::progress(context.store, context.auth_token, &request, &mut stream);
    }
    if request.matches("POST", "/search") {
        return search::single(
            context.store,
            context.auth_token,
            &request,
            stream,
            context.query_service,
            completion,
        );
    }
    if request.matches("POST", "/search/batch") {
        return search::batch(
            context.store,
            context.auth_token,
            &request,
            stream,
            context.query_service,
            completion,
        );
    }
    if request.matches("POST", "/search/cancel") {
        return search::cancel(context.auth_token, &request, stream, context.query_service);
    }
    if request.matches("POST", "/details") {
        return detail::read(context.store, context.auth_token, &request, &mut stream);
    }
    if request.matches("POST", "/details/hydrate") {
        return detail::hydrate(context.store, context.auth_token, &request, &mut stream);
    }
    if request.matches("POST", "/delete") {
        return delete::handle(
            context.owned_store,
            context.auth_token,
            &request,
            &mut stream,
        );
    }

    write(&mut stream, 404, "text/plain", "not found")
}

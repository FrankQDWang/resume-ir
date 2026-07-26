use std::net::TcpStream;

use meta_store::{ImportProcessingContract, OwnedMetaStore, ReadMetaStore};

use super::protocol::Request;
use super::search_service::SearchService;
use super::{
    CapabilityHealth, CapabilityState, ConnectionCompletion, ControlPlaneState, CoreState,
    RequestFailure,
};

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
        "schema_version": "resume-ir.error.v2",
        "status": "error",
        "error": {
            "code": "UNAUTHORIZED",
            "action": "authenticate",
            "capability": serde_json::Value::Null,
            "reason": serde_json::Value::Null,
        },
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
    pub(crate) control_state: &'a ControlPlaneState,
}

pub(crate) fn dispatch_control(
    state: &ControlPlaneState,
    auth_token: &str,
    request: &Request,
    stream: &mut TcpStream,
) -> Option<RouteResult> {
    if request.matches("GET", "/status") {
        return Some(status::status(state, auth_token, request, stream));
    }
    if request.matches("GET", "/diagnostics") {
        return Some(status::diagnostics(state, auth_token, request, stream));
    }
    if !is_business_request(request) {
        return None;
    }
    let snapshot = state.snapshot();
    match snapshot.core.state {
        CoreState::Ready => None,
        CoreState::Initializing | CoreState::Repairing => Some(write_control_error(
            stream,
            auth_token,
            request,
            "SERVICE_INITIALIZING",
            "wait_for_service",
            None,
            snapshot.core.reason.map(|reason| reason.label()),
        )),
        CoreState::Degraded | CoreState::Blocked => Some(write_control_error(
            stream,
            auth_token,
            request,
            "SERVICE_BLOCKED",
            if snapshot.core.state == CoreState::Degraded {
                "retry"
            } else {
                "repair_required"
            },
            None,
            snapshot.core.reason.map(|reason| reason.label()),
        )),
    }
}

pub(crate) fn is_business_request(request: &Request) -> bool {
    [
        ("POST", "/imports"),
        ("POST", "/imports/cancel"),
        ("POST", "/imports/control"),
        ("GET", "/imports/progress"),
        ("POST", "/search"),
        ("POST", "/search/batch"),
        ("POST", "/search/cancel"),
        ("POST", "/details"),
        ("POST", "/details/hydrate"),
        ("POST", "/delete"),
    ]
    .into_iter()
    .any(|(method, path)| request.matches(method, path))
}

fn write_control_error(
    stream: &mut TcpStream,
    auth_token: &str,
    request: &Request,
    code: &str,
    action: &str,
    capability: Option<&str>,
    reason: Option<&str>,
) -> RouteResult {
    if !authorized(auth_token, request) {
        return write(stream, 401, "application/json", &unauthorized_body());
    }
    let request_id = contextual_request_id(request);
    let body = super::response::service_error_body(
        request_id.as_deref(),
        code,
        action,
        capability,
        reason,
    );
    write(stream, 503, "application/json", &body)
}

fn contextual_request_id(request: &Request) -> Option<String> {
    let field = match request.path.as_str() {
        "/search" | "/details" | "/details/hydrate" => "request_id",
        "/search/batch" => "batch_id",
        _ => return None,
    };
    let value: serde_json::Value = serde_json::from_slice(&request.body).ok()?;
    value
        .get(field)
        .and_then(serde_json::Value::as_str)
        .filter(|value| super::search_service::valid_opaque_id(value))
        .map(str::to_owned)
}

fn require_capability(
    stream: &mut TcpStream,
    auth_token: &str,
    request: &Request,
    name: &'static str,
    health: CapabilityHealth,
) -> Option<RouteResult> {
    if !authorized(auth_token, request) {
        return Some(write(stream, 401, "application/json", &unauthorized_body()));
    }
    if health.state == CapabilityState::Available {
        return None;
    }
    Some(write_control_error(
        stream,
        auth_token,
        request,
        "CAPABILITY_UNAVAILABLE",
        "select_supported_mode",
        Some(name),
        health.reason.map(|reason| reason.label()),
    ))
}

/// Dispatches one parsed request. Route code has no process-fatal error type;
/// every failure is bounded to this connection.
pub(crate) fn dispatch(
    context: Context<'_>,
    request: Request,
    mut stream: TcpStream,
    completion: &ConnectionCompletion,
) -> Result<(), RequestFailure> {
    if let Some(result) = dispatch_control(
        context.control_state,
        context.auth_token,
        &request,
        &mut stream,
    ) {
        return result;
    }
    if request.matches("POST", "/imports") {
        let capability = context.control_state.snapshot().capabilities.text_import;
        if capability.state != CapabilityState::Available {
            return write_control_error(
                &mut stream,
                context.auth_token,
                &request,
                "CAPABILITY_UNAVAILABLE",
                "select_supported_mode",
                Some("text_import"),
                capability.reason.map(|reason| reason.label()),
            );
        }
        return r#import::enqueue(
            context.owned_store,
            context.processing_contract,
            context.auth_token,
            &request,
            &mut stream,
        );
    }
    if request.matches("POST", "/imports/cancel") {
        let capability = context.control_state.snapshot().capabilities.text_import;
        if let Some(result) = require_capability(
            &mut stream,
            context.auth_token,
            &request,
            "text_import",
            capability,
        ) {
            return result;
        }
        return r#import::cancel(
            context.owned_store,
            context.auth_token,
            &request,
            &mut stream,
        );
    }
    if request.matches("POST", "/imports/control") {
        let capability = context.control_state.snapshot().capabilities.text_import;
        if let Some(result) = require_capability(
            &mut stream,
            context.auth_token,
            &request,
            "text_import",
            capability,
        ) {
            return result;
        }
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
            context.control_state,
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
            context.control_state,
        );
    }
    if request.matches("POST", "/search/cancel") {
        let capability = context.control_state.snapshot().capabilities.keyword_search;
        if let Some(result) = require_capability(
            &mut stream,
            context.auth_token,
            &request,
            "keyword_search",
            capability,
        ) {
            return result;
        }
        return search::cancel(context.auth_token, &request, stream, context.query_service);
    }
    if request.matches("POST", "/details") {
        let capability = context.control_state.snapshot().capabilities.detail;
        if let Some(result) = require_capability(
            &mut stream,
            context.auth_token,
            &request,
            "detail",
            capability,
        ) {
            return result;
        }
        return detail::read(context.store, context.auth_token, &request, &mut stream);
    }
    if request.matches("POST", "/details/hydrate") {
        let capability = context.control_state.snapshot().capabilities.detail;
        if let Some(result) = require_capability(
            &mut stream,
            context.auth_token,
            &request,
            "detail",
            capability,
        ) {
            return result;
        }
        return detail::hydrate(context.store, context.auth_token, &request, &mut stream);
    }
    if request.matches("POST", "/delete") {
        let capability = context
            .control_state
            .snapshot()
            .capabilities
            .index_publication;
        if let Some(result) = require_capability(
            &mut stream,
            context.auth_token,
            &request,
            "index_publication",
            capability,
        ) {
            return result;
        }
        return delete::handle(
            context.owned_store,
            context.auth_token,
            &request,
            &mut stream,
        );
    }

    write(&mut stream, 404, "text/plain", "not found")
}

#[cfg(test)]
mod tests {
    use super::contextual_request_id;
    use crate::ipc::protocol::Request;

    #[test]
    fn pre_route_service_errors_preserve_only_bounded_contextual_request_ids() {
        for (path, field) in [
            ("/search", "request_id"),
            ("/details", "request_id"),
            ("/details/hydrate", "request_id"),
            ("/search/batch", "batch_id"),
        ] {
            let request = request(
                path,
                serde_json::Value::Object(serde_json::Map::from_iter([(
                    field.to_string(),
                    serde_json::json!("request-1"),
                )])),
            );
            assert_eq!(
                contextual_request_id(&request).as_deref(),
                Some("request-1")
            );
        }
        assert!(contextual_request_id(&request(
            "/delete",
            serde_json::json!({"request_id": "request-1"}),
        ))
        .is_none());
        assert!(contextual_request_id(&request(
            "/search",
            serde_json::json!({"request_id": "private request"}),
        ))
        .is_none());
    }

    fn request(path: &str, body: serde_json::Value) -> Request {
        Request {
            method: "POST".to_string(),
            path: path.to_string(),
            version: "HTTP/1.1".to_string(),
            headers: Vec::new(),
            body: body.to_string().into_bytes(),
        }
    }
}

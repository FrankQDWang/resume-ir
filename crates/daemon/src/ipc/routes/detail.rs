use std::net::TcpStream;

use meta_store::ReadMetaStore;

use super::super::protocol::Request;
use super::{authorized, unified_error_body, write, RouteResult};
use crate::{detail_hydrate, detail_ipc};

pub(super) fn read(
    store: &ReadMetaStore,
    auth_token: &str,
    request: &Request,
    stream: &mut TcpStream,
) -> RouteResult {
    let request_id = detail_ipc::request_id(&request.body);
    if !authorized(auth_token, request) {
        return write_detail_error(
            stream,
            request_id.as_deref(),
            401,
            "UNAUTHORIZED",
            "authenticate",
        );
    }

    match detail_ipc::execute(store, &request.body) {
        Ok(body) => write(stream, 200, "application/json", &body),
        Err(detail_ipc::DetailError::BadRequest) => write_detail_error(
            stream,
            request_id.as_deref(),
            400,
            "BAD_REQUEST",
            "correct_request",
        ),
        Err(detail_ipc::DetailError::StaleSelection) => write_detail_error(
            stream,
            request_id.as_deref(),
            409,
            "STALE_SELECTION",
            "refresh_search",
        ),
        Err(detail_ipc::DetailError::NotFound) => write_detail_error(
            stream,
            request_id.as_deref(),
            404,
            "NOT_FOUND",
            "refresh_search",
        ),
        Err(detail_ipc::DetailError::ResponseTooLarge) => write_detail_error(
            stream,
            request_id.as_deref(),
            413,
            "RESPONSE_TOO_LARGE",
            "reduce_page_size",
        ),
        Err(detail_ipc::DetailError::Repairing) => write_detail_service_unavailable(
            stream,
            request_id.as_deref(),
            super::super::ServiceErrorCode::Repairing,
        ),
        Err(detail_ipc::DetailError::QueryServiceUnavailable) => write_detail_service_unavailable(
            stream,
            request_id.as_deref(),
            super::super::ServiceErrorCode::QueryServiceUnavailable,
        ),
        Err(detail_ipc::DetailError::MetadataUnavailable) => write_detail_service_unavailable(
            stream,
            request_id.as_deref(),
            super::super::ServiceErrorCode::MetadataUnavailable,
        ),
    }
}

pub(super) fn hydrate(
    store: &ReadMetaStore,
    auth_token: &str,
    request: &Request,
    stream: &mut TcpStream,
) -> RouteResult {
    let request_id = detail_ipc::request_id(&request.body);
    if !authorized(auth_token, request) {
        return write_detail_error(
            stream,
            request_id.as_deref(),
            401,
            "UNAUTHORIZED",
            "authenticate",
        );
    }

    match detail_hydrate::execute(store, &request.body) {
        Ok(body) => write(stream, 200, "application/json", &body),
        Err(detail_hydrate::DetailHydrateError::BadRequest) => write_detail_error(
            stream,
            request_id.as_deref(),
            400,
            "BAD_REQUEST",
            "correct_request",
        ),
        Err(detail_hydrate::DetailHydrateError::StaleSelection) => write_detail_error(
            stream,
            request_id.as_deref(),
            409,
            "STALE_SELECTION",
            "refresh_search",
        ),
        Err(detail_hydrate::DetailHydrateError::NotFound) => write_detail_error(
            stream,
            request_id.as_deref(),
            404,
            "NOT_FOUND",
            "refresh_search",
        ),
        Err(detail_hydrate::DetailHydrateError::ResponseTooLarge) => write_detail_error(
            stream,
            request_id.as_deref(),
            413,
            "RESPONSE_TOO_LARGE",
            "reduce_page_size",
        ),
        Err(detail_hydrate::DetailHydrateError::Repairing) => write_detail_service_unavailable(
            stream,
            request_id.as_deref(),
            super::super::ServiceErrorCode::Repairing,
        ),
        Err(detail_hydrate::DetailHydrateError::QueryServiceUnavailable) => {
            write_detail_service_unavailable(
                stream,
                request_id.as_deref(),
                super::super::ServiceErrorCode::QueryServiceUnavailable,
            )
        }
        Err(detail_hydrate::DetailHydrateError::MetadataUnavailable) => {
            write_detail_service_unavailable(
                stream,
                request_id.as_deref(),
                super::super::ServiceErrorCode::MetadataUnavailable,
            )
        }
    }
}

fn write_detail_error(
    stream: &mut TcpStream,
    request_id: Option<&str>,
    status_code: u16,
    code: &'static str,
    action: &'static str,
) -> RouteResult {
    let body = unified_error_body(request_id, code, action);
    write(stream, status_code, "application/json", &body)
}

fn write_detail_service_unavailable(
    stream: &mut TcpStream,
    request_id: Option<&str>,
    code: super::super::ServiceErrorCode,
) -> RouteResult {
    let body = unified_error_body(request_id, code.label(), code.action());
    write(stream, 503, "application/json", &body)
}

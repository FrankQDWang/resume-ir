use std::net::TcpStream;
use std::time::Instant;

use meta_store::ReadMetaStore;

use crate::command_failure::CommandFailure;

use super::super::protocol::Request;
use super::super::search_service;
use super::super::{ConnectionCompletion, RequestFailure};
use super::status::query_service_error;
use super::{authorized, unauthorized_body, write, RouteResult};

pub(super) fn single(
    store: &ReadMetaStore,
    auth_token: &str,
    request: &Request,
    mut stream: TcpStream,
    query_service: &search_service::SearchService,
    completion: &ConnectionCompletion,
) -> RouteResult {
    if !authorized(auth_token, request) {
        return write(&mut stream, 401, "application/json", &unauthorized_body());
    }

    let request_started = Instant::now();
    let envelope = match search_service::parse_request(&request.body) {
        Ok(envelope) => envelope,
        Err(message) => return write_bad_request(&mut stream, message),
    };
    let query_parse_started = Instant::now();
    let args = match crate::search_command::parse_search_command(&envelope.payload) {
        Ok(args) => args,
        Err(CommandFailure::BadRequest(message)) => {
            return write_search_error(
                &mut stream,
                &envelope.request_id,
                400,
                "BAD_REQUEST",
                message,
            );
        }
        Err(_) => {
            return write_search_error(
                &mut stream,
                &envelope.request_id,
                500,
                "INTERNAL",
                "search request validation failed",
            );
        }
    };
    if let Some(code) = query_service_error(store) {
        let body = search_service::service_error_body(&envelope.request_id, code);
        return write(&mut stream, 503, "application/json", &body);
    }
    query_service
        .dispatch(
            stream,
            completion.defer(),
            envelope,
            args,
            query_parse_started.elapsed(),
            request_started,
        )
        .map_err(RequestFailure::from)
}

pub(super) fn cancel(
    auth_token: &str,
    request: &Request,
    mut stream: TcpStream,
    query_service: &search_service::SearchService,
) -> RouteResult {
    if !authorized(auth_token, request) {
        return write(&mut stream, 401, "application/json", &unauthorized_body());
    }
    let cancel_request = match search_service::parse_cancel_request(&request.body) {
        Ok(request) => request,
        Err(message) => return write_bad_request(&mut stream, message),
    };
    query_service
        .cancel(stream, cancel_request)
        .map_err(RequestFailure::from)
}

pub(super) fn batch(
    store: &ReadMetaStore,
    auth_token: &str,
    request: &Request,
    mut stream: TcpStream,
    query_service: &search_service::SearchService,
    completion: &ConnectionCompletion,
) -> RouteResult {
    if !authorized(auth_token, request) {
        return write(&mut stream, 401, "application/json", &unauthorized_body());
    }

    let request_started = Instant::now();
    let batch = match search_service::parse_batch_request(&request.body) {
        Ok(batch) => batch,
        Err(message) => return write_bad_request(&mut stream, message),
    };
    let mut children = Vec::with_capacity(batch.requests.len());
    for envelope in batch.requests {
        let query_parse_started = Instant::now();
        let args = match crate::search_command::parse_search_command(&envelope.payload) {
            Ok(args) => args,
            Err(CommandFailure::BadRequest(message)) => {
                return write_bad_request(&mut stream, message);
            }
            Err(_) => {
                let body = serde_json::json!({
                    "schema_version": "daemon.error.v1",
                    "status": "internal",
                })
                .to_string();
                return write(&mut stream, 500, "application/json", &body);
            }
        };
        children.push((envelope, args, query_parse_started.elapsed()));
    }
    if let Some(code) = query_service_error(store) {
        let body = search_service::service_error_body(&batch.batch_id, code);
        return write(&mut stream, 503, "application/json", &body);
    }
    let Some(admission) = query_service.acquire_batch() else {
        let body = search_service::batch_overload_body(&batch.batch_id);
        return write(&mut stream, 503, "application/json", &body);
    };
    let writer = search_service::BatchWriter::start(
        stream,
        batch.batch_id,
        children.len(),
        admission,
        completion.defer(),
    )
    .map_err(RequestFailure::from)?;
    for (sequence, (envelope, args, query_parse_duration)) in children.into_iter().enumerate() {
        if let Err(error) = query_service.dispatch_batch_child(
            writer.child(sequence, envelope.request_id.clone()),
            envelope,
            args,
            query_parse_duration,
            request_started,
        ) {
            writer.finish_failure(RequestFailure::from(&error));
            return Err(RequestFailure::from(error));
        }
    }
    Ok(())
}

fn write_bad_request(stream: &mut TcpStream, message: &str) -> RouteResult {
    let body = serde_json::json!({
        "schema_version": "daemon.error.v1",
        "status": "bad_request",
        "message": message,
    })
    .to_string();
    write(stream, 400, "application/json", &body)
}

fn write_search_error(
    stream: &mut TcpStream,
    request_id: &str,
    status_code: u16,
    code: &str,
    message: &str,
) -> RouteResult {
    let body = search_service::error_body(request_id, code, message);
    write(stream, status_code, "application/json", &body)
}

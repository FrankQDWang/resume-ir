use std::net::TcpStream;
use std::path::Path;
use std::sync::mpsc::{SyncSender, TrySendError};
use std::time::Duration;

use meta_store::{ImportProcessingContract, OwnedMetaStore, ReadMetaStore};

use super::protocol::ReadOutcome;
use super::search_service::SearchService;
use super::source_file_service::SourceFileService;
use super::{
    response, routes, ConnectionCompletion, ConnectionOutcome, ControlPlaneState, RequestFailure,
    ResponseSinkError,
};

pub(crate) struct Context<'a> {
    pub(crate) data_dir: &'a Path,
    pub(crate) store: &'a ReadMetaStore,
    pub(crate) owned_store: &'a OwnedMetaStore,
    pub(crate) query_service: &'a SearchService,
    pub(crate) processing_contract: &'a ImportProcessingContract,
    pub(crate) auth_token: &'a str,
    pub(crate) control_state: &'a ControlPlaneState,
    pub(crate) source_file_service: &'a SourceFileService,
}

pub(super) struct PreparedBusinessConnection {
    stream: TcpStream,
    request: super::protocol::Request,
    completion: ConnectionCompletion,
    pub(super) final_request: bool,
}

impl PreparedBusinessConnection {
    pub(super) fn into_parts(self) -> (TcpStream, super::protocol::Request, ConnectionCompletion) {
        (self.stream, self.request, self.completion)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ReadyAdmission {
    BusinessForwarded,
    Completed,
}

/// Parses and authenticates a ready-server connection without touching the
/// metadata store. Control requests finish on the front door; business
/// requests move to the single store-owning data lane.
pub(super) fn admit_ready(
    mut stream: TcpStream,
    state: &ControlPlaneState,
    auth_token: &str,
    business_sender: &SyncSender<PreparedBusinessConnection>,
    final_request: bool,
) -> ReadyAdmission {
    let completion = ConnectionCompletion::accepted();
    if let Err(error) = configure(&stream) {
        completion.finish(ConnectionOutcome::from_request_result(Err(error)));
        return ReadyAdmission::Completed;
    }
    let result = match super::protocol::read(&mut stream) {
        ReadOutcome::Request(request) => {
            if let Some(result) = routes::dispatch_control(state, auth_token, &request, &mut stream)
            {
                result
            } else if routes::is_business_request(&request) {
                let task = PreparedBusinessConnection {
                    stream,
                    request,
                    completion: completion.clone(),
                    final_request,
                };
                match business_sender.try_send(task) {
                    Ok(()) => return ReadyAdmission::BusinessForwarded,
                    Err(TrySendError::Full(mut task)) => response::write_http_response(
                        &mut task.stream,
                        503,
                        "application/json",
                        &overloaded_body(),
                    )
                    .map_err(RequestFailure::ResponseSink),
                    Err(TrySendError::Disconnected(mut task)) => response::write_http_response(
                        &mut task.stream,
                        503,
                        "application/json",
                        &response::service_error_body(
                            None,
                            "SERVICE_BLOCKED",
                            "retry",
                            None,
                            Some("runtime_invariant"),
                        ),
                    )
                    .map_err(RequestFailure::ResponseSink),
                }
            } else {
                routes::write(&mut stream, 404, "text/plain", "not found")
            }
        }
        ReadOutcome::TooLarge => {
            response::write_http_response(&mut stream, 413, "text/plain", "request too large")
                .map_err(RequestFailure::ResponseSink)
        }
        ReadOutcome::BadRequest => {
            response::write_http_response(&mut stream, 400, "text/plain", "bad request")
                .map_err(RequestFailure::ResponseSink)
        }
    };
    completion.finish(ConnectionOutcome::from_request_result(result));
    ReadyAdmission::Completed
}

pub(super) fn handle_prepared(
    stream: TcpStream,
    request: super::protocol::Request,
    completion: ConnectionCompletion,
    context: Context<'_>,
) -> ConnectionCompletion {
    let result = routes::dispatch(
        routes::Context {
            data_dir: context.data_dir,
            store: context.store,
            owned_store: context.owned_store,
            query_service: context.query_service,
            processing_contract: context.processing_contract,
            auth_token: context.auth_token,
            control_state: context.control_state,
            source_file_service: context.source_file_service,
        },
        request,
        stream,
        &completion,
    );
    let outcome = match result {
        Ok(()) if completion.was_deferred() => ConnectionOutcome::Deferred,
        Ok(()) => ConnectionOutcome::Completed,
        Err(error) => ConnectionOutcome::from_request_result(Err(error)),
    };
    completion.finish(outcome);
    completion
}

pub(crate) fn handle_control(
    stream: TcpStream,
    state: &ControlPlaneState,
    auth_token: &str,
) -> ConnectionOutcome {
    let completion = ConnectionCompletion::accepted();
    let result = handle_control_request(stream, state, auth_token);
    let outcome = ConnectionOutcome::from_request_result(result);
    completion.finish(outcome);
    outcome
}

fn handle_control_request(
    mut stream: TcpStream,
    state: &ControlPlaneState,
    auth_token: &str,
) -> Result<(), RequestFailure> {
    configure(&stream)?;
    let request = match super::protocol::read(&mut stream) {
        ReadOutcome::Request(request) => request,
        ReadOutcome::TooLarge => {
            return response::write_http_response(
                &mut stream,
                413,
                "text/plain",
                "request too large",
            )
            .map_err(RequestFailure::ResponseSink);
        }
        ReadOutcome::BadRequest => {
            return response::write_http_response(&mut stream, 400, "text/plain", "bad request")
                .map_err(RequestFailure::ResponseSink);
        }
    };
    match routes::dispatch_control(state, auth_token, &request, &mut stream) {
        Some(result) => result,
        None if routes::is_business_request(&request) => routes::write(
            &mut stream,
            503,
            "application/json",
            &super::response::service_error_body(
                None,
                "SERVICE_BLOCKED",
                "repair_required",
                None,
                Some("runtime_invariant"),
            ),
        ),
        None => routes::write(&mut stream, 404, "text/plain", "not found"),
    }
}

fn configure(stream: &TcpStream) -> Result<(), RequestFailure> {
    stream
        .set_nonblocking(false)
        .map_err(|error| RequestFailure::ResponseSink(ResponseSinkError::from_io(&error)))?;
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .map_err(|error| RequestFailure::ResponseSink(ResponseSinkError::from_io(&error)))?;
    response::configure(stream).map_err(RequestFailure::ResponseSink)
}

fn overloaded_body() -> String {
    serde_json::json!({
        "schema_version": "resume-ir.error.v3",
        "status": "error",
        "error": {
            "code": "OVERLOADED",
            "action": "retry",
            "retry_after_ms": 250,
            "capability": serde_json::Value::Null,
            "reason": serde_json::Value::Null,
        },
    })
    .to_string()
}

#[cfg(test)]
mod tests {
    use std::io;

    use super::{RequestFailure, ResponseSinkError};
    use crate::ipc::metrics::IpcMetrics;
    use crate::ipc::ConnectionOutcome;

    fn configure_outcome(result: io::Result<()>) -> Result<(), RequestFailure> {
        result.map_err(|error| RequestFailure::ResponseSink(ResponseSinkError::from_io(&error)))
    }

    #[test]
    fn configure_failure_is_request_scoped_and_deterministic() {
        let result = configure_outcome(Err(io::Error::from(io::ErrorKind::BrokenPipe)));
        assert_eq!(
            result,
            Err(RequestFailure::ResponseSink(
                ResponseSinkError::ClientDisconnected
            ))
        );

        let metrics = IpcMetrics::default();
        metrics.record_accepted();
        metrics.record_connection_outcome(ConnectionOutcome::from_request_result(result));
        let snapshot = metrics.snapshot();
        assert_eq!(snapshot.accepted, 1);
        assert_eq!(
            snapshot.completed + snapshot.request_failure + snapshot.response_failure,
            1
        );
        assert_eq!(snapshot.response_failure, 1);
        assert_eq!(snapshot.client_disconnect, 1);
    }
}

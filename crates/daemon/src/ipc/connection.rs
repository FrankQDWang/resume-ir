use std::net::TcpStream;
use std::time::Duration;

use meta_store::{ImportProcessingContract, OwnedMetaStore, ReadMetaStore};

use super::protocol::ReadOutcome;
use super::search_service::SearchService;
use super::{
    response, routes, ConnectionCompletion, ConnectionOutcome, ControlPlaneState, RequestFailure,
    ResponseSinkError,
};

pub(crate) struct Context<'a> {
    pub(crate) store: &'a ReadMetaStore,
    pub(crate) owned_store: &'a OwnedMetaStore,
    pub(crate) query_service: &'a SearchService,
    pub(crate) processing_contract: &'a ImportProcessingContract,
    pub(crate) auth_token: &'a str,
    pub(crate) control_state: &'a ControlPlaneState,
}

/// Handles one accepted connection. Socket configuration, parsing, routing,
/// and response failures are closed into a connection outcome; this function
/// has no process-fatal return channel.
pub(crate) fn handle(stream: TcpStream, context: Context<'_>) -> ConnectionOutcome {
    let completion = ConnectionCompletion::accepted();
    let result = handle_request(stream, context, &completion);
    let outcome = match result {
        Ok(()) if completion.was_deferred() => ConnectionOutcome::Deferred,
        Ok(()) => ConnectionOutcome::Completed,
        Err(error) => ConnectionOutcome::from_request_result(Err(error)),
    };
    completion.finish(outcome);
    outcome
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

fn handle_request(
    mut stream: TcpStream,
    context: Context<'_>,
    completion: &ConnectionCompletion,
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

    routes::dispatch(
        routes::Context {
            store: context.store,
            owned_store: context.owned_store,
            query_service: context.query_service,
            processing_contract: context.processing_contract,
            auth_token: context.auth_token,
            control_state: context.control_state,
        },
        request,
        stream,
        completion,
    )
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

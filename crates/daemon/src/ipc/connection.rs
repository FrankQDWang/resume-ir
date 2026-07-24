use std::net::{Shutdown, TcpStream};
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

/// Handles one accepted connection and returns its exactly-once completion
/// capability. Deferred response owners finish the shared capability after
/// writing their response; this function has no process-fatal return channel.
pub(crate) fn handle(stream: TcpStream, context: Context<'_>) -> ConnectionCompletion {
    let completion = ConnectionCompletion::accepted();
    let result = handle_request(stream, context, &completion);
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
    let read_outcome = super::protocol::read(&mut stream);
    finish_request_input(&stream)?;
    let request = match read_outcome {
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
    let read_outcome = super::protocol::read(&mut stream);
    finish_request_input(&stream)?;
    let request = match read_outcome {
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

fn finish_request_input(stream: &TcpStream) -> Result<(), RequestFailure> {
    stream
        .shutdown(Shutdown::Read)
        .map_err(|error| RequestFailure::ResponseSink(ResponseSinkError::from_io(&error)))
}

#[cfg(test)]
mod tests {
    use std::io::{self, Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::thread;
    use std::time::Duration;

    use super::{finish_request_input, RequestFailure, ResponseSinkError};
    use crate::ipc::metrics::IpcMetrics;
    use crate::ipc::{response, ConnectionOutcome};

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

    #[test]
    fn terminal_request_input_does_not_abort_the_complete_response() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind connection fixture");
        let mut client =
            TcpStream::connect(listener.local_addr().unwrap()).expect("connect fixture");
        let (mut server, _) = listener.accept().expect("accept fixture");
        client
            .set_read_timeout(Some(Duration::from_secs(2)))
            .expect("bound response read");
        client
            .write_all(&vec![b'x'; 8 * 1024])
            .expect("write request and unread tail");
        let mut request_prefix = [0_u8; 1];
        server
            .read_exact(&mut request_prefix)
            .expect("read terminal request frame");

        let response_thread = thread::spawn(move || {
            finish_request_input(&server).expect("finish request input");
            response::write_http_response(
                &mut server,
                200,
                "application/octet-stream",
                &"y".repeat(32 * 1024),
            )
            .expect("write complete response");
        });
        let mut response = Vec::new();
        client
            .read_to_end(&mut response)
            .expect("response must close without reset");
        response_thread.join().unwrap();

        let header_end = response
            .windows(4)
            .position(|window| window == b"\r\n\r\n")
            .expect("response header")
            + 4;
        assert!(response.starts_with(b"HTTP/1.1 200 OK\r\n"));
        assert_eq!(response.len() - header_end, 32 * 1024);
    }
}

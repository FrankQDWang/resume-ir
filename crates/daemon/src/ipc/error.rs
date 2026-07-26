use std::fmt;
use std::io;

/// A process-level daemon failure. Only the IPC listener and supervised
/// runtime events are allowed to construct this type while the server runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DaemonFatalError {
    OwnershipConflict,
    ConfigurationInvalid,
    RuntimeIntegrity,
    ProtocolMismatch,
    ControlPlaneFailure,
}

/// A closed control-plane event observed by the IPC server.
#[derive(Debug)]
pub(crate) enum RuntimeEvent {
    Running,
    ShutdownRequested,
    ImportWorkerStopped,
    ImportWorkerFailed(DaemonFatalError),
    QueryWorkerStopped,
    StatusUpdaterStopped,
}

/// The terminal outcome of one accepted IPC connection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ConnectionOutcome {
    Completed,
    ClientDisconnected(ResponseSinkError),
    RequestFailed(RequestFailure),
    /// The response stream and its completion capability moved to a supervised
    /// asynchronous responder. This is not a terminal metric outcome.
    Deferred,
}

impl ConnectionOutcome {
    pub(crate) fn from_request_result(result: std::result::Result<(), RequestFailure>) -> Self {
        match result {
            Ok(()) => Self::Completed,
            Err(RequestFailure::ResponseSink(error)) if error.client_disconnected() => {
                Self::ClientDisconnected(error)
            }
            Err(error) => Self::RequestFailed(error),
        }
    }
}

/// A failure scoped to one IPC request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RequestFailure {
    Handler,
    ResponseSink(ResponseSinkError),
}

/// A bounded classification of an error writing to a client-owned socket.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ResponseSinkError {
    ClientDisconnected,
    TimedOut,
    Unavailable,
}

impl ResponseSinkError {
    pub(crate) fn from_io(error: &io::Error) -> Self {
        match error.kind() {
            io::ErrorKind::BrokenPipe
            | io::ErrorKind::ConnectionAborted
            | io::ErrorKind::ConnectionReset
            | io::ErrorKind::NotConnected
            | io::ErrorKind::UnexpectedEof => Self::ClientDisconnected,
            io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock => Self::TimedOut,
            _ => Self::Unavailable,
        }
    }

    pub(crate) fn client_disconnected(self) -> bool {
        self == Self::ClientDisconnected
    }
}

impl fmt::Display for ResponseSinkError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::ClientDisconnected => "daemon ipc client disconnected",
            Self::TimedOut => "daemon ipc response timed out",
            Self::Unavailable => "daemon ipc response sink unavailable",
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{ConnectionOutcome, RequestFailure, ResponseSinkError};

    #[test]
    fn client_disconnect_is_a_connection_outcome_not_a_request_failure() {
        let outcome = ConnectionOutcome::from_request_result(Err(RequestFailure::ResponseSink(
            ResponseSinkError::ClientDisconnected,
        )));

        assert_eq!(
            outcome,
            ConnectionOutcome::ClientDisconnected(ResponseSinkError::ClientDisconnected)
        );
    }

    #[test]
    fn response_timeout_remains_request_scoped() {
        let outcome = ConnectionOutcome::from_request_result(Err(RequestFailure::ResponseSink(
            ResponseSinkError::TimedOut,
        )));

        assert_eq!(
            outcome,
            ConnectionOutcome::RequestFailed(RequestFailure::ResponseSink(
                ResponseSinkError::TimedOut
            ))
        );
    }
}

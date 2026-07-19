/// A bounded application-command failure consumed by transport adapters.
///
/// Command owners classify domain and dependency failures here without
/// depending on IPC routes or response-writing concerns.
pub(crate) enum CommandFailure {
    BadRequest(&'static str),
    Conflict(&'static str),
    NotFound(&'static str),
    TooLarge(&'static str),
    ServiceUnavailable(&'static str),
    Internal,
}

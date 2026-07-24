use std::io::Write;
use std::net::{Shutdown, TcpStream};
use std::time::Duration;

use super::{ResponseSinkError, ServiceErrorCode};

const RESPONSE_WRITE_TIMEOUT: Duration = Duration::from_secs(2);

pub(crate) fn configure(stream: &TcpStream) -> Result<(), ResponseSinkError> {
    stream
        .set_write_timeout(Some(RESPONSE_WRITE_TIMEOUT))
        .map_err(|error| ResponseSinkError::from_io(&error))
}

pub(crate) fn write_http_response(
    stream: &mut TcpStream,
    status_code: u16,
    content_type: &str,
    body: &str,
) -> Result<(), ResponseSinkError> {
    let reason = match status_code {
        200 => "OK",
        202 => "Accepted",
        400 => "Bad Request",
        401 => "Unauthorized",
        404 => "Not Found",
        409 => "Conflict",
        413 => "Payload Too Large",
        500 => "Internal Server Error",
        503 => "Service Unavailable",
        _ => "Error",
    };
    let header = format!(
        "HTTP/1.1 {status_code} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let mut response = Vec::with_capacity(header.len().saturating_add(body.len()));
    response.extend_from_slice(header.as_bytes());
    response.extend_from_slice(body.as_bytes());
    write_complete_response(stream, &response)
}

pub(crate) fn write_search_response(
    stream: &mut TcpStream,
    server_timing: &str,
    body: &str,
) -> Result<(), ResponseSinkError> {
    let header = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nServer-Timing: {server_timing}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let mut response = Vec::with_capacity(header.len().saturating_add(body.len()));
    response.extend_from_slice(header.as_bytes());
    response.extend_from_slice(body.as_bytes());
    write_complete_response(stream, &response)
}

pub(crate) fn write_all(stream: &mut TcpStream, bytes: &[u8]) -> Result<(), ResponseSinkError> {
    stream
        .write_all(bytes)
        .map_err(|error| ResponseSinkError::from_io(&error))
}

fn write_complete_response(stream: &mut TcpStream, bytes: &[u8]) -> Result<(), ResponseSinkError> {
    if let Err(error) = stream.write_all(bytes) {
        if std::env::var_os("RESUME_IR_S49_RESET_DIAGNOSTICS").is_some() {
            eprintln!(
                "[DEBUG-s49-reset] response_failed stage=write kind={:?}",
                error.kind()
            );
        }
        return Err(ResponseSinkError::from_io(&error));
    }
    stream.shutdown(Shutdown::Write).map_err(|error| {
        if std::env::var_os("RESUME_IR_S49_RESET_DIAGNOSTICS").is_some() {
            eprintln!(
                "[DEBUG-s49-reset] response_failed stage=shutdown kind={:?}",
                error.kind()
            );
        }
        ResponseSinkError::from_io(&error)
    })
}

pub(crate) fn flush(stream: &mut TcpStream) -> Result<(), ResponseSinkError> {
    stream
        .flush()
        .map_err(|error| ResponseSinkError::from_io(&error))
}

pub(crate) fn write_service_unavailable(
    stream: &mut TcpStream,
    code: ServiceErrorCode,
) -> Result<(), ResponseSinkError> {
    let body = unified_error_body(None, code.label(), code.action());
    write_http_response(stream, 503, "application/json", &body)
}

pub(crate) fn unified_error_body(request_id: Option<&str>, code: &str, action: &str) -> String {
    let mut body = serde_json::json!({
        "schema_version": "resume-ir.error.v2",
        "status": "error",
        "error": {
            "code": code,
            "action": action,
            "capability": serde_json::Value::Null,
            "reason": serde_json::Value::Null,
        },
    });
    if let Some(request_id) = request_id {
        body["request_id"] = serde_json::json!(request_id);
    }
    body.to_string()
}

pub(crate) fn service_error_body(
    request_id: Option<&str>,
    code: &str,
    action: &str,
    capability: Option<&str>,
    reason: Option<&str>,
) -> String {
    let mut body = serde_json::json!({
        "schema_version": "resume-ir.error.v2",
        "status": "error",
        "error": {
            "code": code,
            "action": action,
            "capability": capability,
            "reason": reason,
        },
    });
    if let Some(request_id) = request_id {
        body["request_id"] = serde_json::json!(request_id);
    }
    body.to_string()
}

#[cfg(all(test, unix))]
mod tests {
    use std::io::Read;
    use std::net::{TcpListener, TcpStream};
    use std::time::Duration;

    use super::{configure, write_all};
    use crate::ipc::ResponseSinkError;

    #[test]
    fn abortive_peer_close_is_a_request_scoped_response_failure() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind response sink fixture");
        let client = TcpStream::connect(listener.local_addr().unwrap()).expect("connect fixture");
        let (mut server, _) = listener.accept().expect("accept fixture");
        configure(&server).expect("configure response sink fixture");
        server
            .set_read_timeout(Some(Duration::from_secs(2)))
            .expect("bound reset observation");
        let linger = nix::libc::linger {
            l_onoff: 1,
            l_linger: 0,
        };
        nix::sys::socket::setsockopt(&client, nix::sys::socket::sockopt::Linger, &linger)
            .expect("configure abortive peer close");
        drop(client);
        let mut byte = [0_u8; 1];
        let _ = server.read(&mut byte);

        assert_eq!(
            write_all(&mut server, b"response after peer reset"),
            Err(ResponseSinkError::ClientDisconnected)
        );
    }
}

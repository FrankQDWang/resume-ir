use std::io::Write;
use std::net::TcpStream;
use std::time::Duration;

use super::{process_metrics, ResponseSinkError};

const RESPONSE_WRITE_TIMEOUT: Duration = Duration::from_secs(2);

pub(crate) fn configure(stream: &TcpStream) -> Result<(), ResponseSinkError> {
    stream
        .set_write_timeout(Some(RESPONSE_WRITE_TIMEOUT))
        .map_err(|error| ResponseSinkError::from_io(&error))
}

pub(crate) fn write_http_response(
    stream: &mut TcpStream,
    status_code: u16,
    reason: &str,
    content_type: &str,
    body: &str,
) -> Result<(), ResponseSinkError> {
    let header = format!(
        "HTTP/1.1 {status_code} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let mut response = Vec::with_capacity(header.len().saturating_add(body.len()));
    response.extend_from_slice(header.as_bytes());
    response.extend_from_slice(body.as_bytes());
    write_all(stream, &response)
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
    write_all(stream, &response)
}

pub(crate) fn write_all(stream: &mut TcpStream, bytes: &[u8]) -> Result<(), ResponseSinkError> {
    let result = stream
        .write_all(bytes)
        .map_err(|error| ResponseSinkError::from_io(&error));
    if let Err(error) = result {
        process_metrics().record_response_failure(error);
    }
    result
}

pub(crate) fn flush(stream: &mut TcpStream) -> Result<(), ResponseSinkError> {
    let result = stream
        .flush()
        .map_err(|error| ResponseSinkError::from_io(&error));
    if let Err(error) = result {
        process_metrics().record_response_failure(error);
    }
    result
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

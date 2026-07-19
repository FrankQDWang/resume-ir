use std::io::Read;
use std::net::TcpStream;

const MAX_REQUEST_BYTES: usize = 64 * 1024;

pub(crate) enum ReadOutcome {
    Request(Request),
    TooLarge,
    BadRequest,
}

pub(crate) struct Request {
    pub(crate) method: String,
    pub(crate) path: String,
    pub(crate) version: String,
    pub(crate) headers: Vec<(String, String)>,
    pub(crate) body: Vec<u8>,
}

impl Request {
    fn empty() -> Self {
        Self {
            method: String::new(),
            path: String::new(),
            version: String::new(),
            headers: Vec::new(),
            body: Vec::new(),
        }
    }

    pub(crate) fn matches(&self, method: &str, path: &str) -> bool {
        self.method == method
            && self.path == path
            && matches!(self.version.as_str(), "HTTP/1.1" | "HTTP/1.0")
    }
}

pub(crate) fn read(stream: &mut TcpStream) -> ReadOutcome {
    let mut request = Vec::new();
    let mut buffer = [0_u8; 512];
    let header_end = loop {
        let read = match stream.read(&mut buffer) {
            Ok(read) => read,
            Err(_) => return ReadOutcome::BadRequest,
        };
        if read == 0 {
            break None;
        }
        request.extend_from_slice(&buffer[..read]);
        if request.len() > MAX_REQUEST_BYTES {
            return ReadOutcome::TooLarge;
        }
        if let Some(header_end) = find_header_end(&request) {
            break Some(header_end);
        }
    };

    let Some(header_end) = header_end else {
        return ReadOutcome::Request(Request::empty());
    };
    let Ok(header_text) = std::str::from_utf8(&request[..header_end]) else {
        return ReadOutcome::BadRequest;
    };
    let mut lines = header_text.lines();
    let mut first_line = lines.next().unwrap_or_default().split_whitespace();
    let method = first_line.next().unwrap_or_default().to_string();
    let path = first_line.next().unwrap_or_default().to_string();
    let version = first_line.next().unwrap_or_default().to_string();
    let headers = lines
        .filter_map(|line| {
            let (name, value) = line.split_once(':')?;
            Some((name.trim().to_string(), value.trim().to_string()))
        })
        .collect::<Vec<_>>();
    let content_length = match header_value(&headers, "content-length") {
        Some(value) => match value.parse::<usize>() {
            Ok(length) => length,
            Err(_) => return ReadOutcome::BadRequest,
        },
        None => 0,
    };
    let Some(request_end) = header_end.checked_add(content_length) else {
        return ReadOutcome::TooLarge;
    };
    if request_end > MAX_REQUEST_BYTES {
        return ReadOutcome::TooLarge;
    }

    while request.len() < request_end {
        let read = match stream.read(&mut buffer) {
            Ok(read) => read,
            Err(_) => return ReadOutcome::BadRequest,
        };
        if read == 0 {
            break;
        }
        request.extend_from_slice(&buffer[..read]);
        if request.len() > MAX_REQUEST_BYTES {
            return ReadOutcome::TooLarge;
        }
    }
    if request.len() < request_end {
        return ReadOutcome::BadRequest;
    }

    ReadOutcome::Request(Request {
        method,
        path,
        version,
        headers,
        body: request[header_end..request_end].to_vec(),
    })
}

pub(crate) fn authorized(expected: &str, headers: &[(String, String)]) -> bool {
    let Some(header) = header_value(headers, "authorization") else {
        return false;
    };
    let Some(token) = header.strip_prefix("Bearer ") else {
        return false;
    };
    constant_time_eq(token.trim().as_bytes(), expected.as_bytes())
}

pub(crate) fn header_value<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(header_name, _)| header_name.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.as_str())
}

fn find_header_end(buffer: &[u8]) -> Option<usize> {
    buffer
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|index| index + 4)
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    let mut diff = 0_u8;
    for (left, right) in left.iter().zip(right.iter()) {
        diff |= left ^ right;
    }
    diff == 0
}

#[cfg(test)]
mod tests {
    use super::{authorized, Request};

    #[test]
    fn route_matching_rejects_unknown_http_versions() {
        let request = Request {
            method: "GET".to_string(),
            path: "/status".to_string(),
            version: "HTTP/2".to_string(),
            headers: Vec::new(),
            body: Vec::new(),
        };
        assert!(!request.matches("GET", "/status"));
    }

    #[test]
    fn authorization_requires_exact_bearer_token() {
        let headers = vec![("Authorization".to_string(), "Bearer token".to_string())];
        assert!(authorized("token", &headers));
        assert!(!authorized("other", &headers));
    }
}

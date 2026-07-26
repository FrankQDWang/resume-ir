use std::io::{self, Read};

pub fn read_response(reader: &mut impl Read, max_response_bytes: usize) -> io::Result<String> {
    let mut response = Vec::new();
    let mut chunk = [0_u8; 4096];
    loop {
        if let Some(frame_len) = complete_frame_len(&response)? {
            if response.len() != frame_len {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "HTTP response contains bytes after the declared body",
                ));
            }
            return String::from_utf8(response).map_err(|_| {
                io::Error::new(io::ErrorKind::InvalidData, "HTTP response is not UTF-8")
            });
        }
        if response.len() == max_response_bytes {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "HTTP response exceeds the test client limit",
            ));
        }
        let remaining = max_response_bytes - response.len();
        let chunk_limit = remaining.min(chunk.len());
        let read = reader.read(&mut chunk[..chunk_limit])?;
        if read == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "HTTP response ended before the declared body was complete",
            ));
        }
        response.extend_from_slice(&chunk[..read]);
    }
}

fn complete_frame_len(response: &[u8]) -> io::Result<Option<usize>> {
    let Some(header_offset) = response.windows(4).position(|window| window == b"\r\n\r\n") else {
        return Ok(None);
    };
    let header_len = header_offset + 4;
    let header = std::str::from_utf8(&response[..header_offset]).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "HTTP response header is not UTF-8",
        )
    })?;
    let content_length = header
        .lines()
        .skip(1)
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim())
        })
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "HTTP response is missing Content-Length",
            )
        })?
        .parse::<usize>()
        .map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "HTTP response has an invalid Content-Length",
            )
        })?;
    let frame_len = header_len.checked_add(content_length).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "HTTP response length overflows usize",
        )
    })?;
    Ok((response.len() >= frame_len).then_some(frame_len))
}

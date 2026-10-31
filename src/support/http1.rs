//! HTTP/1.1 framing shared by the direct TCP benchmark servers.

pub struct Head<'a> {
    pub method: &'a str,
    pub path: &'a str,
    pub header_len: usize,
    pub end: usize,
    pub close: bool,
}

fn parse_with_headers<'a>(
    input: &'a [u8],
    headers: &mut [httparse::Header<'a>],
) -> Result<Option<Head<'a>>, httparse::Error> {
    let mut request = httparse::Request::new(headers);
    let httparse::Status::Complete(header_len) = request.parse(input)? else {
        return Ok(None);
    };
    if request.version != Some(1) {
        return Err(httparse::Error::Version);
    }

    let mut length = None;
    let mut close = false;
    for header in request.headers.iter() {
        if header.name.eq_ignore_ascii_case("content-length") {
            let value = header.value.trim_ascii();
            if length.is_some() || value.is_empty() || !value.iter().all(u8::is_ascii_digit) {
                return Err(httparse::Error::HeaderValue);
            }
            length = Some(
                std::str::from_utf8(value)
                    .map_err(|_| httparse::Error::HeaderValue)?
                    .parse::<usize>()
                    .map_err(|_| httparse::Error::HeaderValue)?,
            );
        } else if header.name.eq_ignore_ascii_case("transfer-encoding")
            || header.name.eq_ignore_ascii_case("expect")
        {
            return Err(httparse::Error::HeaderValue);
        } else if header.name.eq_ignore_ascii_case("connection") {
            close |= header
                .value
                .split(|byte| *byte == b',')
                .any(|value| value.trim_ascii().eq_ignore_ascii_case(b"close"));
        }
    }

    let method = request.method.ok_or(httparse::Error::Token)?;
    let target = request.path.ok_or(httparse::Error::Token)?;
    if !target.starts_with('/') {
        return Err(httparse::Error::Token);
    }
    let path = target.split_once('?').map_or(target, |(path, _)| path);
    let end = header_len
        .checked_add(length.unwrap_or(0))
        .ok_or(httparse::Error::HeaderValue)?;
    Ok(Some(Head {
        method,
        path,
        header_len,
        end,
        close,
    }))
}

pub fn parse_head(input: &[u8]) -> Result<Option<Head<'_>>, httparse::Error> {
    // Stack storage covers small requests; larger header sets grow without a fixed cap.
    let mut headers = [httparse::EMPTY_HEADER; 16];
    match parse_with_headers(input, &mut headers) {
        Err(httparse::Error::TooManyHeaders) => {
            let mut capacity = headers.len() * 2;
            loop {
                let mut headers = vec![httparse::EMPTY_HEADER; capacity];
                match parse_with_headers(input, &mut headers) {
                    Err(httparse::Error::TooManyHeaders) => capacity *= 2,
                    result => return result,
                }
            }
        }
        result => result,
    }
}

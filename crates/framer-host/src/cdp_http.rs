use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::http::Uri;

const MAX_HEADERS: usize = 16 * 1024;
const MAX_BODY: usize = 8 * 1024 * 1024;

pub(super) async fn put(url: &str) -> Result<Vec<u8>, String> {
    tokio::time::timeout(Duration::from_secs(5), request("PUT", url)).await
        .map_err(|_| "devtools HTTP request timed out".to_owned())?
}

pub(super) async fn get(url: &str) -> Result<Vec<u8>, String> {
    tokio::time::timeout(Duration::from_secs(5), request("GET", url)).await
        .map_err(|_| "http get timed out".to_owned())?
}

async fn request(method: &str, url: &str) -> Result<Vec<u8>, String> {
    let uri: Uri = url.parse().map_err(|_| "invalid devtools HTTP URL")?;
    if uri.scheme_str() != Some("http") {
        return Err("invalid devtools HTTP URL".into());
    }
    let authority = uri.authority().ok_or("missing devtools HTTP authority")?;
    let path = uri.path_and_query().map_or("/", |value| value.as_str());
    let mut stream = TcpStream::connect(authority.as_str()).await
        .map_err(|error| format!("connect devtools HTTP: {error}"))?;
    let request = format!("{method} {path} HTTP/1.1\r\nHost: {authority}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
    stream.write_all(request.as_bytes()).await
        .map_err(|error| format!("write devtools HTTP: {error}"))?;
    let mut response = BufReader::new(stream);
    let mut headers = Vec::new();
    while !headers.ends_with(b"\r\n\r\n") {
        if headers.len() == MAX_HEADERS {
            return Err("devtools HTTP headers too large".into());
        }
        headers.push(response.read_u8().await
            .map_err(|error| format!("read devtools HTTP headers: {error}"))?);
    }
    let length = response_length(&headers)?;
    let mut body = Vec::new();
    if let Some(length) = length {
        body.resize(length, 0);
        response.read_exact(&mut body).await
            .map_err(|error| format!("read devtools HTTP body: {error}"))?;
    } else {
        response.take((MAX_BODY + 1) as u64).read_to_end(&mut body).await
            .map_err(|error| format!("read devtools HTTP body: {error}"))?;
        if body.len() > MAX_BODY {
            return Err("devtools HTTP body too large".into());
        }
    }
    Ok(body)
}

fn response_length(headers: &[u8]) -> Result<Option<usize>, String> {
    let headers = std::str::from_utf8(headers).map_err(|_| "invalid devtools HTTP headers")?;
    let mut lines = headers.split("\r\n");
    let mut status = lines.next().unwrap_or_default().split_whitespace();
    if !matches!(status.next(), Some("HTTP/1.0" | "HTTP/1.1")) {
        return Err("invalid devtools HTTP status".into());
    }
    let code: u16 = status.next().and_then(|value| value.parse().ok())
        .ok_or("invalid devtools HTTP status")?;
    if !(200..300).contains(&code) {
        return Err(format!("devtools HTTP status {code}"));
    }
    let mut length = None;
    for line in lines.filter(|line| !line.is_empty()) {
        let (name, value) = line.split_once(':').ok_or("invalid devtools HTTP header")?;
        if name.eq_ignore_ascii_case("content-length") {
            let current: usize = value.trim().parse()
                .map_err(|_| "invalid devtools HTTP content length")?;
            if current > MAX_BODY {
                return Err("devtools HTTP body too large".into());
            }
            if length.is_some_and(|previous| previous != current) {
                return Err("conflicting devtools HTTP content lengths".into());
            }
            length = Some(current);
        } else if name.eq_ignore_ascii_case("transfer-encoding")
            && !value.trim().eq_ignore_ascii_case("identity")
        {
            return Err("unsupported devtools HTTP transfer encoding".into());
        }
    }
    Ok(length)
}

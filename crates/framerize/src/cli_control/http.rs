use reqwest::{Client, Method, RequestBuilder, Url};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

pub(super) struct DaemonClient {
    client: Client,
    base: Url,
}

impl DaemonClient {
    pub(super) fn new(raw: &str) -> Result<Self, String> {
        let mut base = Url::parse(raw).map_err(|_| "invalid daemon URL")?;
        if !matches!(base.scheme(), "http" | "https") || base.host_str().is_none()
            || !base.username().is_empty() || base.password().is_some()
            || base.query().is_some() || base.fragment().is_some()
        {
            return Err("daemon URL must be HTTP(S), without credentials, query or fragment".into());
        }
        if !base.path().ends_with('/') {
            let path = format!("{}/", base.path());
            base.set_path(&path);
        }
        let client = Client::builder().connect_timeout(Duration::from_secs(5))
            .retry(reqwest::retry::never())
            .redirect(reqwest::redirect::Policy::none()).build()
            .map_err(|_| "could not initialize HTTP client")?;
        Ok(Self { client, base })
    }

    pub(super) fn request(&self, method: Method, path: &str) -> Result<RequestBuilder, String> {
        if !path.starts_with('/') || path.starts_with("//") || path.contains('\\')
            || path.chars().any(char::is_control) || path.contains('#')
        {
            return Err("API path must begin with one slash and contain no fragment".into());
        }
        let url = self.base.join(&format!(".{path}")).map_err(|_| "invalid API path")?;
        if url.origin() != self.base.origin() || !url.path().starts_with(self.base.path()) {
            return Err("API path must stay under the configured daemon URL".into());
        }
        Ok(self.client.request(method, url))
    }
}

pub(super) fn transport_error(error: reqwest::Error) -> String {
    if error.is_connect() { "daemon connection failed; check --daemon or FRAMERIZE_URL".into() }
    else if error.is_timeout() { "daemon request timed out; the operation may still be pending".into() }
    else { "daemon transport failed; the operation may already have been received".into() }
}

pub(super) async fn small_input(path: &str, limit: u64) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    if path == "-" {
        tokio::io::stdin().take(limit + 1).read_to_end(&mut bytes).await
            .map_err(|_| "could not read standard input")?;
    } else {
        let file = tokio::fs::File::open(path).await.map_err(|_| "could not open input file")?;
        file.take(limit + 1).read_to_end(&mut bytes).await.map_err(|_| "could not read input file")?;
    }
    if bytes.len() as u64 > limit { return Err("input exceeds the supported size".into()); }
    Ok(bytes)
}

pub(super) async fn headers_file(path: Option<&str>) -> Result<reqwest::header::HeaderMap, String> {
    use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
    let mut headers = HeaderMap::new();
    let Some(path) = path else { return Ok(headers); };
    let bytes = small_input(path, 64 * 1024).await?;
    let value: serde_json::Value = serde_json::from_slice(&bytes)
        .map_err(|_| "headers file must contain a JSON object of string values")?;
    let object = value.as_object().ok_or("headers file must contain a JSON object")?;
    for (name, value) in object {
        let name = HeaderName::from_bytes(name.as_bytes()).map_err(|_| "invalid header name")?;
        if matches!(name.as_str(), "host" | "content-length" | "transfer-encoding" | "connection") {
            return Err("transport framing headers are managed by the HTTP client".into());
        }
        let raw = value.as_str().ok_or("header values must be strings")?;
        let value = HeaderValue::from_str(raw).map_err(|_| "invalid header value")?;
        if headers.insert(name, value).is_some() {
            return Err("duplicate header name after case normalization".into());
        }
    }
    Ok(headers)
}

pub(super) async fn stream_response(mut response: reqwest::Response) -> Result<(), String> {
    let status = response.status();
    let retry_after = response.headers().get(reqwest::header::RETRY_AFTER)
        .and_then(|value| value.to_str().ok()).and_then(|value| value.parse::<u64>().ok());
    let mut output = tokio::io::stdout();
    while let Some(bytes) = response.chunk().await.map_err(transport_error)? {
        output.write_all(&bytes).await.map_err(|_| "could not write response to standard output")?;
        output.flush().await.map_err(|_| "could not flush standard output")?;
    }
    if status.is_success() { return Ok(()); }
    let mut message = format!("daemon returned HTTP {}", status.as_u16());
    if let Some(seconds) = retry_after {
        message.push_str(&format!("; retry after {seconds} seconds"));
    }
    Err(message)
}

pub(super) struct Reply {
    pub status: reqwest::StatusCode,
    pub value: serde_json::Value,
    pub retry_after: Option<u64>,
}

async fn json_response(mut response: reqwest::Response) -> Result<Reply, String> {
    let status = response.status();
    let retry_after = response.headers().get(reqwest::header::RETRY_AFTER)
        .and_then(|value| value.to_str().ok()).and_then(|value| value.parse::<u64>().ok());
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(transport_error)? {
        if bytes.len().saturating_add(chunk.len()) > 1024 * 1024 {
            return Err("control response exceeds 1 MiB; use the streaming api command for large responses".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    let value = match serde_json::from_slice(&bytes) {
        Ok(value) => value,
        Err(_) if status.is_success() && bytes.as_slice() == b"sent" => serde_json::json!({"status": "sent"}),
        Err(_) if !status.is_success() => serde_json::json!({
            "code": "http_error", "message": status.canonical_reason().unwrap_or("request rejected")
        }),
        Err(_) => return Err("control response was not valid JSON".into()),
    };
    Ok(Reply { status, value, retry_after })
}

pub(super) async fn send_json(options: &super::Options, method: Method, path: &str,
    body: Option<serde_json::Value>) -> Result<Reply, String> {
    let client = DaemonClient::new(&options.daemon)?;
    let headers = headers_file(options.headers_file.as_deref()).await?;
    let mut request = client.request(method, path)?.headers(headers)
        .timeout(Duration::from_secs(120));
    if let Some(body) = body { request = request.json(&body); }
    json_response(request.send().await.map_err(transport_error)?).await
}

use clap::Args;
use reqwest::{header::CONTENT_TYPE, Method};
use super::http::{headers_file, small_input, stream_response, transport_error, DaemonClient};

#[derive(Args)]
pub struct ApiArgs {
    /// HTTP method, for example GET or POST.
    pub method: String,
    /// Daemon-relative API path, for example /api/pool or /openai/v1/responses.
    pub path: String,
    #[arg(long, env = "FRAMERIZE_URL", default_value = "http://127.0.0.1:8787")]
    pub daemon: String,
    /// Read the request body from a file, or use - for standard input.
    #[arg(long, value_name = "FILE|-")]
    pub data_file: Option<String>,
    /// JSON object of request headers. Keep credentials in this file.
    #[arg(long, value_name = "FILE")]
    pub headers_file: Option<String>,
    /// Optional total deadline. Streams have no total deadline by default.
    #[arg(long, value_parser = clap::value_parser!(u64).range(1..))]
    pub timeout_seconds: Option<u64>,
}

pub async fn run_api(args: ApiArgs) -> Result<(), String> {
    if args.data_file.as_deref() == Some("-") && args.headers_file.as_deref() == Some("-") {
        return Err("body and headers cannot both read standard input".into());
    }
    let method = Method::from_bytes(args.method.to_ascii_uppercase().as_bytes())
        .map_err(|_| "invalid HTTP method")?;
    let client = DaemonClient::new(&args.daemon)?;
    let headers = headers_file(args.headers_file.as_deref()).await?;
    let has_content_type = headers.contains_key(CONTENT_TYPE);
    let mut request = client.request(method, &args.path)?.headers(headers);
    if let Some(seconds) = args.timeout_seconds {
        request = request.timeout(std::time::Duration::from_secs(seconds));
    }
    if let Some(path) = args.data_file {
        if !has_content_type { request = request.header(CONTENT_TYPE, "application/json"); }
        let body = if path == "-" {
            reqwest::Body::from(small_input("-", 64 * 1024 * 1024).await?)
        } else {
            let file = tokio::fs::File::open(path).await.map_err(|_| "could not open body file")?;
            reqwest::Body::from(file)
        };
        request = request.body(body);
    }
    let response = request.send().await.map_err(transport_error)?;
    stream_response(response).await
}

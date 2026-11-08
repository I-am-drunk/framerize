//! Custom-route HTTP surface: POST /api/custom/complete.
//!
//! Thin shell over the framer-custom engine. The engine's only upstream
//! contact is CustomTransport. Mock mode (FRAMERIZE_CUSTOM_TRANSPORT=mock)
//! runs a scripted transport for route-level checks and the integration
//! test below. Real mode serves the live page stream once the host side is
//! wired (see REAL_MODE below); until then it answers 501 so production is
//! never served scripted data.
//!
//! REAL_MODE (why real still 501s): serving a live leg needs, per request,
//! (a) the serving account + session + project/team scope (AppState path,
//! mirroring native_complete), (b) the AI bearer JWT (minted in framer-api
//! via Transport + per-account token cache -- the server holds neither),
//! (c) the Gq envelope (projectId/category/turnId, built inside
//! proxy::attempt), and (d) a &CdpBridge for CdpPageEval while AppState
//! holds Arc<dyn BrowserBridge> (no downcast exists anywhere in the repo).
//! That is a small, well-understood wiring job in framer-server -- NOT a
//! research problem -- but it touches auth/account selection and must not
//! be improvised inside a build step. The host capability it needs
//! (HostPageStream + CdpPageEval, many small evals, Mutex never held across
//! the stream) is implemented and unit-tested in framer-host::page_stream.

use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use serde::Deserialize;

pub const TRANSPORT_ENV: &str = "FRAMERIZE_CUSTOM_TRANSPORT";

pub fn custom_routes() -> Router {
    Router::new().route("/complete", post(custom_complete_handler))
}

#[derive(Deserialize)]
struct CompleteBody {
    #[serde(default)]
    prompt: String,
    #[serde(default)]
    model: String,
}

fn mock_script() -> Vec<framer_custom::Event> {
    vec![
        framer_custom::Event::Delta("The custom route streams its first prefix here, ".into()),
        framer_custom::Event::Delta("and the second delta carries it past the finish bar. ".into()),
        framer_custom::Event::Done,
    ]
}

async fn custom_complete_handler(
    _headers: HeaderMap,
    Json(body): Json<CompleteBody>,
) -> Response {
    if body.prompt.trim().is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "prompt is required"})),
        )
            .into_response();
    }
    let mode = std::env::var(TRANSPORT_ENV).unwrap_or_else(|_| "real".into());
    if mode != "mock" {
        // Fail closed with the EXACT missing piece (see REAL_MODE above):
        // the host stream exists, the per-request account/token/envelope
        // wiring does not -- yet.
        return (
            StatusCode::NOT_IMPLEMENTED,
            Json(serde_json::json!({"error": "custom transport not wired: live page stream needs per-request account/token/envelope wiring (see REAL_MODE)"})),
        )
            .into_response();
    }
    let _model = body.model.clone();
    let mut transport = framer_custom::mock_util::ScriptedTransport::new(mock_script(), 100.0);
    let settings = framer_custom::ChainSettings::default();
    let assembled = match framer_custom::complete(&mut transport, &body.prompt, &settings).await {
        Ok(a) => a,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": e})),
            )
                .into_response();
        }
    };
    let event = format!(
        "data: {}\n\n",
        serde_json::json!({"text": assembled.text, "legs": assembled.legs.len()})
    );
    ([("content-type", "text/event-stream")], event).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn mock_complete_serves_assembled_sse() {
        std::env::set_var(TRANSPORT_ENV, "mock");
        let app = Router::new().nest("/api/custom", custom_routes());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        let payload =
            serde_json::json!({"prompt": "explain X", "model": "gpt-4.1"}).to_string();
        let req = format!(
            "POST /api/custom/complete HTTP/1.1\r\nhost: 127.0.0.1\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
            payload.len(),
            payload
        );
        let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        stream.write_all(req.as_bytes()).await.unwrap();
        let mut raw = Vec::new();
        stream.read_to_end(&mut raw).await.unwrap();
        let text = String::from_utf8_lossy(&raw);
        let head = &text[..text.len().min(200)];
        assert!(text.contains("200"), "status line: {head}");
        assert!(text.contains("text/event-stream"), "sse content type");
        assert!(text.contains("data: "), "sse frame");
        assert!(text.contains("first prefix here"), "assembled text");
    }
}

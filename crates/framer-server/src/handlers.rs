//! Compat route shims: thin free-vs-paid dispatchers for the three
//! dialects, the token counter, and the header helper. Pure move out of
//! lib.rs (no behavior change). The engines (free_complete, compat_forward)
//! stay in lib.rs.

use super::{AppState, ApiError};
use crate::{compat_forward, free_complete};
use framer_core::convert::ClientDialect;
use axum::extract::State;
use axum::Json;

pub(super) async fn anthropic_messages(
    State(s): State<AppState>,
    headers: axum::http::HeaderMap,
    Json(payload): Json<serde_json::Value>,
) -> Result<axum::response::Response, ApiError> {
    if hdr(&headers, "x-framerize-mode").as_deref() != Some("paid") {
        return free_complete(s, &headers, ClientDialect::AnthropicMessages, payload).await;
    }
    compat_forward(
        s,
        "anthropic-messages",
        hdr(&headers, "x-framerize-account"),
        hdr(&headers, "x-framerize-session"),
        payload,
    )
    .await
}


pub(super) async fn openai_responses(
    State(s): State<AppState>,
    headers: axum::http::HeaderMap,
    Json(payload): Json<serde_json::Value>,
) -> Result<axum::response::Response, ApiError> {
    if hdr(&headers, "x-framerize-mode").as_deref() != Some("paid") {
        return free_complete(s, &headers, ClientDialect::OpenAiResponses, payload).await;
    }
    compat_forward(
        s,
        "openai-responses",
        hdr(&headers, "x-framerize-account"),
        hdr(&headers, "x-framerize-session"),
        payload,
    )
    .await
}


pub(super) async fn openai_chat(
    State(s): State<AppState>,
    headers: axum::http::HeaderMap,
    Json(payload): Json<serde_json::Value>,
) -> Result<axum::response::Response, ApiError> {
    if hdr(&headers, "x-framerize-mode").as_deref() != Some("paid") {
        return free_complete(s, &headers, ClientDialect::OpenAiChat, payload).await;
    }
    compat_forward(
        s,
        "openrouter-chat",
        hdr(&headers, "x-framerize-account"),
        hdr(&headers, "x-framerize-session"),
        payload,
    )
    .await
}




/// Anthropic count_tokens shape; character-based estimate (≈4 chars/token) —
/// the same fallback Claude Code applies when the endpoint is absent.
pub(super) async fn count_tokens(Json(body): Json<serde_json::Value>) -> Json<serde_json::Value> {
    let chars: usize = body["messages"]
        .as_array()
        .map(|ms| {
            ms.iter()
                .map(|m| {
                    m["content"]
                        .as_str()
                        .map(|s| s.len())
                        .or_else(|| {
                            m["content"].as_array().map(|bs| {
                                bs.iter()
                                    .map(|b| b["text"].as_str().map(|s| s.len()).unwrap_or(0))
                                    .sum()
                            })
                        })
                        .unwrap_or(0)
                })
                .sum::<usize>()
        })
        .unwrap_or(0)
        + body["system"].as_str().map(|s| s.len()).unwrap_or(0);
    Json(serde_json::json!({ "input_tokens": (chars / 4).max(1) }))
}

pub(super) async fn anthropic_messages_paid(
    State(s): State<AppState>,
    headers: axum::http::HeaderMap,
    Json(payload): Json<serde_json::Value>,
) -> Result<axum::response::Response, ApiError> {
    compat_forward(
        s,
        "anthropic-messages",
        hdr(&headers, "x-framerize-account"),
        hdr(&headers, "x-framerize-session"),
        payload,
    )
    .await
}


pub(super) async fn openai_responses_paid(
    State(s): State<AppState>,
    headers: axum::http::HeaderMap,
    Json(payload): Json<serde_json::Value>,
) -> Result<axum::response::Response, ApiError> {
    compat_forward(
        s,
        "openai-responses",
        hdr(&headers, "x-framerize-account"),
        hdr(&headers, "x-framerize-session"),
        payload,
    )
    .await
}


pub(super) async fn openai_chat_paid(
    State(s): State<AppState>,
    headers: axum::http::HeaderMap,
    Json(payload): Json<serde_json::Value>,
) -> Result<axum::response::Response, ApiError> {
    compat_forward(
        s,
        "openrouter-chat",
        hdr(&headers, "x-framerize-account"),
        hdr(&headers, "x-framerize-session"),
        payload,
    )
    .await
}


pub(super) fn hdr(headers: &axum::http::HeaderMap, name: &str) -> Option<String> {
    headers.get(name).and_then(|v| v.to_str().ok()).map(str::to_string)
}

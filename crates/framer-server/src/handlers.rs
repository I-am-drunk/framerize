//! Compat route shims: thin free-vs-paid dispatchers for the three
//! dialects, the token counter, and the header helper. Pure move out of
//! lib.rs (no behavior change). The engines (free_complete, compat_forward)
//! stay in lib.rs.

use super::{AppState, ApiError};
use crate::{compat_forward, free_complete};
use framer_core::convert::ClientDialect;
use axum::extract::State;
use axum::Json;

/// Experimental one-request Batch submission through Framer's browser gateway.
/// Disabled unless the runtime owner opts in at boot. This deliberately returns
/// the raw upstream response so a missing Framer route is observable as such.
pub(super) async fn openrouter_batch_probe(
    State(s): State<AppState>,
    Json(payload): Json<serde_json::Value>,
) -> Result<axum::response::Response, ApiError> {
    use axum::response::IntoResponse;

    if std::env::var("FRAMERIZE_BATCH_PROBE").ok().as_deref() != Some("1") {
        return Err(ApiError::InvalidRequest("Batch probe is disabled".into()));
    }
    let requests = payload["requests"].as_array();
    if payload["endpoint"] != "/v1/chat/completions"
        || payload["model"] != "anthropic/claude-opus-4.8"
        || requests.map(|items| items.len()) != Some(1)
        || requests.and_then(|items| items.first())
            .and_then(|item| item["body"]["max_tokens"].as_u64())
            .map_or(true, |tokens| tokens == 0 || tokens > 128)
        || serde_json::to_vec(&payload).map_or(true, |bytes| bytes.len() > 16_384)
    {
        return Err(ApiError::InvalidRequest(
            "Batch probe requires one small Claude Opus chat-completions request".into(),
        ));
    }
    static PROBE_COUNT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    if PROBE_COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed) >= 4 {
        return Err(ApiError::InvalidRequest("Batch probe call limit reached".into()));
    }
    let pool = s.state.list().await.map_err(ApiError::Internal)?;
    let account = s.serving_accounts.select(pool, None)?;
    let _residency = crate::lease_account(&s, account.id).await?;
    let account = crate::current_account(&s, account.id).await?;
    let session = s.sessions.load(&account.framer_user_id).await
        .map_err(ApiError::Internal)?
        .ok_or_else(|| ApiError::Internal("account has no session".into()))?;
    let funded = crate::pick_funded_scope(&s, &account, &session).await?;
    let bearer = s.client.custom_bearer(&session, &account.id).await
        .map_err(ApiError::Internal)?;
    let envelope = framer_api::proxy::envelope(
        &funded.project, "agents", "message", &payload,
        &crate::stable_session(&account.id), 0,
    );
    s.browser.seed_session(&account.id, &session.cookie_header).await
        .map_err(ApiError::Internal)?;
    let (status, _usage_header, body) = s.browser.ai_post(
        &account.id,
        framer_api::routes::openrouter_batch::PATH,
        &bearer,
        &envelope,
    ).await.map_err(ApiError::Internal)?;
    let status = axum::http::StatusCode::from_u16(status)
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok((status, body).into_response())
}

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

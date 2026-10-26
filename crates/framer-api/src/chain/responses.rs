//! Compatibility entry point binding one runtime operation to Responses policy.
use crate::{auth::AiToken, forward::ResponsesForward, transport::Transport};
use framer_core::{convert::{AssembledResponses, ChainEvent}, ids::{AccountId, ProjectId},
    port::{BrowserBridge, ChainConfig, Session}};
use serde_json::Value;
use std::sync::Mutex;

#[allow(clippy::too_many_arguments)]
pub async fn complete_long(
    t: &Transport, token_cache: &Mutex<Vec<(AccountId, AiToken)>>,
    session: &Session, browser: &dyn BrowserBridge, account: &AccountId,
    project: &ProjectId, client_payload: Value, cfg: &ChainConfig,
    chat_session: &str, events: Option<&(dyn Fn(ChainEvent) + Send + Sync)>,
) -> Result<AssembledResponses, String> {
    let forward = ResponsesForward::new(t, token_cache, session, browser, account,
        project, chat_session, cfg.cancelled.as_deref(), cfg.on_429.as_deref());
    framer_protocol::responses::complete_long(&forward, client_payload, &cfg.limits(), events).await
}

#[allow(clippy::too_many_arguments)]
pub async fn complete_long_with_sink(
    t: &Transport, token_cache: &Mutex<Vec<(AccountId, AiToken)>>,
    session: &Session, browser: &dyn BrowserBridge, account: &AccountId,
    project: &ProjectId, client_payload: Value, cfg: &ChainConfig,
    chat_session: &str, sink: &dyn framer_core::port::ResponsesSink,
) -> framer_core::port::ResponseResult {
    let forward = ResponsesForward::new(t, token_cache, session, browser, account,
        project, chat_session, cfg.cancelled.as_deref(), cfg.on_429.as_deref());
    framer_protocol::responses::complete_long_with_sink(
        &forward, client_payload, &cfg.limits(), sink).await
}

#[cfg(test)]
use serde_json::json;
#[cfg(test)]
use framer_protocol::responses::test_support::{allows_transport_retry, parse_streamed_chunk};
#[cfg(test)]
#[path = "../nativechain_tool_tests.rs"]
mod tool_completion_tests;
#[cfg(test)]
#[path = "responses_sse_tests.rs"]
mod responses_sse_tests;
#[cfg(test)]
#[path = "responses_fallback_tests.rs"]
mod responses_fallback_tests;
#[cfg(test)]
#[path = "responses_active_budget_tests.rs"]
mod responses_active_budget_tests;

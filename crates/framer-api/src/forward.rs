//! Runtime binding for one invocation of the free Responses policy.
use crate::{auth::AiToken, proxy::{self, Dialect}, transport::Transport};
use framer_core::{ids::{AccountId, ProjectId}, port::{BrowserBridge, Session}};
use framer_protocol::forward::{AuthorizedForward, ChainControl, ForwardReply};
use framer_protocol::forward_error::{ForwardError, PolicyError};
use futures::StreamExt;
use serde_json::Value;
use std::sync::Mutex;

#[cfg(test)]
#[path = "forward_tests.rs"]
mod tests;

pub(crate) struct ResponsesForward<'a> {
    transport: &'a Transport,
    cache: &'a Mutex<Vec<(AccountId, AiToken)>>,
    session: &'a Session,
    browser: &'a dyn BrowserBridge,
    account: &'a AccountId,
    project: &'a ProjectId,
    chat_session: &'a str,
    cancelled: Option<&'a (dyn Fn() -> bool + Send + Sync)>,
    on_429: Option<&'a (dyn Fn() + Send + Sync)>,
    category: String,
}

impl<'a> ResponsesForward<'a> {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        transport: &'a Transport, cache: &'a Mutex<Vec<(AccountId, AiToken)>>,
        session: &'a Session, browser: &'a dyn BrowserBridge, account: &'a AccountId,
        project: &'a ProjectId, chat_session: &'a str,
        cancelled: Option<&'a (dyn Fn() -> bool + Send + Sync)>,
        on_429: Option<&'a (dyn Fn() + Send + Sync)>,
    ) -> Self {
        Self { transport, cache, session, browser, account, project, chat_session,
            cancelled, on_429, category: proxy::category_from_env() }
    }
}

impl ChainControl for ResponsesForward<'_> {
    fn is_cancelled(&self) -> bool { self.cancelled.is_some_and(|check| check()) }
    fn on_429(&self) { if let Some(callback) = self.on_429 { callback(); } }
}

#[async_trait::async_trait]
impl AuthorizedForward for ResponsesForward<'_> {
    fn category(&self) -> &str { &self.category }
    async fn forward(&self, step: &str, payload: Value, step_index: u32)
        -> Result<ForwardReply, String>
    {
        self.forward_typed(step, payload, step_index).await.map_err(|error| error.to_string())
    }
    async fn forward_typed(&self, step: &str, payload: Value, step_index: u32)
        -> Result<ForwardReply, PolicyError>
    {
        let mut response = proxy::forward_typed_with_category(self.transport, self.cache,
            self.session, self.browser, self.account, self.project,
            Dialect::OpenRouterResponses, step, payload, self.chat_session,
            step_index, self.cancelled, Some(&self.category)).await?;
        let mut body = bytes::BytesMut::new();
        while let Some(chunk) = response.stream.next().await {
            body.extend_from_slice(&chunk.map_err(ForwardError::from_legacy)?);
        }
        Ok(ForwardReply { status: response.status, content_type: response.content_type,
            body: body.freeze(), queue_wait_ms: response.queue_wait_ms })
    }
}

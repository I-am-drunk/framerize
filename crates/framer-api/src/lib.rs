//! THE FRAGILE LAYER. The only code that speaks Framer's private endpoints.
//! One file per concern. When Framer changes, you edit exactly one file here —
//! nothing above framer-api moves.
//!
//! Contract facts (live capture + static analysis, 2026-10-01; see reference/analysis/):
//!   - login state is ONE cookie: `session` on .framer.com (~1y).
//!   - every /ai/v3/* call needs BOTH a 600 s Bearer JWT (GET /auth/ai/access-token,
//!     cookie-authed) AND a fresh single-shot Castle token (BrowserBridge webview).
//!   - 401 on /ai/v3/* = JWT expired → remint and retry once. 401 minting the JWT
//!     = session dead (signed out). Never confuse the two.
//!   - 403 = Castle risk refusal → "risk-refused", cool down, NEVER quarantine.
//!   - 402 = workspace out of credits → "credits-exhausted", pipeline rotates.
//!   - success carries X-Framer-AI-Usage: remaining=<credits> (URLSearchParams).

mod auth;
mod castle;
mod credits;
pub mod freechat;
pub mod inference;
mod login;
mod models;
pub mod proxy;
mod transport;
mod workspace;

use async_trait::async_trait;
use framer_core::ids::{AccountId, TeamId};
use framer_core::port::{
    BrowserBridge, CastleToken, FramerClient, Session, WorkspaceSpec,
};
use framer_core::pipeline::{InferenceRequest, InferenceResult};
use framer_core::workspace::CreditSnapshot;
use futures::StreamExt;
use transport::Transport;

/// Live implementation of the core `FramerClient` port.
pub struct HttpFramerClient {
    transport: Transport,
    /// Cached AI JWT; 600 s TTL upstream, refreshed 30 s early (mirrors the app).
    ai_token: std::sync::Mutex<Option<(AccountId, auth::AiToken)>>,
}

impl HttpFramerClient {
    pub fn new() -> Self {
        Self {
            transport: Transport::new(),
            ai_token: std::sync::Mutex::new(None),
        }
    }
}

impl Default for HttpFramerClient {
    fn default() -> Self {
        Self::new()
    }
}

impl HttpFramerClient {
    /// Compat-layer passthrough: forward one native payload (OpenAI Responses,
    /// OpenAI chat/completions, or Anthropic Messages) through the gateway,
    /// sent in-page from the account's browser (transport-consistent with the
    /// Castle mint — the only configuration the gateway accepts).
    #[allow(clippy::too_many_arguments)] // boundary fn; args are all required
    pub async fn proxy_raw(
        &self,
        session: &Session,
        browser: &dyn BrowserBridge,
        account: &AccountId,
        project: &framer_core::ids::ProjectId,
        dialect: proxy::Dialect,
        payload: serde_json::Value,
        chat_session: &str,
    ) -> Result<framer_core::port::ProxyResponse, String> {
        proxy::forward(
            &self.transport,
            &self.ai_token,
            session,
            browser,
            account,
            project,
            dialect,
            payload,
            chat_session,
        )
        .await
    }

}

fn parse_dialect(d: &str) -> Result<proxy::Dialect, String> {
    match d {
        "openai-responses" => Ok(proxy::Dialect::OpenAiResponses),
        "openrouter-chat" => Ok(proxy::Dialect::OpenRouterChat),
        "anthropic-messages" => Ok(proxy::Dialect::AnthropicMessages),
        "openrouter-messages" => Ok(proxy::Dialect::OpenRouterMessages),
        other => Err(format!("unknown dialect: {other}")),
    }
}

#[async_trait]
impl FramerClient for HttpFramerClient {
    async fn request_magic_link(&self, email: &str, castle: &CastleToken) -> Result<(), String> {
        login::request_magic_link(&self.transport, email, castle).await
    }

    async fn complete_magic_link(
        &self,
        email_token: &str,
        castle: &CastleToken,
    ) -> Result<Session, String> {
        login::complete_magic_link(&self.transport, email_token, castle).await
    }

    async fn me(&self, session: &Session) -> Result<serde_json::Value, String> {
        credits::me(&self.transport, session).await
    }

    async fn run_inference(
        &self,
        session: &Session,
        browser: &dyn BrowserBridge,
        account: &AccountId,
        project: &framer_core::ids::ProjectId,
        req: &InferenceRequest,
    ) -> Result<InferenceResult, String> {
        inference::run(&self.transport, &self.ai_token, session, browser, account, project, req).await
    }

    async fn usage(&self, session: &Session, team: &TeamId) -> Result<CreditSnapshot, String> {
        credits::usage(&self.transport, session, team).await
    }

    async fn proxy(
        &self,
        session: &Session,
        browser: &dyn BrowserBridge,
        account: &AccountId,
        project: &framer_core::ids::ProjectId,
        dialect: &str,
        payload: serde_json::Value,
        chat_session: &str,
    ) -> Result<framer_core::port::ProxyResponse, String> {
        self.proxy_raw(session, browser, account, project, parse_dialect(dialect)?, payload, chat_session)
            .await
    }

    async fn create_workspace(
        &self,
        session: &Session,
        spec: &WorkspaceSpec,
    ) -> Result<TeamId, String> {
        workspace::create(&self.transport, session, spec).await
    }

    async fn freechat_long(
        &self,
        session: &Session,
        browser: &dyn BrowserBridge,
        account: &AccountId,
        project: &framer_core::ids::ProjectId,
        model: &str,
        system: Option<String>,
        messages: Vec<serde_json::Value>,
        cfg: &framer_core::port::ChainConfig,
        chat_session: &str,
        dialect: framer_core::port::ChainDialect,
    ) -> Result<framer_core::port::ChainResult, String> {
        freechat::complete_long(
            &self.transport,
            &self.ai_token,
            session,
            browser,
            account,
            project,
            model,
            system,
            messages,
            cfg,
            chat_session,
            dialect,
        )
        .await
    }
}

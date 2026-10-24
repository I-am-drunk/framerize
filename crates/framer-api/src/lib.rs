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
pub mod classify;
mod castle;
mod credits;
mod error_body;
pub mod freechat;
pub mod inference;
mod login;
mod models;
pub mod nativechain;

#[cfg(feature = "experimental-native")]
pub mod experimental_native;
pub mod proxy;
pub mod provision;
pub mod routes;
mod transport;
mod workspace;

#[cfg(test)]
mod cooldown_tests;
#[cfg(test)]
mod chain_cancellation_tests;
#[cfg(test)]
mod project_collection_tests;

use async_trait::async_trait;
use framer_core::ids::{AccountId, TeamId};
use framer_core::port::{
    BrowserBridge, CastleToken, FramerClient, Session, WorkspaceSpec,
};
use framer_core::pipeline::{InferenceRequest, InferenceResult};
use framer_core::workspace::CreditSnapshot;
use transport::Transport;

/// Live implementation of the core `FramerClient` port.
pub struct HttpFramerClient {
    transport: Transport,
    /// Cached AI JWT; 600 s TTL upstream, refreshed 30 s early (mirrors the app).
    ai_token: std::sync::Mutex<Vec<(AccountId, auth::AiToken)>>,
}

impl HttpFramerClient {
    pub fn new() -> Self {
        Self {
            transport: Transport::new(),
            ai_token: std::sync::Mutex::new(Vec::new()),
        }
    }

    /// Clear legacy account-keyed JWTs after all traffic holding the shared
    /// account lease has drained, before publishing a changed login/session.
    /// The session bridge separately invalidates the cookie-keyed cache.
    #[cfg(feature = "experimental-native")]
    pub fn invalidate_account_authorization(&self, account: &AccountId) {
        self.ai_token.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
            .retain(|(id, _)| id != account);
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
            "message",
            payload,
            chat_session,
            0,
        )
        .await
    }

}

/// Native entry points require an explicit account scope and per-attempt
/// authorization. The existing FramerClient implementation remains browser based.
#[cfg(feature = "experimental-native")]
impl HttpFramerClient {
    pub async fn proxy_raw_native(
        &self,
        native: &experimental_native::application::NativeInvocation<'_>,
        project: &framer_core::ids::ProjectId,
        dialect: proxy::Dialect,
        payload: serde_json::Value,
        chat_session: &str,
    ) -> Result<experimental_native::downstream_owner::Pending,
                experimental_native::application::ApplicationError> {
        native.start(project, dialect, payload, chat_session, 0).await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn freechat_long_native(
        &self,
        native: &experimental_native::application::NativeInvocation<'_>,
        project: &framer_core::ids::ProjectId,
        anthro_payload: serde_json::Value,
        cfg: &framer_core::port::ChainConfig,
        chat_session: &str,
        events: Option<&(dyn Fn(framer_core::convert::ChainEvent) + Send + Sync)>,
    ) -> Result<experimental_native::application::NativeChain<framer_core::convert::AssembledResponse>,
                experimental_native::application::ApplicationError> {
        freechat::complete_long_native(native, project, anthro_payload, cfg, chat_session, events).await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn native_long_native(
        &self,
        native: &experimental_native::application::NativeInvocation<'_>,
        project: &framer_core::ids::ProjectId,
        responses_payload: serde_json::Value,
        cfg: &framer_core::port::ChainConfig,
        chat_session: &str,
        events: Option<&(dyn Fn(framer_core::convert::ChainEvent) + Send + Sync)>,
    ) -> Result<experimental_native::application::NativeChain<framer_core::convert::AssembledResponses>,
                experimental_native::application::ApplicationError> {
        nativechain::complete_long_native(native, project, responses_payload, cfg, chat_session, events).await
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

    async fn claim_workspace_grant(&self, session: &Session) -> Result<bool, String> {
        provision::claim_workspace_grant(&self.transport, session).await
    }

    async fn create_project(
        &self,
        session: &Session,
        team: &TeamId,
    ) -> Result<framer_core::ids::ProjectId, String> {
        // Look up the workspace's space id (creation needs it, not the team id).
        let me = credits::me(&self.transport, session).await?;
        let space_id = me["spaces"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .iter()
            .find(|sp| {
                sp["scope"].as_str() == Some("organization") && sp["ownerId"].as_str() == Some(team.0.as_str())
            })
            .and_then(|sp| sp["id"].as_str().map(|s| s.to_string()))
            .ok_or_else(|| format!("no space found for team {}", team.0))?;
        provision::ensure_project(&self.transport, session, team, &space_id).await
    }

    async fn team_projects(
        &self,
        session: &Session,
        team: &TeamId,
    ) -> Result<Vec<framer_core::ids::ProjectId>, String> {
        // Cookie auth: this is a dashboard route, not a /web Bearer one
        // (reference/analysis/framer-ai-endpoints.md:97).
        let v = self
            .transport
            .get(
                &format!(
                    "/web/v2/dashboard/teams/{}?collectionId=recent&orderBy=lastOpenedAt&limit=300",
                    team.0
                ),
                crate::transport::Auth::Cookie(session),
            )
            .await?
            .json::<serde_json::Value>()
            .await
            .map_err(|e| e.to_string())?;

        // The payload nests projects under a few possible keys across versions;
        // collect any object carrying an id that looks like a project id rather
        // than hardcoding one shape.
        let mut out = Vec::new();
        collect_project_ids(&v, team, &mut out);
        Ok(out)
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
        anthro_payload: serde_json::Value,
        cfg: &framer_core::port::ChainConfig,
        chat_session: &str,
        events: Option<&(dyn Fn(framer_core::convert::ChainEvent) + Send + Sync)>,
    ) -> Result<framer_core::convert::AssembledResponse, String> {
        freechat::complete_long(
            &self.transport,
            &self.ai_token,
            session,
            browser,
            account,
            project,
            anthro_payload,
            cfg,
            chat_session,
            events,
        )
        .await
    }

    async fn native_long(
        &self,
        session: &Session,
        browser: &dyn BrowserBridge,
        account: &AccountId,
        project: &framer_core::ids::ProjectId,
        responses_payload: serde_json::Value,
        cfg: &framer_core::port::ChainConfig,
        chat_session: &str,
        events: Option<&(dyn Fn(framer_core::convert::ChainEvent) + Send + Sync)>,
    ) -> Result<framer_core::convert::AssembledResponses, String> {
        nativechain::complete_long(
            &self.transport,
            &self.ai_token,
            session,
            browser,
            account,
            project,
            responses_payload,
            cfg,
            chat_session,
            events,
        )
        .await
    }
}


/// Walk a dashboard payload and collect project ids.
///
/// Framer's dashboard response has changed shape across versions (`projects`,
/// `items`, `collection.items`), so this looks for the INVARIANT instead: a
/// 20-char base62 `id` on an object that also looks like a project. Being
/// shape-agnostic here means a dashboard tweak degrades to "found nothing"
/// rather than a parse error. Absent team metadata relies on the team-bound
/// route; explicit contradictions reject the object and all its descendants.
fn collect_project_ids(
    v: &serde_json::Value, team: &TeamId, out: &mut Vec<framer_core::ids::ProjectId>,
) {
    match v {
        serde_json::Value::Object(map) => {
            if map.get("teamId").is_some_and(|id| id.as_str() != Some(team.0.as_str())) {
                return;
            }
            if let Some(id) = map.get("id").and_then(|i| i.as_str()) {
                let looks_like_project = id.len() == 20
                    && id.chars().all(|c| c.is_ascii_alphanumeric())
                    && (map.contains_key("name") || map.contains_key("lastOpenedAt"));
                if looks_like_project {
                    let pid = framer_core::ids::ProjectId(id.to_string());
                    if !out.contains(&pid) {
                        out.push(pid);
                    }
                }
            }
            for child in map.values() {
                collect_project_ids(child, team, out);
            }
        }
        serde_json::Value::Array(items) => {
            for child in items {
                collect_project_ids(child, team, out);
            }
        }
        _ => {}
    }
}

#[cfg(feature = "native-dispatch")]
pub mod native_dispatch;

#[cfg(all(test, feature = "native-dispatch"))]
extern crate self as framer_api;
#[cfg(all(test, feature = "native-dispatch"))]
#[path = "observed_factory_fixture.rs"]
mod observed_factory;

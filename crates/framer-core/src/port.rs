//! Ports: the seams between stable core and the swappable adapters.
//! `FramerClient` is implemented by `framer-api` (the fragile layer);
//! `SessionStore` + `StateStore` by `framer-store`. `BrowserBridge` is supplied by
//! the per-account isolated WebKit session — Castle tokens cannot be minted in Rust.

use crate::account::ManagedAccount;
use crate::ids::{AccountId, FramerUserId, ProjectId, TeamId};
use crate::pipeline::{InferenceRequest, InferenceResult};
use crate::workspace::CreditSnapshot;
use async_trait::async_trait;
use bytes::Bytes;
use futures::Stream;
use std::pin::Pin;

/// An in-memory, resolved session for one account: the single httpOnly `session`
/// cookie on .framer.com. Never persisted through `StateStore`, never logged.
#[derive(Clone)]
pub struct Session {
    pub active_user: FramerUserId,
    /// Raw `Cookie:` header value — just `session=<value>`.
    pub cookie_header: String,
}

impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Session")
            .field("active_user", &self.active_user)
            .field("cookie_header", &"<redacted>")
            .finish()
    }
}

/// A Castle request token minted inside the account's own webview. Single-shot:
/// mint one per upstream request, never cache.
pub struct CastleToken(pub String);

#[derive(Debug, Clone)]
pub struct WorkspaceSpec {
    pub name: String,
}

/// Bridge to the account's isolated browser engine. Accounts are devices: Castle
/// binds trust to the engine fingerprint, so each account's tokens come from ITS
/// webview only.
#[async_trait]
pub trait BrowserBridge: Send + Sync {
    async fn castle_token(&self, account: &AccountId) -> Result<CastleToken, String>;

    /// In-page `POST /auth/v4/signin` from the account's own browser. Auth
    /// requests MUST originate from the same browser context that minted the
    /// Castle token: api.framer.com validates request/environment consistency
    /// (Origin, User-Agent, TLS/fingerprint). The same body via a bare HTTP
    /// client gets a bare 400 — observed live 2026-10-01.
    async fn auth_begin(&self, account: &AccountId, email: &str) -> Result<(), String>;

    /// In-page `POST /auth/v4/continue`: exchanges the emailed token, then
    /// reads the httpOnly `session` cookie out of the browser's cookie jar via
    /// CDP and fetches /web/users/me in-page. Returns (cookie_header, me_json).
    async fn auth_complete(
        &self,
        account: &AccountId,
        email_token: &str,
    ) -> Result<(String, serde_json::Value), String>;

    /// Per-account egress proxy (Castle binds tokens to egress IP — verified:
    /// clean token + flagged IP = 403 on /ai/*). Default no-op for bridges
    /// without proxy support; CdpBridge applies it on next launch.
    async fn set_proxy(&self, _account: &AccountId, _proxy: Option<String>) -> Result<(), String> {
        Ok(())
    }

    /// Ensure the account's browser cookie jar holds the session cookie.
    /// Imported accounts start with an empty jar (the session lives in the
    /// session store); in-page fetches authenticate from the jar, so the
    /// bridge seeds it over CDP. Idempotent — called before every AI call.
    async fn seed_session(&self, _account: &AccountId, _cookie_header: &str) -> Result<(), String> {
        Ok(())
    }

    /// In-page authenticated AI POST to api.framer.com from the account's own
    /// browser: mints a fresh Castle token and sends in the same environment.
    /// THE rule (verified quadrant matrix 2026-10-02): a Castle token is only
    /// accepted when token+transport share one environment — same TLS stack,
    /// same UA, same Origin. The identical envelope via a bare HTTP client
    /// gets risk-refused even with a perfect token.
    /// Returns (status, x-framer-ai-usage header, body); SSE bodies are
    /// consumed to completion in-page and returned whole.
    async fn ai_post(
        &self,
        _account: &AccountId,
        _path: &str,
        _bearer: &str,
        _body: &serde_json::Value,
    ) -> Result<(u16, Option<String>, String), String> {
        Err("bridge has no in-page AI transport".into())
    }

    /// Fresh-device escape hatch: drop the account's browser, wipe its profile
    /// (cookies/storage/fingerprint surface). The daemon's stored session is
    /// unaffected — the browser is only the Castle minting device. Pair with a
    /// new castle_device_id on the account record. This is the remediation for
    /// a device id that has accumulated Castle refusals — never fast-retry.
    async fn rotate_identity(&self, _account: &AccountId) -> Result<(), String> {
        Ok(())
    }
}

/// Raw upstream response for compat pass-through: status, content type, and the
/// body as a byte stream (SSE responses stream through untouched).
pub struct ProxyResponse {
    pub status: u16,
    pub content_type: String,
    pub stream: Pin<Box<dyn Stream<Item = Result<Bytes, String>> + Send>>,
}

/// Which unmetered OpenRouter passthrough the chain runs on. Both skip the
/// meter when stream:false; pick by which wire format the CLIENT speaks so no
/// dialect conversion is needed (Anthropic clients chain natively).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChainDialect {
    /// /ai/v3/chat/ — OpenAI chat/completions wire format.
    OpenAiChat,
    /// /ai/v3/messages/ — Anthropic Messages wire format (via OpenRouter).
    AnthropicMessages,
}

/// Config for free continuation-chained chat (unmetered stream:false chunks —
/// see framer-api::freechat for the live evidence).
#[derive(Debug, Clone)]
pub struct ChainConfig {
    /// Tokens per chunk (~6-10 s each on gpt-4.1; under the ~21.5 s idle kill).
    pub chunk_tokens: u32,
    /// Runaway protection.
    pub max_chunks: u32,
    pub continue_prompt: String,
    /// Client-requested total cap (anthropic max_tokens etc.); each chunk's
    /// upstream max_tokens is min(chunk_tokens, remaining). None = chain until
    /// finish_reason == "stop".
    pub max_total_tokens: Option<u32>,
    /// Never-fail ladder's last rung: when a chunk keeps dying to the upstream
    /// idle-kill (e.g. provider queue stall > window), finish THAT chunk as a
    /// metered stream (SSE flows bytes, cannot idle-die) and resume free
    /// non-streaming for the next chunk. Credits touched only on failure paths.
    /// Default ON: automations must not fail; free is the optimization, not the
    /// constraint.
    pub metered_fallback: bool,
}


impl Default for ChainConfig {
    fn default() -> Self {
        Self {
            chunk_tokens: 600,
            max_chunks: 16,
            continue_prompt: "Continue exactly where you stopped. No preamble, no recap.".into(),
            max_total_tokens: None,
            metered_fallback: true,
        }
    }
}

// Chain results are framer_core::convert::AssembledResponse (block-faithful).

/// The one port that talks to Framer. Everything undocumented/fragile is behind this.
#[async_trait]
pub trait FramerClient: Send + Sync {
    /// POST /auth/v4/signin — sends the magic-link email. Needs a Castle token
    /// from the account's (not yet authed) webview.
    async fn request_magic_link(&self, email: &str, castle: &CastleToken) -> Result<(), String>;

    /// POST /auth/v4/continue — exchange the emailed #token (+ a second Castle
    /// token) for the `session` cookie. Links expire after 1 hour, single-use.
    async fn complete_magic_link(
        &self,
        email_token: &str,
        castle: &CastleToken,
    ) -> Result<Session, String>;

    /// GET /web/users/me — account profile + workspace list (spaces with ownerId
    /// = credit-bearing team).
    async fn me(&self, session: &Session) -> Result<serde_json::Value, String>;

    /// Run one agent inference turn, scoped to `project` (which determines the
    /// workspace whose credits pay for it). `Err("credits-exhausted")` on HTTP
    /// 402 — the pipeline rotates on it. `Err("risk-refused")` on 403.
    async fn run_inference(
        &self,
        session: &Session,
        browser: &dyn BrowserBridge,
        account: &AccountId,
        project: &ProjectId,
        req: &InferenceRequest,
    ) -> Result<InferenceResult, String>;

    /// Compat passthrough: forward one native payload through the gateway dialect
    /// routes. `dialect` is a path selector (see framer-api::proxy::Dialect);
    /// keeping it a string here avoids leaking the fragile enum into core.
    #[allow(clippy::too_many_arguments)] // boundary; all args required
    async fn proxy(
        &self,
        session: &Session,
        browser: &dyn BrowserBridge,
        account: &AccountId,
        project: &ProjectId,
        dialect: &str,
        payload: serde_json::Value,
        chat_session: &str,
    ) -> Result<ProxyResponse, String>;

    /// Free unmetered long-form chat: chain windowed non-streaming calls on an
    /// OpenRouter passthrough route (meter hooks SSE only — verified for BOTH
    /// passthroughs 2026-10-01). `model` is a PREFIXED OpenRouter catalog id.
    /// `dialect` picks the upstream wire format; `system` is only used by the
    /// Anthropic dialect (its API keeps system outside messages).
    /// Lossless free chain: takes the client request already converted to
    /// Anthropic Messages shape (convert::to_anthropic_request), returns the
    /// block-faithful assembled response.
    #[allow(clippy::too_many_arguments)]
    async fn freechat_long(
        &self,
        session: &Session,
        browser: &dyn BrowserBridge,
        account: &AccountId,
        project: &ProjectId,
        anthro_payload: serde_json::Value,
        cfg: &ChainConfig,
        chat_session: &str,
    ) -> Result<crate::convert::AssembledResponse, String>;

    /// GET /web/teams/{teamId}/ai-credits.
    async fn usage(&self, session: &Session, team: &TeamId) -> Result<CreditSnapshot, String>;

    /// TODO(unmapped): the create-team endpoint is not yet captured
    /// (TeamCreationModal receives `api.createTeam` from its host). Until captured,
    /// workspace rotation uses pre-existing teams from `me()`.
    async fn create_workspace(
        &self,
        session: &Session,
        spec: &WorkspaceSpec,
    ) -> Result<TeamId, String>;
}

/// Secret storage (keychain). Separate from `StateStore` by design (ADR-001).
#[async_trait]
pub trait SessionStore: Send + Sync {
    async fn load(&self, user: &FramerUserId) -> Result<Option<Session>, String>;
    async fn save(&self, session: &Session) -> Result<(), String>;
    async fn delete(&self, user: &FramerUserId) -> Result<(), String>;
}

/// Non-secret state (SQLite). Never stores cookies/tokens.
#[async_trait]
pub trait StateStore: Send + Sync {
    async fn get(&self, id: AccountId) -> Result<Option<ManagedAccount>, String>;
    async fn list(&self) -> Result<Vec<ManagedAccount>, String>;
    async fn upsert(&self, account: &ManagedAccount) -> Result<(), String>;
}

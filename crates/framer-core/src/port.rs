//! Ports: the seams between stable core and the swappable adapters.
//! `FramerClient` is implemented by `framer-api` (the fragile layer);
//! `SessionStore` + `StateStore` by `framer-store`. `BrowserBridge` is supplied by
//! the per-account isolated WebKit session — Castle tokens cannot be minted in Rust.

use crate::account::ManagedAccount;
use crate::ids::{AccountId, FramerUserId, ProjectId, TeamId};
use crate::pipeline::{InferenceRequest, InferenceResult};
use crate::workspace::CreditSnapshot;
use async_trait::async_trait;
pub use framer_protocol::responses_sink::{OutputError, ResponseError, ResponseResult,
    ResponsesSink, ValidatedChunk, emit_assembled};
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

/// Exact target and private login for one credential preparation.
pub struct BrowserAuthorizationRequest<'a> {
    pub project: &'a ProjectId,
    pub path: &'a str,
    pub cookie_header: &'a str,
}

/// Private material from the account page and its current cookie jar.
/// Credentials deliberately have no Debug or serialization implementation.
pub struct BrowserAuthorization {
    pub project: ProjectId,
    pub path: String,
    pub page_url: String,
    pub castle: CastleToken,
    pub cookie_header: String,
}

#[derive(Debug, Clone)]
pub struct WorkspaceSpec {
    pub name: String,
}

/// Bridge to the account's isolated browser engine. Accounts are devices: Castle
/// binds trust to the engine fingerprint, so each account's tokens come from ITS
/// webview only.
#[async_trait]
pub trait BrowserBridge: Send + Sync {
    /// Validate local account history without browsers, credentials or upstream work.
    /// Adapters without a separate history process have no extra dependency.
    async fn check_account_history(&self, _accounts: &[AccountId]) -> Result<(), String> {
        Ok(())
    }

    /// Whether admission can inspect incarnation-bound browser lifecycle state.
    /// Bridges without this facility retain their existing admission behavior.
    fn supports_browser_lifecycle(&self) -> bool { false }

    /// Inspect residency without starting a missing browser or host.
    async fn browser_status(&self, _account: &AccountId)
        -> Result<crate::browser_lifecycle::BrowserAccountStatus, String> {
        Err("browser lifecycle inspection is unsupported".into())
    }

    /// Non-destructive close bound to the previously observed incarnation.
    async fn close_idle_browser(&self, _account: &AccountId,
        _expected: &crate::browser_lifecycle::BrowserAccountStatus)
        -> Result<crate::browser_lifecycle::IdleCloseResult, String> {
        Err("idle browser close is unsupported".into())
    }

    /// Stop this bridge's owned resources after application work has drained.
    /// Attached user browsers must remain running.
    async fn shutdown(&self) -> Result<(), String> {
        Err("browser shutdown is unsupported".into())
    }

    async fn castle_token(&self, account: &AccountId) -> Result<CastleToken, String>;

    /// Prepare on the requested project's actual page, without a legacy mint fallback.
    async fn prepare_inference_authorization(
        &self, _account: &AccountId, _request: BrowserAuthorizationRequest<'_>,
    ) -> Result<BrowserAuthorization, String> {
        Err("bridge has no contextual authorization".into())
    }

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
    /// without proxy support. CdpBridge refuses changes after startup; callers
    /// should use nonlaunching registration after acknowledged closure.
    async fn set_proxy(&self, _account: &AccountId, _proxy: Option<String>) -> Result<(), String> {
        Ok(())
    }

    /// Register saved egress without starting or closing a browser or host.
    /// A changed route requires confirmed absence; an unchanged route is a no-op.
    /// Unsupported bridges fail explicitly instead of silently losing egress.
    async fn register_account_proxy(&self, _account: &AccountId, _proxy: Option<String>)
        -> Result<(), String> {
        Err("nonlaunching account proxy registration is unsupported".into())
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

    /// Cancellation stops further attempts, not the observation of a sent call.
    /// Bridges with internal retries must check between those attempts too.
    async fn ai_post_cancellable(
        &self, account: &AccountId, path: &str, bearer: &str,
        body: &serde_json::Value,
        cancelled: Option<&(dyn Fn() -> bool + Send + Sync)>,
    ) -> Result<(u16, Option<String>, String), String> {
        if cancelled.is_some_and(|check| check()) {
            return Err("client cancelled".into());
        }
        self.ai_post(account, path, bearer, body).await
    }

    /// Queue wait, in ms, for the most recent `ai_post` on this account: time
    /// spent waiting for a free browser page rather than on the upstream.
    /// Bridges without a page pool report 0.
    async fn last_queue_wait_ms(&self, _account: &AccountId) -> u64 {
        0
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
    /// Time this request spent waiting for a free browser page, i.e. OUR queue,
    /// not upstream latency. The two are indistinguishable in wall-clock, and
    /// conflating them makes the chunk sizer shrink chunks under concurrency —
    /// which multiplies round trips and deepens the queue it is reacting to.
    /// Subtract this before estimating tok/s. Zero when a page was free.
    pub queue_wait_ms: u64,
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
#[derive(Clone)]
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
    /// Allow metered streaming. Adaptive thinking streams from the first
    /// attempt because its full response may exceed the buffered window.
    /// Other requests retain their existing fallback policy. Default OFF:
    /// credit spend requires FRAMERIZE_ALLOW_METERED=1.
    pub metered_fallback: bool,
    /// Fired on the FIRST 429 a chain sees (and each after): lets the server
    /// mark the workspace saturated for rotation steering immediately instead
    /// of waiting for chain exhaustion (2026-10-05: 15 429s in 4 min with zero
    /// preemptive marks).
    pub on_429: Option<std::sync::Arc<dyn Fn() + Send + Sync>>,
    /// Stop before another upstream attempt when the response consumer leaves.
    /// A call already sent keeps running until its own completion is observed.
    pub cancelled: Option<std::sync::Arc<dyn Fn() -> bool + Send + Sync>>,
}

impl ChainConfig {
    pub fn limits(&self) -> framer_protocol::forward::ChainLimits {
        framer_protocol::forward::ChainLimits {
            chunk_tokens: self.chunk_tokens,
            max_chunks: self.max_chunks,
            continue_prompt: self.continue_prompt.clone(),
            max_total_tokens: self.max_total_tokens,
            metered_fallback: self.metered_fallback,
        }
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.as_ref().is_some_and(|cancelled| cancelled())
    }
    pub fn check_cancelled(&self) -> Result<(), String> {
        if self.is_cancelled() { Err("client cancelled".into()) } else { Ok(()) }
    }
}

impl framer_protocol::forward::ChainControl for ChainConfig {
    fn is_cancelled(&self) -> bool { self.is_cancelled() }
    fn on_429(&self) {
        if let Some(callback) = &self.on_429 { callback(); }
    }
}

impl std::fmt::Debug for ChainConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChainConfig")
            .field("chunk_tokens", &self.chunk_tokens)
            .field("max_chunks", &self.max_chunks)
            .field("max_total_tokens", &self.max_total_tokens)
            .field("metered_fallback", &self.metered_fallback)
            .field("on_429", &self.on_429.as_ref().map(|_| "<callback>"))
            .field("cancelled", &self.cancelled.as_ref().map(|_| "<callback>"))
            .finish()
    }
}

impl Default for ChainConfig {
    fn default() -> Self {
        Self {
            chunk_tokens: 600,
            max_chunks: 16,
            continue_prompt: "Continue exactly where you stopped. No preamble, no recap.".into(),
            max_total_tokens: None,
            metered_fallback: false,
            on_429: None,
            cancelled: None,
        }
    }
}

// Chain results are framer_core::convert::AssembledResponse (block-faithful).

/// The one port that talks to Framer. Everything undocumented/fragile is behind this.
#[async_trait]
pub trait FramerClient: Send + Sync {
    /// Qualified model identifiers exposed by the compatibility endpoints.
    fn model_catalog(&self) -> &'static [&'static str] { &[] }

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
        events: Option<&(dyn Fn(crate::convert::ChainEvent) + Send + Sync)>,
    ) -> Result<crate::convert::AssembledResponse, String>;

    /// The responses-native free chain (docs/native-chains.md): takes the
    /// client request in OpenAI Responses shape, runs it on
    /// /ai/v3/responses/ verbatim, returns the assembled native object.
    #[allow(clippy::too_many_arguments)]
    async fn native_long(
        &self,
        session: &Session,
        browser: &dyn BrowserBridge,
        account: &AccountId,
        project: &ProjectId,
        responses_payload: serde_json::Value,
        cfg: &ChainConfig,
        chat_session: &str,
        events: Option<&(dyn Fn(crate::convert::ChainEvent) + Send + Sync)>,
    ) -> Result<crate::convert::AssembledResponses, String>;

    /// Awaited publication for free Responses SSE. No buffering fallback.
    #[allow(clippy::too_many_arguments)]
    async fn native_long_with_sink(
        &self, _session: &Session, _browser: &dyn BrowserBridge,
        _account: &AccountId, _project: &ProjectId, _responses_payload: serde_json::Value,
        _cfg: &ChainConfig, _chat_session: &str, _sink: &dyn ResponsesSink,
    ) -> ResponseResult {
        Err(OutputError::Unsupported.into())
    }

    /// GET /web/teams/{teamId}/ai-credits.
    async fn usage(&self, session: &Session, team: &TeamId) -> Result<CreditSnapshot, String>;

    /// Claim the account's second-workspace credit grant (first TWO workspaces
    /// each get 500, verified 2026-10-05). Creates the workspace when the
    /// account has just one. Best-effort; default impl errors (bridge without it).
    async fn claim_workspace_grant(&self, _session: &Session) -> Result<bool, String> {
        Err("bridge has no grant claiming".into())
    }

    /// Create a project in the given workspace (every AI call needs one).
    /// Default impl errors; HttpFramerClient implements via the page route.
    async fn create_project(
        &self,
        _session: &Session,
        _team: &TeamId,
    ) -> Result<crate::ids::ProjectId, String> {
        Err("bridge has no project creation".into())
    }

    /// GET /web/v2/dashboard/teams/{teamId} — the team's existing projects.
    ///
    /// Needed because an imported account arrives with no `current_project`,
    /// and every AI call needs one (`projectId` in the envelope). An account
    /// that already has a project should ADOPT it rather than sit unusable or,
    /// worse, be mistaken for dead. Returns project ids, most recent first.
    async fn team_projects(
        &self,
        _session: &Session,
        _team: &TeamId,
    ) -> Result<Vec<crate::ids::ProjectId>, String> {
        Err("bridge has no project listing".into())
    }

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
    /// Non-secret service settings. Missing keys use the startup defaults.
    async fn setting(&self, _key: &str) -> Result<Option<String>, String> {
        Ok(None)
    }
    async fn save_setting(&self, _key: &str, _value: &str) -> Result<(), String> {
        Err("persistent settings are unsupported by this state store".into())
    }

    async fn get(&self, id: AccountId) -> Result<Option<ManagedAccount>, String>;
    async fn list(&self) -> Result<Vec<ManagedAccount>, String>;
    async fn upsert(&self, account: &ManagedAccount) -> Result<(), String>;
    /// Remove an account record. Idempotent: removing an absent id succeeds.
    /// The browser profile and stored session are NOT touched here -- the
    /// caller decides whether to tear those down too.
    async fn remove(&self, id: AccountId) -> Result<(), String>;
}

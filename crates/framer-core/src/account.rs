use crate::ids::{AccountId, FramerUserId, ProjectId, TeamId};
use crate::workspace::CreditSnapshot;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

fn default_castle_device_id() -> String {
    Uuid::new_v4().to_string()
}

/// Persisted, non-secret account record. The session cookie is NOT here — it lives
/// in the keychain, referenced by `framer_user_id`. See ADR-001.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManagedAccount {
    pub id: AccountId,
    pub alias: String,
    /// The account's login email, normalized (trimmed + lowercased). This is
    /// the human-meaningful identity -- a uuid tells the operator nothing --
    /// and the DEDUPE key: the same email must never be added twice.
    #[serde(default)]
    pub email: Option<String>,
    pub framer_user_id: FramerUserId,
    /// Stable Castle device identity (`__cuid`), generated once at account
    /// creation and seeded into the account's webview before the first token
    /// mint (see framer-api::castle::SEED_DEVICE_SNIPPET). Castle embeds this
    /// in every request token; regenerating it makes the account look like a
    /// brand-new device each run and tanks its risk score. Never rotate except
    /// when the account gets env-sticky 403s AND a fresh identity is wanted
    /// (that is exactly the "rotate session" remediation).
    #[serde(default = "default_castle_device_id")]
    pub castle_device_id: String,
    /// Per-account egress proxy (e.g. `http://user:pass@residential.example:8080`
    /// or `socks5://…`). Castle binds request tokens to the minting
    /// environment INCLUDING its egress IP (verified 2026-10-01: a clean-
    /// browser-minted token fired from a flagged IP gets 403 on /ai/*).
    /// "Accounts are devices" extends to "accounts are networks": each
    /// account's browser runs with its own proxy so a daemon on any host can
    /// operate accounts whose login/inference only pass from clean egress.
    #[serde(default)]
    pub proxy: Option<String>,
    /// The workspace (team) this account is currently pointed at. Rotated on 402.
    pub current_team: Option<TeamId>,
    /// The project inside `current_team` that inference calls are scoped to.
    pub current_project: Option<ProjectId>,
    pub state: AccountState,
    pub credit: Option<CreditSnapshot>,
    pub consecutive_failures: u32,
    pub cooldown_step: u32,
    /// Unix ms until which this account must not be used. Castle refusals are
    /// transient but STICKY-ish: hammering a refused device deepens the
    /// refusal, so a refused account has to be taken out of rotation for a
    /// while rather than retried. `None` means no active cooldown.
    #[serde(default)]
    pub cooling_until_ms: Option<u64>,
}

/// The account lifecycle. Transitions are centralized here so they live in one place.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AccountState {
    Disconnected,
    Authenticating,
    Ready,
    /// Risk-engine (Castle) refusal — 403s. Temporary by definition; never fatal.
    CoolingDown,
    Exhausted,
    Quarantined,
}

/// Live health, kept separate from `AccountState` and merged only for display.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionHealth {
    Healthy,
    RefreshRequired,
    Expired,
    Failed,
}

/// Canonical form for comparison and storage: trimmed, lowercased. Gmail-style
/// dot/plus folding is deliberately NOT applied -- Framer treats those as
/// distinct logins, so folding them would reject a legitimately separate
/// account.
pub fn normalize_email(raw: &str) -> String {
    raw.trim().to_lowercase()
}

impl ManagedAccount {
    /// Can this account actually serve a request right now?
    ///
    /// `Ready` is necessary but NOT sufficient: every AI call needs a
    /// `current_project` (the payload's `projectId`) and a `current_team`
    /// (the credit + concurrency scope). A newly added account sits Ready with
    /// neither until provisioning finishes, and rotation handing it work means
    /// every request routed there dies instantly with "account has no current
    /// project" -- 1/N of all traffic for N accounts. Observed live 2026-10-04
    /// when a third account was added mid-session.
    pub fn is_eligible(&self) -> bool {
        matches!(self.state, AccountState::Ready)
            && self.current_project.is_some()
            && self.current_team.is_some()
    }

    /// Eligible AND not inside an active cooldown window.
    ///
    /// Separate from `is_eligible` because cooldown needs the clock, and
    /// `is_eligible` is used in contexts that should stay pure.
    pub fn is_available_at(&self, now_ms: u64) -> bool {
        self.is_eligible() && self.cooling_until_ms.is_none_or(|until| now_ms >= until)
    }

    /// Exponential backoff for repeated Castle refusals: 2s, 4s, 8s ... capped
    /// at 5 minutes. Deliberately bounded -- a refusal is never fatal
    /// (AGENTS.md rule 3), so the account must always come back.
    pub fn cooldown_backoff_ms(step: u32) -> u64 {
        const BASE_MS: u64 = 2_000;
        const CAP_MS: u64 = 300_000;
        BASE_MS.saturating_mul(1u64 << step.min(8)).min(CAP_MS)
    }

    /// Failure count for ranking purposes: zero once a cooldown window has
    /// elapsed, because serving the window already was the penalty.
    pub fn effective_failures(&self, now_ms: u64) -> u32 {
        match self.cooling_until_ms {
            Some(until) if now_ms >= until => 0,
            _ => self.consecutive_failures,
        }
    }

    /// Record a Castle refusal: cool down, never quarantine.
    pub fn note_risk_refused(&mut self, now_ms: u64) {
        self.consecutive_failures = self.consecutive_failures.saturating_add(1);
        self.cooldown_step = self.cooldown_step.saturating_add(1);
        self.cooling_until_ms = Some(now_ms + Self::cooldown_backoff_ms(self.cooldown_step - 1));
    }

    /// Record a healthy response: clear failure state and any cooldown.
    pub fn note_success(&mut self) {
        self.consecutive_failures = 0;
        self.cooldown_step = 0;
        self.cooling_until_ms = None;
    }
}

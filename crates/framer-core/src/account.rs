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

impl ManagedAccount {
    pub fn is_eligible(&self) -> bool {
        matches!(self.state, AccountState::Ready)
    }
}

use crate::ids::{AccountId, FramerUserId, ProjectId, TeamId};
use crate::workspace::CreditSnapshot;
use serde::{Deserialize, Serialize};

/// Persisted, non-secret account record. The session cookie is NOT here — it lives
/// in the keychain, referenced by `framer_user_id`. See ADR-001.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManagedAccount {
    pub id: AccountId,
    pub alias: String,
    pub framer_user_id: FramerUserId,
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

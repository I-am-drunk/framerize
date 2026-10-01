use crate::ids::{AccountId, TeamId};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Workspace {
    pub team_id: TeamId,
    pub account_id: AccountId,
    pub credit: CreditSnapshot,
}

/// A point-in-time read of a team's credit ledger
/// (GET /web/teams/{teamId}/ai-credits, or the X-Framer-AI-Usage response header).
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct CreditSnapshot {
    pub available: f64,
    /// Number of non-expired funds in the ledger.
    pub funds: u32,
}

impl CreditSnapshot {
    /// Pre-flight guess at exhaustion. The authoritative signal is HTTP 402 from a
    /// run (see pipeline), so this is advisory.
    pub fn looks_exhausted(&self) -> bool {
        self.available <= 0.0
    }
}

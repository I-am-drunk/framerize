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
    /// Total grants across all funds. A free workspace gets one 500-credit
    /// `free-intro` grant per credited project, and allowances STACK across
    /// projects (reference/analysis/framer-credits.md:11-13), so this is the
    /// count of 500-credit grants the account actually holds.
    #[serde(default)]
    pub grants: u32,
    /// Sum of `allocatedCredits` across grants: lifetime entitlement, as
    /// opposed to `available` which is what is left after consumption.
    #[serde(default)]
    pub allocated: f64,
}

impl CreditSnapshot {
    /// Pre-flight guess at exhaustion. The authoritative signal is HTTP 402 from a
    /// run (see pipeline), so this is advisory.
    pub fn looks_exhausted(&self) -> bool {
        self.available <= 0.0
    }

    /// One free grant is 500 credits.
    pub const GRANT_CREDITS: f64 = 500.0;

    /// How many 500-credit grants this workspace has, inferred from the
    /// entitlement when the grant list is unavailable (e.g. a snapshot restored
    /// from an older record without the field).
    pub fn grant_count(&self) -> u32 {
        if self.grants > 0 {
            return self.grants;
        }
        (self.allocated / Self::GRANT_CREDITS).round().max(0.0) as u32
    }
}

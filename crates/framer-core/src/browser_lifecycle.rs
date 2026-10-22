//! Account browser lifetime receipts, separate from saved account health.
use crate::ids::AccountId;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BrowserAccountState {
    Absent,
    Starting,
    Resident,
    Closing,
    Unconfirmed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BrowserAccountStatus {
    pub account_id: AccountId,
    pub host_generation: Option<String>,
    pub account_incarnation: u64,
    pub state: BrowserAccountState,
    pub active_commands: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IdleCloseOutcome {
    Closed,
    AlreadyAbsent,
    Busy,
    StaleGeneration,
    Unconfirmed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IdleCloseResult {
    pub outcome: IdleCloseOutcome,
    pub status: BrowserAccountStatus,
}

impl BrowserAccountStatus {
    pub fn same_identity(&self, other: &Self) -> bool {
        self.account_id == other.account_id
            && self.host_generation == other.host_generation
            && self.account_incarnation == other.account_incarnation
    }
}

impl IdleCloseResult {
    /// An error, stale receipt or ambiguous cleanup never releases capacity.
    pub fn confirms_absence(&self, expected: &BrowserAccountStatus) -> bool {
        matches!(self.outcome, IdleCloseOutcome::Closed | IdleCloseOutcome::AlreadyAbsent)
            && self.status.same_identity(expected)
            && self.status.state == BrowserAccountState::Absent
            && self.status.active_commands == 0
    }
}

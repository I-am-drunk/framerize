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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cleanup_cause: Option<String>,
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
    pub fn cleanup_explanation(&self) -> String {
        if let Some(cause) = self.cleanup_cause.as_deref().filter(|v| !v.is_empty()) {
            return crate::error::safe_failure_summary(cause);
        }
        match self.state {
            BrowserAccountState::Closing => "browser closure is still running".into(),
            BrowserAccountState::Unconfirmed =>
                "the browser owner did not retain the failure that prevented confirmed cleanup".into(),
            _ => format!("the browser reported state {:?} with {} active commands",
                self.state, self.active_commands),
        }
    }
    pub fn same_identity(&self, other: &Self) -> bool {
        self.account_id == other.account_id
            && self.host_generation == other.host_generation
            && self.account_incarnation == other.account_incarnation
    }
}

impl IdleCloseResult {
    /// The host refused before closing this exact, still-live browser.
    pub fn confirms_busy(&self, expected: &BrowserAccountStatus) -> bool {
        self.outcome == IdleCloseOutcome::Busy
            && self.status.same_identity(expected)
            && matches!(self.status.state, BrowserAccountState::Resident | BrowserAccountState::Starting)
            && (self.status.active_commands > 0 || self.status.state == BrowserAccountState::Starting)
    }

    /// An error, stale receipt or ambiguous cleanup never releases capacity.
    pub fn confirms_absence(&self, expected: &BrowserAccountStatus) -> bool {
        matches!(self.outcome, IdleCloseOutcome::Closed | IdleCloseOutcome::AlreadyAbsent)
            && self.status.same_identity(expected)
            && self.status.state == BrowserAccountState::Absent
            && self.status.active_commands == 0
    }
}

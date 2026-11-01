use super::*;
use framer_core::browser_lifecycle::{
    BrowserAccountState, BrowserAccountStatus, IdleCloseOutcome, IdleCloseResult,
};
use std::time::Duration;

const STATUS_TIMEOUT: Duration = Duration::from_secs(5);
const CLOSE_TIMEOUT: Duration = Duration::from_secs(20);

fn absent_without_host(account: &AccountId) -> BrowserAccountStatus {
    BrowserAccountStatus {
        account_id: *account,
        host_generation: None,
        account_incarnation: 0,
        state: BrowserAccountState::Absent,
        active_commands: 0,
        cleanup_cause: None,
    }
}

fn validate_status(account: &AccountId, status: BrowserAccountStatus)
    -> Result<BrowserAccountStatus, String> {
    if status.account_id != *account
        || status.host_generation.as_ref().map_or(true, |generation| generation.is_empty())
        || (status.account_incarnation == 0 && status.state != BrowserAccountState::Absent) {
        return Err("sidecar returned a mismatched browser identity".into());
    }
    Ok(status)
}

impl SidecarBridge {
    pub(super) async fn inspect_browser(&self, account: &AccountId)
        -> Result<BrowserAccountStatus, String> {
        let _admission = self.admit_unscoped()?;
        tokio::time::timeout(STATUS_TIMEOUT, async {
            {
                let inner = self.inner.lock().await;
                if inner.is_none() {
                    self.check_unused_startup()?;
                    return Ok(absent_without_host(account));
                }
            }
            let value = self.call_with_policy("browser_status",
                json!({ "account": account.0.to_string() }), STATUS_TIMEOUT, false).await?;
            let status = serde_json::from_value(value)
                .map_err(|_| "sidecar returned a malformed browser status".to_owned())?;
            validate_status(account, status)
        }).await.map_err(|_| "browser status deadline exceeded".to_owned())?
    }

    pub(super) async fn close_observed_browser(&self, account: &AccountId,
        expected: &BrowserAccountStatus) -> Result<IdleCloseResult, String> {
        let _admission = self.admit_unscoped()?;
        if expected.account_id != *account {
            return Err("idle close account does not match the observed browser".into());
        }
        if expected.host_generation.is_none() {
            let current = self.inspect_browser(account).await?;
            return Ok(IdleCloseResult {
                outcome: if current.same_identity(expected) { IdleCloseOutcome::AlreadyAbsent }
                    else { IdleCloseOutcome::StaleGeneration },
                status: current,
            });
        }
        self.close_running_browser(account, expected).await
    }

    async fn close_running_browser(&self, account: &AccountId,
        expected: &BrowserAccountStatus) -> Result<IdleCloseResult, String> {
        let value = self.call_with_policy("close_idle", json!({
            "account": account.0.to_string(),
            "host_generation": expected.host_generation,
            "account_incarnation": expected.account_incarnation,
        }), CLOSE_TIMEOUT, false).await?;
        let mut result: IdleCloseResult = serde_json::from_value(value)
            .map_err(|_| "sidecar returned a malformed idle-close result".to_owned())?;
        result.status = validate_status(account, result.status)?;
        if matches!(result.outcome, IdleCloseOutcome::Closed | IdleCloseOutcome::AlreadyAbsent)
            && !result.confirms_absence(expected) {
            return Err("idle-close result did not confirm this browser's absence".into());
        }
        Ok(result)
    }
}

#[cfg(test)]
#[path = "sidecar_browser_lifecycle_tests.rs"]
mod tests;

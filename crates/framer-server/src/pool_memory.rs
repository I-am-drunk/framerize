use std::collections::HashSet;
use std::time::Duration;
use framer_core::{AccountId, account::ManagedAccount};
use framer_core::browser_lifecycle::BrowserAccountState;
use crate::{ApiError, AppState, ServingAccountPolicy};
use crate::pool_control::AcquisitionOutcome;

pub(super) struct ResidentAdmission {
    pub loaded: HashSet<AccountId>,
    pub cause: String,
}

pub(super) async fn observe(s: &AppState, accounts: &[ManagedAccount],
    excluded: &[String]) -> AcquisitionOutcome<Option<ResidentAdmission>> {
    let Some(cause) = s.memory_admission.browser_deferral()
        .filter(|_| s.browser.supports_browser_lifecycle()) else {
        return AcquisitionOutcome { result: Ok(None), cleanup_confirmed: true };
    };
    observe_loaded(s, accounts, excluded, cause).await
}

async fn observe_loaded(s: &AppState, accounts: &[ManagedAccount],
    excluded: &[String], cause: String) -> AcquisitionOutcome<Option<ResidentAdmission>> {
    let mut cleanup_confirmed = true;
    let result = async {
        let snapshot = s.serving_accounts.residents.snapshot()
            .map_err(|e| ApiError::PoolBusy(e.to_string()))?;
        let protected = s.serving_accounts.mutations.lock()
            .map_err(|_| ApiError::PoolBusy("account management is unavailable".into()))?.clone();
        let now = framer_core::rotation::now_ms();
        let candidates: Vec<_> = accounts.iter().filter(|account|
            account.is_available_at(now) && s.serving_accounts.allows(&account.id)
            && !excluded.contains(&account.id.0.to_string())
            && ServingAccountPolicy::residency_allows(&snapshot, &account.id, &protected)).collect();
        // Let the normal selector retain authentication, cooldown and ownership
        // causes when no account would be eligible even with enough memory.
        if candidates.is_empty() { return Ok(None); }
        let checks = candidates.into_iter()
            .filter(|account| snapshot.residents.iter().any(|slot| slot.account == account.id))
            .map(|account| async move {
                (account.id, tokio::time::timeout(Duration::from_secs(5),
                    s.browser.browser_status(&account.id)).await)
            });
        let mut loaded = HashSet::new();
        let mut unavailable = std::collections::BTreeSet::new();
        for (id, observed) in futures::future::join_all(checks).await {
            let status = match observed {
                Ok(Ok(status)) => status,
                failure => {
                    cleanup_confirmed = false;
                    let detail = match failure {
                        Ok(Err(error)) => framer_core::error::safe_failure_summary(&error),
                        _ => "browser status check did not reply within 5 seconds".into(),
                    };
                    return Err(ApiError::PoolBusy(format!("Memory admission could not confirm resident browser state: {detail}")));
                }
            };
            let identity_valid = match status.host_generation.as_deref() {
                Some(generation) => !generation.is_empty()
                    && (status.account_incarnation > 0 || status.state == BrowserAccountState::Absent),
                None => status.account_incarnation == 0 && status.state == BrowserAccountState::Absent,
            };
            if status.account_id != id || !identity_valid {
                cleanup_confirmed = false;
                return Err(ApiError::PoolBusy("Memory admission received a browser status with the wrong account or invalid process identity".into()));
            }
            if status.state == BrowserAccountState::Resident {
                loaded.insert(id);
            } else {
                unavailable.insert(status.cleanup_explanation());
            }
        }
        if loaded.is_empty() {
            let observed = unavailable.into_iter().collect::<Vec<_>>().join("; ");
            return Err(ApiError::ServingCapacity(if observed.is_empty() { cause }
                else { format!("{cause} Existing browser state: {observed}.") }));
        }
        Ok(Some(ResidentAdmission { loaded, cause }))
    }.await;
    AcquisitionOutcome { result, cleanup_confirmed }
}

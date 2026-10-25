//! A bounded local-capacity wait for an already-open, unbound stream.
use crate::{pick_fresh_account, pool_control::AcquisitionOutcome, ApiError, AppState};
use framer_core::{account::ManagedAccount, resident_pool::ResidentLease};
use std::time::Duration;

pub(super) const MAX_WAIT: Duration = Duration::from_secs(600);

pub(super) async fn pick(
    state: &AppState,
    excluded: &[String],
    local_cooldown: bool,
    budget: &mut Duration,
) -> AcquisitionOutcome<(ManagedAccount, ResidentLease)> {
    loop {
        let outcome = pick_fresh_account(state, excluded).await;
        let delay = match (&outcome.result, outcome.cleanup_confirmed) {
            (Err(ApiError::AccountCooling { retry_after_secs }), true)
                if local_cooldown && !budget.is_zero() => {
                    Duration::from_secs((*retry_after_secs).max(1)).min(*budget)
                        .min(Duration::from_secs(1))
                },
            _ => return outcome,
        };
        // No acquired lease or model permit survives this sleep. Selection
        // rechecks current eligibility and cleanup before any further attempt.
        let started = tokio::time::Instant::now();
        tokio::time::sleep(delay).await;
        *budget = budget.saturating_sub(started.elapsed());
    }
}

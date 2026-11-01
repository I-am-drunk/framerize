//! Bounded admission waits. No resident or model permit is held while paused.
use crate::{pick_fresh_account, pool_control::AcquisitionOutcome, ApiError, AppState};
use framer_core::{account::ManagedAccount, resident_pool::ResidentLease};
use std::time::Duration;

pub(super) const MAX_WAIT: Duration = Duration::from_secs(960);

#[derive(Clone, Copy, PartialEq)]
pub(super) enum WaitFor { None, GlobalPause, Capacity }
impl WaitFor {
    pub(super) fn after_attempt(local_cooldown: bool, cleanup_confirmed: bool) -> Self {
        if !cleanup_confirmed { Self::None }
        else if local_cooldown { Self::Capacity }
        else { Self::GlobalPause }
    }
}

pub(super) async fn pick(
    state: &AppState,
    excluded: &[String],
    wait: WaitFor,
    budget: &mut Duration,
) -> AcquisitionOutcome<(ManagedAccount, ResidentLease)> {
    pick_cancellable(state, excluded, wait, budget, || false).await
}

pub(super) async fn pick_cancellable(
    state: &AppState,
    excluded: &[String],
    wait: WaitFor,
    budget: &mut Duration,
    cancelled: impl Fn() -> bool,
) -> AcquisitionOutcome<(ManagedAccount, ResidentLease)> {
    loop {
        if cancelled() {
            return AcquisitionOutcome { result: Err(ApiError::Internal("client cancelled".into())),
                cleanup_confirmed: true };
        }
        let outcome = match state.wave.check() {
            Some(remaining) => AcquisitionOutcome {
                result: Err(ApiError::Cooling { retry_after_secs: remaining.as_secs() + 1 }),
                cleanup_confirmed: true,
            },
            None => pick_fresh_account(state, excluded).await,
        };
        let delay = match (&outcome.result, outcome.cleanup_confirmed) {
            (Err(error @ (ApiError::AccountCooling { retry_after_secs, .. }
                | ApiError::Cooling { retry_after_secs })), true)
                if !budget.is_zero() && (wait == WaitFor::Capacity
                    || (wait == WaitFor::GlobalPause && matches!(error, ApiError::Cooling { .. }))) => {
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

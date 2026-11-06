//! Bounded admission waits. Initial waits hold no resident or model permit.
//! An admitted Messages chain retains its account and permit through cooldown.
use crate::{pick_fresh_account, pool_control::AcquisitionOutcome, ApiError, AppState};
use framer_core::{account::ManagedAccount, resident_pool::ResidentLease};
use std::time::Duration;

pub(super) const MAX_WAIT: Duration = Duration::from_secs(960);

pub(super) struct MessagesAdmission {
    state: AppState,
    account: ManagedAccount,
    latest: std::sync::Mutex<ManagedAccount>,
    accounted: std::sync::atomic::AtomicBool,
    budget: Duration,
}

impl MessagesAdmission {
    pub(super) fn new(state: AppState, account: ManagedAccount, budget: Duration) -> Self {
        Self { state, latest: std::sync::Mutex::new(account.clone()), account,
            accounted: false.into(), budget }
    }
}

#[async_trait::async_trait]
impl framer_core::port::ChainAdmission for MessagesAdmission {
    async fn before_attempt(&self) -> Result<(), String> {
        let current = crate::current_account(&self.state, self.account.id).await
            .map_err(crate::chain_admission_error)?;
        if current.framer_user_id != self.account.framer_user_id {
            return Err("account identity changed during request".into());
        }
        *self.latest.lock().map_err(|_| "account admission state unavailable")? = current;
        self.accounted.store(false, std::sync::atomic::Ordering::SeqCst);
        Ok(())
    }

    async fn refused(&self, cause: &str) -> Result<(), String> {
        crate::record_account_health(&self.state, &self.account,
            &Err::<(), _>(cause.to_owned())).await;
        self.accounted.store(true, std::sync::atomic::Ordering::SeqCst);
        match crate::current_account(&self.state, self.account.id).await
            .map_err(crate::chain_admission_error) {
            Err(error) => Err(error),
            // A failed persistence operation cannot authorize another send.
            Ok(_) => Err(cause.to_owned()),
        }
    }

    async fn finished(&self, error: Option<&str>) {
        if error.is_some() && self.accounted.load(std::sync::atomic::Ordering::SeqCst) { return; }
        let account = match self.latest.lock() {
            Ok(account) => account.clone(),
            Err(_) => return,
        };
        let result = error.map_or(Ok(()), |error| Err(error.to_owned()));
        crate::record_account_health(&self.state, &account, &result).await;
    }

    fn wait_budget(&self) -> Duration { self.budget }
}

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
        let outcome = pick_fresh_account(state, excluded).await;
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
        tokio::time::sleep(delay.min(Duration::from_secs(1))).await;
        *budget = budget.saturating_sub(started.elapsed());
    }
}

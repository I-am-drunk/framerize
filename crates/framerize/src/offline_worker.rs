//! App-owned bridge from the isolated worker to account-owned preparation.
//! WIP: requires the reviewed worker revision with `freshness_origin`.
#[cfg(test)]
#[path = "offline_worker_tests.rs"]
mod tests;

use std::time::{Duration, Instant};
use std::sync::Arc;
use futures::{future::BoxFuture, FutureExt};
use framer_api::experimental_native::native_account::{
    AccountScope, PreparationControl, PreparationFailure, PreparationResult,
};
use framer_api::experimental_native::control_authorization::{
    MintedCastle, OfflineMintTarget, OfflineMinter,
};
use framer_core::port::CastleToken;
use super::offline_worker_owner::{Cleanup, Failure, Worker};

pub struct OfflineWorkerMinter {
    scope: AccountScope,
    worker: Arc<Worker>,
}

impl OfflineWorkerMinter {
    /// Inert construction. Each issuance creates a new, isolated SDK process.
    pub fn new(scope: AccountScope, worker: Arc<Worker>) -> Self {
        Self { scope, worker }
    }
}

fn cancelled(control: &PreparationControl) -> PreparationFailure {
    if Instant::now() >= control.deadline().into_std() { PreparationFailure::Deadline }
    else { PreparationFailure::Cancelled }
}

fn failure(value: Failure) -> PreparationFailure {
    match value {
        Failure::Cancelled => PreparationFailure::Cancelled,
        Failure::Deadline => PreparationFailure::Deadline,
        Failure::Input | Failure::Configuration | Failure::Identity
            | Failure::Protocol => PreparationFailure::Rejected,
        Failure::Runtime | Failure::Spawn => PreparationFailure::Unavailable,
        Failure::Io | Failure::Worker => PreparationFailure::WorkerLost,
    }
}

impl OfflineMinter for OfflineWorkerMinter {
    fn scope(&self) -> &AccountScope { &self.scope }

    fn mint<'a>(&'a self, target: &'a OfflineMintTarget,
        control: &'a PreparationControl)
        -> BoxFuture<'a, PreparationResult<MintedCastle>> {
        self.issue(target, control).boxed()
    }
}

impl OfflineWorkerMinter {
    async fn issue(&self, target: &OfflineMintTarget, control: &PreparationControl)
        -> PreparationResult<MintedCastle> {
        if control.is_cancelled() {
            return PreparationResult::finished(Err(cancelled(control)));
        }
        let target = super::offline_worker_owner::Target {
            project_id: target.project().0.clone(),
            path: target.path().to_owned(),
            page_url: target.page_url().to_owned(),
        };
        let mut run = match self.worker.begin(target, control.deadline().into_std()) {
            Ok(run) => run,
            Err(error) => return PreparationResult::finished(Err(failure(error))),
        };
        let completion = run.completion();
        let result = {
            let wait = run.result();
            tokio::pin!(wait);
            let mut poll = tokio::time::interval(Duration::from_millis(10));
            poll.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                tokio::select! {
                    biased;
                    _ = poll.tick() => {
                        if control.is_cancelled() { break Err(cancelled(control)); }
                    }
                    value = &mut wait => break value.map_err(failure),
                }
            }
        };
        // Dropping the result wait above requests cancellation. Its separate
        // completion remains owned until the actual supervisor and child join.
        let cleanup = completion.wait().await;
        if cleanup == Cleanup::Unconfirmed {
            return PreparationResult::unconfirmed();
        }
        if control.is_cancelled() {
            return PreparationResult::finished(Err(cancelled(control)));
        }
        let result = result.and_then(|minted| {
            if !matches!(cleanup, Cleanup::Joined { .. })
                || minted.freshness_origin > minted.received_at
                || minted.received_at > Instant::now() {
                return Err(PreparationFailure::Rejected);
            }
            let origin = minted.freshness_origin;
            let token = CastleToken(minted.into_token());
            Ok(MintedCastle::new(self.scope.clone(), token, origin))
        });
        PreparationResult::finished(result)
    }
}

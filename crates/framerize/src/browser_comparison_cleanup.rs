//! One consuming observer, shared by owned preparation and finalization.
use framer_api::experimental_native::native_account::{PreparationFailure, PreparationResult};
use framer_host::sidecar::observer::{CleanupObserver, CleanupOutcome, ScopeControl};
use futures::{future::{BoxFuture, Shared}, FutureExt};

#[cfg(test)]
#[path = "browser_comparison_cleanup_tests.rs"]
mod tests;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Verdict { NotStarted, Closed, Unconfirmed }

impl Verdict {
    fn observed(control: &ScopeControl, outcome: CleanupOutcome) -> Self {
        match outcome {
            // Definitive no-launch evidence remains distinct from a receipt.
            CleanupOutcome::NotStarted { .. } => Self::NotStarted,
            CleanupOutcome::Closed(receipt) if control.matches_closed_receipt(&receipt) =>
                Self::Closed,
            _ => Self::Unconfirmed,
        }
    }

    fn settled(self) -> bool { matches!(self, Self::NotStarted | Self::Closed) }
}

#[derive(Clone)]
pub(super) struct Completion {
    control: ScopeControl,
    verdict: Shared<BoxFuture<'static, Verdict>>,
}

impl Completion {
    pub(super) fn new(control: ScopeControl, observer: CleanupObserver) -> Self {
        let matching = control.clone();
        let verdict = async move {
            Verdict::observed(&matching, observer.wait().await)
        }.boxed().shared();
        Self { control, verdict }
    }

    pub(super) async fn failed_prepare<T>(&self) -> PreparationResult<T> {
        self.control.request_stop();
        // Stay inside prepare_owned until the actual observer settles. Neither
        // caller cancellation nor a command error is evidence of RPC cleanup.
        if self.verdict.clone().await.settled() {
            PreparationResult::finished(Err(PreparationFailure::Rejected))
        } else {
            PreparationResult::unconfirmed()
        }
    }

    pub(super) async fn finish(&self, failed: bool) -> bool {
        if failed { self.control.request_stop(); }
        else { self.control.seal_for_account_teardown(); }
        self.verdict.clone().await.settled()
    }
}

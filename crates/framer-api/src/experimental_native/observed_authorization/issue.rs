use super::*;
use std::{future::Future, time::Duration};

struct StopOnDrop { control: Arc<dyn SourceControl>, armed: bool, failed: AtomicBool }
impl StopOnDrop {
    fn invoke(&self, action: impl FnOnce(&dyn SourceControl)) {
        if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| action(self.control.as_ref()))).is_err() {
            self.failed.store(true, Ordering::Release);
        }
    }
    fn request_stop(&self) { self.invoke(|control| control.request_stop()); }
    fn seal(&self) { self.invoke(|control| control.seal()); }
}
impl Drop for StopOnDrop {
    fn drop(&mut self) {
        if self.armed { self.request_stop(); }
    }
}

// PreparationControl exposes a shared flag rather than a notification. Poll
// only while this owned operation is alive; cancellation never drops its work.
async fn observe_stop<F: Future + Send>(work: F, preparation: &PreparationControl,
    source: &StopOnDrop) -> F::Output {
    tokio::pin!(work);
    loop {
        if preparation.is_cancelled() {
            source.request_stop();
            return work.await;
        }
        tokio::select! {
            value = &mut work => return value,
            _ = tokio::time::sleep(Duration::from_millis(25)) => {},
        }
    }
}

impl ObservedCredentialSource {
    pub(super) async fn issue_observed(&self, scope: &AccountScope, session: &Session,
        preparation: &PreparationControl, target: Option<&CredentialTarget>)
        -> PreparationResult<PreparedCredentials> {
        if !self.expected.same_generation(scope) || scope.active_user() != &session.active_user {
            return PreparationResult::finished(Err(PreparationFailure::Rejected));
        }
        if preparation.is_cancelled() {
            return PreparationResult::finished(Err(PreparationFailure::Cancelled));
        }
        if self.used.compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire).is_err() {
            return PreparationResult::finished(Err(PreparationFailure::Rejected));
        }
        self.open_and_issue(scope, session, preparation, target).await
    }

    async fn open_and_issue(&self, scope: &AccountScope, session: &Session,
        preparation: &PreparationControl, target: Option<&CredentialTarget>)
        -> PreparationResult<PreparedCredentials> {
        let opened = match self.factory.open(scope) {
            Ok(value) => value,
            Err(OpenFailure::NotStarted) =>
                return PreparationResult::finished(Err(PreparationFailure::Unavailable)),
            Err(OpenFailure::Unconfirmed) => return PreparationResult::unconfirmed(),
        };
        let mut guard = StopOnDrop { control: opened.control.clone(), armed: true,
            failed: AtomicBool::new(false) };
        if !opened.scope.same_generation(scope) {
            guard.request_stop();
            guard.seal();
            let _ = observe_stop(opened.cleanup, preparation, &guard).await;
            return PreparationResult::unconfirmed();
        }
        let result = if preparation.is_cancelled() {
            guard.request_stop();
            PreparationResult::finished(Err(PreparationFailure::Cancelled))
        } else if let Some(target) = target {
            observe_stop(opened.source.issue_for_target(scope, session, target, preparation),
                preparation, &guard).await
        } else {
            let result = observe_stop(opened.source.issue(scope, session, preparation),
                preparation, &guard).await;
            PreparationResult { value: result.value.map(PreparedCredentials::from),
                cleanup_confirmed: result.cleanup_confirmed }
        };
        // The source can no longer produce work. Its independent observer
        // must still join browser tasks, RPCs and reply writers before release.
        guard.seal();
        let cleanup = observe_stop(opened.cleanup, preparation, &guard).await;
        let known = match cleanup {
            SourceCleanup::Closed(identity) => identity == SourceIdentity::from_scope(scope),
            SourceCleanup::NotStarted => result.value.is_err(),
            SourceCleanup::Unconfirmed => false,
        };
        if !known || !result.cleanup_confirmed || guard.failed.load(Ordering::Acquire) {
            return PreparationResult::unconfirmed();
        }
        guard.armed = false;
        if preparation.is_cancelled() {
            return PreparationResult::finished(Err(PreparationFailure::Cancelled));
        }
        result
    }
}

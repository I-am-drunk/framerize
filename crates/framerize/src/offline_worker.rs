//! App-owned bridge from the isolated worker to account-owned preparation.
//! Device admission and worker cleanup use the existing shared host authority.
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
use framer_core::{port::{BrowserBridge, CastleToken},
    native_device::{DeviceFailure, DeviceGrant}};
use super::offline_worker_owner::{Cleanup, Failure, Run, V2Worker, Worker};

#[path = "offline_v2_seed.rs"]
mod v2_seed;
pub(crate) use v2_seed::V2SeedBinding;

#[derive(Clone)]
pub(crate) enum WorkerBackend {
    V3(Arc<Worker>),
    V2(Arc<V2Worker>),
}

impl From<Arc<Worker>> for WorkerBackend {
    fn from(worker: Arc<Worker>) -> Self { Self::V3(worker) }
}

impl From<Arc<V2Worker>> for WorkerBackend {
    fn from(worker: Arc<V2Worker>) -> Self { Self::V2(worker) }
}

pub struct OfflineWorkerMinter {
    scope: AccountScope,
    worker: WorkerBackend,
    device: Option<(Arc<dyn BrowserBridge>, Arc<DeviceGrant>)>,
    sdk_seed: Option<V2SeedBinding>,
}

impl OfflineWorkerMinter {
    /// Inert construction. Each issuance creates a new, isolated SDK process.
    #[cfg(test)]
    pub fn new(scope: AccountScope, worker: Arc<Worker>) -> Self {
        Self { scope, worker: WorkerBackend::V3(worker), device: None, sdk_seed: None }
    }

    pub fn with_device(scope: AccountScope, worker: Arc<Worker>,
        bridge: Arc<dyn BrowserBridge>, grant: Arc<DeviceGrant>) -> Result<Self, Failure> {
        Self::with_backend(scope, WorkerBackend::V3(worker), bridge, grant)
    }

    pub fn with_v2_device(scope: AccountScope, worker: Arc<V2Worker>,
        bridge: Arc<dyn BrowserBridge>, grant: Arc<DeviceGrant>) -> Result<Self, Failure> {
        Self::with_backend(scope, WorkerBackend::V2(worker), bridge, grant)
    }

    pub(crate) fn with_backend(scope: AccountScope, worker: WorkerBackend,
        bridge: Arc<dyn BrowserBridge>, grant: Arc<DeviceGrant>) -> Result<Self, Failure> {
        if grant.account != scope.account() { return Err(Failure::Identity); }
        Ok(Self { scope, worker, device: Some((bridge, grant)), sdk_seed: None })
    }

    pub(crate) fn with_v2_seed(scope: AccountScope, worker: Arc<V2Worker>,
        bridge: Arc<dyn BrowserBridge>, grant: Arc<DeviceGrant>, seed: V2SeedBinding)
        -> Result<Self, Failure> {
        if !seed.belongs_to(&grant) { return Err(Failure::Identity); }
        let mut minter = Self::with_v2_device(scope, worker, bridge, grant)?;
        minter.sdk_seed = Some(seed);
        Ok(minter)
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
        Failure::WorkerStage(stage) => {
            framer_core::observability::Event::new("offline.worker.failed")
                .field("phase", stage.as_str()).emit();
            PreparationFailure::WorkerLost
        }
        Failure::V2WorkerStage(stage) => {
            let mut event = framer_core::observability::Event::new("offline.v2.failed")
                .field("phase", stage.phase());
            if let Some(label) = stage.operation() { event = event.field("operation", label); }
            if let Some(label) = stage.device() { event = event.field("device", label); }
            event.emit();
            PreparationFailure::WorkerLost
        }
    }
}

impl OfflineMinter for OfflineWorkerMinter {
    fn scope(&self) -> &AccountScope { &self.scope }

    fn saved_device(&self) -> Option<&str> {
        self.device.as_ref().map(|(_, grant)| grant.device.as_str())
    }

    fn mint<'a>(&'a self, target: &'a OfflineMintTarget,
        control: &'a PreparationControl)
        -> BoxFuture<'a, PreparationResult<MintedCastle>> {
        self.issue(target, control).boxed()
    }
}

impl OfflineWorkerMinter {
    async fn issue(&self, target: &OfflineMintTarget, control: &PreparationControl)
        -> PreparationResult<MintedCastle> {
        let Some((bridge, grant)) = &self.device else {
            return self.issue_worker(target, control).await;
        };
        if control.is_cancelled() {
            return PreparationResult::finished(Err(cancelled(control)));
        }
        let budget = control.deadline().saturating_duration_since(tokio::time::Instant::now())
            .as_millis().min(16000) as u64;
        if budget == 0 { return PreparationResult::finished(Err(PreparationFailure::Deadline)); }
        let sequence = match bridge.admit_native_mint(grant, budget).await {
            Ok(sequence) => sequence,
            Err(DeviceFailure::Rejected) =>
                return PreparationResult::finished(Err(PreparationFailure::Rejected)),
            Err(DeviceFailure::Unconfirmed) => return PreparationResult::unconfirmed(),
        };
        let result = self.issue_worker(target, control).await;
        match bridge.finish_native_mint(grant, sequence, result.cleanup_confirmed).await {
            Ok(ready) if result.cleanup_confirmed => {
                if control.is_cancelled() {
                    PreparationResult::finished(Err(cancelled(control)))
                } else if !ready && result.value.is_ok() {
                    PreparationResult::finished(Err(PreparationFailure::Rejected))
                } else { result }
            }
            _ => PreparationResult::unconfirmed(),
        }
    }

    async fn issue_worker(&self, target: &OfflineMintTarget, control: &PreparationControl)
        -> PreparationResult<MintedCastle> {
        if control.is_cancelled() {
            return PreparationResult::finished(Err(cancelled(control)));
        }
        let target = super::offline_worker_owner::Target {
            project_id: target.project().0.clone(),
            path: target.path().to_owned(),
            page_url: target.page_url().to_owned(),
        };
        let device = self.device.as_ref().map(|(_, grant)|
            super::offline_worker_owner::SavedDevice::new(grant.device.clone())).transpose();
        let device = match device {
            Ok(value) => value,
            Err(error) => return PreparationResult::finished(Err(failure(error))),
        };
        let deadline = control.deadline().into_std();
        match &self.worker {
            WorkerBackend::V3(worker) => {
                let started = match device {
                    Some(device) => worker.begin_with_device(target, &device, deadline),
                    None => worker.begin(target, deadline),
                };
                self.finish_worker(started, control).await
            }
            WorkerBackend::V2(worker) => {
                let device = self.sdk_seed.as_ref().map(V2SeedBinding::device)
                    .or(device.as_ref());
                let Some(device) = device else {
                    return PreparationResult::finished(Err(PreparationFailure::Rejected));
                };
                self.finish_worker(worker.begin_with_device(target, device, deadline), control).await
            }
        }
    }

    async fn finish_worker<M: Send + 'static>(&self, started: Result<Run<M>, Failure>,
        control: &PreparationControl) -> PreparationResult<MintedCastle> {
        let mut run = match started {
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
        // The result future may not have been polled, so its drop guard may
        // not exist yet. Signal the child before awaiting the same owner.
        if matches!(&result, Err(PreparationFailure::Cancelled | PreparationFailure::Deadline)) {
            run.cancel();
        }
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
            let minted = MintedCastle::new(self.scope.clone(), token, origin);
            Ok(match &self.device {
                Some((_, grant)) => minted.with_saved_device(grant.device.clone()),
                None => minted,
            })
        });
        PreparationResult::finished(result)
    }
}

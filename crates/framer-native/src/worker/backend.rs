//! App-owned bridge from the isolated worker to account-owned preparation.
//! Device admission and worker cleanup use the supplied native device owner.
#[cfg(test)]
#[path = "offline_worker_tests.rs"]
mod tests;

use std::time::Duration;

use std::time::Instant;
use std::sync::Arc;
use futures::future::BoxFuture;
use futures::FutureExt;
use crate::account::client::AccountScope;
use crate::account::client::PreparationControl;
use crate::account::client::PreparationFailure;
use crate::account::client::PreparationResult;
use crate::authorization::control::MintedCastle;
use crate::authorization::control::OfflineMintTarget;
use crate::authorization::control::OfflineMinter;
use crate::account::device_owner::DeviceOwner;
use framer_core::port::CastleToken;
use crate::account::device::DeviceAuthority;
use crate::account::device::DeviceFailure;
use crate::account::device::DeviceGrant;
use crate::worker::process::Cleanup;
use crate::worker::process::Failure;
use crate::worker::process::Run;
use crate::worker::process::V2Worker;
use crate::worker::process::Worker;

#[path = "offline_v2_seed.rs"]
mod v2_seed;
pub(crate) use v2_seed::V2SeedBinding;
pub(crate) use v2_seed::V2SeedTransfer;

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
    diagnostic_id: Option<uuid::Uuid>,
    scope: AccountScope,
    worker: WorkerBackend,
    device: Option<(Arc<dyn DeviceOwner>, Arc<DeviceGrant>)>,
    sdk_seed: Option<V2SeedBinding>,
}

impl OfflineWorkerMinter {
    /// Inert construction. Each issuance creates a new, isolated SDK process.
    #[cfg(test)]
    pub fn new(scope: AccountScope, worker: Arc<Worker>) -> Self {
        Self { scope, worker: WorkerBackend::V3(worker), device: None, sdk_seed: None, diagnostic_id: None }
    }

    pub fn with_device(scope: AccountScope, worker: Arc<Worker>,
        bridge: Arc<dyn DeviceOwner>, grant: Arc<DeviceGrant>) -> Result<Self, Failure> {
        Self::with_backend(scope, WorkerBackend::V3(worker), bridge, grant)
    }

    pub fn with_v2_device(scope: AccountScope, worker: Arc<V2Worker>,
        bridge: Arc<dyn DeviceOwner>, grant: Arc<DeviceGrant>) -> Result<Self, Failure> {
        Self::with_backend(scope, WorkerBackend::V2(worker), bridge, grant)
    }

    pub(crate) fn with_backend(scope: AccountScope, worker: WorkerBackend,
        bridge: Arc<dyn DeviceOwner>, grant: Arc<DeviceGrant>) -> Result<Self, Failure> {
        if grant.account != scope.account() { return Err(Failure::Identity); }
        Ok(Self { scope, worker, device: Some((bridge, grant)), sdk_seed: None, diagnostic_id: None })
    }

    pub(crate) fn with_diagnostic_id(mut self, id: Option<uuid::Uuid>) -> Self {
        self.diagnostic_id = id;
        self
    }

    pub(crate) fn with_v2_seed(scope: AccountScope, worker: Arc<V2Worker>,
        bridge: Arc<dyn DeviceOwner>, grant: Arc<DeviceGrant>, seed: V2SeedBinding)
        -> Result<Self, Failure> {
        Self::with_host_proof(scope, WorkerBackend::V2(worker), bridge, grant, seed)
    }

    pub(crate) fn with_host_proof(scope: AccountScope, worker: WorkerBackend,
        bridge: Arc<dyn DeviceOwner>, grant: Arc<DeviceGrant>, seed: V2SeedBinding)
        -> Result<Self, Failure> {
        if !seed.belongs_to(&grant) || !seed.belongs_to_scope(&scope) {
            return Err(Failure::Identity);
        }
        let mut minter = Self::with_backend(scope, worker, bridge, grant)?;
        // This proof authenticates host admission. Only V2 consumes its seed;
        // V3 still sends the saved device to its worker.
        minter.sdk_seed = Some(seed);
        Ok(minter)
    }
}

fn cancelled(control: &PreparationControl) -> PreparationFailure {
    if Instant::now() >= control.deadline().into_std() { PreparationFailure::Deadline }
    else { PreparationFailure::Cancelled }
}

fn failure(value: Failure, diagnostic_id: Option<uuid::Uuid>) -> PreparationFailure {
    match value {
        Failure::Cancelled => PreparationFailure::Cancelled,
        Failure::Deadline => PreparationFailure::Deadline,
        Failure::Input | Failure::Configuration | Failure::Identity
            | Failure::Protocol => PreparationFailure::Rejected,
        Failure::Runtime | Failure::Spawn => PreparationFailure::Unavailable,
        Failure::Io | Failure::Worker => PreparationFailure::WorkerLost,
        Failure::WorkerStage(stage) => {
            let mut event = framer_core::observability::Event::new("offline.worker.failed")
                .field("phase", stage.as_str());
            if let Some(id) = diagnostic_id { event = event.request_id(&id.to_string()); }
            if let Some(summary) = stage.summary() {
                event = event.field("version", summary.version());
                for (name, value) in summary.flags() { event = event.field(name, value); }
                for (name, value) in summary.counts() { event = event.field(name, value); }
                for (name, value) in summary.timings() {
                    event = event.field(name, value.map(serde_json::Value::from)
                        .unwrap_or(serde_json::Value::Null));
                }
            }
            event.emit();
            PreparationFailure::WorkerLost
        }
        Failure::V2WorkerStage(stage) => {
            let mut event = framer_core::observability::Event::new("offline.v2.failed")
                .field("phase", stage.phase());
            if let Some(id) = diagnostic_id { event = event.request_id(&id.to_string()); }
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
        if control.is_cancelled() {
            return PreparationResult::finished(Err(cancelled(control)));
        }
        let proof_required = matches!(&self.worker, WorkerBackend::V2(_))
            || self.device.as_ref().is_some_and(|(_, grant)|
                grant.authority == DeviceAuthority::Observation);
        if proof_required && self.sdk_seed.is_none() {
            return PreparationResult::finished(Err(PreparationFailure::Rejected));
        }
        let Some((bridge, grant)) = &self.device else {
            return self.issue_worker(target, control).await;
        };
        let budget = control.deadline().saturating_duration_since(tokio::time::Instant::now())
            .as_millis().min(16000) as u64;
        if budget == 0 { return PreparationResult::finished(Err(PreparationFailure::Deadline)); }
        let proof = self.sdk_seed.as_ref().and_then(V2SeedBinding::proof);
        let admitted = match proof {
            Some(proof) => bridge.admit_native_sdk_mint(proof, budget).await,
            None => bridge.admit_native_mint(grant, budget).await,
        };
        let sequence = match admitted {
            Ok(sequence) => sequence,
            Err(DeviceFailure::Rejected) =>
                return PreparationResult::finished(Err(PreparationFailure::Rejected)),
            Err(DeviceFailure::Unconfirmed) => return PreparationResult::unconfirmed(),
        };
        let result = self.issue_worker(target, control).await;
        let finished = match proof {
            Some(proof) => bridge.finish_native_sdk_mint(proof, sequence,
                result.cleanup_confirmed).await,
            None => bridge.finish_native_mint(grant, sequence, result.cleanup_confirmed).await,
        };
        match finished {
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
        let target = crate::worker::process::Target {
            project_id: target.project().0.clone(),
            path: target.path().to_owned(),
            page_url: target.page_url().to_owned(),
        };
        let device = self.device.as_ref().map(|(_, grant)|
            crate::worker::process::SavedDevice::new(grant.device.clone())).transpose();
        let device = match device {
            Ok(value) => value,
            Err(error) => return PreparationResult::finished(Err(failure(error, self.diagnostic_id))),
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
                let device = self.sdk_seed.as_ref().map(V2SeedBinding::device);
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
            Err(error) => return PreparationResult::finished(Err(failure(error, self.diagnostic_id))),
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
                    value = &mut wait => break value.map_err(|error| failure(error, self.diagnostic_id)),
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

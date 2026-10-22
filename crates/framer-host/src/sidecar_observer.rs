//! Explicit one-shot account teardown. This is not a warm-serving default.
use super::*;
use std::time::Duration;
use tokio::sync::{broadcast, Notify, Semaphore, OwnedSemaphorePermit};
use uuid::Uuid;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ApplicationGeneration(pub u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CleanupExtent { AccountBrowserClosed }

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CleanupReceipt {
    pub account: AccountId,
    pub application_generation: ApplicationGeneration,
    pub sidecar_incarnation: Uuid,
    pub scope: Uuid,
    pub extent: CleanupExtent,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ObserverError {
    InvalidLimits, LimitsConflict, Capacity, AccountBusy, AccountUnconfirmed,
    AccountMismatch, Sealed, Stopped, Unsupported, GenerationLost,
    Protocol, CleanupTimeout, WorkerLost, Transport, IdentifierExhausted,
    RuntimeUnavailable, Profile, UnsupportedMutation,
}
impl std::fmt::Display for ObserverError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "sidecar observation: {self:?}")
    }
}
impl std::error::Error for ObserverError {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ObserverLimits {
    pub scopes: usize,
    pub commands: usize,
    pub cleanup_timeout: Duration,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CleanupOutcome {
    NotStarted { reason: Option<ObserverError> },
    Closed(CleanupReceipt),
    Unconfirmed(ObserverError),
}

#[path = "sidecar_observer/state.rs"]
mod state;
#[path = "sidecar_observer/wire.rs"]
mod wire;
#[path = "sidecar_observer/scope.rs"]
mod scope;
#[path = "sidecar_observer/bridge.rs"]
mod bridge;
#[path = "sidecar_observer/protocol.rs"]
mod protocol;
#[path = "sidecar_observer/process.rs"]
mod process;
#[cfg(test)]
#[path = "sidecar_observer/tests.rs"]
mod tests;
pub use scope::{CleanupObserver, ObservedAccountScope, ObserverFactory, ScopeControl};
pub(super) use state::Registry;
pub(super) use wire::RemoteEvent;
pub(super) use process::ProcessMode;

impl SidecarBridge {
    /// Construct once. Only explicit one-shot account teardown is supported.
    pub fn observer_factory(self: &Arc<Self>, limits: ObserverLimits)
        -> Result<ObserverFactory, ObserverError> {
        if !matches!(self.mode, ProcessMode::Observed { .. }) {
            return Err(ObserverError::Unsupported);
        }
        if limits.scopes == 0 || limits.scopes > 512 || limits.commands == 0
            || limits.commands > 512 || limits.scopes + limits.commands >= 512
            || limits.cleanup_timeout.is_zero()
            || limits.cleanup_timeout > Duration::from_secs(900) {
            return Err(ObserverError::InvalidLimits);
        }
        let registry = self.observed.get_or_init(|| Arc::new(Registry::new(limits)));
        if registry.limits != limits { return Err(ObserverError::LimitsConflict); }
        Ok(ObserverFactory { bridge: self.clone(), registry: registry.clone() })
    }
}

#[cfg(test)]
#[path = "sidecar_observer/pipe_tests.rs"]
mod pipe_tests;

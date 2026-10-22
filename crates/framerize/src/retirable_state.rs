//! Explicit state construction for a future owned serving controller.
//! This module does not enable startup or supply external cleanup authority.
use crate::retirable_assembly::RetirableServingHost;
use framer_core::{
    gate::ConcurrencyGate,
    operation_gate::{ApplicationDrained, DrainTicket, GateError},
    port::{FramerClient, SessionStore, StateStore},
    wave::WaveGuard,
};
use framer_host::sidecar::retirement::{RetirementError, SealedHostAdmission};
use framer_server::{AppState, ServingAccountPolicy};
use std::{path::PathBuf, sync::Arc};

pub struct ServingStateInputs {
    pub client: Arc<dyn FramerClient>,
    pub sessions: Arc<dyn SessionStore>,
    pub state: Arc<dyn StateStore>,
    pub gate: Arc<ConcurrencyGate>,
    pub wave: Arc<WaveGuard>,
    pub serving_accounts: Arc<ServingAccountPolicy>,
}

/// Retain this owner while using any state clones for startup or serving.
/// Raw port holders and external writers still need separate exclusion.
#[must_use = "retain the serving-state owner through retirement"]
pub struct RetirableServingState {
    state: AppState,
    host: RetirableServingHost,
}

impl RetirableServingState {
    pub fn new(root: impl Into<PathBuf>, inputs: ServingStateInputs, limit: usize)
        -> Result<Self, GateError> {
        let host = RetirableServingHost::new(root, inputs.wave.clone(), limit)?;
        let state = AppState {
            operation_ownership: Some(host.ownership()),
            native_dispatch: None,
            browser: host.browser(),
            client: inputs.client,
            sessions: inputs.sessions,
            state: inputs.state,
            gate: inputs.gate,
            wave: inputs.wave,
            serving_accounts: inputs.serving_accounts,
        };
        Ok(Self { state, host })
    }

    pub fn state(&self) -> &AppState { &self.state }
    pub fn seal(&self) -> Result<DrainTicket, GateError> { self.host.seal() }

    pub async fn begin_retirement(&self, proof: &ApplicationDrained<'_>)
        -> Result<SealedHostAdmission, RetirementError> {
        self.host.begin_retirement(proof).await
    }
}

#[cfg(test)]
#[path = "retirable_state_tests.rs"]
mod tests;

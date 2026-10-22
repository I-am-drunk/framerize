//! Inert construction of one host and its exact serving operation gate.
//! This does not establish external process ownership or enable a serving mode.
use framer_core::operation_gate::{ApplicationDrained, DrainTicket, GateError};
use framer_core::port::BrowserBridge;
use framer_core::wave::WaveGuard;
use framer_host::sidecar::{SidecarBridge, retirement::{
    RetirementControl, RetirementError, SealedHostAdmission,
}};
use framer_server::operation_ownership::ServingOwnership;
use std::{path::PathBuf, sync::Arc};

/// Retain alongside the state built from its browser and ownership handles.
/// An existing bridge cannot be attached to this owner.
pub struct RetirableServingHost {
    browser: Arc<SidecarBridge>,
    ownership: Arc<ServingOwnership>,
    control: RetirementControl,
}

impl RetirableServingHost {
    pub fn new(root: impl Into<PathBuf>, wave: Arc<WaveGuard>, limit: usize)
        -> Result<Self, GateError> {
        let (ownership, (browser, control)) = ServingOwnership::new_with_host_factory(
            wave, limit, |gate| SidecarBridge::new_retirable(root, gate))?;
        Ok(Self { browser, ownership, control })
    }

    pub fn browser(&self) -> Arc<dyn BrowserBridge> { self.browser.clone() }
    pub fn ownership(&self) -> Arc<ServingOwnership> { self.ownership.clone() }
    pub fn seal(&self) -> Result<DrainTicket, GateError> { self.ownership.seal() }

    /// Verify application drain against this assembly before closing its host.
    /// The returned owner still needs external process/profile observations.
    pub async fn begin_retirement(&self, proof: &ApplicationDrained<'_>)
        -> Result<SealedHostAdmission, RetirementError> {
        self.ownership.check_application_drained(proof)
            .map_err(RetirementError::Gate)?;
        self.control.seal_admission(proof).await
    }
}

#[cfg(test)]
#[path = "retirable_assembly_tests.rs"]
mod tests;

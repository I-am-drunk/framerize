//! Transfer the existing pool slot and application owner to native cleanup.
use super::FailureKind;
use crate::{ids::AccountId, operation_gate::OperationOwner, resident_pool::ResidentLease};

#[cfg(test)]
#[path = "native_dispatch_admission_tests.rs"]
mod tests;

#[must_use = "retain through native cleanup, not just the HTTP result"]
pub struct PoolAdmission {
    resident: Option<ResidentLease>,
    operation: Option<OperationOwner>,
    started: bool,
}

impl Drop for PoolAdmission {
    fn drop(&mut self) {
        if self.started {
            if let Some(resident) = self.resident.take() { resident.retain_unconfirmed(); }
            // An abandoned application owner independently records uncertainty.
        } else if let Some(owner) = self.operation.take() {
            let _ = owner.complete();
        }
    }
}

impl PoolAdmission {
    pub fn new(resident: ResidentLease, operation: Option<OperationOwner>) -> Self {
        Self { resident: Some(resident), operation, started: false }
    }
    pub fn account(&self) -> AccountId {
        self.resident.as_ref().expect("owned pool admission").account()
    }
    /// Mark before starting any independent preparation or transport work.
    pub fn start(&mut self) { self.started = true; }
    /// Call only after affirmative cleanup, including retained output owners.
    pub fn complete(mut self) -> Result<(), FailureKind> {
        if let Some(owner) = self.operation.take() {
            owner.complete().map_err(|_| FailureKind::Cleanup)?;
        }
        self.started = false;
        self.resident.take();
        Ok(())
    }
}

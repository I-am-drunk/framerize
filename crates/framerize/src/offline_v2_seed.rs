use std::sync::Arc;
use framer_core::{native_device::{DeviceGrant, NativeSdkSeed}, port::Session};
use framer_api::experimental_native::native_account::AccountScope;
use super::super::offline_worker_owner::SavedDevice;

/// Keeps a host-checked observation in its original grant and account scope.
pub(crate) struct V2SeedBinding {
    grant: Arc<DeviceGrant>,
    seed: SavedDevice,
    proof: Option<NativeSdkSeed>,
    scope: Option<AccountScope>,
}

impl V2SeedBinding {
    #[cfg(test)]
    pub(crate) fn new(grant: Arc<DeviceGrant>, seed: SavedDevice) -> Self {
        Self { grant, seed, proof: None, scope: None }
    }

    pub(crate) fn from_proof(scope: &AccountScope, grant: Arc<DeviceGrant>, session: &Session,
        proof: NativeSdkSeed) -> Result<Self, super::Failure> {
        if !proof.matches(&grant, session) || scope.account() != grant.account
            || scope.active_user() != &session.active_user {
            return Err(super::Failure::Identity);
        }
        let seed = SavedDevice::new(proof.seed().to_owned())?;
        Ok(Self { grant, seed, proof: Some(proof), scope: Some(scope.clone()) })
    }

    pub(super) fn belongs_to(&self, grant: &Arc<DeviceGrant>) -> bool {
        Arc::ptr_eq(&self.grant, grant)
    }

    pub(super) fn device(&self) -> &SavedDevice { &self.seed }
    pub(super) fn proof(&self) -> Option<&NativeSdkSeed> { self.proof.as_ref() }
    pub(super) fn belongs_to_scope(&self, scope: &AccountScope) -> bool {
        self.scope.as_ref().is_none_or(|bound| bound.same_generation(scope))
    }
}

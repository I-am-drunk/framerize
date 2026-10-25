use std::sync::Arc;
use crate::account::device::DeviceFailure;
use crate::account::device::DeviceGrant;
use crate::account::device::NativeSdkSeed;
use crate::account::device_owner::DeviceOwner;
use framer_core::port::Session;
use crate::account::client::AccountScope;
use crate::worker::process::SavedDevice;

/// Keeps a host-checked observation in its original grant and account scope.
pub(crate) struct V2SeedBinding {
    grant: Arc<DeviceGrant>,
    seed: SavedDevice,
    proof: Option<Arc<NativeSdkSeed>>,
    scope: Option<AccountScope>,
}

impl V2SeedBinding {
    #[cfg(test)]
    pub(crate) fn new(grant: Arc<DeviceGrant>, seed: SavedDevice) -> Self {
        Self { grant, seed, proof: None, scope: None }
    }

    pub(crate) fn from_proof(scope: &AccountScope, grant: Arc<DeviceGrant>, session: &Session,
        proof: impl Into<Arc<NativeSdkSeed>>) -> Result<Self, super::Failure> {
        let proof = proof.into();
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
    pub(super) fn proof(&self) -> Option<&NativeSdkSeed> { self.proof.as_deref() }
    pub(super) fn belongs_to_scope(&self, scope: &AccountScope) -> bool {
        self.scope.as_ref().is_none_or(|bound| bound.same_generation(scope))
    }
}

enum TransferState {
    Idle,
    Bound(Arc<NativeSdkSeed>),
    Failed(DeviceFailure),
}

/// A host attachment belongs to the whole original grant, not one mint.
pub(crate) struct V2SeedTransfer {
    scope: AccountScope,
    grant: Arc<DeviceGrant>,
    state: tokio::sync::Mutex<TransferState>,
}

impl V2SeedTransfer {
    pub(crate) fn new(scope: AccountScope, grant: Arc<DeviceGrant>) -> Self {
        Self { scope, grant, state: tokio::sync::Mutex::new(TransferState::Idle) }
    }

    pub(crate) async fn get(&self, scope: &AccountScope, session: &Session,
        bridge: &dyn DeviceOwner) -> Result<Arc<NativeSdkSeed>, DeviceFailure> {
        if !self.scope.same_generation(scope) || scope.account() != self.grant.account
            || scope.active_user() != &session.active_user {
            return Err(DeviceFailure::Rejected);
        }
        let mut state = self.state.lock().await;
        match &*state {
            TransferState::Bound(proof) => return if proof.matches(&self.grant, session) {
                Ok(proof.clone())
            } else { Err(DeviceFailure::Rejected) },
            TransferState::Failed(error) => return Err(*error),
            TransferState::Idle => {}
        }
        // If this future is lost or panics, no later caller may repeat the transfer.
        *state = TransferState::Failed(DeviceFailure::Unconfirmed);
        let result = bridge.transfer_native_sdk_seed(self.grant.clone(), session).await
            .and_then(|proof| if proof.matches(&self.grant, session) {
                Ok(Arc::new(proof))
            } else { Err(DeviceFailure::Unconfirmed) });
        *state = match &result {
            Ok(proof) => TransferState::Bound(proof.clone()),
            Err(error) => TransferState::Failed(*error),
        };
        result
    }
}

//! One request's handle into its explicitly supplied native device owner.
use framer_core::ids::AccountId;
use crate::account::device::DeviceFailure;
use crate::account::device::DeviceGrant;
use crate::account::device_owner::DeviceOwner;
use futures::FutureExt;
use std::panic::AssertUnwindSafe;
use std::sync::Arc;
use crate::authorization::source::SourceBuilder;

pub(crate) struct DeviceLease {
    owned: Option<(Arc<dyn DeviceOwner>, Arc<DeviceGrant>)>,
}

impl DeviceLease {
    pub(crate) async fn open(source: &dyn SourceBuilder, account: AccountId)
        -> Result<Self, DeviceFailure> {
        let Some(bridge) = source.device_owner() else {
            return Ok(Self { owned: None });
        };
        let grant = AssertUnwindSafe(bridge.open_native_device(account))
            .catch_unwind().await.map_err(|_| DeviceFailure::Unconfirmed)??;
        if grant.account != account { return Err(DeviceFailure::Unconfirmed); }
        Ok(Self { owned: Some((bridge, Arc::new(grant))) })
    }

    pub(crate) fn grant(&self) -> Option<Arc<DeviceGrant>> {
        self.owned.as_ref().map(|(_, grant)| grant.clone())
    }

    /// Called only after request work and transport cleanup have settled.
    /// An idle client can be retained only after this and final publication succeed.
    /// A lost acknowledgement keeps the caller's existing account owner armed.
    pub(crate) async fn finish(self, joined: bool, risk_refused: bool) -> bool {
        let Some((bridge, grant)) = self.owned else { return joined; };
        matches!(AssertUnwindSafe(bridge.close_native_device(&grant, joined, risk_refused))
            .catch_unwind().await, Ok(Ok(true))) && joined
    }
}

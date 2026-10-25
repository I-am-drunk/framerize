//! One request's handle into the existing host device authority.
use framer_core::ids::AccountId;
use framer_core::native_device::DeviceFailure;
use framer_core::native_device::DeviceGrant;
use framer_core::port::BrowserBridge;
use futures::FutureExt;
use std::panic::AssertUnwindSafe;
use std::sync::Arc;
use crate::authorization::source::SourceBuilder;

pub(crate) struct DeviceLease {
    owned: Option<(Arc<dyn BrowserBridge>, Arc<DeviceGrant>)>,
}

impl DeviceLease {
    pub(crate) async fn open(source: &dyn SourceBuilder, account: AccountId)
        -> Result<Self, DeviceFailure> {
        let Some(bridge) = source.device_bridge() else {
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

    /// Called only after the native scope and transport cleanup have settled.
    /// A lost acknowledgement keeps the caller's existing account owner armed.
    pub(crate) async fn finish(self, joined: bool, risk_refused: bool) -> bool {
        let Some((bridge, grant)) = self.owned else { return joined; };
        matches!(AssertUnwindSafe(bridge.close_native_device(&grant, joined, risk_refused))
            .catch_unwind().await, Ok(Ok(true))) && joined
    }
}

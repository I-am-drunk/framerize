use std::sync::Arc;
use framer_core::native_device::DeviceGrant;
use super::super::offline_worker_owner::SavedDevice;

/// Binds a separately supplied SDK seed to its exact native device grant.
/// The caller owns the observation. This type neither observes nor derives it.
pub(crate) struct V2SeedBinding {
    grant: Arc<DeviceGrant>,
    seed: SavedDevice,
}

impl V2SeedBinding {
    pub(crate) fn new(grant: Arc<DeviceGrant>, seed: SavedDevice) -> Self {
        Self { grant, seed }
    }

    pub(super) fn belongs_to(&self, grant: &Arc<DeviceGrant>) -> bool {
        Arc::ptr_eq(&self.grant, grant)
    }

    pub(super) fn device(&self) -> &SavedDevice { &self.seed }
}

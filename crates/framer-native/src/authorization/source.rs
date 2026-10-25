use crate::authorization::CredentialSource;
use crate::account::client::AccountScope;
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Failure { MissingAuthority, Source }

pub(crate) trait SourceBuilder: Send + Sync {
    fn build(&self, scope: AccountScope, proxy: Option<String>)
        -> Result<Arc<dyn CredentialSource>, Failure>;

    /// The caller retains the host grant through the whole request cleanup.
    /// Merely querying this capability starts no external work.
    fn device_bridge(&self) -> Option<Arc<dyn framer_core::port::BrowserBridge>> { None }

    fn build_with_device(&self, scope: AccountScope, proxy: Option<String>,
        device: Option<Arc<framer_core::native_device::DeviceGrant>>)
        -> Result<Arc<dyn CredentialSource>, Failure> {
        if device.is_some() { return Err(Failure::MissingAuthority); }
        self.build(scope, proxy)
    }
}

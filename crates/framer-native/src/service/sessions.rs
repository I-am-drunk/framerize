//! One shared session-generation boundary for native requests and store writers.
use crate::account::client::NativeAccountClient;
use crate::account::sessions::SessionStoreBridge;
use framer_core::port::SessionStore;
use framer_server::operation_ownership::ServingOwnership;
use std::sync::Arc;

#[derive(Clone)]
pub(crate) struct Sessions {
    native: NativeAccountClient,
    bridge: Arc<SessionStoreBridge>,
    ownership: Arc<ServingOwnership>,
}

impl Sessions {
    pub fn new(store: Arc<dyn SessionStore>, ownership: Arc<ServingOwnership>)
        -> Result<Self, &'static str> {
        Self::with_browser(store, ownership, None, Default::default())
    }
    pub fn with_browser(store: Arc<dyn SessionStore>, ownership: Arc<ServingOwnership>,
        browser: Option<Arc<dyn crate::transport::browser::BrowserTransport>>,
        authorization: framer_api::SessionAuthorization)
        -> Result<Self, &'static str> {
        let native = match browser {
            Some(browser) => NativeAccountClient::with_browser_transport(browser),
            None => NativeAccountClient::new(),
        }.map_err(|_| "native session owner unavailable")?;
        let bridge = Arc::new(SessionStoreBridge::with_authorization(native.clone(), store, authorization)
            .map_err(|_| "shared session store unavailable")?);
        Ok(Self { native, bridge, ownership })
    }
    pub fn store(&self) -> Arc<dyn SessionStore> { self.bridge.clone() }
    pub fn native(&self) -> &NativeAccountClient { &self.native }
    pub fn bridge(&self) -> &SessionStoreBridge { &self.bridge }
    pub fn matches(&self, store: &Arc<dyn SessionStore>, owner: Option<&Arc<ServingOwnership>>) -> bool {
        Arc::ptr_eq(&self.store(), store)
            && owner.is_some_and(|owner| Arc::ptr_eq(&self.ownership, owner))
    }
    pub async fn shutdown(&self) -> Result<(), &'static str> {
        let mut seal = self.ownership.seal().map_err(|_| "session shutdown admission failed")?;
        let drained = seal.wait_application().await.map_err(|_| "session operations unconfirmed")?;
        self.ownership.check_application_drained(&drained).map_err(|_| "session operations unconfirmed")?;
        self.bridge.shutdown().await?;
        self.native.shutdown_idle().await.map_err(|_| "native session shutdown unconfirmed")
    }
}

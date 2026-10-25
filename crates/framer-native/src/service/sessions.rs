//! One shared session-generation boundary for native requests and store writers.
use crate::account::client::NativeAccountClient;
use crate::account::sessions::SessionStoreBridge;
use framer_core::port::SessionStore;
use framer_server::AppState;
use std::sync::Arc;

#[derive(Clone)]
pub(crate) struct Sessions {
    native: NativeAccountClient,
    bridge: Arc<SessionStoreBridge>,
}

impl Sessions {
    pub fn install(state: &mut AppState) -> Result<Self, &'static str> {
        let native = NativeAccountClient::new().map_err(|_| "native session owner unavailable")?;
        let bridge = Arc::new(SessionStoreBridge::new(native.clone(), state.sessions.clone())
            .map_err(|_| "shared session store unavailable")?);
        state.sessions = bridge.clone();
        Ok(Self { native, bridge })
    }
    pub fn native(&self) -> &NativeAccountClient { &self.native }
    pub fn bridge(&self) -> &SessionStoreBridge { &self.bridge }
    pub fn matches(&self, state: &AppState) -> bool {
        let shared: Arc<dyn SessionStore> = self.bridge.clone();
        Arc::ptr_eq(&shared, &state.sessions)
    }
    pub async fn shutdown(&self, state: &AppState) -> Result<(), &'static str> {
        if !self.matches(state) { return Err("different shared session store"); }
        let owner = state.operation_ownership.as_ref().ok_or("missing serving owner")?;
        let mut seal = owner.seal().map_err(|_| "session shutdown admission failed")?;
        let drained = seal.wait_application().await.map_err(|_| "session operations unconfirmed")?;
        owner.check_application_drained(&drained).map_err(|_| "session operations unconfirmed")?;
        self.bridge.shutdown().await?;
        self.native.shutdown_idle().await.map_err(|_| "native session shutdown unconfirmed")
    }
}

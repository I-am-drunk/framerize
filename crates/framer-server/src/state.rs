//! Resources shared by every request. Replacing request handling must not
//! construct a second session store, admission policy, or browser owner.
use crate::{operation_ownership::ServingOwnership, ServingAccountPolicy};
use framer_core::port::{BrowserBridge, FramerClient, SessionStore, StateStore};
use std::sync::Arc;

#[derive(Clone)]
pub struct AppState {
    runtime: Arc<AccountRuntime>,
}

impl AppState {
    pub fn new(runtime: Arc<AccountRuntime>) -> Self { Self { runtime } }
    pub fn runtime(&self) -> &Arc<AccountRuntime> { &self.runtime }
    pub(crate) fn runtime_mut(&mut self) -> Option<&mut AccountRuntime> {
        Arc::get_mut(&mut self.runtime)
    }
}

impl std::ops::Deref for AppState {
    type Target = AccountRuntime;
    fn deref(&self) -> &Self::Target { &self.runtime }
}

/// Construct once from owned stores, then share across application routers.
/// Public port handles preserve legacy application wiring. AppState exposes
/// only shared access; configuring owners after sharing is rejected.
pub struct AccountRuntime {
    /// Explicit application ownership; no default activation.
    pub operation_ownership: Option<Arc<ServingOwnership>>,
    pub client: Arc<dyn FramerClient>,
    pub browser: Arc<dyn BrowserBridge>,
    pub sessions: Arc<dyn SessionStore>,
    pub state: Arc<dyn StateStore>,
    /// Paces requests against Framer's per-workspace concurrency cap.
    pub gate: Arc<framer_core::gate::ConcurrencyGate>,
    /// Environment-level guard for refusals clustered across accounts.
    pub wave: Arc<framer_core::wave::WaveGuard>,
    /// Shared eligibility, residency and admission controls.
    pub serving_accounts: Arc<ServingAccountPolicy>,
}

impl AccountRuntime {
    pub fn with_gate(mut self, gate: framer_core::gate::ConcurrencyGate) -> Self {
        self.gate = Arc::new(gate);
        self
    }
    pub fn new(
        client: Arc<dyn FramerClient>,
        browser: Arc<dyn BrowserBridge>,
        sessions: Box<dyn SessionStore>,
        state: Box<dyn StateStore>,
        serving_accounts: ServingAccountPolicy,
    ) -> Self {
        Self {
            operation_ownership: None,
            client, browser, sessions: Arc::from(sessions), state: Arc::from(state),
            serving_accounts: Arc::new(serving_accounts),
            gate: Arc::new(framer_core::gate::ConcurrencyGate::new()),
            wave: Arc::new(framer_core::wave::WaveGuard::new()),
        }
    }
}

#[cfg(test)]
impl AppState {
    pub(crate) fn fixture(
        client: Arc<dyn FramerClient>, browser: Arc<dyn BrowserBridge>,
        sessions: Arc<dyn SessionStore>, state: Arc<dyn StateStore>,
        serving_accounts: ServingAccountPolicy, gate: framer_core::gate::ConcurrencyGate,
    ) -> Self {
        Self::new(Arc::new(AccountRuntime {
            operation_ownership: None, client, browser, sessions, state,
            serving_accounts: Arc::new(serving_accounts), gate: Arc::new(gate),
            wave: Arc::new(framer_core::wave::WaveGuard::new()),
        }))
    }
}

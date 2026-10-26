//! Resources shared by every request. Replacing request handling must not
//! construct a second session store, admission policy, or browser owner.
use crate::{operation_ownership::ServingOwnership, ServingAccountPolicy};
use framer_core::port::{BrowserBridge, FramerClient, SessionStore, StateStore};
use std::sync::Arc;

#[derive(Clone)]
pub struct AppState {
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

impl AppState {
    pub fn new(
        client: Arc<dyn FramerClient>,
        browser: Arc<dyn BrowserBridge>,
        sessions: Arc<dyn SessionStore>,
        state: Arc<dyn StateStore>,
        serving_accounts: Arc<ServingAccountPolicy>,
    ) -> Self {
        Self {
            operation_ownership: None,
            client, browser, sessions, state, serving_accounts,
            gate: Arc::new(framer_core::gate::ConcurrencyGate::new()),
            wave: Arc::new(framer_core::wave::WaveGuard::new()),
        }
    }
}

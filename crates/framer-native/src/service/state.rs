//! Native application state. No browser client or browser residency owner.
use framer_core::{ids::AccountId, port::{SessionStore, StateStore},
    gate::ConcurrencyGate, resident_pool::{Acquire, ResidentLease, ResidentPool}, wave::WaveGuard};
use framer_server::operation_ownership::ServingOwnership;
use std::{collections::HashSet, sync::Arc};

#[derive(Clone)]
pub(crate) struct ServiceState {
    // Standalone native control auth is uncached. A shared legacy owner must
    // supply its own handle before configuring the native session bridge.
    pub session_authorization: framer_api::SessionAuthorization,
    pub operation_ownership: Option<Arc<ServingOwnership>>,
    pub sessions: Arc<dyn SessionStore>,
    pub state: Arc<dyn StateStore>,
    pub gate: Arc<ConcurrencyGate>,
    pub wave: Arc<WaveGuard>,
    pub serving_accounts: Arc<AccountPool>,
}

pub(crate) struct AccountPool {
    allowed: HashSet<AccountId>,
    pub residents: ResidentPool,
    pub health_update: tokio::sync::Mutex<()>,
}

impl ServiceState {
    pub fn new(sessions: Arc<dyn SessionStore>, state: Arc<dyn StateStore>,
        selected: impl IntoIterator<Item = AccountId>) -> Result<Self, &'static str> {
        let serving_accounts = Arc::new(AccountPool::new(selected)?);
        let wave = Arc::new(WaveGuard::new());
        let operation_ownership = Some(ServingOwnership::new_unmanaged(wave.clone(), 256)
            .map_err(|_| "native operation owner unavailable")?);
        Ok(Self { sessions, state, serving_accounts, wave, operation_ownership,
            session_authorization: Default::default(),
            gate: Arc::new(ConcurrencyGate::new()) })
    }
}

impl AccountPool {
    pub fn new(allowed: impl IntoIterator<Item = AccountId>) -> Result<Self, &'static str> {
        let allowed: HashSet<_> = allowed.into_iter().collect();
        if allowed.is_empty() || allowed.len() > 64 { return Err("select 1 to 64 native accounts"); }
        let residents = ResidentPool::new(allowed.len()).map_err(|_| "invalid account capacity")?;
        Ok(Self { allowed, residents, health_update: tokio::sync::Mutex::new(()) })
    }
    pub fn allows(&self, id: &AccountId) -> bool { self.allowed.contains(id) }

    pub fn acquire(&self, id: AccountId) -> Result<ResidentLease, &'static str> {
        if !self.allows(&id) { return Err("account is not selected"); }
        match self.residents.try_acquire(id, None).map_err(|_| "account is unavailable")? {
            Acquire::Lease(lease) => Ok(lease),
            Acquire::Evict(ticket) => {
                // A fixed selected set has one slot per identity. Never claim
                // external cleanup or evict a resource to admit another one.
                let _ = ticket.cancel_before_io();
                Err("selected account capacity is inconsistent")
            }
        }
    }
}

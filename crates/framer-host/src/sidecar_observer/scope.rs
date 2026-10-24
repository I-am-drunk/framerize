use super::*;
use state::ScopeState;

#[cfg(test)]
#[path = "scope_panic_tests.rs"]
mod panic_tests;

#[derive(Clone)]
pub struct ObserverFactory {
    pub(super) bridge: Arc<SidecarBridge>,
    pub(super) registry: Arc<Registry>,
}
pub struct ObservedAccountScope {
    pub browser: Arc<dyn BrowserBridge>,
    pub control: ScopeControl,
    pub cleanup: CleanupObserver,
}
#[derive(Clone)]
pub struct ScopeControl { pub(super) state: Arc<ScopeState> }
impl ScopeControl {
    pub fn scope_id(&self) -> Uuid { self.state.id }

    pub fn matches_closed_receipt(&self, receipt: &CleanupReceipt) -> bool {
        let p = self.state.lock_phase();
        receipt.account == self.state.account
            && receipt.application_generation == self.state.application
            && receipt.scope == self.state.id
            && receipt.extent == CleanupExtent::AccountBrowserClosed
            && p.connection.as_ref().is_some_and(|c| c.incarnation == receipt.sidecar_incarnation)
            && matches!(p.outcome.as_ref(), Some(CleanupOutcome::Closed(saved)) if saved == receipt)
    }

    pub fn request_stop(&self) { self.state.stop(); }
    /// Full account-browser teardown. This is not warm request completion.
    pub fn seal_for_account_teardown(&self) { self.state.seal(); }
}
pub struct CleanupObserver { state: Arc<ScopeState> }
impl Drop for CleanupObserver {
    fn drop(&mut self) { self.state.stop(); }
}
impl CleanupObserver {
    pub async fn wait(self) -> CleanupOutcome {
        loop {
            let changed = self.state.changed.notified();
            tokio::pin!(changed); changed.as_mut().enable();
            if let Some(outcome) = self.state.lock_phase().outcome.clone() {
                return outcome;
            }
            changed.await;
        }
    }
}

impl ObserverFactory {
    pub fn one_shot_scope(&self, account: AccountId, application: ApplicationGeneration)
        -> Result<ObservedAccountScope, ObserverError> {
        let runtime = tokio::runtime::Handle::try_current()
            .map_err(|_| ObserverError::RuntimeUnavailable)?;
        let mut accounts = self.registry.accounts.lock()
            .map_err(|_| ObserverError::AccountUnconfirmed)?;
        if let Some(old) = accounts.get(&account) {
            return Err(if matches!(old.lock_phase().outcome.as_ref(), Some(CleanupOutcome::Unconfirmed(_))) {
                ObserverError::AccountUnconfirmed
            } else { ObserverError::AccountBusy });
        }
        let permit = self.registry.slots.clone().try_acquire_owned()
            .map_err(|_| ObserverError::Capacity)?;
        let state = Arc::new(ScopeState { account, application, id: Uuid::new_v4(),
            _owner: self.bridge.clone(),
            phase: StdMutex::new(state::Phase::default()), changed: Notify::new(),
            proxy: StdMutex::new(None), queue_wait: AtomicU64::new(0),
            _slot: permit });
        accounts.insert(account, state.clone());
        drop(accounts);
        let guard = RunGuard { state: state.clone(), registry: self.registry.clone(), finished: false };
        let base = self.bridge.clone();
        let registry = self.registry.clone();
        runtime.spawn(async move { supervise(base, registry, guard).await; });
        Ok(ObservedAccountScope {
            browser: Arc::new(bridge::ScopedBridge {
                registry: self.registry.clone(), state: state.clone() }),
            control: ScopeControl { state: state.clone() },
            cleanup: CleanupObserver { state },
        })
    }
}

struct RunGuard { state: Arc<ScopeState>, registry: Arc<Registry>, finished: bool }
impl Drop for RunGuard {
    fn drop(&mut self) {
        if !self.finished {
            let never_started = {
                let p = self.state.lock_phase();
                !p.started && p.connection.is_none() && p.outcome.is_none()
            };
            if never_started {
                if let Ok(mut accounts) = self.registry.accounts.lock() {
                    accounts.remove(&self.state.account);
                    self.state.publish(CleanupOutcome::NotStarted { reason: Some(ObserverError::WorkerLost) });
                    return;
                }
            }
            self.state.publish(CleanupOutcome::Unconfirmed(ObserverError::WorkerLost));
            let connection = self.state.lock_phase().connection.clone();
            if let Some(c) = connection {
                c.invalidate();
            }
        }
    }
}

async fn supervise(base: Arc<SidecarBridge>, registry: Arc<Registry>, mut guard: RunGuard) {
    let state = guard.state.clone();
    let work = protocol::run(base.clone(), state.clone());
    tokio::pin!(work);
    let mut deadline = None;
    let outcome = loop {
        let changed = state.changed.notified();
        tokio::pin!(changed); changed.as_mut().enable();
        if deadline.is_none() {
            deadline = state.lock_phase().sealed_at
                .map(|at| at + registry.limits.cleanup_timeout);
        }
        let expiry = async {
            if let Some(at) = deadline { tokio::time::sleep_until(at).await; }
            else { std::future::pending::<()>().await; }
        };
        tokio::select! {
            biased;
            _ = expiry => {
                state.publish(CleanupOutcome::Unconfirmed(ObserverError::CleanupTimeout));
                // Keep ownership after reporting the bounded wait failure.
                break work.await;
            }
            result = &mut work => break result,
            _ = changed => {}
        }
    };
    let mut outcome = state.lock_phase().outcome.clone().unwrap_or(outcome);
    if !matches!(outcome, CleanupOutcome::Unconfirmed(_)) {
        match registry.accounts.lock() {
            Ok(mut accounts) => { accounts.remove(&state.account); }
            Err(_) => { outcome = CleanupOutcome::Unconfirmed(ObserverError::WorkerLost); }
        }
    }
    state.publish(outcome);
    let unconfirmed = matches!(state.lock_phase().outcome,
        Some(CleanupOutcome::Unconfirmed(_)));
    if unconfirmed {
        // Retain the original child, slot and account fence. A replacement
        // process cannot reconcile this generation. Runtime shutdown is not
        // a successful cleanup receipt.
        let incarnation = state.lock_phase().connection.as_ref().map(|c| c.incarnation);
        if let Some(incarnation) = incarnation {
            reap_natural_exit(&base, incarnation).await;
        }
        let retained = (base, registry);
        std::future::pending::<()>().await;
        drop(retained);
    }
    guard.finished = true;
}

// Reaping an exited leader releases its kernel resources, not its account
// reservation. Never signal a live child or hold the lifecycle lock while idle.
async fn reap_natural_exit(base: &SidecarBridge, incarnation: Uuid) {
    loop {
        {
            let mut inner = base.inner.lock().await;
            let Some(host) = inner.as_mut() else { return; };
            if host.incarnation != incarnation { return; }
            match host.child.try_wait() {
                Ok(None) => {},
                Ok(Some(_)) | Err(_) => return,
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
}

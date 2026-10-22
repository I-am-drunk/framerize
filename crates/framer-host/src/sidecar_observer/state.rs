use super::*;

pub(crate) struct Registry {
    pub limits: ObserverLimits,
    pub slots: Arc<Semaphore>,
    pub commands: Arc<Semaphore>,
    pub accounts: StdMutex<HashMap<AccountId, Arc<ScopeState>>>,
}
impl Registry {
    pub fn new(limits: ObserverLimits) -> Self {
        Self { limits, slots: Arc::new(Semaphore::new(limits.scopes)),
            commands: Arc::new(Semaphore::new(limits.commands)),
            accounts: StdMutex::new(HashMap::new()) }
    }
}

#[derive(Default)]
pub(super) struct Phase {
    pub started: bool,
    pub sealed: bool,
    pub sealed_at: Option<tokio::time::Instant>,
    pub stop: bool,
    pub active: usize,
    pub connection: Option<Arc<wire::Connection>>,
    pub admitted: bool,
    pub setup_error: Option<ObserverError>,
    pub outcome: Option<CleanupOutcome>,
}

pub(super) struct ScopeState {
    // A poisoned registry retains its original process even if all public
    // handles disappear. Clean removal breaks this bounded ownership cycle.
    pub _owner: Arc<SidecarBridge>,
    pub account: AccountId,
    pub application: ApplicationGeneration,
    pub id: Uuid,
    pub phase: StdMutex<Phase>,
    pub changed: Notify,
    pub proxy: StdMutex<Option<String>>,
    pub queue_wait: AtomicU64,
    pub _slot: OwnedSemaphorePermit,
}
impl ScopeState {
    /// A poisoned state cannot prove cleanup. Recover the lock only to fence
    /// further work and publish that uncertainty without panicking in Drop.
    pub fn lock_phase(&self) -> std::sync::MutexGuard<'_, Phase> {
        match self.phase.lock() {
            Ok(phase) => phase,
            Err(poisoned) => {
                let mut phase = poisoned.into_inner();
                phase.stop = true;
                phase.sealed = true;
                phase.sealed_at.get_or_insert_with(tokio::time::Instant::now);
                phase.outcome = Some(CleanupOutcome::Unconfirmed(ObserverError::WorkerLost));
                self.changed.notify_waiters();
                phase
            }
        }
    }

    pub fn identity(&self) -> Value {
        json!({"account": self.account.0.to_string(), "scope": self.id.to_string(),
            "application_generation": self.application.0})
    }
    pub fn begin(self: &Arc<Self>, registry: &Registry)
        -> Result<CommandGuard, ObserverError> {
        let mut p = self.lock_phase();
        if p.sealed || p.outcome.is_some() { return Err(ObserverError::Sealed); }
        let permit = registry.commands.clone().try_acquire_owned()
            .map_err(|_| ObserverError::Capacity)?;
        p.active += 1;
        p.started = true;
        drop(p); self.changed.notify_waiters();
        Ok(CommandGuard { state: self.clone(), _permit: permit, finished: false })
    }
    pub fn stop(&self) {
        let mut p = self.lock_phase();
        p.stop = true; p.sealed = true;
        p.sealed_at.get_or_insert_with(tokio::time::Instant::now);
        drop(p); self.changed.notify_waiters();
    }
    pub fn seal(&self) {
        let mut p = self.lock_phase();
        p.sealed = true;
        p.sealed_at.get_or_insert_with(tokio::time::Instant::now);
        drop(p);
        self.changed.notify_waiters();
    }
    pub fn publish(&self, result: CleanupOutcome) {
        let mut p = self.lock_phase();
        p.sealed = true;
        p.sealed_at.get_or_insert_with(tokio::time::Instant::now);
        if matches!(result, CleanupOutcome::Unconfirmed(_)) { p.stop = true; p.sealed = true; }
        if p.outcome.is_none() { p.outcome = Some(result); }
        drop(p); self.changed.notify_waiters();
    }

    pub async fn stopped(&self) {
        loop {
            let changed = self.changed.notified();
            tokio::pin!(changed); changed.as_mut().enable();
            if self.lock_phase().stop { return; }
            changed.await;
        }
    }
    pub async fn ready(&self) -> Result<Arc<wire::Connection>, ObserverError> {
        loop {
            let changed = self.changed.notified();
            tokio::pin!(changed); changed.as_mut().enable();
            {
                let p = self.lock_phase();
                if let Some(error) = p.setup_error { return Err(error); }
                if p.stop { return Err(ObserverError::Stopped); }
                if let Some(outcome) = p.outcome.as_ref() {
                    return Err(match outcome {
                        CleanupOutcome::Unconfirmed(e) => *e,
                        CleanupOutcome::NotStarted { reason } => reason.unwrap_or(ObserverError::Stopped),
                        CleanupOutcome::Closed(_) => ObserverError::Sealed,
                    });
                }
                if p.admitted {
                    if let Some(c) = p.connection.as_ref() { return Ok(c.clone()); }
                }
            }
            changed.await;
        }
    }
}

pub(super) struct CommandGuard {
    state: Arc<ScopeState>,
    _permit: OwnedSemaphorePermit,
    finished: bool,
}
impl CommandGuard {
    pub fn complete(mut self) { self.finished = true; }
}
impl Drop for CommandGuard {
    fn drop(&mut self) {
        if !self.finished { self.state.publish(CleanupOutcome::Unconfirmed(ObserverError::WorkerLost)); }
        let mut p = self.state.lock_phase();
        if p.active == 0 {
            p.stop = true;
            p.sealed = true;
            p.outcome = Some(CleanupOutcome::Unconfirmed(ObserverError::WorkerLost));
        } else {
            p.active -= 1;
        }
        let connection = if !self.finished || matches!(p.outcome, Some(CleanupOutcome::Unconfirmed(_))) {
            p.connection.clone()
        } else { None };
        drop(p);
        if let Some(c) = connection { c.invalidate(); }
        self.state.changed.notify_waiters();
    }
}

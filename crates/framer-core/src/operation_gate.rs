//! Owned operation admission. UNWIRED: only registered work is fenced.
//! This local gate is not outer process exclusion or a drain of unmanaged work.
use std::sync::{Arc, Mutex};
use std::collections::BTreeMap;
use tokio::sync::watch;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GateError { InvalidLimit, Full, Sealed, Unconfirmed, Poisoned,
    WrongGate, StaleSeal, NotDrained, Exhausted }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OperationKind { Inference, Admin, SessionWrite, StateWrite,
    Background, Authorization, ExternalCleanup }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GateStats { pub active: usize, pub sealed: bool, pub unconfirmed: bool }
#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase { Open, Draining(u64), Retiring(u64), Held(u64) }
#[derive(Clone, Copy, PartialEq, Eq)]
enum LegacyHostState { Absent, Retained, Abandoned }
struct State {
    phase: Phase, next: u64, next_operation: u64,
    applications: BTreeMap<u64, OperationKind>,
    legacy_host: LegacyHostState, unconfirmed: bool,
}
impl State {
    fn occupied(&self) -> usize {
        self.applications.len() + usize::from(self.legacy_host != LegacyHostState::Absent)
    }
}
struct Inner { state: Mutex<State>, changed: watch::Sender<()>, limit: usize }

#[derive(Clone)]
pub struct OperationGate { inner: Arc<Inner> }
impl OperationGate {
    pub fn new(limit: usize) -> Result<Self, GateError> {
        if limit == 0 || limit > 65536 { return Err(GateError::InvalidLimit); }
        let (changed, _) = watch::channel(());
        Ok(Self { inner: Arc::new(Inner { limit, changed,
            state: Mutex::new(State { phase: Phase::Open, next: 1, next_operation: 1,
                applications: BTreeMap::new(), legacy_host: LegacyHostState::Absent,
                unconfirmed: false }) }) })
    }
    pub fn stats(&self) -> Result<GateStats, GateError> {
        let s = self.inner.state.lock().map_err(|_| GateError::Poisoned)?;
        Ok(GateStats { active: s.occupied(), sealed: s.phase != Phase::Open,
            unconfirmed: s.unconfirmed })
    }
    /// Construct before any work. The named host owner has no completion API.
    pub fn new_with_legacy_host(limit: usize) -> Result<(Self, LegacyHostOwner), GateError> {
        let gate = Self::new(limit)?;
        gate.inner.state.lock().map_err(|_| GateError::Poisoned)?.legacy_host =
            LegacyHostState::Retained;
        let host = LegacyHostOwner { gate: gate.clone() };
        Ok((gate, host))
    }
    pub fn admit(&self, kind: OperationKind) -> Result<OperationOwner, GateError> {
        let mut s = self.inner.state.lock().map_err(|_| GateError::Poisoned)?;
        if s.unconfirmed { return Err(GateError::Unconfirmed); }
        if s.phase != Phase::Open { return Err(GateError::Sealed); }
        self.reserve(&mut s, kind)
    }
    fn reserve(&self, s: &mut State, kind: OperationKind) -> Result<OperationOwner, GateError> {
        if s.unconfirmed { return Err(GateError::Unconfirmed); }
        if s.occupied() >= self.inner.limit { return Err(GateError::Full); }
        let id = s.next_operation;
        s.next_operation = id.checked_add(1).ok_or(GateError::Exhausted)?;
        s.applications.insert(id, kind);
        Ok(OperationOwner { gate: self.clone(), id, kind, armed: true })
    }
    pub fn seal(&self) -> Result<DrainTicket, GateError> {
        let mut s = self.inner.state.lock().map_err(|_| GateError::Poisoned)?;
        if s.unconfirmed { return Err(GateError::Unconfirmed); }
        if s.phase != Phase::Open { return Err(GateError::Sealed); }
        let id = s.next;
        s.next = id.checked_add(1).ok_or(GateError::Exhausted)?;
        s.phase = Phase::Draining(id);
        Ok(DrainTicket { gate: self.clone(), id, armed: true })
    }
    fn check_held(&self, lease: &DrainLease) -> Result<(), GateError> {
        if !Arc::ptr_eq(&self.inner, &lease.gate.inner) { return Err(GateError::WrongGate); }
        let s = self.inner.state.lock().map_err(|_| GateError::Poisoned)?;
        if s.unconfirmed { return Err(GateError::Unconfirmed); }
        if s.phase != Phase::Held(lease.id) { return Err(GateError::StaleSeal); }
        if !s.applications.is_empty() || s.legacy_host != LegacyHostState::Absent {
            return Err(GateError::NotDrained);
        }
        Ok(())
    }
    /// Local application completion only. Never a host-close or control lease.
    pub fn check_application_drained(&self, proof: &ApplicationDrained<'_>)
        -> Result<(), GateError> {
        if !Arc::ptr_eq(&self.inner, &proof.ticket.gate.inner) {
            return Err(GateError::WrongGate);
        }
        let s = self.inner.state.lock().map_err(|_| GateError::Poisoned)?;
        Self::application_drained(&s, proof.ticket.id)
    }
    /// Transfers this seal to an irreversible retirement owner. Call under the
    /// host lifecycle/admission lock immediately before closing local admission.
    pub fn begin_host_retirement(&self, proof: &ApplicationDrained<'_>)
        -> Result<HostRetirement, GateError> {
        if !Arc::ptr_eq(&self.inner, &proof.ticket.gate.inner) {
            return Err(GateError::WrongGate);
        }
        let mut s = self.inner.state.lock().map_err(|_| GateError::Poisoned)?;
        Self::application_drained(&s, proof.ticket.id)?;
        s.phase = Phase::Retiring(proof.ticket.id);
        self.inner.changed.send_replace(());
        Ok(HostRetirement { gate: self.clone(), id: proof.ticket.id })
    }
    pub fn check_host_retirement(&self, retirement: &HostRetirement)
        -> Result<(), GateError> {
        if !Arc::ptr_eq(&self.inner, &retirement.gate.inner) {
            return Err(GateError::WrongGate);
        }
        let s = self.inner.state.lock().map_err(|_| GateError::Poisoned)?;
        if s.unconfirmed { return Err(GateError::Unconfirmed); }
        if s.phase != Phase::Retiring(retirement.id) { return Err(GateError::StaleSeal); }
        if s.legacy_host != LegacyHostState::Retained || !s.applications.is_empty() {
            return Err(GateError::NotDrained);
        }
        Ok(())
    }
    fn application_drained(s: &State, id: u64) -> Result<(), GateError> {
        if s.unconfirmed { return Err(GateError::Unconfirmed); }
        if s.phase != Phase::Draining(id) { return Err(GateError::StaleSeal); }
        if s.legacy_host != LegacyHostState::Retained || !s.applications.is_empty() {
            return Err(GateError::NotDrained);
        }
        Ok(())
    }
    fn abandon(&self, id: u64) {
        let mut s = self.inner.state.lock().unwrap_or_else(|e| e.into_inner());
        if matches!(s.phase, Phase::Draining(n) | Phase::Retiring(n) | Phase::Held(n) if n == id) {
            s.unconfirmed = true;
        }
        self.inner.changed.send_replace(());
    }
}

/// Move this owner into the request/background supervisor. Dropping an HTTP
/// response/body or a future is not affirmative completion of external work.
#[must_use = "retain through all state writes and observed external cleanup"]
pub struct OperationOwner { gate: OperationGate, id: u64, kind: OperationKind, armed: bool }
impl OperationOwner {
    pub fn kind(&self) -> OperationKind { self.kind }
    /// Only a continuation/cleanup of this already admitted operation. This
    /// cannot admit unrelated requests. Retain the parent until registration.
    pub fn register_cleanup(&self) -> Result<OperationOwner, GateError> {
        let mut s = self.gate.inner.state.lock().map_err(|_| GateError::Poisoned)?;
        if s.applications.get(&self.id) != Some(&self.kind) {
            return Err(GateError::Unconfirmed);
        }
        self.gate.reserve(&mut s, OperationKind::ExternalCleanup)
    }
    /// Caller attests actual work, state writes and external cleanup are done.
    /// This local completion is not a browser/process teardown proof by itself.
    pub fn complete(mut self) -> Result<(), GateError> {
        let mut s = self.gate.inner.state.lock().map_err(|_| GateError::Poisoned)?;
        if s.applications.get(&self.id) != Some(&self.kind) {
            s.unconfirmed = true;
            return Err(GateError::Unconfirmed);
        }
        s.applications.remove(&self.id);
        self.armed = false;
        self.gate.inner.changed.send_replace(());
        Ok(())
    }
}
impl Drop for OperationOwner {
    fn drop(&mut self) {
        if !self.armed { return; }
        let mut s = self.gate.inner.state.lock().unwrap_or_else(|e| e.into_inner());
        s.applications.remove(&self.id);
        s.unconfirmed = true;
        self.gate.inner.changed.send_replace(());
    }
}

/// Named retained host barrier. There is deliberately no completion method.
#[must_use = "retain the legacy host barrier until genuine retirement exists"]
pub struct LegacyHostOwner { gate: OperationGate }
impl Drop for LegacyHostOwner {
    fn drop(&mut self) {
        let mut s = self.gate.inner.state.lock().unwrap_or_else(|e| e.into_inner());
        s.legacy_host = LegacyHostState::Abandoned;
        s.unconfirmed = true;
        self.gate.inner.changed.send_replace(());
    }
}

#[must_use = "retain seal through drain or explicit pre-child cancellation"]
pub struct DrainTicket { gate: OperationGate, id: u64, armed: bool }
impl DrainTicket {
    /// Borrows this seal. Cancellation of the wait never cancels admitted work.
    /// The observation cannot outlive, cancel or replace its ticket.
    pub async fn wait_application(&mut self) -> Result<ApplicationDrained<'_>, GateError> {
        let mut changed = self.gate.inner.changed.subscribe();
        loop {
            let ready = {
                let s = self.gate.inner.state.lock().map_err(|_| GateError::Poisoned)?;
                match OperationGate::application_drained(&s, self.id) {
                    Ok(()) => true,
                    Err(GateError::NotDrained) if s.legacy_host == LegacyHostState::Retained => false,
                    Err(error) => return Err(error),
                }
            };
            if ready { return Ok(ApplicationDrained { ticket: self }); }
            changed.changed().await.map_err(|_| GateError::Unconfirmed)?;
        }
    }
    /// Cancellation of this wait keeps the ticket and its seal alive. A caller
    /// deadline must NOT cancel existing operations. No timeout unlock exists.
    pub async fn wait(&mut self) -> Result<DrainLease, GateError> {
        let mut changed = self.gate.inner.changed.subscribe();
        loop {
            {
                let mut s = self.gate.inner.state.lock().map_err(|_| GateError::Poisoned)?;
                if s.unconfirmed { return Err(GateError::Unconfirmed); }
                if s.phase != Phase::Draining(self.id) { return Err(GateError::StaleSeal); }
                if s.applications.is_empty() && s.legacy_host == LegacyHostState::Absent {
                    s.phase = Phase::Held(self.id);
                    self.armed = false;
                    return Ok(DrainLease { gate: self.gate.clone(), id: self.id });
                }
            }
            changed.changed().await.map_err(|_| GateError::Unconfirmed)?;
        }
    }
    /// Abandon the experiment before a DrainLease/child exists. Admitted work
    /// continues normally. This never erases uncertain operation completion.
    pub fn cancel_before_control(mut self) -> Result<(), GateError> {
        let mut s = self.gate.inner.state.lock().map_err(|_| GateError::Poisoned)?;
        if s.unconfirmed { return Err(GateError::Unconfirmed); }
        if s.phase != Phase::Draining(self.id) { return Err(GateError::StaleSeal); }
        s.phase = Phase::Open;
        self.armed = false;
        self.gate.inner.changed.send_replace(());
        Ok(())
    }
}
impl Drop for DrainTicket {
    fn drop(&mut self) {
        if !self.armed { return; }
        let mut s = self.gate.inner.state.lock().unwrap_or_else(|e| e.into_inner());
        // begin_host_retirement transferred the seal to its token. The old
        // ticket no longer owns that phase, including if cancellation failed.
        if matches!(s.phase, Phase::Draining(n) | Phase::Held(n) if n == self.id) {
            s.unconfirmed = true;
            self.gate.inner.changed.send_replace(());
        }
    }
}

/// All named application owners completed under this borrowed seal. Retained
/// host ownership remains outstanding; this cannot be used as a DrainLease.
#[must_use = "recheck the same gate before using this local observation"]
pub struct ApplicationDrained<'ticket> { ticket: &'ticket DrainTicket }

/// Owns irreversible host retirement, not successful host closure. No release,
/// completion or restoration API exists; keep it through owned local joins.
#[must_use = "dropping retirement retains sealed uncertainty"]
pub struct HostRetirement { gate: OperationGate, id: u64 }
impl Drop for HostRetirement {
    fn drop(&mut self) { self.gate.abandon(self.id); }
}

/// Proves only this gate's registered owners drained. Has no public release
/// method in this slice: the future outer controller must validate all external
/// cleanup/policy proofs before implementing release. Drop leaves a sticky seal.
#[must_use = "local drain is not global exclusion or descendant teardown"]
pub struct DrainLease { gate: OperationGate, id: u64 }
impl Drop for DrainLease {
    fn drop(&mut self) { self.gate.abandon(self.id); }
}

#[cfg(test)]
#[path = "operation_gate_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "application_drain_tests.rs"]
mod application_drain_tests;

#[cfg(test)]
#[path = "host_retirement_tests.rs"]
mod host_retirement_tests;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GuardAdmission { KnownClear, Paused(std::time::Duration) }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GuardCheckError { Gate(GateError), Unavailable }

/// Bind the actual existing guard to its actual operation gate exactly once
/// at construction wiring. This does not prove legacy paths use that gate.
/// Retains live history/pause/escalation; never constructs or resets a guard.
pub struct ControlGuard {
    gate: OperationGate,
    wave: Arc<crate::wave::WaveGuard>,
}
impl ControlGuard {
    pub fn bind(gate: OperationGate, wave: Arc<crate::wave::WaveGuard>) -> Self {
        Self { gate, wave }
    }
    pub fn check(&self, lease: &DrainLease) -> Result<GuardAdmission, GuardCheckError> {
        self.gate.check_held(lease).map_err(GuardCheckError::Gate)?;
        match self.wave.control_admission_status().map_err(|_| GuardCheckError::Unavailable)? {
            Some(remaining) => Ok(GuardAdmission::Paused(remaining)),
            None => Ok(GuardAdmission::KnownClear),
        }
    }
}

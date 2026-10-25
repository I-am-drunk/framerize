//! Process-local residency accounting. Hold a lease through the entire SSE chain.
//! Only a confirmed dedicated close may free or replace an occupied slot.
use crate::ids::AccountId;
use crate::browser_lifecycle::{BrowserAccountStatus, IdleCloseResult};
use serde::Serialize;
use std::sync::{Arc, Mutex, MutexGuard};

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum PoolError {
    #[error("resident capacity must be positive")]
    InvalidCapacity,
    #[error("capacity is below the number of occupied slots")]
    BusyShrink,
    #[error("account has active work or an unfinished close")]
    AccountBusy,
    #[error("no idle resident slot is available")]
    NoIdleSlot,
    #[error("account is not resident")]
    NotResident,
    #[error("close has already started or was not started")]
    CloseState,
    #[error("close ticket is stale")]
    StaleTicket,
    #[error("residency accounting is unavailable")]
    Unavailable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ResidentStatus { Active, Idle, Closing, Uncertain }
#[derive(Debug, Clone, Serialize)]
pub struct ResidentSnapshot {
    pub account: AccountId,
    pub users: usize,
    pub status: ResidentStatus,
    pub replacement: Option<AccountId>,
    pub close_started: bool,
    pub uncertain: bool,
}
#[derive(Debug, Clone, Serialize)]
pub struct PoolSnapshot {
    pub capacity: usize,
    pub occupied: usize,
    pub residents: Vec<ResidentSnapshot>,
}
#[derive(Clone, Copy)]
enum SlotState {
    Resident,
    Closing { id: u64, target: Option<AccountId>, started: bool, was_uncertain: bool },
    Uncertain,
}
struct Slot { account: AccountId, users: usize, idle_order: u64, state: SlotState }
struct Inner { capacity: usize, slots: Vec<Slot>, clock: u64, next_ticket: u64 }
#[derive(Clone)]
pub struct ResidentPool { inner: Arc<Mutex<Inner>> }
pub enum Acquire { Lease(ResidentLease), Evict(EvictionTicket) }
#[must_use = "retain through the full request or SSE chain"]
pub struct ResidentLease { pool: ResidentPool, account: AccountId, release_on_drop: bool }
struct CloseTicket { pool: ResidentPool, victim: AccountId, target: Option<AccountId>, id: u64, done: bool }
#[must_use = "close the victim before confirming replacement"]
pub struct EvictionTicket(CloseTicket);
#[must_use = "close the parked account before freeing its slot"]
pub struct ParkTicket(CloseTicket);

impl std::fmt::Debug for ResidentPool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResidentPool").field("snapshot", &self.snapshot()).finish()
    }
}
impl ResidentPool {
    pub fn new(capacity: usize) -> Result<Self, PoolError> {
        if capacity == 0 { return Err(PoolError::InvalidCapacity); }
        Ok(Self { inner: Arc::new(Mutex::new(Inner {
            capacity, slots: Vec::new(), clock: 0, next_ticket: 0,
        })) })
    }
    fn lock(&self) -> Result<MutexGuard<'_, Inner>, PoolError> {
        self.inner.lock().map_err(|_| PoolError::Unavailable)
    }
    pub fn capacity(&self) -> Result<usize, PoolError> { Ok(self.lock()?.capacity) }
    pub fn set_capacity(&self, capacity: usize) -> Result<(), PoolError> {
        if capacity == 0 { return Err(PoolError::InvalidCapacity); }
        let mut inner = self.lock()?;
        if capacity < inner.slots.len() { return Err(PoolError::BusyShrink); }
        inner.capacity = capacity;
        Ok(())
    }
    pub fn snapshot(&self) -> Result<PoolSnapshot, PoolError> {
        let inner = self.lock()?;
        let residents = inner.slots.iter().map(|slot| {
            let (status, replacement, close_started) = match slot.state {
                SlotState::Resident => (if slot.users == 0 { ResidentStatus::Idle }
                    else { ResidentStatus::Active }, None, false),
                SlotState::Closing { target, started, .. } => (ResidentStatus::Closing, target, started),
                SlotState::Uncertain => (ResidentStatus::Uncertain, None, false),
            };
            let uncertain = matches!(slot.state, SlotState::Uncertain |
                SlotState::Closing { was_uncertain: true, .. });
            ResidentSnapshot { account: slot.account, users: slot.users, status, replacement, close_started, uncertain }
        }).collect();
        Ok(PoolSnapshot { capacity: inner.capacity, occupied: inner.slots.len(), residents })
    }
    /// All occupied identities, including closing and uncertain slots.
    pub fn resident_ids(&self) -> Result<Vec<AccountId>, PoolError> {
        Ok(self.lock()?.slots.iter().map(|s| s.account).collect())
    }
    pub fn idle_ids(&self) -> Result<Vec<AccountId>, PoolError> {
        let inner = self.lock()?;
        let mut slots: Vec<_> = inner.slots.iter().filter(|s|
            s.users == 0 && matches!(s.state, SlotState::Resident)).collect();
        slots.sort_by_key(|s| (s.idle_order, s.account.0));
        Ok(slots.iter().map(|s| s.account).collect())
    }
    pub fn try_acquire(&self, account: AccountId, preferred_idle: Option<AccountId>)
        -> Result<Acquire, PoolError> {
        self.try_acquire_excluding(account, preferred_idle, &[])
    }
    /// Retain an idle healthy slot without allocating or selecting a victim.
    /// This is accounting only; the caller must also bind the live host objects.
    pub fn try_acquire_idle_existing(&self, account: AccountId) -> Result<ResidentLease, PoolError> {
        let mut inner = self.lock()?;
        let slot = inner.slots.iter_mut().find(|slot| slot.account == account)
            .ok_or(PoolError::NotResident)?;
        if slot.users != 0 || !matches!(slot.state, SlotState::Resident) {
            return Err(PoolError::AccountBusy);
        }
        slot.users = slot.users.checked_add(1).ok_or(PoolError::Unavailable)?;
        Ok(ResidentLease { pool: self.clone(), account, release_on_drop: true })
    }
    /// Accounts undergoing a management operation cannot be eviction victims.
    pub fn try_acquire_excluding(&self, account: AccountId, preferred_idle: Option<AccountId>,
        protected: &[AccountId]) -> Result<Acquire, PoolError> {
        let mut inner = self.lock()?;
        if inner.slots.iter().any(|s| matches!(s.state,
            SlotState::Closing { target: Some(target), .. } if target == account)) {
            return Err(PoolError::AccountBusy);
        }
        if let Some(slot) = inner.slots.iter_mut().find(|s| s.account == account) {
            if !matches!(slot.state, SlotState::Resident) { return Err(PoolError::AccountBusy); }
            slot.users = slot.users.checked_add(1).ok_or(PoolError::Unavailable)?;
            return Ok(Acquire::Lease(ResidentLease { pool: self.clone(), account, release_on_drop: true }));
        }
        if inner.slots.len() < inner.capacity {
            inner.slots.push(Slot { account, users: 1, idle_order: 0, state: SlotState::Resident });
            return Ok(Acquire::Lease(ResidentLease { pool: self.clone(), account, release_on_drop: true }));
        }
        let idle = |s: &&Slot| s.users == 0 && matches!(s.state, SlotState::Resident)
            && !protected.contains(&s.account);
        let victim = inner.slots.iter().filter(idle)
            .find(|s| Some(s.account) == preferred_idle)
            .or_else(|| inner.slots.iter().filter(idle).min_by_key(|s| (s.idle_order, s.account.0)))
            .map(|s| s.account).ok_or(PoolError::NoIdleSlot)?;
        Ok(Acquire::Evict(EvictionTicket(self.reserve(&mut inner, victim, Some(account))?)))
    }
    fn reserve(&self, inner: &mut Inner, victim: AccountId, target: Option<AccountId>)
        -> Result<CloseTicket, PoolError> {
        let id = inner.next_ticket.checked_add(1).ok_or(PoolError::Unavailable)?;
        let slot = inner.slots.iter_mut().find(|s| s.account == victim).ok_or(PoolError::NotResident)?;
        if slot.users != 0 || matches!(slot.state, SlotState::Closing { .. }) {
            return Err(PoolError::AccountBusy);
        }
        let was_uncertain = matches!(slot.state, SlotState::Uncertain);
        slot.state = SlotState::Closing { id, target, started: false, was_uncertain };
        inner.next_ticket = id;
        Ok(CloseTicket { pool: self.clone(), victim, target, id, done: false })
    }
    /// A retry may reconcile this same uncertain victim, never another account.
    pub fn begin_park(&self, account: AccountId) -> Result<ParkTicket, PoolError> {
        let mut inner = self.lock()?;
        Ok(ParkTicket(self.reserve(&mut inner, account, None)?))
    }
}
impl ResidentLease {
    pub fn account(&self) -> AccountId { self.account }
    /// Retain an unresolved user count when external cleanup is unconfirmed.
    /// Local idle-close evidence cannot release this reservation.
    pub fn retain_unconfirmed(mut self) {
        self.release_on_drop = false;
        if let Ok(mut inner) = self.pool.lock() {
            if let Some(slot) = inner.slots.iter_mut().find(|s| s.account == self.account) {
                slot.state = SlotState::Uncertain;
            }
        }
    }

    /// Linearize admission after an asynchronous browser observation.
    pub fn ensure_admissible(&self) -> Result<(), PoolError> {
        let inner = self.pool.lock()?;
        let slot = inner.slots.iter().find(|slot| slot.account == self.account)
            .ok_or(PoolError::NotResident)?;
        if slot.users > 0 && matches!(slot.state, SlotState::Resident) {
            Ok(())
        } else {
            Err(PoolError::AccountBusy)
        }
    }

    /// Retain this occupied slot after a negative browser observation.
    /// Existing users keep their leases; only a proved close can free it.
    pub fn mark_unconfirmed(&self) -> Result<(), PoolError> {
        let mut inner = self.pool.lock()?;
        let slot = inner.slots.iter_mut().find(|slot| slot.account == self.account)
            .ok_or(PoolError::NotResident)?;
        if slot.users == 0 || matches!(slot.state, SlotState::Closing { .. }) {
            return Err(PoolError::StaleTicket);
        }
        slot.state = SlotState::Uncertain;
        Ok(())
    }
}
impl Drop for ResidentLease {
    fn drop(&mut self) {
        if !self.release_on_drop { return; }
        if let Ok(mut inner) = self.pool.lock() {
            inner.clock = inner.clock.saturating_add(1);
            let order = inner.clock;
            if let Some(slot) = inner.slots.iter_mut().find(|s| s.account == self.account) {
                if slot.users > 0 { slot.users -= 1; }
                if slot.users == 0 { slot.idle_order = order; }
            }
        }
    }
}
impl CloseTicket {
    fn position(&self, inner: &Inner) -> Result<usize, PoolError> {
        inner.slots.iter().position(|s| s.account == self.victim && s.users == 0 &&
            matches!(s.state, SlotState::Closing { id, target, .. }
                if id == self.id && target == self.target)).ok_or(PoolError::StaleTicket)
    }
    fn close_started(&mut self) -> Result<(), PoolError> {
        let mut inner = self.pool.lock()?;
        let index = self.position(&inner)?;
        match &mut inner.slots[index].state {
            SlotState::Closing { started, .. } if !*started => { *started = true; Ok(()) }
            _ => Err(PoolError::CloseState),
        }
    }
    fn cancel_before_io(mut self) -> Result<(), PoolError> {
        let mut inner = self.pool.lock()?;
        let index = self.position(&inner)?;
        match inner.slots[index].state {
            SlotState::Closing { started: false, was_uncertain, .. } => {
                inner.slots[index].state = if was_uncertain { SlotState::Uncertain } else { SlotState::Resident };
                self.done = true;
                Ok(())
            }
            _ => Err(PoolError::CloseState),
        }
    }
    fn confirm_close(mut self) -> Result<Option<ResidentLease>, PoolError> {
        let mut inner = self.pool.lock()?;
        let index = self.position(&inner)?;
        if !matches!(inner.slots[index].state, SlotState::Closing { started: true, .. }) {
            return Err(PoolError::CloseState);
        }
        let lease = if let Some(account) = self.target {
            if inner.slots.iter().any(|s| s.account == account) { return Err(PoolError::AccountBusy); }
            inner.slots[index] = Slot { account, users: 1, idle_order: 0, state: SlotState::Resident };
            Some(ResidentLease { pool: self.pool.clone(), account, release_on_drop: true })
        } else {
            inner.slots.remove(index);
            None
        };
        self.done = true;
        Ok(lease)
    }
    fn confirm_busy(mut self, expected: &BrowserAccountStatus, result: &IdleCloseResult)
        -> Result<(), PoolError> {
        if expected.account_id != self.victim || !result.confirms_busy(expected) {
            return Err(PoolError::StaleTicket);
        }
        let mut inner = self.pool.lock()?;
        let index = self.position(&inner)?;
        match inner.slots[index].state {
            SlotState::Closing { started: true, was_uncertain, .. } => {
                inner.slots[index].state = if was_uncertain { SlotState::Uncertain }
                    else { SlotState::Resident };
                self.done = true;
                Ok(())
            }
            _ => Err(PoolError::CloseState),
        }
    }
}
impl Drop for CloseTicket {
    fn drop(&mut self) {
        if self.done { return; }
        if let Ok(mut inner) = self.pool.lock() {
            if let Ok(index) = self.position(&inner) { inner.slots[index].state = SlotState::Uncertain; }
        }
    }
}
impl EvictionTicket {
    pub fn victim(&self) -> AccountId { self.0.victim }
    pub fn target(&self) -> AccountId { self.0.target.expect("eviction has replacement") }
    pub fn close_started(&mut self) -> Result<(), PoolError> { self.0.close_started() }
    pub fn cancel_before_io(self) -> Result<(), PoolError> { self.0.cancel_before_io() }
    /// Restore the same occupied slot only after a matching no-close receipt.
    pub fn confirm_busy(self, expected: &BrowserAccountStatus, result: &IdleCloseResult)
        -> Result<(), PoolError> { self.0.confirm_busy(expected, result) }
    /// Call only after the dedicated close confirms this victim's host generation.
    pub fn confirm_close(self) -> Result<ResidentLease, PoolError> {
        self.0.confirm_close()?.ok_or(PoolError::StaleTicket)
    }
}
impl ParkTicket {
    pub fn victim(&self) -> AccountId { self.0.victim }
    pub fn close_started(&mut self) -> Result<(), PoolError> { self.0.close_started() }
    pub fn cancel_before_io(self) -> Result<(), PoolError> { self.0.cancel_before_io() }
    /// A refused close does not free capacity or clear existing uncertainty.
    pub fn confirm_busy(self, expected: &BrowserAccountStatus, result: &IdleCloseResult)
        -> Result<(), PoolError> { self.0.confirm_busy(expected, result) }
    /// A fresh close of this exact victim may also reconcile an uncertain slot.
    pub fn confirm_close(self) -> Result<(), PoolError> {
        match self.0.confirm_close()? {
            None => Ok(()),
            Some(_) => Err(PoolError::StaleTicket),
        }
    }
}

#[cfg(test)]
#[path = "resident_pool_tests.rs"]
mod tests;

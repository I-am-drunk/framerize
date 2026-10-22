use super::{AccountId, ChangeResult, LeaseError as E, LeaseLimits, LeaseStats};
use std::sync::{Mutex, MutexGuard};
use tokio::sync::watch;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Access { Open, Closed, Poisoned }
struct Account {
    id: AccountId, user: Option<u64>, version: u64,
    operation: Option<u64>, access: Access,
}
struct User {
    key: u64, text: Box<str>, access: Access,
    fence: Option<(u64, bool)>,
}
#[derive(Clone, Copy)]
enum Kind { Operation { account: AccountId, version: u64 }, Change { account: AccountId, version: Option<u64> } }
impl Kind {
    fn account(self) -> AccountId {
        match self { Self::Operation { account, .. } | Self::Change { account, .. } => account }
    }
}
struct Waiter { id: u64, kind: Kind }
struct Lifecycle { id: u64, primary: AccountId, issued: bool }
struct State {
    accounts: Vec<Account>, users: Vec<User>, waiters: Vec<Waiter>,
    updates: Vec<(AccountId, Option<u64>)>, lifecycle: Option<Lifecycle>, next: u64,
}
pub(super) struct Inner {
    limits: LeaseLimits, state: Mutex<State>, changed: watch::Sender<()>,
}
impl State {
    fn token(&mut self) -> Result<u64, E> {
        self.next = self.next.checked_add(1).ok_or(E::IdentifierExhausted)?;
        Ok(self.next)
    }
    fn account(&self, id: AccountId) -> Result<usize, E> {
        self.accounts.iter().position(|a| a.id == id).ok_or(E::UnknownAccount)
    }
    fn user(&self, key: u64) -> Result<usize, E> {
        self.users.iter().position(|u| u.key == key).ok_or(E::Poisoned)
    }
    fn owner(&self, id: u64) -> Result<&Lifecycle, E> {
        self.lifecycle.as_ref().filter(|l| l.id == id && l.issued).ok_or(E::NotOwned)
    }
    fn poisoned(&self, a: &Account) -> bool {
        a.access == Access::Poisoned || a.user.is_some_and(|key| {
            self.user(key).map_or(true, |i| self.users[i].access == Access::Poisoned)
        })
    }
    fn fenced(&self, a: &Account, id: u64) -> bool {
        self.lifecycle.as_ref().is_some_and(|l| l.id == id && l.primary == a.id)
            || a.user.is_some_and(|key| self.user(key).is_ok_and(|i| {
                self.users[i].fence.is_some_and(|(owner, _)| owner == id)
            }))
    }
    fn compact(&mut self) {
        self.accounts.retain(|a| a.user.is_some() || a.operation.is_some()
            || a.access != Access::Open
            || self.waiters.iter().any(|w| w.kind.account() == a.id)
            || self.lifecycle.as_ref().is_some_and(|l| l.primary == a.id)
            || self.updates.iter().any(|(id, _)| *id == a.id));
        self.users.retain(|u| u.access != Access::Open || u.fence.is_some()
            || self.accounts.iter().any(|a| a.user == Some(u.key))
            || self.updates.iter().any(|(_, key)| *key == Some(u.key)));
    }
}

impl Inner {
    pub fn new(limits: LeaseLimits) -> Result<Self, E> {
        if limits.accounts == 0 || limits.users == 0 || limits.waiters == 0
            || limits.user_id_bytes == 0 || limits.user_id_bytes > limits.total_user_bytes {
            return Err(E::InvalidLimits);
        }
        let mut state = State { accounts: Vec::new(), users: Vec::new(),
            waiters: Vec::new(), updates: Vec::new(), lifecycle: None, next: 0 };
        state.accounts.try_reserve_exact(limits.accounts).map_err(|_| E::InvalidLimits)?;
        state.users.try_reserve_exact(limits.users).map_err(|_| E::InvalidLimits)?;
        state.waiters.try_reserve_exact(limits.waiters).map_err(|_| E::InvalidLimits)?;
        state.updates.try_reserve_exact(limits.accounts).map_err(|_| E::InvalidLimits)?;
        let (changed, _) = watch::channel(());
        Ok(Self { limits, state: Mutex::new(state), changed })
    }
    fn lock(&self) -> Result<MutexGuard<'_, State>, E> {
        self.state.lock().map_err(|_| E::Poisoned)
    }
    pub fn subscribe(&self) -> watch::Receiver<()> { self.changed.subscribe() }
    fn notify(&self) { self.changed.send_replace(()); }
    fn validate_user(&self, user: &str) -> Result<(), E> {
        if user.is_empty() || user.len() > self.limits.user_id_bytes {
            Err(E::InvalidUser)
        } else { Ok(()) }
    }
    fn queue(&self, s: &mut State, kind: Kind) -> Result<u64, E> {
        if s.waiters.len() == self.limits.waiters { return Err(E::QueueFull); }
        let id = s.token()?;
        s.waiters.push(Waiter { id, kind });
        Ok(id)
    }
    pub fn reserve_operation(&self, account: AccountId, expected: &str) -> Result<u64, E> {
        self.validate_user(expected)?;
        let mut s = self.lock()?;
        let a = &s.accounts[s.account(account)?];
        let user = &s.users[s.user(a.user.ok_or(E::UnknownAccount)?)?];
        if user.text.as_ref() != expected { return Err(E::BindingMismatch); }
        if s.poisoned(a) { return Err(E::Poisoned); }
        if a.access == Access::Closed || user.access == Access::Closed { return Err(E::Closed); }
        let kind = Kind::Operation { account, version: a.version };
        self.queue(&mut s, kind)
    }
    pub fn reserve_change(&self, account: AccountId) -> Result<u64, E> {
        let mut s = self.lock()?;
        let version = if let Ok(i) = s.account(account) {
            if s.poisoned(&s.accounts[i]) { return Err(E::Poisoned); }
            Some(s.accounts[i].version)
        } else { None };
        self.queue(&mut s, Kind::Change { account, version })
    }
    pub fn poll_operation(&self, id: u64) -> Result<Option<AccountId>, E> {
        let mut s = self.lock()?;
        let wi = s.waiters.iter().position(|w| w.id == id).ok_or(E::NotOwned)?;
        let Kind::Operation { account, version } = s.waiters[wi].kind else { return Err(E::NotOwned); };
        let ai = s.account(account)?;
        let a = &s.accounts[ai];
        if a.version != version { return Err(E::BindingMismatch); }
        let user = &s.users[s.user(a.user.ok_or(E::UnknownAccount)?)?];
        if s.poisoned(a) { return Err(E::Poisoned); }
        if a.access == Access::Closed || user.access == Access::Closed { return Err(E::Closed); }
        let earlier = s.waiters[..wi].iter().any(|w| matches!(w.kind,
            Kind::Operation { account: old, version: v } if old == account && v == version));
        let fenced = s.lifecycle.as_ref().is_some_and(|l| s.fenced(a, l.id));
        if a.operation.is_some() || earlier || fenced { return Ok(None); }
        s.accounts[ai].operation = Some(id);
        s.waiters.remove(wi);
        drop(s);
        self.notify();
        Ok(Some(account))
    }
    pub fn poll_change(&self, id: u64) -> Result<Option<AccountId>, E> {
        let mut s = self.lock()?;
        let wi = s.waiters.iter().position(|w| w.id == id).ok_or(E::NotOwned)?;
        let Kind::Change { account, version } = s.waiters[wi].kind else { return Err(E::NotOwned); };
        if let Some(l) = &s.lifecycle {
            if l.id != id { return Ok(None); }
        } else {
            let first = s.waiters.iter().find(|w| matches!(w.kind, Kind::Change { .. }));
            if !first.is_some_and(|w| w.id == id) { return Ok(None); }
            // A queued lifecycle request must not act on a replacement binding.
            let current = s.account(account).ok().map(|i| s.accounts[i].version);
            if current != version { return Err(E::BindingMismatch); }
            if current.is_none() {
                if s.accounts.len() == self.limits.accounts { return Err(E::AccountLimit); }
                s.accounts.push(Account { id: account, user: None, version: id,
                    operation: None, access: Access::Open });
            }
            s.lifecycle = Some(Lifecycle { id, primary: account, issued: false });
        }
        let a = &s.accounts[s.account(account)?];
        if s.poisoned(a) { return Err(E::Poisoned); }
        if a.operation.is_some() { return Ok(None); }
        s.lifecycle.as_mut().ok_or(E::NotOwned)?.issued = true;
        s.waiters.remove(wi);
        drop(s);
        self.notify();
        Ok(Some(account))
    }
    pub fn cancel_waiter(&self, id: u64) {
        if let Ok(mut s) = self.lock() {
            s.waiters.retain(|w| w.id != id);
            if s.lifecycle.as_ref().is_some_and(|l| l.id == id && !l.issued) {
                s.lifecycle = None;
                s.updates.clear();
            }
            s.compact();
        }
        self.notify();
    }
    pub fn finish_operation(&self, id: u64, account: AccountId, clean: bool) -> Result<(), E> {
        let mut s = self.lock()?;
        let ai = s.account(account)?;
        let a = &mut s.accounts[ai];
        if a.operation != Some(id) { return Err(E::NotOwned); }
        a.operation = None;
        if !clean { a.access = Access::Poisoned; }
        // Existing lifecycle closure/poison is never cleared by this receipt.
        s.compact();
        drop(s);
        self.notify();
        Ok(())
    }
    pub fn fence_user(&self, id: u64, user: &str) -> Result<u64, E> {
        self.validate_user(user)?;
        let mut s = self.lock()?;
        s.owner(id)?;
        let ui = if let Some(i) = s.users.iter().position(|u| u.text.as_ref() == user) { i }
        else {
            if s.users.len() == self.limits.users { return Err(E::UserLimit); }
            let bytes = s.users.iter().try_fold(user.len(), |n, u| n.checked_add(u.text.len()))
                .ok_or(E::UserBytesLimit)?;
            if bytes > self.limits.total_user_bytes { return Err(E::UserBytesLimit); }
            let key = s.token()?;
            s.users.push(User { key, text: user.into(), access: Access::Open, fence: None });
            s.users.len() - 1
        };
        let u = &mut s.users[ui];
        if u.access == Access::Poisoned { return Err(E::Poisoned); }
        // Re-requesting an already completed fence preserves that evidence.
        if u.fence.is_none() { u.fence = Some((id, false)); }
        if !u.fence.is_some_and(|(owner, _)| owner == id) { return Err(E::NotOwned); }
        let key = u.key;
        drop(s);
        self.notify();
        Ok(key)
    }
    pub fn poll_user(&self, id: u64, key: u64) -> Result<bool, E> {
        let mut s = self.lock()?;
        s.owner(id)?;
        let ui = s.user(key)?;
        if !s.users[ui].fence.is_some_and(|(owner, _)| owner == id) { return Err(E::NotOwned); }
        for a in s.accounts.iter().filter(|a| a.user == Some(key)) {
            if s.poisoned(a) { return Err(E::Poisoned); }
            if a.operation.is_some() { return Ok(false); }
        }
        s.users[ui].fence = Some((id, true));
        Ok(true)
    }
    pub fn fenced_accounts(&self, id: u64) -> Result<Vec<AccountId>, E> {
        let s = self.lock()?;
        s.owner(id)?;
        if !s.drained(id) { return Err(E::NotDrained); }
        Ok(s.accounts.iter().filter(|a| s.fenced(a, id)).map(|a| a.id).collect())
    }
    pub fn stage_binding(&self, id: u64, account: AccountId, user: Option<&str>) -> Result<(), E> {
        if let Some(user) = user { self.validate_user(user)?; }
        let mut s = self.lock()?;
        s.owner(id)?;
        if !s.drained(id) { return Err(E::NotDrained); }
        let a = &s.accounts[s.account(account)?];
        if !s.fenced(a, id) { return Err(E::NotOwned); }
        let key = match user {
            None => None,
            Some(user) => {
                let u = s.users.iter().find(|u| u.text.as_ref() == user).ok_or(E::NotOwned)?;
                if u.fence != Some((id, true)) { return Err(E::NotOwned); }
                Some(u.key)
            }
        };
        if let Some(update) = s.updates.iter_mut().find(|(old, _)| *old == account) {
            update.1 = key;
        } else {
            if s.updates.len() == self.limits.accounts { return Err(E::AccountLimit); }
            s.updates.push((account, key));
        }
        Ok(())
    }
    pub fn cancel_unstarted(&self, id: u64) -> Result<(), E> {
        let mut s = self.lock()?;
        s.owner(id)?;
        if !s.updates.is_empty()
            || s.users.iter().any(|u| u.fence.is_some_and(|(owner, _)| owner == id)) {
            return Err(E::NotOwned);
        }
        s.lifecycle = None;
        s.compact();
        drop(s);
        self.notify();
        Ok(())
    }
    pub fn finish_change(&self, id: u64, requested: ChangeResult) -> Result<(), E> {
        let mut s = self.lock()?;
        let primary = s.owner(id)?.primary;
        let invalid = requested == ChangeResult::Completed && !s.drained(id);
        let result = if invalid { ChangeResult::Unconfirmed } else { requested };
        // No await or user callback occurs between mapping updates and release.
        for index in 0..s.accounts.len() {
            let account = s.accounts[index].id;
            let update = s.updates.iter().find(|(old, _)| *old == account).map(|(_, key)| *key);
            if account == primary || update.is_some() {
                let a = &mut s.accounts[index];
                a.access = match result {
                    ChangeResult::Completed => Access::Open,
                    ChangeResult::RetryableFailure => Access::Closed,
                    ChangeResult::Unconfirmed => Access::Poisoned,
                };
                if result == ChangeResult::Completed {
                    if let Some(user) = update { a.user = user; }
                    a.version = id;
                }
            }
        }
        for u in &mut s.users {
            if u.fence.is_some_and(|(owner, _)| owner == id) {
                u.access = match result {
                    ChangeResult::Completed => Access::Open,
                    ChangeResult::RetryableFailure => Access::Closed,
                    ChangeResult::Unconfirmed => Access::Poisoned,
                };
                u.fence = None;
            }
        }
        s.updates.clear();
        s.lifecycle = None;
        s.compact();
        drop(s);
        self.notify();
        if invalid { Err(E::NotDrained) } else { Ok(()) }
    }
    pub fn stats(&self) -> Result<LeaseStats, E> {
        let s = self.lock()?;
        Ok(LeaseStats {
            accounts: s.accounts.len(), users: s.users.len(),
            user_bytes: s.users.iter().map(|u| u.text.len()).sum(),
            waiters: s.waiters.len(),
            operations: s.accounts.iter().filter(|a| a.operation.is_some()).count(),
            lifecycle_active: s.lifecycle.is_some(),
            closed_accounts: s.accounts.iter().filter(|a| a.access == Access::Closed).count(),
            poisoned_accounts: s.accounts.iter().filter(|a| a.access == Access::Poisoned).count(),
            closed_users: s.users.iter().filter(|u| u.access == Access::Closed).count(),
            poisoned_users: s.users.iter().filter(|u| u.access == Access::Poisoned).count(),
        })
    }
}
impl State {
    fn drained(&self, id: u64) -> bool {
        self.users.iter().filter(|u| u.fence.is_some_and(|(owner, _)| owner == id))
            .all(|u| u.fence == Some((id, true)) && u.access != Access::Poisoned)
            && self.accounts.iter().filter(|a| self.fenced(a, id))
                .all(|a| a.operation.is_none() && !self.poisoned(a))
    }
}

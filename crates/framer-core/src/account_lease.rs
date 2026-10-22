//! Shared ownership for legacy and native account work. This module does not
//! perform transport cleanup. Callers must retain each lease until real cleanup
//! completes. Dropping issued ownership is unconfirmed and fails closed.
use crate::ids::AccountId;
use std::{future::Future, sync::Arc};

#[path = "account_lease_state.rs"]
mod state;
use state::Inner;

#[derive(Clone, Copy, Debug)]
pub struct LeaseLimits {
    pub accounts: usize,
    pub users: usize,
    pub waiters: usize,
    pub user_id_bytes: usize,
    pub total_user_bytes: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LeaseError {
    InvalidLimits, InvalidUser, AccountLimit, UserLimit, UserBytesLimit,
    QueueFull, UnknownAccount, BindingMismatch, Closed, Poisoned,
    NotOwned, NotDrained, IdentifierExhausted,
}
impl std::fmt::Display for LeaseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "account lease: {self:?}")
    }
}
impl std::error::Error for LeaseError {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChangeResult {
    /// External work, credential save, publication and cleanup all completed.
    Completed,
    /// Cleanup is confirmed, but state still needs explicit reconciliation.
    RetryableFailure,
    /// Cleanup is missing, failed or otherwise unconfirmed. Never auto-reopen.
    Unconfirmed,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LeaseStats {
    pub accounts: usize,
    pub users: usize,
    pub user_bytes: usize,
    pub waiters: usize,
    pub operations: usize,
    pub lifecycle_active: bool,
    pub closed_accounts: usize,
    pub poisoned_accounts: usize,
    pub closed_users: usize,
    pub poisoned_users: usize,
}

/// Construct once and share across both transports and background refresh.
/// No account bindings exist until an owned lifecycle change commits them.
#[derive(Clone)]
pub struct AccountLeaseCoordinator { inner: Arc<Inner> }

struct Pending { inner: Arc<Inner>, id: u64, armed: bool }
impl Pending {
    // Call a method so the async block captures the entire cancellation guard.
    fn disarm(&mut self) { self.armed = false; }
}
impl Drop for Pending {
    fn drop(&mut self) {
        if self.armed { self.inner.cancel_waiter(self.id); }
    }
}

impl AccountLeaseCoordinator {
    pub fn new(limits: LeaseLimits) -> Result<Self, LeaseError> {
        Ok(Self { inner: Arc::new(Inner::new(limits)?) })
    }
    pub fn stats(&self) -> Result<LeaseStats, LeaseError> { self.inner.stats() }

    /// Reserve a bounded waiter synchronously. An unpolled or cancelled wait
    /// releases its slot. The selected user's binding is checked again on grant.
    pub fn operation(&self, account: AccountId, expected_user: &str)
        -> Result<impl Future<Output = Result<OperationLease, LeaseError>> + Send + 'static, LeaseError> {
        let id = self.inner.reserve_operation(account, expected_user)?;
        let inner = self.inner.clone();
        let mut pending = Pending { inner: inner.clone(), id, armed: true };
        let mut changed = inner.subscribe();
        Ok(async move {
            loop {
                if let Some(account) = inner.poll_operation(id)? {
                    pending.disarm();
                    return Ok(OperationLease { inner, id, account, armed: true });
                }
                changed.changed().await.map_err(|_| LeaseError::Poisoned)?;
            }
        })
    }

    /// A single FIFO lifecycle queue prevents cross-login expansion deadlock.
    /// Queued changes hold no accounts. The head fences its primary account
    /// before waiting for existing work, then yields permission for side effects.
    pub fn begin_change(&self, account: AccountId)
        -> Result<impl Future<Output = Result<AccountChange, LeaseError>> + Send + 'static, LeaseError> {
        let id = self.inner.reserve_change(account)?;
        let inner = self.inner.clone();
        let mut pending = Pending { inner: inner.clone(), id, armed: true };
        let mut changed = inner.subscribe();
        Ok(async move {
            loop {
                if let Some(account) = inner.poll_change(id)? {
                    pending.disarm();
                    return Ok(AccountChange { inner, id, account, armed: true });
                }
                changed.changed().await.map_err(|_| LeaseError::Poisoned)?;
            }
        })
    }
}

#[must_use = "retain until real transport and authorization cleanup completes"]
pub struct OperationLease {
    inner: Arc<Inner>, id: u64, account: AccountId, armed: bool,
}
impl OperationLease {
    pub fn account(&self) -> AccountId { self.account }
    /// Call only on affirmative cleanup evidence, including error-side work.
    pub fn complete(mut self) -> Result<(), LeaseError> {
        let result = self.inner.finish_operation(self.id, self.account, true);
        self.armed = false;
        result
    }
}
impl Drop for OperationLease {
    fn drop(&mut self) {
        if self.armed { let _ = self.inner.finish_operation(self.id, self.account, false); }
    }
}

#[must_use = "an abandoned change poisons its account and target users"]
pub struct AccountChange {
    inner: Arc<Inner>, id: u64, account: AccountId, armed: bool,
}
impl AccountChange {
    pub fn account(&self) -> AccountId { self.account }

    /// Release a change only when its downstream admission proves that no
    /// mutation began. Preserve the previous binding, version and availability.
    /// No user fence or staged update may have been installed.
    pub fn cancel_unstarted(mut self) -> Result<(), LeaseError> {
        let result = self.inner.cancel_unstarted(self.id);
        if result.is_ok() { self.armed = false; }
        result
    }

    /// Call after discovering the login user, before any credential save.
    /// Fences every current account for that user atomically. Dropping this
    /// wait leaves the fence unready; it cannot be bypassed by successful finish.
    pub fn fence_user(&mut self, user: &str)
        -> Result<impl Future<Output = Result<(), LeaseError>> + Send + '_, LeaseError> {
        let key = self.inner.fence_user(self.id, user)?;
        let mut changed = self.inner.subscribe();
        Ok(async move {
            loop {
                if self.inner.poll_user(self.id, key)? { return Ok(()); }
                changed.changed().await.map_err(|_| LeaseError::Poisoned)?;
            }
        })
    }

    /// Includes the primary account and all accounts covered by user fences.
    /// Useful for invalidating account-keyed caches after the fences drain.
    pub fn fenced_accounts(&self) -> Result<Vec<AccountId>, LeaseError> {
        self.inner.fenced_accounts(self.id)
    }

    /// Stage binding/rebinding or removal. The account must be owned and
    /// drained; a new user must have a completed fence. Updates apply together
    /// on Completed, under the same mutex as admission and fence release.
    /// Staging an unchanged binding explicitly reconciles that account too.
    pub fn stage_binding(&mut self, account: AccountId, user: Option<&str>)
        -> Result<(), LeaseError> {
        self.inner.stage_binding(self.id, account, user)
    }

    /// Completed requires every selected fence to have finished draining.
    /// Trying to bypass an unfinished fence poisons the change and returns
    /// NotDrained. RetryableFailure discards staged bindings and keeps affected
    /// selection closed; only a later owned change can reconcile it.
    pub fn finish(mut self, result: ChangeResult) -> Result<(), LeaseError> {
        let outcome = self.inner.finish_change(self.id, result);
        self.armed = false;
        outcome
    }
}
impl Drop for AccountChange {
    fn drop(&mut self) {
        if self.armed { let _ = self.inner.finish_change(self.id, ChangeResult::Unconfirmed); }
    }
}

#[cfg(test)]
#[path = "account_lease_tests.rs"]
mod tests;

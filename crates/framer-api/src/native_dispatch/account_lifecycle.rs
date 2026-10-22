//! Own changes across shared account leases and native session generations.
use crate::experimental_native::session_store::{
    AccountChangeResult, ChangeOutcome, ChangingAccount, ChangingSession, SessionStoreBridge,
};
use framer_core::account_lease::{AccountChange, AccountLeaseCoordinator, ChangeResult};
use framer_core::ids::{AccountId, FramerUserId};
use futures::{future::{BoxFuture, Shared}, stream::FuturesUnordered, FutureExt, StreamExt};
use std::{panic::AssertUnwindSafe, sync::{Arc, Mutex, MutexGuard}};
use tokio::sync::{mpsc, oneshot, Semaphore};

type Job = BoxFuture<'static, ()>;
type Closed = Shared<BoxFuture<'static, Result<(), &'static str>>>;
type Invalidate = dyn Fn(&AccountId) -> Result<(), String> + Send + Sync;
const MAX_CHANGES: usize = 10;

#[cfg(test)]
#[path = "account_lifecycle_tests.rs"]
mod tests;

/// Rejection keeps selection closed until explicit reconciliation. Use
/// Unconfirmed whenever external work or a private write lacks cleanup proof.
pub enum MutationOutcome<T> {
    Completed(T),
    Rejected(String),
    Unconfirmed,
}

struct Worker(tokio::task::JoinHandle<()>);
impl Drop for Worker {
    fn drop(&mut self) { self.0.abort(); }
}

struct Inner {
    leases: AccountLeaseCoordinator,
    sessions: SessionStoreBridge,
    invalidate: Arc<Invalidate>,
    sender: Mutex<Option<mpsc::Sender<Job>>>,
    slots: Arc<Semaphore>,
    closed: Closed,
}
impl Drop for Inner {
    fn drop(&mut self) { lock(&self.sender).take(); }
}
fn lock<T>(value: &Mutex<T>) -> MutexGuard<'_, T> {
    value.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

async fn run_change<T, F>(sessions: SessionStoreBridge, invalidate: Arc<Invalidate>,
    account: AccountId, lease: AccountChange, update: F) -> Result<T, String>
where T: Send + 'static,
    F: for<'a, 'b> FnOnce(&'a mut Mutation<'b>) -> BoxFuture<'a, MutationOutcome<T>>
        + Send + 'static,
{
    let shared = Arc::new(tokio::sync::Mutex::new(Some(lease)));
    let callback_lease = shared.clone();
    let report = sessions.change_account_observed(account, move |native| {
        async move {
            let mut held = callback_lease.lock().await;
            let mut access = Mutation { lease: held.as_mut().expect("owned change"), native,
                invalidate, user: None, staged: false, failed: false };
            let outcome = match (access.invalidate)(&account) {
                Ok(()) => update(&mut access).await,
                Err(_) => MutationOutcome::Rejected("authorization invalidation failed".into()),
            };
            match outcome {
                MutationOutcome::Completed(value) if access.staged && !access.failed =>
                    ChangeOutcome::finished(Ok(value)),
                MutationOutcome::Unconfirmed => ChangeOutcome::unconfirmed(),
                MutationOutcome::Rejected(error) => ChangeOutcome::finished(Err(error)),
                _ => ChangeOutcome::finished(Err("account mutation has no completed binding".into())),
            }
        }.boxed()
    }).await;
    let lease = shared.lock().await.take().expect("owned change");
    finish_shared_change(lease, report)
}

fn finish_shared_change<T>(lease: AccountChange, report: AccountChangeResult<T>) -> Result<T, String> {
    let (completion, value) = match report {
        AccountChangeResult::NotStarted(error) => {
            lease.cancel_unstarted().map_err(|_| "unstarted account release failed".to_owned())?;
            return Err(error);
        }
        AccountChangeResult::Finished(Ok(value)) => (ChangeResult::Completed, Ok(value)),
        AccountChangeResult::Finished(Err(error)) => (ChangeResult::RetryableFailure, Err(error)),
        AccountChangeResult::Unconfirmed(error) => (ChangeResult::Unconfirmed, Err(error)),
    };
    // Native claims and private work have now settled. Never release this
    // ownership from inside the callback, including a callback error.
    lease.finish(completion).map_err(|_| "shared account completion failed".to_owned())?;
    value
}

#[derive(Clone)]
pub struct AccountLifecycle { inner: Arc<Inner> }

/// A callback receives this only after shared traffic and old native work drain.
/// Save and publish through the same callback, then explicitly stage the binding.
pub struct Mutation<'a> {
    lease: &'a mut AccountChange,
    native: &'a mut ChangingAccount,
    invalidate: Arc<Invalidate>,
    user: Option<FramerUserId>,
    staged: bool,
    failed: bool,
}

impl Mutation<'_> {
    pub fn account(&self) -> AccountId { self.native.account() }

    /// Hold every shared account for this user before the native user barrier
    /// or private store is touched. Repeated calls can use only the same user.
    pub async fn session(&mut self, user: FramerUserId) -> Result<&ChangingSession, String> {
        let previous_failure = self.failed;
        self.failed = true;
        if self.user.as_ref().is_some_and(|current| current != &user) {
            return Err("account mutation already bound to another user".to_owned());
        }
        let lease = &mut self.lease;
        lease.fence_user(&user.0)
            .map_err(|_| "shared user fence unavailable".to_owned())?.await
            .map_err(|_| "shared user cleanup failed".to_owned())?;
        let affected = lease.fenced_accounts()
            .map_err(|_| "shared user ownership failed".to_owned())?;
        let access = self.native.session(user.clone()).await?;
        for account in affected { (self.invalidate)(&account)?; }
        self.user = Some(user);
        self.failed = previous_failure;
        Ok(access)
    }

    /// Call after the private save and StateStore publication both complete.
    pub fn stage_binding(&mut self) -> Result<(), String> {
        let previous_failure = self.failed;
        self.failed = true;
        let user = self.user.as_ref()
            .ok_or_else(|| "account mutation has no prepared user".to_owned())?;
        let account = self.account();
        self.lease.stage_binding(account, Some(&user.0))
            .map_err(|_| "shared account binding failed".to_owned())?;
        self.staged = true;
        self.failed = previous_failure;
        Ok(())
    }

    /// Keep shared credentials for any other account; remove only this binding.
    pub fn stage_removal(&mut self) -> Result<(), String> {
        let previous_failure = self.failed;
        self.failed = true;
        let account = self.account();
        self.lease.stage_binding(account, None)
            .map_err(|_| "shared account removal failed".to_owned())?;
        self.staged = true;
        self.failed = previous_failure;
        Ok(())
    }
}

impl AccountLifecycle {
    /// Share these exact instances with both request paths. The invalidator
    /// clears legacy account-keyed authorization only after affected work drains.
    pub fn new(leases: AccountLeaseCoordinator, sessions: SessionStoreBridge,
        invalidate: Arc<Invalidate>, capacity: usize) -> Result<Self, &'static str> {
        if capacity == 0 || capacity > MAX_CHANGES {
            return Err("invalid account lifecycle capacity");
        }
        let runtime = tokio::runtime::Handle::try_current()
            .map_err(|_| "account lifecycle runtime unavailable")?;
        let (sender, mut receiver) = mpsc::channel::<Job>(capacity);
        let (finished, completion) = oneshot::channel();
        let worker = Worker(runtime.spawn(async move {
            let mut jobs = FuturesUnordered::new();
            let mut receiving = true;
            while receiving || !jobs.is_empty() {
                tokio::select! {
                    job = receiver.recv(), if receiving => match job {
                        Some(job) => jobs.push(job),
                        None => receiving = false,
                    },
                    _ = jobs.next(), if !jobs.is_empty() => {},
                }
            }
            let _ = finished.send(());
        }));
        let closed = async move {
            let _worker = worker;
            completion.await.map_err(|_| "account lifecycle worker stopped")
        }.boxed().shared();
        Ok(Self { inner: Arc::new(Inner { leases, sessions, invalidate,
            sender: Mutex::new(Some(sender)), slots: Arc::new(Semaphore::new(capacity)),
            closed }) })
    }

    /// Stop admission and join every already admitted change.
    pub async fn shutdown(&self) -> Result<(), &'static str> {
        lock(&self.inner.sender).take();
        self.inner.closed.clone().await
    }

    /// Admission is bounded before any work starts. Once admitted, losing the
    /// waiter never cancels a private write or releases a running account fence.
    pub async fn change<T, F>(&self, account: AccountId, update: F) -> Result<T, String>
    where T: Send + 'static,
        F: for<'a, 'b> FnOnce(&'a mut Mutation<'b>) -> BoxFuture<'a, MutationOutcome<T>>
            + Send + 'static,
    {
        let slot = self.inner.slots.clone().try_acquire_owned()
            .map_err(|_| "account lifecycle capacity exhausted".to_owned())?;
        let waiting = self.inner.leases.begin_change(account)
            .map_err(|_| "account lifecycle admission failed".to_owned())?;
        let sessions = self.inner.sessions.clone();
        let invalidate = self.inner.invalidate.clone();
        let (sender, receiver) = oneshot::channel();
        let job = async move {
            let _slot = slot;
            let work = async move {
                let lease = waiting.await
                    .map_err(|_| "account lifecycle drain failed".to_owned())?;
                run_change(sessions, invalidate, account, lease, update).await
            };
            let result = AssertUnwindSafe(work).catch_unwind().await
                .unwrap_or_else(|_| Err("account lifecycle worker failed".to_owned()));
            let _ = sender.send(result);
        }.boxed();
        lock(&self.inner.sender).as_ref()
            .ok_or_else(|| "account lifecycle is closed".to_owned())?
            .try_send(job).map_err(|_| "account lifecycle queue unavailable".to_owned())?;
        receiver.await.map_err(|_| "account lifecycle worker stopped".to_owned())?
    }
}

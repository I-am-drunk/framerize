//! Bounded private-store bridge. All service loads, opens and mutations must
//! use the same instance. No automatic dispatch selection is performed.
use crate::account::client::AccountChange;
use crate::account::client::AccountScope;
use crate::account::client::NativeAccountClient;
use crate::account::client::SessionChange;
use crate::account::client::CAPACITY;
use crate::transport::http::NativeEgress;
use crate::transport::http::NativeHttpLimits;
use framer_core::ids::AccountId;
use framer_core::ids::FramerUserId;
use framer_core::port::Session;
use framer_core::port::SessionStore;
use futures::future::BoxFuture;
use futures::stream::FuturesUnordered;
use futures::FutureExt;
use futures::StreamExt;
use std::sync::Arc;
use std::sync::Mutex;
use tokio::sync::mpsc;
use tokio::sync::oneshot;
use tokio::sync::Semaphore;
use tokio::task::JoinHandle;

type Job = BoxFuture<'static, ()>;
type Closed = futures::future::Shared<BoxFuture<'static, Result<(), &'static str>>>;
const MAX_CREDENTIAL_BYTES: usize = 64 * 1024;
const STOPPED: &str = "native session store unavailable";
const CHANGED: &str = "native session changed during read";

#[path = "session_versions.rs"]
mod versions;
use versions::Key;
use versions::Stamp;
use versions::Versions;

#[path = "session_leases.rs"]
mod leases;
use leases::LeaseCache;
pub(crate) use leases::AccountLease;

#[path = "session_account_change.rs"]
mod account_change;
pub use account_change::AccountChangeResult;
pub use account_change::ChangingAccount;

/// Unconfirmed external work poisons the barrier against later mutation.
pub struct ChangeOutcome<T> { value: Result<T, String>, cleanup_confirmed: bool }
impl<T> ChangeOutcome<T> {
    pub fn finished(value: Result<T, String>) -> Self {
        Self { value, cleanup_confirmed: true }
    }
    pub fn unconfirmed() -> Self {
        Self { value: Err("session cleanup unconfirmed".to_owned()), cleanup_confirmed: false }
    }
}

/// Private store access for one user while its generation barrier is held.
pub struct ChangingSession { user: FramerUserId, store: Arc<dyn SessionStore> }
impl ChangingSession {
    pub fn user(&self) -> &FramerUserId { &self.user }
    pub async fn save(&self, session: &Session) -> Result<(), String> {
        if session.active_user != self.user || session.cookie_header.is_empty()
            || session.cookie_header.len() > MAX_CREDENTIAL_BYTES {
            return Err("invalid native session".to_owned());
        }
        self.store.save(session).await.map_err(|_| "session mutation failed".to_owned())
    }
    pub async fn delete(&self) -> Result<(), String> {
        self.store.delete(&self.user).await.map_err(|_| "session mutation failed".to_owned())
    }
}

struct Admission {
    versions: Versions,
    leases: LeaseCache,
    sender: Option<mpsc::Sender<Job>>,
}
impl Admission {
    fn user_change(&mut self, native: &NativeAccountClient, user: &FramerUserId)
        -> Result<(Stamp, SessionChange), String> {
        validate_user(user)?;
        self.versions.change(Key::User(user.clone()), || native.begin_session_change(user)
            .map_err(|_| "native session change unavailable".to_owned()))
    }
    fn account_change(&mut self, native: &NativeAccountClient, account: AccountId)
        -> Result<(Stamp, AccountChange), String> {
        self.versions.change(Key::Account(account), || native.begin_account_change(account)
            .map_err(|_| "native account change unavailable".to_owned()))
    }
}

fn validate_user(user: &FramerUserId) -> Result<(), String> {
    if user.0.is_empty() || user.0.len() > 256 {
        return Err("invalid native session user".to_owned());
    }
    Ok(())
}
struct Inner {
    authorization: framer_api::SessionAuthorization,
    native: NativeAccountClient,
    store: Arc<dyn SessionStore>,
    admission: Arc<Mutex<Admission>>,
    slots: Arc<Semaphore>,
    closed: Closed,
}
#[derive(Clone)]
pub struct SessionStoreBridge { inner: Arc<Inner> }

/// A rejected open completed normally. Worker loss leaves cleanup unconfirmed.
#[derive(Debug)]
pub enum OpenAccountError {
    Rejected(String),
    CleanupUnconfirmed,
}
impl From<String> for OpenAccountError {
    fn from(error: String) -> Self { Self::Rejected(error) }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}
struct Worker(JoinHandle<()>);
impl Drop for Worker { fn drop(&mut self) { self.0.abort(); } }
struct WorkerGuard(Arc<Mutex<Admission>>);
impl Drop for WorkerGuard {
    fn drop(&mut self) {
        let mut admission = lock(&self.0);
        admission.sender.take();
        admission.leases.close_all();
    }
}
impl Drop for Inner {
    fn drop(&mut self) { lock(&self.admission).sender.take(); }
}
// A canceled open must not leave an unpublished generation permanently open.
struct Opened { native: NativeAccountClient, scope: Option<AccountScope> }
// Validate again when the waiter receives the result. A completed actor job
// can otherwise leave old credentials queued while a replacement finishes.
struct Versioned<T> { value: T, user: Stamp, account: Option<Stamp> }
impl<T> Versioned<T> {
    fn publish(self, gate: &Mutex<Admission>) -> Result<T, String> {
        self.validate(gate)?;
        Ok(self.value)
    }
    fn validate(&self, gate: &Mutex<Admission>) -> Result<(), String> {
        let admission = lock(gate);
        if !admission.versions.current(&self.user)
            || self.account.as_ref().is_some_and(|s| !admission.versions.current(s)) {
            return Err(CHANGED.to_owned());
        }
        if admission.sender.is_none() { return Err(STOPPED.to_owned()); }
        Ok(())
    }
}
struct Mutation {
    native: NativeAccountClient,
    ticket: SessionChange,
    uncertain: bool,
}
impl Drop for Mutation {
    fn drop(&mut self) {
        if self.uncertain { let _ = self.native.fail_session_change(&self.ticket); }
    }
}
struct UserMutation {
    authorization: framer_api::SessionAuthorization,
    mutation: Mutation,
    stamp: Stamp,
    access: ChangingSession,
    ready: bool,
}
impl UserMutation {
    fn new(admission: &mut Admission, native: &NativeAccountClient,
        store: Arc<dyn SessionStore>, user: FramerUserId,
        authorization: framer_api::SessionAuthorization) -> Result<Self, String> {
        let (stamp, ticket) = admission.user_change(native, &user)?;
        Ok(Self { mutation: Mutation { native: native.clone(), ticket, uncertain: false },
            stamp, access: ChangingSession { user, store }, ready: false, authorization })
    }
    async fn initialize(&mut self) -> Result<(), String> {
        self.mutation.ticket.ready().await
            .map_err(|_| "session cleanup unconfirmed".to_owned())?;
        self.mutation.uncertain = true;
        // No old-generation authorization can repopulate this cache now.
        let previous = self.access.store.load(&self.access.user).await;
        match previous {
            Ok(Some(old)) if old.active_user == self.access.user => self.authorization.invalidate(&old),
            Ok(None) => {},
            Ok(Some(_)) => {
                self.mutation.uncertain = false;
                return Err("session identity mismatch".to_owned());
            }
            Err(_) => {
                self.mutation.uncertain = false;
                return Err("session load failed".to_owned());
            }
        }
        self.ready = true;
        Ok(())
    }
}
impl Opened {
    fn publish(mut self) -> AccountScope { self.scope.take().expect("owned scope") }
    async fn close(mut self) -> bool {
        let scope = self.scope.take().expect("owned scope");
        match self.native.close_account(&scope) {
            Ok(observer) => observer.await.is_ok(),
            Err(_) => false,
        }
    }
}
impl Drop for Opened {
    fn drop(&mut self) {
        if let Some(scope) = self.scope.take() { let _ = self.native.close_account(&scope); }
    }
}
impl SessionStoreBridge {
    pub fn new(native: NativeAccountClient, store: Arc<dyn SessionStore>) -> Result<Self, String> {
        Self::with_authorization(native, store, Default::default())
    }

    /// Use the forwarding client's handle when sharing its private sessions.
    pub fn with_authorization(native: NativeAccountClient, store: Arc<dyn SessionStore>,
        authorization: framer_api::SessionAuthorization) -> Result<Self, String> {
        let runtime = tokio::runtime::Handle::try_current().map_err(|_| STOPPED.to_owned())?;
        let (sender, mut receiver) = mpsc::channel::<Job>(CAPACITY);
        let admission = Arc::new(Mutex::new(Admission {
            versions: Versions::default(), leases: LeaseCache::default(), sender: Some(sender),
        }));
        let guard = WorkerGuard(admission.clone());
        let expiry = admission.clone();
        let worker = runtime.spawn(async move {
            let _guard = guard;
            let mut pending = FuturesUnordered::<Job>::new();
            let mut receiving = true;
            let mut sweep = tokio::time::interval(std::time::Duration::from_secs(1));
            sweep.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                if !receiving && pending.is_empty() { break; }
                tokio::select! {
                    job = receiver.recv(), if receiving => match job {
                        Some(job) => pending.push(job),
                        None => receiving = false,
                    },
                    _ = pending.next(), if !pending.is_empty() => {}
                    _ = sweep.tick() => { lock(&expiry).leases.expire(); }
                }
            }
        });
        let mut worker = Worker(worker);
        let closing = admission.clone();
        let closed = async move {
            let joined = (&mut worker.0).await.map_err(|_| STOPPED);
            let retained = lock(&closing).leases.take();
            let mut confirmed = true;
            for owner in retained { confirmed &= owner.close().await; }
            if !confirmed { return Err("native retained account cleanup unconfirmed"); }
            joined
        }.boxed().shared();
        Ok(Self { inner: Arc::new(Inner {
            native, store, authorization, admission, slots: Arc::new(Semaphore::new(CAPACITY)), closed,
        }) })
    }

    /// Stop new work, finish every admitted mutation, then observe the join.
    pub async fn shutdown(&self) -> Result<(), &'static str> {
        lock(&self.inner.admission).sender.take();
        self.inner.closed.clone().await
    }
    #[cfg(test)]
    pub(crate) fn jobs_in_flight_for_test(&self) -> usize {
        CAPACITY - self.inner.slots.available_permits()
    }
    fn submit<T, F, Fut>(&self, prepare: F)
        -> Result<oneshot::Receiver<Result<T, String>>, String>
    where T: Send + 'static,
        F: FnOnce(&mut Admission) -> Result<Fut, String>,
        Fut: std::future::Future<Output = Result<T, String>> + Send + 'static,
    {
        // Includes queued and running jobs. No unbounded send waiters.
        let slot = self.inner.slots.clone().try_acquire_owned()
            .map_err(|_| "native session store busy".to_owned())?;
        let mut gate = lock(&self.inner.admission);
        if gate.sender.is_none() { return Err(STOPPED.to_owned()); }
        let operation = prepare(&mut gate)?;
        let admission = self.inner.admission.clone();
        let (sender, receiver) = oneshot::channel();
        let job = async move {
            let result = operation.await;
            // Cancelled HTTP callers do not cancel the owned store operation.
            let _ = sender.send(result);
            drop(slot);
            lock(&admission).versions.prune();
        }.boxed();
        gate.sender.as_ref().ok_or(STOPPED)?.try_send(job)
            .map_err(|_| STOPPED.to_owned())?;
        Ok(receiver)
    }

    pub async fn open_account(&self, account: AccountId, user: FramerUserId,
        limits: NativeHttpLimits, egress: NativeEgress<'_>) -> Result<AccountScope, String> {
        self.open_account_observed(account, user, limits, egress).await.map_err(|error| match error {
            OpenAccountError::Rejected(message) => message,
            OpenAccountError::CleanupUnconfirmed => STOPPED.to_owned(),
        })
    }

    /// Observe this operation without shutting down the shared store worker.
    pub async fn open_account_observed(&self, account: AccountId, user: FramerUserId,
        limits: NativeHttpLimits, egress: NativeEgress<'_>) -> Result<AccountScope, OpenAccountError> {
        let proxy = match egress { NativeEgress::Direct => None, NativeEgress::Proxy(p) => Some(p.to_owned()) };
        let native = self.inner.native.clone();
        let store = self.inner.store.clone();
        let gate = self.inner.admission.clone();
        validate_user(&user)?;
        let receiver = self.submit(move |admission| {
            let user_stamp = admission.versions.read(Key::User(user.clone()))?;
            let account_stamp = admission.versions.read(Key::Account(account))?;
            Ok(async move {
                let session = store.load(&user).await.map_err(|_| "session load failed".to_owned())?
                    .ok_or_else(|| "session not found".to_owned())?;
                if session.active_user != user { return Err("session identity mismatch".to_owned()); }
                // Serialize the final open against mutation admission. A read
                // that straddled even a completed replacement is rejected.
                let current = lock(&gate);
                if !current.versions.current(&user_stamp) || !current.versions.current(&account_stamp) {
                    return Err(CHANGED.to_owned());
                }
                if current.sender.is_none() { return Err(STOPPED.to_owned()); }
                let egress = match proxy.as_deref() {
                    Some(proxy) => NativeEgress::Proxy(proxy), None => NativeEgress::Direct,
                };
                let value = native.open_account_owned(account, session, limits, egress, Some(store))
                    .map(|scope| Opened { native: native.clone(), scope: Some(scope) })
                    .map_err(|_| "native account open failed".to_owned())?;
                Ok(Versioned { value, user: user_stamp, account: Some(account_stamp) })
            })
        })?;
        let result = match receiver.await {
            Ok(result) => result?,
            Err(_) => {
                // Observe the failed owner and all dropped sibling jobs. This
                // joins only an already-lost worker; it does not close admission.
                let _ = self.inner.closed.clone().await;
                return Err(OpenAccountError::CleanupUnconfirmed);
            }
        };
        if let Err(error) = result.validate(&self.inner.admission) {
            // A buffered success still owns its unpublished native resources.
            // Keep that ownership until their independent close is observed.
            let closed = result.value.close().await;
            let stopping = lock(&self.inner.admission).sender.is_none();
            let joined = !stopping || self.inner.closed.clone().await.is_ok();
            return Err(if closed && joined { OpenAccountError::Rejected(error) }
                else { OpenAccountError::CleanupUnconfirmed });
        }
        Ok(result.value.publish())
    }

    async fn mutate(&self, user: FramerUserId, replacement: Option<Session>) -> Result<(), String> {
        self.change(user, move |access| async move {
            ChangeOutcome::finished(match replacement {
                Some(session) => access.save(&session).await,
                None => access.delete().await,
            })
        }.boxed()).await
    }

    /// Own a login, removal or other lifecycle update through true cleanup.
    /// Do not call this bridge recursively for the same user inside `update`.
    pub async fn change<T, F>(&self, user: FramerUserId, update: F) -> Result<T, String>
    where T: Send + 'static,
        F: for<'a> FnOnce(&'a ChangingSession) -> BoxFuture<'a, ChangeOutcome<T>> + Send + 'static,
    {
        let native = self.inner.native.clone();
        let authorization = self.inner.authorization.clone();
        let store = self.inner.store.clone();
        let gate = self.inner.admission.clone();
        let receiver = self.submit(move |admission| {
            let mut held = UserMutation::new(admission, &native, store, user, authorization)?;
            Ok(async move {
                held.initialize().await?;
                let outcome = update(&held.access).await;
                held.mutation.uncertain = !outcome.cleanup_confirmed;
                if held.mutation.uncertain { return Err("session cleanup unconfirmed".to_owned()); }
                let value = outcome.value?;
                let mut admission = lock(&gate);
                native.finish_session_change(&mut held.mutation.ticket)
                    .map_err(|_| "session change completion failed".to_owned())?;
                admission.versions.unblock(&held.stamp);
                Ok(value)
            })
        })?;
        receiver.await.map_err(|_| STOPPED.to_owned())?
    }
}
#[async_trait::async_trait]
impl SessionStore for SessionStoreBridge {
    async fn load(&self, user: &FramerUserId) -> Result<Option<Session>, String> {
        let user = user.clone();
        let store = self.inner.store.clone();
        let gate = self.inner.admission.clone();
        validate_user(&user)?;
        let receiver = self.submit(move |admission| {
            let stamp = admission.versions.read(Key::User(user.clone()))?;
            Ok(async move {
                let result = store.load(&user).await.map_err(|_| "session load failed".to_owned())?;
                if result.as_ref().is_some_and(|s| s.active_user != user) {
                    return Err("session identity mismatch".to_owned());
                }
                let current = lock(&gate);
                if !current.versions.current(&stamp) {
                    return Err(CHANGED.to_owned());
                }
                if current.sender.is_none() { return Err(STOPPED.to_owned()); }
                Ok(Versioned { value: result, user: stamp, account: None })
            })
        })?;
        let result = receiver.await.map_err(|_| STOPPED.to_owned())??;
        result.publish(&self.inner.admission)
    }
    async fn save(&self, session: &Session) -> Result<(), String> {
        if session.active_user.0.is_empty() || session.cookie_header.is_empty()
            || session.cookie_header.len() > MAX_CREDENTIAL_BYTES {
            return Err("invalid native session".to_owned());
        }
        self.mutate(session.active_user.clone(), Some(session.clone())).await
    }
    async fn delete(&self, user: &FramerUserId) -> Result<(), String> {
        if user.0.is_empty() { return Err("invalid native session user".to_owned()); }
        self.mutate(user.clone(), None).await
    }
}

#[cfg(test)]
#[path = "session_open_tests.rs"]
mod open_tests;

#[cfg(test)]
#[path = "session_lease_race_tests.rs"]
mod lease_race_tests;

#[cfg(test)]
#[path = "session_lease_tests.rs"]
mod lease_tests;

#[cfg(test)]
#[path = "session_lease_shutdown_tests.rs"]
mod lease_shutdown_tests;

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn unreceived_open_result_closes_its_unpublished_generation() {
        let native = NativeAccountClient::new().unwrap();
        let account = AccountId(uuid::Uuid::new_v4());
        let session = Session { active_user: FramerUserId("fixture-user".to_owned()),
            cookie_header: "session=fixture-private".to_owned() };
        let scope = native.open_account(account, session.clone(),
            NativeHttpLimits::default(), NativeEgress::Direct).unwrap();
        let (sender, receiver) = oneshot::channel();
        assert!(sender.send(Opened { native: native.clone(), scope: Some(scope.clone()) }).is_ok());
        drop(receiver);
        scope.closed().await.unwrap();
        let next = native.open_account(account, session,
            NativeHttpLimits::default(), NativeEgress::Direct).unwrap();
        assert!(!scope.same_generation(&next));
        native.close_account(&next).unwrap().await.unwrap();
    }
}

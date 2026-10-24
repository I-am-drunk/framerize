//! Opt-in native account client. Source candidate, not application dispatch.
//! One shared request pool and one weakly owned supervisor retain real cleanup.
//! Supplied auth does not establish token provenance or upstream acceptance.

pub const CAPACITY: usize = 10;

#[cfg(test)]
#[path = "idle_shutdown_tests.rs"]
mod idle_shutdown_tests;

#[path = "native_preparation.rs"]
mod preparation;
pub use preparation::{PendingPreparation, PreparationControl, PreparationFailure, PreparationResult};

use super::downstream_owner::{self, Cleanup, Pending};
use super::native_http::{FreshCastleToken, NativeEgress, NativeHttp, NativeHttpLimits};
use super::native_owner::{AdmissionError, Cancellation, CompletionError, Inputs, RequestPool};
use crate::proxy::Dialect;
use framer_core::ids::{AccountId, FramerUserId, ProjectId};
use framer_core::port::Session;
use futures::channel::oneshot;
use futures::future::{BoxFuture, Shared};
use futures::task::AtomicWaker;
use futures::FutureExt;
use serde_json::Value;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex, MutexGuard, Weak};
use std::task::{Context, Poll};
use tokio::task::JoinHandle;
use tokio::time::Instant;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CloseError {
    Request(CompletionError), StartUnconfirmed, Attachment, SupervisorLost, ClientDropped,
    PreparationUnconfirmed, SessionMutationUnconfirmed,
}
pub type CloseResult = Result<(), CloseError>;

#[derive(Clone)]
pub struct CloseObserver(Shared<BoxFuture<'static, CloseResult>>);

impl Future for CloseObserver {
    type Output = CloseResult;
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<CloseResult> {
        Pin::new(&mut self.get_mut().0).poll(cx)
    }
}

#[derive(Debug)]
pub enum ClientError {
    NoRuntime, Unavailable, Full, AlreadyOpen, WrongClient, StaleScope,
    Draining, SessionChanging, InvalidContext, SequenceExhausted,
    NotReady, CleanupFailed(CloseError), Native(String),
    Admission(AdmissionError), Attachment,
}

/// Caller-supplied truthful context. No category default or policy fallback.
pub struct RequestContext {
    pub dialect: Dialect,
    pub project: ProjectId,
    pub feature_category: String,
    pub feature_name: String,
    pub step: String,
    pub chat_session: String,
    pub step_index: u32,
    pub turn_id: String,
    pub user_prompt: String,
    pub payload: Value,
}

pub struct SuppliedAuth {
    owner: Weak<Inner>,
    generation: u64,
    bearer: String,
    castle: FreshCastleToken,
    current_cookies: Option<super::authorization::CurrentRequestCookies>,
}

impl SuppliedAuth {
    /// Bind caller-verified material to the generation that obtained it.
    /// This tag does not prove JWT or token provenance.
    pub fn new(scope: &AccountScope, bearer: String, castle: FreshCastleToken) -> Self {
        Self {
            owner: scope.owner.clone(), generation: scope.identity.generation,
            bearer, castle, current_cookies: None,
        }
    }

    pub(crate) fn with_current_cookies(mut self,
        cookies: Option<super::authorization::CurrentRequestCookies>) -> Self {
        self.current_cookies = cookies;
        self
    }
}

impl RequestContext {
    pub(crate) fn prepare(mut self) -> Result<(String, Value), ClientError> {
        if self.project.0.is_empty() || self.feature_category.is_empty()
            || self.feature_name.is_empty() || self.step.is_empty()
            || self.chat_session.is_empty() || self.turn_id.is_empty()
            || !self.payload.is_object()
        { return Err(ClientError::InvalidContext); }
        let model = self.payload.get("model")
            .filter(|v| v.as_str().is_some_and(|s| !s.is_empty()))
            .cloned().ok_or(ClientError::InvalidContext)?;
        let route = crate::routes::by_dialect(self.dialect);
        if !matches!(route.id, "direct-responses" | "direct-anthropic"
            | "openrouter-responses" | "openrouter-messages" | "openrouter-chat")
        { return Err(ClientError::InvalidContext); }
        // The current Framer OpenRouter Responses mapper derives the nested
        // session_id from its outer metadata, replacing any payload value.
        // Other routes retain their payload unchanged.
        if matches!(self.dialect, Dialect::OpenRouterResponses) {
            self.payload["session_id"] = Value::String(self.chat_session.clone());
        }
        // Move the request tree instead of cloning it through json!.
        let pairs = [
            ("projectId", Value::String(self.project.0)),
            ("featureCategory", Value::String(self.feature_category)),
            ("featureName", Value::String(self.feature_name)),
            ("step", Value::String(self.step)),
            ("sessionId", Value::String(self.chat_session)),
            ("stepIndex", Value::Number(self.step_index.into())),
            ("turnId", Value::String(self.turn_id)),
            ("userPrompt", Value::String(self.user_prompt)),
            ("model", model), ("request", self.payload),
        ];
        let body = Value::Object(pairs.into_iter().map(|(k, v)| (k.to_owned(), v)).collect());
        Ok((route.path.to_owned(), body))
    }
}

#[derive(Clone)]
pub struct NativeAccountClient { inner: Arc<Inner> }

#[derive(Clone)]
pub struct AccountScope { owner: Weak<Inner>, identity: Arc<Identity> }
struct Identity {
    account: AccountId,
    user: FramerUserId,
    generation: u64,
    closed: CloseObserver,
}
impl AccountScope {
    pub fn account(&self) -> AccountId { self.identity.account }
    pub fn active_user(&self) -> &FramerUserId { &self.identity.user }
    pub fn generation(&self) -> u64 { self.identity.generation }
    pub fn same_generation(&self, other: &Self) -> bool {
        Weak::ptr_eq(&self.owner, &other.owner) && self.generation() == other.generation()
    }
    pub fn closed(&self) -> CloseObserver { self.identity.closed.clone() }
    /// Compare a supplied route without exposing the stored proxy or client.
    /// This is an identity check, not a work reservation.
    pub fn matches_egress(&self, egress: &NativeEgress<'_>) -> bool {
        let Some(owner) = self.owner.upgrade() else { return false; };
        let state = lock(&owner.state);
        if !state.healthy { return false; }
        state.accounts.iter().find(|account| Arc::ptr_eq(&account.identity, &self.identity))
            .filter(|account| !account.opening && !account.closing && account.failure.is_none())
            .and_then(|account| account.resources.as_ref())
            .is_some_and(|resources| resources.client.matches_egress(egress))
    }
}

/// Exclusive store-mutation authority. Retain this non-Clone ticket until the
/// actual store operation completes, including any cancellation cleanup.
pub struct SessionChange {
    owner: Weak<Inner>, target: ChangeTarget, id: u64, claim: u64, ready: CloseObserver,
}
impl SessionChange {
    /// Status observation alone does not convey mutation or release authority.
    pub fn ready(&self) -> CloseObserver { self.ready.clone() }
}

/// Fence one browser/account before a login can reveal its resulting user.
/// This is native generation ownership, not a lease against legacy traffic.
pub struct AccountChange(SessionChange);
impl AccountChange {
    pub fn ready(&self) -> CloseObserver { self.0.ready() }
}

#[derive(Clone, PartialEq, Eq)]
enum ChangeTarget { User(FramerUserId), Account(AccountId) }
impl ChangeTarget {
    fn matches(&self, identity: &Identity) -> bool {
        match self {
            Self::User(user) => user == &identity.user,
            Self::Account(account) => account == &identity.account,
        }
    }
    fn same_kind(&self, other: &Self) -> bool {
        matches!((self, other), (Self::User(_), Self::User(_)) | (Self::Account(_), Self::Account(_)))
    }
}

impl Drop for SessionChange {
    fn drop(&mut self) {
        if let Some(owner) = self.owner.upgrade() {
            let mut state = lock(&owner.state);
            if let Some(change) = state.changes.iter_mut().find(|c| {
                c.id == self.id && c.target == self.target && c.claim == Some(self.claim)
            }) {
                // Only relinquish this claim. Account admission remains closed
                // until a later exclusive owner finishes the same barrier.
                change.claim = None;
            }
        }
    }
}

struct Inner {
    pool: RequestPool,
    state: Mutex<State>,
    wake: AtomicWaker,
    supervisor: Mutex<Option<JoinHandle<()>>>,
}
struct State {
    healthy: bool,
    next: u64,
    accounts: Vec<Account>,
    changes: Vec<Change>,
}
struct Resources { session: Session, client: Arc<NativeHttp> }
struct Account {
    identity: Arc<Identity>,
    opening: bool,
    closing: bool,
    releasing: bool,
    failure: Option<CloseError>,
    resources: Option<Resources>,
    requests: Vec<Operation>,
    closed: Option<oneshot::Sender<CloseResult>>,
}
struct Operation { id: u64, stage: OperationStage }
enum OperationStage {
    Preparing,
    OwnedPreparation(preparation::Work),
    Running { cancellation: Cancellation, cleanup: Option<Cleanup> },
    // No original completion can be recovered. Never release this reservation.
    Unconfirmed,
}
struct Change {
    target: ChangeTarget,
    id: u64,
    claim: Option<u64>,
    ready: CloseObserver,
    result: Option<CloseResult>,
    sender: Option<oneshot::Sender<CloseResult>>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}
enum AccountCancellation { Native(Cancellation), Preparation(PreparationControl) }
impl AccountCancellation {
    fn cancel(&self) {
        match self { Self::Native(c) => c.cancel(), Self::Preparation(c) => c.cancel() }
    }
}
fn cancellations(account: &Account) -> Vec<AccountCancellation> {
    account.requests.iter().filter_map(|request| match &request.stage {
        OperationStage::Running { cancellation, .. } => Some(AccountCancellation::Native(cancellation.clone())),
        OperationStage::OwnedPreparation(work) => Some(AccountCancellation::Preparation(work.control.clone())),
        _ => None,
    }).collect()
}
fn signal() -> (oneshot::Sender<CloseResult>, CloseObserver) {
    let (tx, rx) = oneshot::channel();
    let future = async move { rx.await.unwrap_or(Err(CloseError::SupervisorLost)) };
    (tx, CloseObserver(future.boxed().shared()))
}
fn sequence(state: &mut State) -> Result<u64, ClientError> {
    let id = state.next;
    state.next = id.checked_add(1).ok_or(ClientError::SequenceExhausted)?;
    Ok(id)
}

// A synchronous opening/starting operation reserves its place before work can
// begin. Closing sees that reservation even before any native handle exists.
struct Reservation {
    inner: Arc<Inner>,
    generation: u64,
    operation: Option<u64>,
    armed: bool,
    may_have_started: bool,
}

impl Drop for Reservation {
    fn drop(&mut self) {
        if !self.armed { return; }
        let mut discarded = None;
        {
            let mut state = lock(&self.inner.state);
            if let Some(account) = state.accounts.iter_mut()
                .find(|a| a.identity.generation == self.generation)
            {
                if let Some(id) = self.operation {
                    if let Some(index) = account.requests.iter()
                        .position(|r| r.id == id && matches!(r.stage, OperationStage::Preparing))
                    {
                        if self.may_have_started {
                            account.requests[index].stage = OperationStage::Unconfirmed;
                            account.failure.get_or_insert(CloseError::StartUnconfirmed);
                            account.closing = true;
                        } else {
                            discarded = Some(account.requests.remove(index));
                        }
                    }
                } else {
                    account.opening = false;
                    account.closing = true;
                }
            }
        }
        drop(discarded);
        self.inner.wake.wake();
    }
}

struct SupervisorGuard(Weak<Inner>);

impl Inner {
    fn restore_cleanup(&self, generation: u64, operation: u64, cleanup: Cleanup) {
        // Preserve the SAME Shared handle and therefore its registered waker.
        let mut state = lock(&self.state);
        let account = state.accounts.iter_mut()
            .find(|a| a.identity.generation == generation)
            .expect("checked-out cleanup retains its account");
        let request = account.requests.iter_mut().find(|r| r.id == operation)
            .expect("checked-out cleanup retains its operation");
        let OperationStage::Running { cleanup: slot, .. } = &mut request.stage
            else { unreachable!("only the supervisor changes a registered operation"); };
        debug_assert!(slot.is_none());
        *slot = Some(cleanup);
    }

    fn supervisor_lost(&self) {
        let mut pending = Vec::new();
        {
            let mut state = lock(&self.state);
            if !state.healthy { return; }
            state.healthy = false;
            for account in &mut state.accounts {
                account.closing = true;
                let error = *account.failure.get_or_insert(CloseError::SupervisorLost);
                for request in &mut account.requests {
                    if let OperationStage::OwnedPreparation(work) = &mut request.stage {
                        work.control.cancel();
                        if let Some(sender) = work.settled.take() {
                            pending.push((sender, Err(error)));
                        }
                    }
                }
                if let Some(sender) = account.closed.take() {
                    pending.push((sender, Err(error)));
                }
            }
            for change in &mut state.changes {
                if change.result.is_none() {
                    let result = Err(CloseError::SupervisorLost);
                    change.result = Some(result);
                    if let Some(sender) = change.sender.take() { pending.push((sender, result)); }
                }
            }
        }
        self.pool.shutdown();
        for (sender, result) in pending { let _ = sender.send(result); }
    }

    fn resolve_changes(&self) {
        let mut ready = Vec::new();
        {
            let mut state = lock(&self.state);
            let State { accounts, changes, .. } = &mut *state;
            for change in changes {
                if change.result.is_some() { continue; }
                let failed = accounts.iter().filter(|a| change.target.matches(&a.identity))
                    .find_map(|a| a.failure);
                let result = if let Some(error) = failed {
                    Some(Err(error))
                } else if accounts.iter().all(|a| !change.target.matches(&a.identity)) {
                    Some(Ok(()))
                } else { None };
                if let Some(result) = result {
                    change.result = Some(result);
                    if let Some(sender) = change.sender.take() { ready.push((sender, result)); }
                }
            }
        }
        for (sender, result) in ready { let _ = sender.send(result); }
    }

    fn finish_releases(&self, generations: Vec<u64>) {
        let mut removed = Vec::new();
        {
            let mut state = lock(&self.state);
            for generation in generations {
                let Some(index) = state.accounts.iter()
                    .position(|a| a.identity.generation == generation) else { continue; };
                let account = &state.accounts[index];
                if account.releasing && account.requests.is_empty() && account.failure.is_none() {
                    removed.push(state.accounts.remove(index));
                }
            }
        }
        for mut account in removed {
            let sender = account.closed.take();
            drop(account);
            if let Some(sender) = sender { let _ = sender.send(Ok(())); }
        }
    }

    fn release_accounts(&self) {
        let mut releases = Vec::new();
        let mut failed = Vec::new();
        {
            let mut state = lock(&self.state);
            for account in &mut state.accounts {
                if let Some(error) = account.failure {
                    if let Some(sender) = account.closed.take() { failed.push((sender, error)); }
                } else if account.closing && !account.opening
                    && account.requests.is_empty() && !account.releasing
                {
                    account.releasing = true;
                    releases.push((account.identity.generation, account.resources.take()));
                }
            }
        }
        for (sender, error) in failed { let _ = sender.send(Err(error)); }
        let mut generations = Vec::new();
        for (generation, resources) in releases {
            // Keep the closing record while the final client/session drops.
            drop(resources);
            generations.push(generation);
        }
        self.finish_releases(generations);
    }

    fn complete_requests(
        &self, completed: Vec<(u64, u64, super::native_owner::CompletionResult)>,
    ) {
        let mut removed = Vec::new();
        let controls: Vec<_> = {
            let mut state = lock(&self.state);
            for (generation, operation, result) in completed {
                let Some(account) = state.accounts.iter_mut()
                    .find(|a| a.identity.generation == generation) else { continue; };
                let Some(index) = account.requests.iter()
                    .position(|r| r.id == operation) else { continue; };
                removed.push(account.requests.remove(index));
                if let Err(error) = result {
                    account.failure.get_or_insert(CloseError::Request(error));
                    account.closing = true;
                }
            }
            state.accounts.iter().filter(|a| a.closing)
                .flat_map(cancellations).collect()
        };
        drop(removed);
        for control in controls { control.cancel(); }
    }

    fn poll_requests(&self, cx: &mut Context<'_>) {
        // No external poll or waker runs under the account lock.
        let snapshot: Vec<_> = {
            let mut state = lock(&self.state);
            state.accounts.iter_mut().flat_map(|account| {
                let generation = account.identity.generation;
                account.requests.iter_mut().filter_map(move |request| match &mut request.stage {
                    OperationStage::Running { cleanup, .. } =>
                        cleanup.take().map(|c| (generation, request.id, c)),
                    _ => None,
                })
            }).collect()
        };
        let mut completed = Vec::new();
        for (generation, operation, mut cleanup) in snapshot {
            match Pin::new(&mut cleanup).poll(cx) {
                Poll::Ready(result) => completed.push((generation, operation, result)),
                Poll::Pending => self.restore_cleanup(generation, operation, cleanup),
            }
        }
        self.complete_requests(completed);
    }

    fn poll_cleanup(&self, cx: &mut Context<'_>) {
        self.wake.register(cx.waker());
        self.poll_preparations(cx);
        self.poll_requests(cx);
        self.release_accounts();
        self.resolve_changes();
    }
}

impl Drop for SupervisorGuard {
    fn drop(&mut self) {
        if let Some(inner) = self.0.upgrade() { inner.supervisor_lost(); }
    }
}

impl Drop for Inner {
    fn drop(&mut self) {
        // The supervisor and public scopes have only Weak<Inner>; neither can
        // keep the client alive. Abort its sole task when the last client drops.
        self.pool.shutdown();
        if let Some(task) = self.supervisor.get_mut()
            .unwrap_or_else(std::sync::PoisonError::into_inner).take()
        { task.abort(); }
        drop(self.wake.take());
        let state = self.state.get_mut().unwrap_or_else(std::sync::PoisonError::into_inner);
        state.healthy = false;
        for account in &mut state.accounts {
            if let Some(sender) = account.closed.take() {
                let _ = sender.send(Err(CloseError::ClientDropped));
            }
        }
        for change in &mut state.changes {
            if let Some(sender) = change.sender.take() {
                let _ = sender.send(Err(CloseError::ClientDropped));
            }
        }
        // Drop cannot await joins. Existing Pending.cleanup remains the real
        // request result; unresolved account observers receive an error above.
    }
}

impl NativeAccountClient {
    /// Terminal shutdown for a private client whose accounts have all closed.
    /// Do not use this on the application's shared, still-serving client.
    pub async fn shutdown_idle(&self) -> Result<(), ClientError> {
        {
            let mut state = lock(&self.inner.state);
            if !state.healthy || !state.accounts.is_empty() || !state.changes.is_empty() {
                return Err(ClientError::NotReady);
            }
            state.healthy = false;
        }
        self.inner.pool.shutdown();
        if self.inner.pool.snapshot().occupied != 0 { return Err(ClientError::NotReady); }
        let task = lock(&self.inner.supervisor).take().ok_or(ClientError::Unavailable)?;
        task.abort();
        match task.await {
            Ok(()) => Ok(()),
            Err(error) if error.is_cancelled() => Ok(()),
            Err(_) => Err(ClientError::Unavailable),
        }
    }

    /// An owned store task disappeared during mutation. A later ticket must
    /// observe that uncertainty before it can issue another store operation.
    pub(crate) fn fail_session_change(&self, ticket: &SessionChange) -> Result<(), ClientError> {
        if !Weak::ptr_eq(&ticket.owner, &Arc::downgrade(&self.inner)) {
            return Err(ClientError::WrongClient);
        }
        let result = Err(CloseError::SessionMutationUnconfirmed);
        let sender = {
            let mut state = lock(&self.inner.state);
            let change = state.changes.iter_mut().find(|change|
                change.id == ticket.id && change.target == ticket.target
                    && change.claim == Some(ticket.claim)).ok_or(ClientError::StaleScope)?;
            change.result = Some(result);
            change.ready = CloseObserver(async move { result }.boxed().shared());
            change.sender.take()
        };
        if let Some(sender) = sender { let _ = sender.send(result); }
        Ok(())
    }

    /// After ready() succeeds, mutate the private session store while this
    /// barrier remains. Call this only after the store operation finishes.
    /// Errors retain the exclusive claim; only this matching owner can finish.
    pub fn finish_session_change(&self, ticket: &mut SessionChange) -> Result<(), ClientError> {
        self.finish_changes(&[ticket])
    }

    pub fn finish_account_change(&self, ticket: &mut AccountChange) -> Result<(), ClientError> {
        self.finish_changes(&[&ticket.0])
    }

    pub(crate) fn fail_account_change(&self, ticket: &AccountChange) -> Result<(), ClientError> {
        self.fail_session_change(&ticket.0)
    }

    pub(crate) fn finish_account_and_session_change(&self, account: &mut AccountChange,
        session: Option<&mut SessionChange>) -> Result<(), ClientError> {
        match session {
            Some(session) => self.finish_changes(&[&account.0, session]),
            None => self.finish_changes(&[&account.0]),
        }
    }

    fn finish_changes(&self, tickets: &[&SessionChange]) -> Result<(), ClientError> {
        for ticket in tickets {
            if !Weak::ptr_eq(&ticket.owner, &Arc::downgrade(&self.inner)) {
                return Err(ClientError::WrongClient);
            }
        }
        let removed = {
            let mut state = lock(&self.inner.state);
            if !state.healthy { return Err(ClientError::Unavailable); }
            let mut indices = Vec::with_capacity(tickets.len());
            for ticket in tickets {
                let index = state.changes.iter()
                    .position(|c| c.id == ticket.id && c.target == ticket.target
                        && c.claim == Some(ticket.claim))
                    .ok_or(ClientError::StaleScope)?;
                match state.changes[index].result {
                    None => return Err(ClientError::NotReady),
                    Some(Err(error)) => return Err(ClientError::CleanupFailed(error)),
                    Some(Ok(())) => indices.push(index),
                }
            }
            indices.sort_unstable();
            indices.dedup();
            indices.into_iter().rev().map(|index| state.changes.remove(index)).collect::<Vec<_>>()
        };
        drop(removed);
        Ok(())
    }

    /// Close every generation and claim exclusive session-change ownership.
    /// A live owner prevents another begin. Dropping it keeps the barrier
    /// but releases its claim, allowing a later begin to recover ownership.
    pub fn begin_session_change(&self, user: &FramerUserId) -> Result<SessionChange, ClientError> {
        if user.0.is_empty() || user.0.len() > 256 { return Err(ClientError::InvalidContext); }
        self.begin_change(ChangeTarget::User(user.clone()))
    }

    /// Fence native admission before auth_complete or a profile mutation.
    pub fn begin_account_change(&self, account: AccountId) -> Result<AccountChange, ClientError> {
        self.begin_change(ChangeTarget::Account(account)).map(AccountChange)
    }

    fn begin_change(&self, target: ChangeTarget) -> Result<SessionChange, ClientError> {
        let (id, ready, controls) = {
            let mut state = lock(&self.inner.state);
            if !state.healthy { return Err(ClientError::Unavailable); }
            if let Some(index) = state.changes.iter().position(|c| c.target == target) {
                if state.changes[index].claim.is_some() {
                    return Err(ClientError::SessionChanging);
                }
                let claim = sequence(&mut state)?;
                let change = &mut state.changes[index];
                change.claim = Some(claim);
                return Ok(SessionChange {
                    owner: Arc::downgrade(&self.inner), target,
                    id: change.id, claim, ready: change.ready.clone(),
                });
            }
            // Ten account fences and ten user fences let every admitted login
            // discover its user without consuming another actor job.
            if state.changes.iter().filter(|c| c.target.same_kind(&target)).count() >= CAPACITY {
                return Err(ClientError::Full);
            }
            let id = sequence(&mut state)?;
            let (sender, ready) = signal();
            state.changes.push(Change {
                target: target.clone(), id, claim: Some(id), ready: ready.clone(), result: None,
                sender: Some(sender),
            });
            let mut controls = Vec::new();
            for account in &mut state.accounts {
                if target.matches(&account.identity) {
                    account.closing = true;
                    controls.extend(cancellations(account));
                }
            }
            (id, ready, controls)
        };
        for control in controls { control.cancel(); }
        self.inner.wake.wake();
        Ok(SessionChange {
            owner: Arc::downgrade(&self.inner), target, id, claim: id, ready,
        })
    }

    /// Idempotent for this generation. A stale scope never closes a newer
    /// generation with the same AccountId.
    pub fn close_account(&self, scope: &AccountScope) -> Result<CloseObserver, ClientError> {
        if !Weak::ptr_eq(&scope.owner, &Arc::downgrade(&self.inner)) {
            return Err(ClientError::WrongClient);
        }
        let controls = {
            let mut state = lock(&self.inner.state);
            match state.accounts.iter_mut()
                .find(|a| a.identity.generation == scope.identity.generation)
            {
                Some(account) => {
                    account.closing = true;
                    cancellations(account)
                }
                None => Vec::new(),
            }
        };
        for control in controls { control.cancel(); }
        self.inner.wake.wake();
        Ok(scope.identity.closed.clone())
    }

    pub fn new() -> Result<Self, ClientError> {
        let runtime = tokio::runtime::Handle::try_current().map_err(|_| ClientError::NoRuntime)?;
        let inner = Arc::new(Inner {
            pool: RequestPool::new(CAPACITY).map_err(ClientError::Admission)?,
            state: Mutex::new(State { healthy: true, next: 1, accounts: Vec::new(), changes: Vec::new() }),
            wake: AtomicWaker::new(), supervisor: Mutex::new(None),
        });
        let weak = Arc::downgrade(&inner);
        // Capture the guard before spawn. Runtime shutdown before first poll
        // must report supervisor loss too, not leave healthy-looking accounts.
        let guard = SupervisorGuard(weak.clone());
        let spawned = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| runtime.spawn(async move {
            let _guard = guard;
            futures::future::poll_fn(|cx| {
                let Some(inner) = weak.upgrade() else { return Poll::Ready(()); };
                inner.poll_cleanup(cx);
                Poll::Pending
            }).await;
        })));
        let task = spawned.map_err(|_| ClientError::Unavailable)?;
        *lock(&inner.supervisor) = Some(task);
        Ok(Self { inner })
    }

    pub fn open_account(
        &self, account: AccountId, session: Session,
        limits: NativeHttpLimits, egress: NativeEgress<'_>,
    ) -> Result<AccountScope, ClientError> {
        if session.cookie_header.len() > limits.credential_bytes {
            return Err(ClientError::InvalidContext);
        }
        let user = session.active_user.clone();
        self.open_with(account, session, || NativeHttp::new(account, user, limits, egress))
    }

    fn open_with(
        &self, account: AccountId, session: Session,
        build: impl FnOnce() -> Result<NativeHttp, String>,
    ) -> Result<AccountScope, ClientError> {
        if session.active_user.0.is_empty() || session.cookie_header.is_empty() {
            return Err(ClientError::InvalidContext);
        }
        let (closed, observer) = signal();
        let identity = {
            let mut state = lock(&self.inner.state);
            if !state.healthy { return Err(ClientError::Unavailable); }
            if state.changes.iter().any(|c| match &c.target {
                ChangeTarget::User(user) => user == &session.active_user,
                ChangeTarget::Account(id) => id == &account,
            }) {
                return Err(ClientError::SessionChanging);
            }
            if state.accounts.iter().any(|a| a.identity.account == account) {
                return Err(ClientError::AlreadyOpen);
            }
            if state.accounts.len() >= CAPACITY { return Err(ClientError::Full); }
            let identity = Arc::new(Identity {
                account, user: session.active_user.clone(),
                generation: sequence(&mut state)?, closed: observer,
            });
            state.accounts.push(Account {
                identity: identity.clone(), opening: true, closing: false, releasing: false,
                failure: None, resources: None, requests: Vec::new(), closed: Some(closed),
            });
            identity
        };
        let mut reservation = Reservation {
            inner: self.inner.clone(), generation: identity.generation,
            operation: None, armed: true, may_have_started: false,
        };
        let client = Arc::new(build().map_err(ClientError::Native)?);
        let active = {
            let mut state = lock(&self.inner.state);
            let entry = state.accounts.iter_mut().find(|a| a.identity.generation == identity.generation)
                .expect("opening reservation retains its account");
            entry.resources = Some(Resources { session, client });
            entry.opening = false;
            !entry.closing
        };
        reservation.armed = false;
        self.inner.wake.wake();
        if !active { return Err(ClientError::Draining); }
        Ok(AccountScope { owner: Arc::downgrade(&self.inner), identity })
    }
}

impl NativeAccountClient {
    /// Fixture-only factory, including a deterministic opening race gate.
    /// Ordinary builds always construct their own account-bound NativeHttp.
    #[cfg(test)]
    pub(crate) fn open_account_for_test(
        &self, account: AccountId, session: Session,
        build: impl FnOnce() -> Result<NativeHttp, String>,
    ) -> Result<AccountScope, ClientError> {
        self.open_with(account, session, build)
    }

    /// Fixture-only entry for the existing owner tail/ack hooks and for a
    /// controlled close between native admission and cleanup registration.
    #[cfg(test)]
    pub(crate) fn start_for_test(
        &self, scope: &AccountScope, context: RequestContext, auth: SuppliedAuth,
        deadline: Instant, missing_content_type: String,
        start: impl FnOnce(&RequestPool, Inputs, Instant, String)
            -> Result<Pending, downstream_owner::StartError>,
    ) -> Result<Pending, ClientError> {
        self.start_with(scope, context, auth, deadline, missing_content_type, start)
    }

    /// The handle can observe actual supervisor termination after client
    /// drop, or inject supervisor loss, without exposing account resources.
    #[cfg(test)]
    pub(crate) fn supervisor_for_test(&self) -> tokio::task::AbortHandle {
        lock(&self.inner.supervisor).as_ref()
            .expect("constructed clients have one supervisor").abort_handle()
    }

    /// Submit a real project/dialect envelope through this immutable session.
    /// Cleanup is registered before the caller can receive or drop Pending.
    pub fn start(
        &self, scope: &AccountScope, context: RequestContext, auth: SuppliedAuth,
        deadline: Instant, missing_content_type: String,
    ) -> Result<Pending, ClientError> {
        self.start_with(scope, context, auth, deadline, missing_content_type,
            downstream_owner::start)
    }

    fn start_with(
        &self, scope: &AccountScope, context: RequestContext, auth: SuppliedAuth,
        deadline: Instant, missing_content_type: String,
        start: impl FnOnce(&RequestPool, Inputs, Instant, String)
            -> Result<Pending, downstream_owner::StartError>,
    ) -> Result<Pending, ClientError> {
        if !Weak::ptr_eq(&scope.owner, &Arc::downgrade(&self.inner)) {
            return Err(ClientError::WrongClient);
        }
        if !Weak::ptr_eq(&auth.owner, &Arc::downgrade(&self.inner)) {
            return Err(ClientError::WrongClient);
        }
        if auth.generation != scope.identity.generation {
            return Err(ClientError::StaleScope);
        }
        let (path, body) = context.prepare()?;
        let (operation, session, client) = {
            let mut state = lock(&self.inner.state);
            if !state.healthy { return Err(ClientError::Unavailable); }
            let index = state.accounts.iter()
                .position(|a| a.identity.generation == scope.identity.generation)
                .ok_or(ClientError::StaleScope)?;
            if state.accounts[index].closing { return Err(ClientError::Draining); }
            if state.accounts.iter().map(|a| a.requests.len()).sum::<usize>() >= CAPACITY {
                return Err(ClientError::Full);
            }
            let operation = sequence(&mut state)?;
            let entry = &mut state.accounts[index];
            let resources = entry.resources.as_ref().ok_or(ClientError::Draining)?;
            let session = match auth.current_cookies {
                Some(cookies) => cookies.into_session(&resources.session)
                    .map_err(|_| ClientError::InvalidContext)?,
                None => resources.session.clone(),
            };
            let client = resources.client.clone();
            entry.requests.push(Operation { id: operation, stage: OperationStage::Preparing });
            (operation, session, client)
        };
        let mut reservation = Reservation {
            inner: self.inner.clone(), generation: scope.identity.generation,
            operation: Some(operation), armed: true, may_have_started: false,
        };
        let inputs = Inputs::new(scope.identity.account, session, auth.bearer,
            path, body, auth.castle, client);
        // Unwinding after this point cannot prove that no task was started.
        // Only a known pre-admission error may remove an empty reservation.
        reservation.may_have_started = true;
        match start(&self.inner.pool, inputs, deadline, missing_content_type) {
            Ok(pending) => {
                let active = self.register(&mut reservation, OperationStage::Running {
                    cancellation: pending.cancellation.clone(), cleanup: Some(pending.cleanup.clone()),
                }, None);
                if active { Ok(pending) } else {
                    pending.cancellation.cancel();
                    drop(pending);
                    Err(ClientError::Draining)
                }
            }
            Err(downstream_owner::StartError::Admission(error)) => {
                if error == AdmissionError::SpawnFailed {
                    self.register(&mut reservation, OperationStage::Unconfirmed,
                        Some(CloseError::StartUnconfirmed));
                } else {
                    reservation.may_have_started = false;
                }
                Err(ClientError::Admission(error))
            }
            Err(downstream_owner::StartError::Attach(mut error)) => {
                // Normally impossible for a fresh owner. Recover its actual
                // completion if available; never claim success without it.
                let cancellation = error.owner.cancellation();
                cancellation.cancel();
                let completion = error.completion.take()
                    .or_else(|| error.owner.take_completion());
                let stage = match completion {
                    Some(completion) => OperationStage::Running {
                        cancellation, cleanup: Some(completion.shared()),
                    },
                    None => OperationStage::Unconfirmed,
                };
                self.register(&mut reservation, stage, Some(CloseError::Attachment));
                drop(error);
                Err(ClientError::Attachment)
            }
        }
    }

    fn register(
        &self, reservation: &mut Reservation, stage: OperationStage,
        failure: Option<CloseError>,
    ) -> bool {
        let mut lost_preparation = None;
        let active = {
            let mut state = lock(&self.inner.state);
            let healthy = state.healthy;
            let account = state.accounts.iter_mut()
                .find(|a| a.identity.generation == reservation.generation)
                .expect("request reservation retains its account");
            let operation = account.requests.iter_mut()
                .find(|r| Some(r.id) == reservation.operation)
                .expect("request reservation retains its record");
            debug_assert!(matches!(operation.stage, OperationStage::Preparing));
            operation.stage = stage;
            // Loss may have been reported while this slot was still Preparing.
            // An absent supervisor cannot settle a newly registered observer.
            // Report failure without releasing the owned work or reservation.
            if !healthy {
                if let OperationStage::OwnedPreparation(work) = &mut operation.stage {
                    lost_preparation = work.settled.take();
                }
            }
            if let Some(error) = failure {
                account.failure.get_or_insert(error);
                account.closing = true;
            }
            reservation.armed = false;
            healthy && !account.closing
        };
        if let Some(settled) = lost_preparation {
            let _ = settled.send(Err(CloseError::SupervisorLost));
        }
        self.inner.wake.wake();
        active
    }
}

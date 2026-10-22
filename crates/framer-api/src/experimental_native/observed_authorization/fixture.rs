use super::super::*;
use crate::experimental_native::{native_account::{NativeAccountClient, PendingPreparation},
    native_http::{FreshCastleToken, NativeEgress, NativeHttpLimits}};
use framer_core::{ids::FramerUserId, port::CastleToken};
use std::sync::{Mutex, atomic::AtomicUsize};
use tokio::{sync::Semaphore, time::{timeout, Duration, Instant}};

pub const BOUND: Duration = Duration::from_secs(2);
pub async fn gate(signal: &Semaphore) {
    timeout(BOUND, signal.acquire()).await.expect("fixture gate timed out")
        .expect("fixture gate closed").forget();
}
pub fn session() -> Session {
    Session { active_user: FramerUserId("observed-fixture-user".into()),
        cookie_header: "session=offline-fixture".into() }
}
pub fn account() -> (NativeAccountClient, AccountScope) {
    let client = NativeAccountClient::new().unwrap();
    let scope = client.open_account(AccountId(uuid::Uuid::new_v4()), session(),
        NativeHttpLimits::default(), NativeEgress::Direct).unwrap();
    (client, scope)
}
pub async fn close(client: &NativeAccountClient, scope: &AccountScope) {
    timeout(BOUND, client.close_account(scope).unwrap()).await.unwrap().unwrap();
}
#[derive(Clone, Copy)]
pub enum Inner { Success, Rejected, Unconfirmed }
#[derive(Clone, Copy)]
pub enum Cleanup { Closed, Identity(SourceIdentity), NotStarted, Unconfirmed, Lost }
#[derive(Clone, Copy)]
pub enum Opening { Started, NotStarted, Unconfirmed }
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snapshot {
    pub value: Result<String, PreparationFailure>,
    pub confirmed: bool,
}
pub struct Probe {
    pub opens: AtomicUsize,
    pub calls: AtomicUsize,
    pub stops: AtomicUsize,
    pub seals: AtomicUsize,
    pub source_entered: Semaphore,
    pub source_release: Semaphore,
    pub cleanup_entered: Semaphore,
    pub cleanup_release: Semaphore,
    pub stopped: Semaphore,
    pub sealed: Semaphore,
    pub source_abandoned: AtomicBool,
    pub cleanup_abandoned: AtomicBool,
    pub results: Mutex<Vec<Snapshot>>,
}
impl Probe {
    pub fn new() -> Arc<Self> {
        Arc::new(Self { opens: AtomicUsize::new(0), calls: AtomicUsize::new(0),
            stops: AtomicUsize::new(0), seals: AtomicUsize::new(0),
            source_entered: Semaphore::new(0), source_release: Semaphore::new(0),
            cleanup_entered: Semaphore::new(0), cleanup_release: Semaphore::new(0),
            stopped: Semaphore::new(0), sealed: Semaphore::new(0),
            source_abandoned: AtomicBool::new(false), cleanup_abandoned: AtomicBool::new(false),
            results: Mutex::new(Vec::new()) })
    }
    pub fn release_all(&self) {
        self.source_release.add_permits(1); self.cleanup_release.add_permits(1);
    }
}
struct Guard { probe: Arc<Probe>, cleanup: bool, finished: bool }
impl Guard {
    // Consuming the receiver makes the future capture the complete Drop guard.
    fn finish(mut self) { self.finished = true; }
}
impl Drop for Guard {
    fn drop(&mut self) {
        if !self.finished {
            let flag = if self.cleanup { &self.probe.cleanup_abandoned }
                else { &self.probe.source_abandoned };
            flag.store(true, Ordering::SeqCst);
        }
    }
}
struct Control(Arc<Probe>);
impl SourceControl for Control {
    fn request_stop(&self) {
        self.0.stops.fetch_add(1, Ordering::SeqCst);
        self.0.stopped.add_permits(1);
    }
    fn seal(&self) {
        self.0.seals.fetch_add(1, Ordering::SeqCst);
        self.0.sealed.add_permits(1);
    }
}
struct Source { probe: Arc<Probe>, inner: Inner }
impl CredentialSource for Source {
    fn issue<'a>(&'a self, scope: &'a AccountScope, _: &'a Session,
        _: &'a PreparationControl) -> BoxFuture<'a, PreparationResult<CredentialMaterial>> {
        async move {
            let mut guard = Guard { probe: self.probe.clone(), cleanup: false, finished: false };
            self.probe.calls.fetch_add(1, Ordering::SeqCst);
            self.probe.source_entered.add_permits(1);
            self.probe.source_release.acquire().await.unwrap().forget();
            guard.finished = true;
            match self.inner {
                Inner::Success => PreparationResult::finished(Ok(CredentialMaterial {
                    bearer: "fixture-bearer".into(), castle: FreshCastleToken::from_browser(
                        scope.account(), CastleToken("fixture-castle".into())) })),
                Inner::Rejected => PreparationResult::finished(Err(PreparationFailure::Rejected)),
                Inner::Unconfirmed => PreparationResult::unconfirmed(),
            }
        }.boxed()
    }
}
struct Factory { scope: AccountScope, probe: Arc<Probe>, inner: Inner,
    cleanup: Cleanup, opening: Opening }
impl ObservedSourceFactory for Factory {
    fn open(&self, _: &AccountScope) -> Result<OpenedSource, OpenFailure> {
        self.probe.opens.fetch_add(1, Ordering::SeqCst);
        match self.opening {
            Opening::NotStarted => return Err(OpenFailure::NotStarted),
            Opening::Unconfirmed => return Err(OpenFailure::Unconfirmed),
            Opening::Started => {},
        }
        let probe = self.probe.clone();
        let outcome = self.cleanup;
        let identity = SourceIdentity::from_scope(&self.scope);
        let guard = Guard { probe: probe.clone(), cleanup: true, finished: false };
        let cleanup = async move {
            probe.cleanup_entered.add_permits(1);
            probe.cleanup_release.acquire().await.unwrap().forget();
            let result = match outcome {
                Cleanup::Closed => SourceCleanup::Closed(identity),
                Cleanup::Identity(value) => SourceCleanup::Closed(value),
                Cleanup::NotStarted => SourceCleanup::NotStarted,
                Cleanup::Unconfirmed => SourceCleanup::Unconfirmed,
                Cleanup::Lost => panic!("fixture cleanup observer lost"),
            };
            guard.finish();
            result
        }.boxed();
        Ok(OpenedSource { scope: self.scope.clone(),
            source: Arc::new(Source { probe: self.probe.clone(), inner: self.inner }),
            control: Arc::new(Control(self.probe.clone())), cleanup })
    }
}
pub fn source(expected: &AccountScope, opened: &AccountScope, inner: Inner,
    cleanup: Cleanup, opening: Opening) -> (Arc<ObservedCredentialSource>, Arc<Probe>) {
    let probe = Probe::new();
    let factory = Arc::new(Factory { scope: opened.clone(), probe: probe.clone(), inner,
        cleanup, opening });
    (Arc::new(ObservedCredentialSource::new(expected.clone(), factory)), probe)
}
pub fn prepare(client: &NativeAccountClient, scope: &AccountScope,
    source: Arc<ObservedCredentialSource>, probe: Arc<Probe>) -> PendingPreparation<String> {
    prepare_as(client, scope, scope.clone(), source, probe, None)
}
pub fn prepare_as(client: &NativeAccountClient, owner: &AccountScope,
    requested: AccountScope, source: Arc<ObservedCredentialSource>, probe: Arc<Probe>,
    session_override: Option<Session>) -> PendingPreparation<String> {
    client.prepare_owned(owner, Instant::now() + Duration::from_secs(30),
        move |session, control| async move {
            let session = session_override.unwrap_or(session);
            let result = source.issue(&requested, &session, &control).await;
            let value = result.value.map(|material| material.bearer);
            probe.results.lock().unwrap().push(Snapshot {
                value: value.clone(), confirmed: result.cleanup_confirmed });
            PreparationResult { value, cleanup_confirmed: result.cleanup_confirmed }
        }).unwrap()
}
pub async fn settled(pending: PendingPreparation<String>) -> Result<String, PreparationFailure> {
    timeout(BOUND, pending).await.expect("preparation did not settle")
}
pub fn count(value: &AtomicUsize) -> usize { value.load(Ordering::SeqCst) }
pub fn intact(probe: &Probe) {
    assert!(!probe.source_abandoned.load(Ordering::SeqCst));
    assert!(!probe.cleanup_abandoned.load(Ordering::SeqCst));
}
pub fn reopen(client: &NativeAccountClient, scope: &AccountScope) -> AccountScope {
    client.open_account(scope.account(), session(), NativeHttpLimits::default(),
        NativeEgress::Direct).unwrap()
}
pub fn fenced(client: &NativeAccountClient, scope: &AccountScope) {
    assert!(client.open_account(scope.account(), session(), NativeHttpLimits::default(),
        NativeEgress::Direct).is_err());
}

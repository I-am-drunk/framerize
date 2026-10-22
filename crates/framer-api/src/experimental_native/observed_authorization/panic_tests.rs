use super::*;
use crate::experimental_native::{native_account::{NativeAccountClient, PendingPreparation},
    native_http::{FreshCastleToken, NativeEgress, NativeHttpLimits}};
use framer_core::{ids::FramerUserId, port::CastleToken};
use std::sync::{Mutex, atomic::AtomicUsize};
use tokio::{sync::Semaphore, time::{timeout, Duration, Instant}};
const BOUND: Duration = Duration::from_secs(2);

struct Probe {
    source_entered: Semaphore, source_release: Semaphore,
    cleanup_entered: Semaphore, cleanup_release: Semaphore, stopped: Semaphore,
    stops: AtomicUsize, seals: AtomicUsize, returned: Mutex<Option<bool>>,
}
impl Probe {
    fn new() -> Arc<Self> {
        Arc::new(Self { source_entered: Semaphore::new(0), source_release: Semaphore::new(0),
            cleanup_entered: Semaphore::new(0), cleanup_release: Semaphore::new(0),
            stopped: Semaphore::new(0), stops: AtomicUsize::new(0), seals: AtomicUsize::new(0),
            returned: Mutex::new(None) })
    }
}
#[derive(Clone, Copy)]
struct Panics { stop: bool, seal: bool, source: bool, cleanup: bool }
struct Control { probe: Arc<Probe>, panics: Panics }
impl SourceControl for Control {
    fn request_stop(&self) {
        self.probe.stops.fetch_add(1, Ordering::SeqCst);
        self.probe.stopped.add_permits(1);
        if self.panics.stop { panic!("fixture stop panic"); }
    }
    fn seal(&self) {
        self.probe.seals.fetch_add(1, Ordering::SeqCst);
        if self.panics.seal { panic!("fixture seal panic"); }
    }
}
struct Source { probe: Arc<Probe>, panics: Panics }
impl CredentialSource for Source {
    fn issue<'a>(&'a self, scope: &'a AccountScope, _: &'a Session,
        _: &'a PreparationControl) -> BoxFuture<'a, PreparationResult<CredentialMaterial>> {
        async move {
            self.probe.source_entered.add_permits(1);
            self.probe.source_release.acquire().await.unwrap().forget();
            if self.panics.source { panic!("fixture source unwind"); }
            PreparationResult::finished(Ok(CredentialMaterial {
                bearer: "offline-fixture-bearer".into(),
                castle: FreshCastleToken::from_browser(scope.account(), CastleToken("fixture".into())),
            }))
        }.boxed()
    }
}
struct Factory { probe: Arc<Probe>, panics: Panics }
impl ObservedSourceFactory for Factory {
    fn open(&self, scope: &AccountScope) -> Result<OpenedSource, OpenFailure> {
        let probe = self.probe.clone();
        let panics = self.panics;
        let identity = SourceIdentity::from_scope(scope);
        Ok(OpenedSource { scope: scope.clone(),
            source: Arc::new(Source { probe: probe.clone(), panics }),
            control: Arc::new(Control { probe: probe.clone(), panics }),
            cleanup: async move {
                probe.cleanup_entered.add_permits(1);
                probe.cleanup_release.acquire().await.unwrap().forget();
                if panics.cleanup { panic!("fixture cleanup unwind"); }
                SourceCleanup::Closed(identity)
            }.boxed(),
        })
    }
}
async fn gate(signal: &Semaphore) {
    timeout(BOUND, signal.acquire()).await.unwrap().unwrap().forget();
}
fn session() -> Session {
    Session { active_user: FramerUserId("panic-fixture-user".into()),
        cookie_header: "session=panic-offline-fixture".into() }
}
fn start(panics: Panics) -> (NativeAccountClient, AccountScope, Arc<Probe>, PendingPreparation<String>) {
    let client = NativeAccountClient::new().unwrap();
    let scope = client.open_account(AccountId(uuid::Uuid::new_v4()), session(),
        NativeHttpLimits::default(), NativeEgress::Direct).unwrap();
    let probe = Probe::new();
    let source = ObservedCredentialSource::new(scope.clone(),
        Arc::new(Factory { probe: probe.clone(), panics }));
    let requested = scope.clone();
    let record = probe.clone();
    let pending = client.prepare_owned(&scope, Instant::now() + Duration::from_secs(30),
        move |session, control| async move {
            let result = source.issue(&requested, &session, &control).await;
            *record.returned.lock().unwrap() = Some(result.cleanup_confirmed);
            PreparationResult { value: result.value.map(|value| value.bearer),
                cleanup_confirmed: result.cleanup_confirmed }
        }).unwrap();
    (client, scope, probe, pending)
}
async fn fenced_finish(client: NativeAccountClient, scope: AccountScope,
    pending: PendingPreparation<String>) {
    assert_eq!(timeout(BOUND, pending).await.unwrap(), Err(PreparationFailure::CleanupUnconfirmed));
    assert!(client.open_account(scope.account(), session(), NativeHttpLimits::default(),
        NativeEgress::Direct).is_err());
    let supervisor = client.supervisor_for_test();
    drop(client);
    timeout(BOUND, async {
        while !supervisor.is_finished() { tokio::task::yield_now().await; }
    }).await.expect("native supervisor survived fixture teardown");
    assert!(timeout(BOUND, scope.closed()).await.unwrap().is_err());
}

#[tokio::test]
async fn stop_panic_during_source_keeps_work_owned_and_poisoned_after_closed() {
    let (client, scope, probe, pending) = start(Panics {
        stop: true, seal: false, source: false, cleanup: false });
    gate(&probe.source_entered).await;
    pending.cancellation().cancel();
    gate(&probe.stopped).await;
    assert!(pending.completion().now_or_never().is_none());
    probe.source_release.add_permits(1);
    gate(&probe.cleanup_entered).await;
    assert!(pending.completion().now_or_never().is_none());
    probe.cleanup_release.add_permits(1);
    fenced_finish(client, scope, pending).await;
    assert_eq!(*probe.returned.lock().unwrap(), Some(false));
    assert!(probe.stops.load(Ordering::SeqCst) >= 1);
}

#[tokio::test]
async fn seal_panic_waits_for_cleanup_and_never_publishes_material() {
    let (client, scope, probe, pending) = start(Panics {
        stop: true, seal: true, source: false, cleanup: false });
    probe.source_release.add_permits(1);
    gate(&probe.cleanup_entered).await;
    assert_eq!(probe.seals.load(Ordering::SeqCst), 1);
    assert!(pending.completion().now_or_never().is_none());
    assert!(probe.returned.lock().unwrap().is_none());
    probe.cleanup_release.add_permits(1);
    fenced_finish(client, scope, pending).await;
    assert_eq!(*probe.returned.lock().unwrap(), Some(false));
    assert!(probe.stops.load(Ordering::SeqCst) >= 1);
}

#[tokio::test]
async fn stop_panic_during_cleanup_retains_original_observer_and_fence() {
    let (client, scope, probe, pending) = start(Panics {
        stop: true, seal: false, source: false, cleanup: false });
    probe.source_release.add_permits(1);
    gate(&probe.cleanup_entered).await;
    pending.cancellation().cancel();
    gate(&probe.stopped).await;
    assert!(pending.completion().now_or_never().is_none());
    probe.cleanup_release.add_permits(1);
    fenced_finish(client, scope, pending).await;
    assert_eq!(*probe.returned.lock().unwrap(), Some(false));
}

#[tokio::test]
async fn source_unwind_and_panicking_drop_stop_do_not_abort_process() {
    let (client, scope, probe, pending) = start(Panics {
        stop: true, seal: false, source: true, cleanup: false });
    gate(&probe.source_entered).await;
    probe.source_release.add_permits(1);
    fenced_finish(client, scope, pending).await;
    assert!(probe.stops.load(Ordering::SeqCst) >= 1);
    assert!(probe.returned.lock().unwrap().is_none());
    assert_eq!(probe.cleanup_entered.available_permits(), 0);
}

#[tokio::test]
async fn cleanup_unwind_and_panicking_drop_stop_do_not_abort_process() {
    let (client, scope, probe, pending) = start(Panics {
        stop: true, seal: false, source: false, cleanup: true });
    probe.source_release.add_permits(1);
    gate(&probe.cleanup_entered).await;
    probe.cleanup_release.add_permits(1);
    fenced_finish(client, scope, pending).await;
    assert_eq!(probe.seals.load(Ordering::SeqCst), 1);
    assert!(probe.stops.load(Ordering::SeqCst) >= 1);
    assert!(probe.returned.lock().unwrap().is_none());
}

use super::*;
use super::super::control_scope as scoped_startup;
use crate::account::client::{AccountScope, PreparationControl, PreparationResult, PreparationFailure};
use crate::account::device::DeviceGrant;
use crate::authorization::control::{BrowserControlMint, OfflineMintTarget};
use crate::service::sessions::Sessions;
use crate::transport::http::{NativeEgress, NativeHttpLimits};
use framer_core::port::{Session, SessionStore};
use futures::future::BoxFuture;
use std::sync::{Mutex, atomic::{AtomicBool, AtomicUsize, Ordering}};
use tokio::sync::Semaphore;

struct Scoped {
    scope: Mutex<Option<AccountScope>>,
    entered: Semaphore, release: Semaphore,
    mode: usize, closed: AtomicUsize,
    valid: AtomicBool, cancelled: AtomicBool, shutdown_after_account: AtomicBool,
}

impl Scoped {
    fn new(mode: usize, released: bool) -> Arc<Self> {
        Arc::new(Self { scope: Mutex::new(None), entered: Semaphore::new(0),
            release: Semaphore::new(usize::from(released)), mode, closed: AtomicUsize::new(0),
            valid: AtomicBool::new(false), cancelled: AtomicBool::new(false),
            shutdown_after_account: AtomicBool::new(false) })
    }
    async fn entered(&self) {
        timeout(Duration::from_secs(5), self.entered.acquire()).await.unwrap().unwrap().forget();
    }
}

fn account() -> AccountId { AccountId(uuid::Uuid::from_u128(1)) }

async fn seeded() -> Fixture {
    let fixture = Fixture::new().await;
    let store = FileSessionStore::new(fixture.config.profiles.join(".sessions")).unwrap();
    store.save(&Session { active_user: framer_core::ids::FramerUserId("fixture-user".into()),
        cookie_header: "session=fixture".into() }).await.unwrap();
    drop(store);
    fixture
}

fn local(fixture: &Fixture) -> (ServiceState, Sessions) {
    let store = Arc::new(SqliteStateStore::open(fixture.config.database.to_str().unwrap()).unwrap());
    let cookies = Arc::new(FileSessionStore::new(fixture.config.profiles.join(".sessions")).unwrap());
    let mut state = ServiceState::new(cookies, store, [account()]).unwrap();
    let sessions = Sessions::new(state.sessions.clone(), state.operation_ownership.clone().unwrap()).unwrap();
    state.sessions = sessions.store();
    (state, sessions)
}

#[tokio::test]
async fn prepared_lease_survives_idle_expiry_and_matches_the_request_lease() {
    let fixture = seeded().await;
    let (state, sessions) = local(&fixture);
    let peer = Scoped::new(0, true);
    let lease = scoped_startup::prepare(peer.clone(), &sessions, &state, account(),
        std::future::pending()).await.unwrap().unwrap();
    let scope = lease.scope().clone();
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(16)).await;
    tokio::task::yield_now().await;
    let still_open = scope.closed().now_or_never().is_none();
    let request = sessions.bridge().lease_account_observed(account(), scope.active_user().clone(),
        NativeHttpLimits::default(), NativeEgress::Direct).await.unwrap();
    let same = scope.same_generation(request.scope());
    let reusable = request.finish(true).await;
    tokio::time::resume();
    let closed = scoped_startup::close(Some(sessions.clone()), Some(lease)).await;
    let scope_closed = scope.closed().await;
    let peer_closed = peer.shutdown().await;
    drop((state, sessions));
    assert!(still_open && same && reusable && peer_closed);
    assert_eq!(closed, Ok(()));
    assert_eq!(scope_closed, Ok(()));
    assert!(peer.valid.load(Ordering::SeqCst));
    assert!(peer.shutdown_after_account.load(Ordering::SeqCst));
    assert!(fixture.sessions_available());
}

#[tokio::test]
async fn preparation_uses_the_selected_accounts_proxy_and_session() {
    let fixture = seeded().await;
    let (state, sessions) = local(&fixture);
    let mut record = state.state.get(account()).await.unwrap().unwrap();
    record.proxy = Some("http://127.0.0.1:9".into());
    state.state.upsert(&record).await.unwrap();
    let peer = Scoped::new(0, true);
    let lease = scoped_startup::prepare(peer.clone(), &sessions, &state, account(),
        std::future::pending()).await.unwrap().unwrap();
    let route = lease.scope().matches_egress(&NativeEgress::Proxy("http://127.0.0.1:9"));
    let closed = scoped_startup::close(Some(sessions.clone()), Some(lease)).await;
    drop((state, sessions));
    assert!(route && peer.valid.load(Ordering::SeqCst));
    assert_eq!(closed, Ok(()));
    assert!(fixture.sessions_available());
}

#[tokio::test]
async fn stopped_preparation_keeps_the_lease_until_the_peer_returns() {
    let fixture = seeded().await;
    let (state, sessions) = local(&fixture);
    let peer = Scoped::new(0, false);
    let (stop, stopped) = tokio::sync::oneshot::channel();
    let (owned_state, owned_sessions, owned_peer) = (state.clone(), sessions.clone(), peer.clone());
    let task = tokio::spawn(async move {
        scoped_startup::prepare(owned_peer, &owned_sessions, &owned_state, account(),
            async { let _ = stopped.await; }).await
    });
    peer.entered().await;
    stop.send(()).unwrap();
    tokio::task::yield_now().await;
    let held = !task.is_finished();
    peer.release.add_permits(1);
    let result = task.await.unwrap();
    let stopped = matches!(&result, Ok(None));
    let closed = scoped_startup::close(Some(sessions.clone()), result.ok().flatten()).await;
    drop((state, sessions));
    assert!(stopped && held && peer.cancelled.load(Ordering::SeqCst));
    assert_eq!(closed, Ok(()));
    assert!(fixture.sessions_available());
}

#[tokio::test]
async fn cancelled_or_unselected_preparation_does_not_invoke_the_peer() {
    let fixture = seeded().await;
    let (state, sessions) = local(&fixture);
    let peer = Scoped::new(0, true);
    let stopped = scoped_startup::prepare(peer.clone(), &sessions, &state, account(), async {}).await;
    let other = scoped_startup::prepare(peer.clone(), &sessions, &state, AccountId::new(),
        std::future::pending()).await;
    let closed = scoped_startup::close(Some(sessions.clone()), None).await;
    drop((state, sessions));
    assert!(matches!(stopped, Ok(None)));
    assert!(other.is_err());
    assert!(peer.scope.lock().unwrap().is_none());
    assert_eq!(closed, Ok(()));
    assert!(fixture.sessions_available());
}

#[tokio::test]
async fn startup_withholds_health_until_scoped_preparation_and_joins_on_stop() {
    let fixture = seeded().await;
    let socket = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let mut config = fixture.config.clone();
    config.address = socket.local_addr().unwrap();
    drop(socket);
    let url = format!("http://{}/api/health", config.address);
    let peer = Scoped::new(0, false);
    let (stop, stopped) = tokio::sync::oneshot::channel();
    let owned_peer = peer.clone();
    let task = tokio::spawn(crate::serve_with_browser_control(config, owned_peer, None,
        async { let _ = stopped.await; }));
    peer.entered().await;
    let client = reqwest::Client::builder().no_proxy().timeout(Duration::from_millis(100)).build().unwrap();
    let premature = client.get(&url).send().await;
    peer.release.add_permits(1);
    let ready = timeout(Duration::from_secs(5), async {
        loop {
            if let Ok(response) = client.get(&url).send().await {
                if response.status() == 200 { break; }
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }).await;
    stop.send(()).unwrap();
    let result = timeout(Duration::from_secs(5), task).await.unwrap().unwrap();
    assert!(premature.is_err() && ready.is_ok());
    assert_eq!(result, Ok(()));
    assert!(peer.valid.load(Ordering::SeqCst));
    assert!(peer.shutdown_after_account.load(Ordering::SeqCst));
    assert_eq!(peer.closed.load(Ordering::SeqCst), 1);
    assert!(fixture.sessions_available());
}

async fn rejected_startup(mode: usize) -> String {
    let fixture = seeded().await;
    let peer = Scoped::new(mode, true);
    let result = timeout(Duration::from_secs(5), crate::serve_with_browser_control(
        fixture.config.clone(), peer.clone(), None, std::future::pending())).await.unwrap();
    assert_eq!(peer.closed.load(Ordering::SeqCst), 1);
    assert!(peer.shutdown_after_account.load(Ordering::SeqCst));
    assert!(fixture.sessions_available());
    result.unwrap_err()
}

#[tokio::test]
async fn rejected_preparation_releases_the_startup_session_owner() {
    assert_eq!(rejected_startup(1).await, "browser credential control preparation failed");
}

#[tokio::test]
async fn panicking_preparation_still_joins_shutdown_and_releases_the_profile() {
    assert!(rejected_startup(2).await.contains("unconfirmed"));
}

#[tokio::test]
async fn missing_session_refuses_before_scoped_preparation() {
    let fixture = Fixture::new().await;
    let peer = Scoped::new(0, true);
    let result = timeout(Duration::from_secs(5), crate::serve_with_browser_control(
        fixture.config.clone(), peer.clone(), None, std::future::pending())).await.unwrap();
    assert_eq!(result.unwrap_err(), "browser credential control session unavailable");
    assert!(peer.scope.lock().unwrap().is_none());
    assert_eq!(peer.closed.load(Ordering::SeqCst), 1);
    assert!(fixture.sessions_available());
}

#[tokio::test]
async fn control_shutdown_error_keeps_its_precedence_after_scoped_stop() {
    let fixture = seeded().await;
    let peer = Scoped::new(3, true);
    let (stop, stopped) = tokio::sync::oneshot::channel();
    let task = tokio::spawn(crate::serve_with_browser_control(fixture.config.clone(),
        peer.clone(), None, async { let _ = stopped.await; }));
    peer.entered().await;
    stop.send(()).unwrap();

    let result = timeout(Duration::from_secs(5), task).await.unwrap().unwrap();
    assert_eq!(result.unwrap_err(), "browser credential control shutdown is unconfirmed");
    assert_eq!(peer.closed.load(Ordering::SeqCst), 1);
    assert!(peer.shutdown_after_account.load(Ordering::SeqCst));
    assert!(fixture.sessions_available());
}

#[tokio::test]
async fn cancelled_startup_joins_the_held_peer_before_releasing_the_profile() {
    let fixture = seeded().await;
    let peer = Scoped::new(0, false);
    let (stop, stopped) = tokio::sync::oneshot::channel();
    let task = tokio::spawn(crate::serve_with_browser_control(fixture.config.clone(),
        peer.clone(), None, async { let _ = stopped.await; }));
    peer.entered().await;
    stop.send(()).unwrap();
    tokio::task::yield_now().await;
    let held = !task.is_finished() && !fixture.sessions_available();
    peer.release.add_permits(1);
    let result = timeout(Duration::from_secs(5), task).await.unwrap().unwrap();
    assert!(held && peer.cancelled.load(Ordering::SeqCst));
    assert_eq!(result, Ok(()));
    assert_eq!(peer.closed.load(Ordering::SeqCst), 1);
    assert!(peer.shutdown_after_account.load(Ordering::SeqCst));
    assert!(fixture.sessions_available());
}

impl BrowserCredentialControl for Scoped {
    fn mint<'a>(&'a self, _: &'a AccountScope, _: &'a Session, _: &'a DeviceGrant,
        _: &'a OfflineMintTarget, _: u64, _: &'a PreparationControl)
        -> BoxFuture<'a, PreparationResult<BrowserControlMint>> {
        async { PreparationResult::finished(Err(PreparationFailure::Rejected)) }.boxed()
    }
    fn finish<'a>(&'a self, _: &'a DeviceGrant, _: bool, _: bool) -> BoxFuture<'a, bool> {
        async { true }.boxed()
    }
    fn shutdown(&self) -> BoxFuture<'_, bool> {
        async move {
            self.closed.fetch_add(1, Ordering::SeqCst);
            let settled = self.scope.lock().unwrap().as_ref()
                .is_none_or(|scope| scope.closed().now_or_never().is_some());
            self.shutdown_after_account.store(settled, Ordering::SeqCst);
            self.mode != 3
        }.boxed()
    }
    fn requires_account_scope(&self) -> bool { true }
    fn prepare(&self) -> BoxFuture<'_, bool> { async { panic!("unbound preparation") }.boxed() }
    fn prepare_for_account<'a>(&'a self, scope: &'a AccountScope, session: &'a Session,
        control: &'a PreparationControl) -> BoxFuture<'a, bool> {
        async move {
            *self.scope.lock().unwrap() = Some(scope.clone());
            self.valid.store(scope.active_user() == &session.active_user
                && session.cookie_header == "session=fixture", Ordering::SeqCst);
            self.entered.add_permits(1);
            self.release.acquire().await.unwrap().forget();
            self.cancelled.store(control.is_cancelled(), Ordering::SeqCst);
            assert_ne!(self.mode, 2, "fixture preparation panic");
            self.mode != 1
        }.boxed()
    }
}

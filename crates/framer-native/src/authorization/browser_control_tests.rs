use super::*;
use crate::account::client::{NativeAccountClient, PendingPreparation};
use crate::account::device::DeviceAuthority;
use std::sync::{Mutex, atomic::{AtomicUsize, Ordering}};
use std::time::Duration;
use tokio::sync::Semaphore;

#[path = "control_authorization_fixture.rs"]
pub(crate) mod fixture;

pub(crate) struct Ports {
    pub events: Mutex<Vec<&'static str>>,
    pub mode: AtomicUsize,
    pub change: AtomicUsize,
    pub mint_entered: Semaphore, pub mint_release: Semaphore,
    pub finish_entered: Semaphore, pub finish_release: Semaphore,
    pub prepare_entered: Semaphore, pub prepare_release: Semaphore,
    pub closed: Mutex<Vec<(bool, bool)>>,
    pub finished: Mutex<Vec<(bool, bool)>>,
}
impl Ports {
    pub fn new() -> Arc<Self> {
        Arc::new(Self { events: Mutex::new(vec![]), mode: AtomicUsize::new(0),
            change: AtomicUsize::new(0), mint_entered: Semaphore::new(0),
            mint_release: Semaphore::new(1), finish_entered: Semaphore::new(0),
            finish_release: Semaphore::new(1), prepare_entered: Semaphore::new(0),
            prepare_release: Semaphore::new(1), closed: Mutex::new(vec![]),
            finished: Mutex::new(vec![]) })
    }
    pub fn event(&self, event: &'static str) { self.events.lock().unwrap().push(event); }
}
pub(crate) async fn wait(gate: &Semaphore) {
    tokio::time::timeout(Duration::from_secs(3), gate.acquire()).await.unwrap().unwrap().forget();
}
#[async_trait::async_trait]
impl DeviceOwner for Ports {
    async fn open_native_device(&self, account: AccountId) -> Result<DeviceGrant, DeviceFailure> {
        self.event("open");
        Ok(DeviceGrant { account, host_generation: "a".repeat(32), lease: uuid::Uuid::new_v4(),
            device: "fixture-device".into(), authority: DeviceAuthority::Ledger })
    }
    async fn admit_native_mint(&self, _: &DeviceGrant, budget: u64) -> Result<u64, DeviceFailure> {
        assert!((1..=16_000).contains(&budget));
        self.event("admit");
        if self.mode.load(Ordering::SeqCst) == 5 { Err(DeviceFailure::Rejected) } else { Ok(71) }
    }
    async fn finish_native_mint(&self, _: &DeviceGrant, sequence: u64, joined: bool)
        -> Result<bool, DeviceFailure> {
        assert_eq!(sequence, 71);
        self.event(if joined { "rate_joined" } else { "rate_unknown" });
        if self.mode.load(Ordering::SeqCst) == 6 { Err(DeviceFailure::Unconfirmed) }
        else { Ok(joined) }
    }
    async fn close_native_device(&self, _: &DeviceGrant, joined: bool, risk: bool)
        -> Result<bool, DeviceFailure> {
        self.event("device_close");
        self.closed.lock().unwrap().push((joined, risk));
        Ok(joined)
    }
}

pub(crate) fn owner(ports: &Arc<Ports>) -> Arc<ControlDeviceOwner> {
    Arc::new(ControlDeviceOwner::new(ports.clone(), ports.clone()))
}
pub(crate) fn target() -> OfflineMintTarget {
    OfflineMintTarget::new(CredentialTarget { project: framer_core::ids::ProjectId("fixture-project".into()),
        path: "/ai/v3/responses/" }, "https://framer.com/projects/fixture-project".into()).unwrap()
}
impl BrowserCredentialControl for Ports {
    fn prepare(&self) -> BoxFuture<'_, bool> {
        async move {
            self.event("prepare"); self.prepare_entered.add_permits(1);
            wait(&self.prepare_release).await;
            self.event("prepared"); true
        }.boxed()
    }
    fn mint<'a>(&'a self, scope: &'a AccountScope, session: &'a Session, grant: &'a DeviceGrant,
        target: &'a OfflineMintTarget, budget: u64, _: &'a PreparationControl)
        -> BoxFuture<'a, PreparationResult<BrowserControlMint>> {
        async move {
            assert!((1..=16_000).contains(&budget));
            assert_eq!(&session.active_user, scope.active_user());
            assert!(session.cookie_header.contains("__cuid=fixture-device"));
            self.event("mint"); self.mint_entered.add_permits(1);
            wait(&self.mint_release).await;
            match self.mode.load(Ordering::SeqCst) {
                1 => return PreparationResult::unconfirmed(),
                2 => panic!("fixture mint panic"), _ => {}
            }
            let mut reply = BrowserControlMint { account: scope.account(), user: session.active_user.clone(),
                generation: scope.generation(), host_generation: grant.host_generation.clone(),
                lease: grant.lease, device: grant.device.clone(), page_url: target.page_url().into(),
                token: CastleToken("fixture-browser-token".into()), received_at: Instant::now() };
            corrupt(&mut reply, self.change.load(Ordering::SeqCst));
            self.event("mint_settled"); PreparationResult::finished(Ok(reply))
        }.boxed()
    }
    fn finish<'a>(&'a self, _: &'a DeviceGrant, joined: bool, risk: bool) -> BoxFuture<'a, bool> {
        assert_ne!(self.mode.load(Ordering::SeqCst), 4, "fixture synchronous finish panic");
        async move {
            self.event("control_finish"); self.finished.lock().unwrap().push((joined, risk));
            self.finish_entered.add_permits(1); wait(&self.finish_release).await;
            self.event("control_closed"); self.mode.load(Ordering::SeqCst) != 3
        }.boxed()
    }
    fn shutdown(&self) -> BoxFuture<'_, bool> {
        async move { self.event("shutdown"); true }.boxed()
    }
}

fn corrupt(reply: &mut BrowserControlMint, change: usize) {
    match change {
        1 => reply.account = AccountId::new(), 2 => reply.user.0.push('x'),
        3 => reply.generation += 1, 4 => reply.host_generation = "b".repeat(32),
        5 => reply.lease = uuid::Uuid::new_v4(), 6 => reply.device.push('x'),
        7 => reply.page_url.push('x'), 8 => reply.token.0.clear(),
        9 => reply.token.0 = "x".repeat(16 * 1024 + 1),
        10 => reply.received_at += Duration::from_secs(30), _ => {}
    }
}

pub(crate) struct Builder {
    pub owner: Arc<ControlDeviceOwner>, pub address: std::net::SocketAddr,
}
impl SourceBuilder for Builder {
    fn build(&self, _: AccountScope, _: Option<String>) -> Result<Arc<dyn CredentialSource>, Failure> {
        Err(Failure::MissingAuthority)
    }
    fn device_owner(&self) -> Option<Arc<dyn DeviceOwner>> { Some(self.owner.clone()) }
    fn build_with_device(&self, scope: AccountScope, proxy: Option<String>, device: Option<Arc<DeviceGrant>>)
        -> Result<Arc<dyn CredentialSource>, Failure> {
        assert!(proxy.is_none());
        let grant = device.ok_or(Failure::MissingAuthority)?;
        self.owner.claim(&grant).map_err(|_| Failure::MissingAuthority)?;
        Ok(Arc::new(BrowserControlSource { api: fixture::bound(&scope, self.address, NativeEgress::Direct),
            scope, target: target(), grant, owner: self.owner.clone() }))
    }
}

struct Case {
    client: NativeAccountClient, scope: AccountScope, ports: Arc<Ports>,
    owner: Arc<ControlDeviceOwner>, grant: Arc<DeviceGrant>, source: Arc<dyn CredentialSource>,
}
async fn case() -> (Case, tokio::net::TcpListener) {
    let (client, scope) = fixture::scope(NativeEgress::Direct);
    let ports = Ports::new(); let owner = owner(&ports);
    let grant = Arc::new(owner.open_native_device(scope.account()).await.unwrap());
    let listener = fixture::listener().await;
    let builder = Builder { owner: owner.clone(), address: listener.local_addr().unwrap() };
    let source = builder.build_with_device(scope.clone(), None, Some(grant.clone())).unwrap();
    (Case { client, scope, ports, owner, grant, source }, listener)
}
impl Case {
    fn start(&self, duration: Duration) -> PendingPreparation<()> {
        let source = self.source.clone(); let scope = self.scope.clone();
        self.client.prepare_owned(&self.scope, tokio::time::Instant::now() + duration,
            move |session, control| async move {
                let target = target();
                let wanted = CredentialTarget { project: target.project().clone(), path: target.path() };
                let result = source.issue_for_target(&scope, &session, &wanted, &control).await;
                PreparationResult { value: result.value.map(|_| ()), cleanup_confirmed: result.cleanup_confirmed }
            }).unwrap()
    }
}
fn auth(listener: tokio::net::TcpListener) -> tokio::task::JoinHandle<String> {
    fixture::serve_once(listener, fixture::response(200, &fixture::token().to_string(), ""))
}

#[tokio::test]
async fn settled_mint_stays_owned_until_request_finish() {
    let (case, listener) = case().await; let server = auth(listener);
    case.start(Duration::from_secs(2)).await.unwrap();
    assert!(fixture::result(server).await.contains("__cuid=fixture-device"));
    assert_eq!(*case.ports.events.lock().unwrap(), ["open", "admit", "mint", "mint_settled", "rate_joined"]);
    assert_eq!(case.start(Duration::from_secs(2)).await, Err(PreparationFailure::Rejected));
    assert!(case.owner.open_native_device(case.scope.account()).await.is_err());
    case.ports.finish_release.try_acquire().unwrap().forget();
    let closing = case.owner.close_native_device(&case.grant, true, false);
    tokio::pin!(closing);
    assert!(closing.as_mut().now_or_never().is_none());
    wait(&case.ports.finish_entered).await;
    assert!(case.ports.closed.lock().unwrap().is_empty());
    case.ports.finish_release.add_permits(1);
    assert_eq!(closing.await, Ok(true));
    assert_eq!(*case.ports.closed.lock().unwrap(), [(true, false)]);
    assert!(case.owner.close_native_device(&case.grant, true, false).await.is_err());
    fixture::close(&case.client, &case.scope).await;
}

#[tokio::test]
async fn every_mint_binding_and_future_timestamp_is_checked() {
    for change in 1..=10 {
        let (case, listener) = case().await; case.ports.change.store(change, Ordering::SeqCst);
        let server = auth(listener);
        assert_eq!(case.start(Duration::from_secs(2)).await, Err(PreparationFailure::Rejected), "{change}");
        fixture::result(server).await;
        assert_eq!(case.owner.close_native_device(&case.grant, true, false).await, Ok(true));
        fixture::close(&case.client, &case.scope).await;
    }
}
#[tokio::test]
async fn cancellation_waits_for_actual_mint_before_device_finish() {
    let (case, listener) = case().await; let server = auth(listener);
    case.ports.mint_release.try_acquire().unwrap().forget();
    let pending = case.start(Duration::from_secs(2));
    wait(&case.ports.mint_entered).await;
    pending.cancellation().cancel();
    assert!(pending.completion().now_or_never().is_none());
    assert!(case.ports.closed.lock().unwrap().is_empty());
    case.ports.mint_release.add_permits(1);
    assert_eq!(pending.await, Err(PreparationFailure::Cancelled));
    fixture::result(server).await;
    assert_eq!(case.owner.close_native_device(&case.grant, true, false).await, Ok(true));
    assert_eq!(*case.ports.closed.lock().unwrap(), [(true, false)]);
    fixture::close(&case.client, &case.scope).await;
}

#[tokio::test]
async fn unknown_mint_or_rate_cleanup_cannot_release_as_joined() {
    for mode in [1, 2, 6] {
        let (case, listener) = case().await; let server = auth(listener);
        case.ports.mode.store(mode, Ordering::SeqCst);
        assert_eq!(case.start(Duration::from_secs(2)).await, Err(PreparationFailure::CleanupUnconfirmed));
        fixture::result(server).await;
        assert_eq!(case.owner.close_native_device(&case.grant, true, false).await, Ok(false));
        assert_eq!(*case.ports.closed.lock().unwrap(), [(false, false)]);
        assert_eq!(*case.ports.finished.lock().unwrap(), [(false, false)]);
        assert!(case.scope.closed().await.is_err());
        assert!(case.owner.open_native_device(case.scope.account()).await.is_err());
    }
}
#[tokio::test]
async fn failed_or_panicking_finish_preserves_risk_and_refusal() {
    for mode in [3, 4] {
        let (case, listener) = case().await; let server = auth(listener);
        case.start(Duration::from_secs(2)).await.unwrap();
        fixture::result(server).await; case.ports.mode.store(mode, Ordering::SeqCst);
        assert_eq!(case.owner.close_native_device(&case.grant, true, true).await, Ok(false));
        assert_eq!(*case.ports.closed.lock().unwrap(), [(false, true)]);
        assert!(case.owner.open_native_device(case.scope.account()).await.is_err());
        fixture::close(&case.client, &case.scope).await;
    }
}

#[tokio::test]
async fn native_rate_refusal_prevents_browser_mint() {
    let (case, listener) = case().await; let server = auth(listener);
    case.ports.mode.store(5, Ordering::SeqCst);
    assert_eq!(case.start(Duration::from_secs(2)).await, Err(PreparationFailure::Rejected));
    fixture::result(server).await;
    assert_eq!(*case.ports.events.lock().unwrap(), ["open", "admit"]);
    assert_eq!(case.owner.close_native_device(&case.grant, true, false).await, Ok(true));
    assert!(case.ports.finished.lock().unwrap().is_empty());
    fixture::close(&case.client, &case.scope).await;
}

#[tokio::test]
async fn diagnostic_deadline_over_180_seconds_refuses_before_auth() {
    let (case, listener) = case().await;
    assert_eq!(case.start(Duration::from_secs(181)).await, Err(PreparationFailure::Rejected));
    assert_eq!(*case.ports.events.lock().unwrap(), ["open"]);
    drop(listener);
    assert_eq!(case.owner.close_native_device(&case.grant, true, false).await, Ok(true));
    fixture::close(&case.client, &case.scope).await;
}
#[tokio::test]
async fn bearer_must_cover_the_complete_diagnostic_deadline() {
    let (case, listener) = case().await;
    let mut token = fixture::token();
    token["expiresAt"] = (chrono::Utc::now() + chrono::Duration::seconds(60)).to_rfc3339().into();
    let server = fixture::serve_once(listener, fixture::response(200, &token.to_string(), ""));
    assert_eq!(case.start(Duration::from_secs(120)).await, Err(PreparationFailure::Rejected));
    fixture::result(server).await;
    assert_eq!(*case.ports.events.lock().unwrap(), ["open"]);
    assert_eq!(case.owner.close_native_device(&case.grant, true, false).await, Ok(true));
    fixture::close(&case.client, &case.scope).await;
}

#[tokio::test]
async fn factory_requires_original_grant_route_and_one_claim() {
    let (client, scope) = fixture::scope(NativeEgress::Direct);
    let ports = Ports::new(); let owner = owner(&ports);
    let factory = BrowserControlFactory { target: target(), owner: owner.clone() };
    assert!(factory.build(scope.clone(), None).is_err());
    assert!(factory.build_with_device(scope.clone(), None, None).is_err());
    let grant = Arc::new(owner.open_native_device(scope.account()).await.unwrap());
    assert!(factory.build_with_device(scope.clone(), Some("http://127.0.0.1:9".into()), Some(grant.clone())).is_err());
    let other = Arc::new(DeviceGrant { account: scope.account(), lease: uuid::Uuid::new_v4(),
        host_generation: grant.host_generation.clone(), device: grant.device.clone(), authority: grant.authority });
    assert!(factory.build_with_device(scope.clone(), None, Some(other)).is_err());
    assert!(factory.build_with_device(scope.clone(), None, Some(grant.clone())).is_ok());
    assert!(factory.build_with_device(scope.clone(), None, Some(grant.clone())).is_err());
    assert_eq!(*ports.events.lock().unwrap(), ["open"]);
    assert_eq!(owner.close_native_device(&grant, true, false).await, Ok(true));
    fixture::close(&client, &scope).await;
}

#[tokio::test]
async fn changed_session_or_target_refuses_before_auth_or_mint() {
    for change in 0..4 {
        let (case, listener) = case().await;
        let source = case.source.clone(); let scope = case.scope.clone();
        let pending = case.client.prepare_owned(&case.scope, tokio::time::Instant::now() + Duration::from_secs(2),
            move |mut session, control| async move {
                let mut wanted = CredentialTarget { project: target().project().clone(), path: target().path() };
                match change {
                    0 => session.active_user.0.push('x'),
                    1 => session.cookie_header.push_str("; __cuid=other-device"),
                    2 => wanted.project.0.push('x'), _ => wanted.path = "/ai/v3/messages/",
                }
                let result = source.issue_for_target(&scope, &session, &wanted, &control).await;
                PreparationResult { value: result.value.map(|_| ()), cleanup_confirmed: result.cleanup_confirmed }
            }).unwrap();
        assert_eq!(pending.await, Err(PreparationFailure::Rejected), "{change}");
        assert_eq!(*case.ports.events.lock().unwrap(), ["open"]);
        drop(listener);
        assert_eq!(case.owner.close_native_device(&case.grant, true, false).await, Ok(true));
        fixture::close(&case.client, &case.scope).await;
    }
}

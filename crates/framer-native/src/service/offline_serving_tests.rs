use super::*;
use crate::request::invocation::AttemptContext;
use crate::authorization::CredentialMaterial;
use crate::authorization::CredentialSource;
use crate::account::client::PreparationControl;
use crate::account::client::PreparationFailure;
use crate::account::client::PreparationResult;
use framer_core::account::AccountState;
use framer_core::account::ManagedAccount;
use framer_core::ids::AccountId;
use framer_core::ids::FramerUserId;
use framer_core::ids::ProjectId;
use framer_core::ids::TeamId;
use crate::account::device_owner::DeviceOwner;
use framer_core::port::Session;
use framer_core::port::SessionStore;
use framer_core::port::StateStore;
use std::sync::Mutex;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use std::time::Duration;
use tokio::sync::Semaphore;
use crate::authorization::source::Failure;

#[path = "offline_session_tests.rs"]
mod shared_sessions;
#[path = "retained_transport_tests.rs"]
mod retained_transport;
#[path = "browser_transport_tests.rs"]
mod browser_transport;
#[path = "retained_finalization_tests.rs"]
mod retained_finalization;
#[path = "admission_tests.rs"]
mod admission;
#[path = "offline_store_failure_tests.rs"]
mod store_failures;
#[path = "offline_device_tests.rs"]
mod device;
#[path = "disconnect_diagnostics_tests.rs"]
mod disconnect_diagnostics;

async fn wait(semaphore: &Semaphore) {
    tokio::time::timeout(Duration::from_secs(2), semaphore.acquire())
        .await.unwrap().unwrap().forget();
}
async fn finish(run: Run) -> Outcome {
    let outcome = tokio::time::timeout(Duration::from_secs(2), run.result).await.unwrap();
    assert_eq!(run.cleanup.await.is_ok(), outcome.cleanup_confirmed);
    outcome
}
fn start(state: ServiceState, ports: &Arc<Ports>, spec: Specification, sessions: &Sessions) -> Run {
    launch_with_source(state, spec, Arc::new(Builder(ports.clone())), sessions.clone(), None).unwrap()
}

#[tokio::test]
async fn invalid_context_and_target_reject_before_account_or_private_io() {
    for change in 0..4 {
        let (_, ports, mut spec, _sessions) = fixture();
        let mut url = "https://framer.com/projects/fixture-project".to_owned();
        match change { 0 => spec.context.turn_id.clear(),
            1 => spec.payload["stream"] = true.into(),
            2 => spec.token_limit = 4096, _ => url.push_str("-different") }
        assert!(Request::new(spec, url).is_err());
        assert!(ports.events.lock().unwrap().is_empty());
    }
}

#[tokio::test]
async fn sealed_app_launch_rejects_before_status_or_session_load() {
    let (state, ports, spec, sessions) = fixture();
    let seal = state.operation_ownership.as_ref().unwrap().seal().unwrap();
    assert!(launch_with_source(state, spec, Arc::new(Builder(ports.clone())), sessions.clone(), None).is_err());
    assert!(ports.events.lock().unwrap().is_empty());
    drop(seal);
}

#[tokio::test]
async fn cancellation_during_private_load_waits_without_issuing() {
    for drop_result in [false, true] {
        let (state, ports, spec, sessions) = fixture();
        ports.read.try_acquire().unwrap().forget();
        let run = start(state.clone(), &ports, spec, &sessions);
        wait(&ports.loading).await;
        if !drop_result { run.cancel(); }
        let mut result = Some(run.result);
        if drop_result { drop(result.take()); }
        let cleanup = run.cleanup;
        let mut seal = state.operation_ownership.as_ref().unwrap().seal().unwrap();
        assert!(cleanup.clone().now_or_never().is_none());
        assert!(seal.wait_application().now_or_never().is_none());
        assert_eq!(ports.builds.load(Ordering::SeqCst), 0);
        ports.read.add_permits(1);
        if let Some(result) = result {
            let outcome = tokio::time::timeout(Duration::from_secs(2), result).await.unwrap();
            assert_eq!(outcome.failure, Some(FailureKind::Cancelled));
            assert!(outcome.response.is_none() && outcome.cleanup_confirmed);
        }
        assert_eq!(tokio::time::timeout(Duration::from_secs(2), cleanup).await.unwrap(), Ok(()));
        let _drained = seal.wait_application().await.unwrap();
        assert_eq!(ports.builds.load(Ordering::SeqCst), 0);
        assert_eq!(*ports.events.lock().unwrap(), vec!["load", "loaded"]);
    }
}

#[tokio::test]
async fn cancellation_during_issuance_retains_source_and_session_cleanup() {
    for drop_result in [false, true] {
        let (state, ports, spec, sessions) = fixture();
        ports.issue.try_acquire().unwrap().forget();
        let run = start(state.clone(), &ports, spec, &sessions);
        wait(&ports.issuing).await;
        if !drop_result { run.cancel(); }
        let mut result = Some(run.result);
        if drop_result { drop(result.take()); }
        let cleanup = run.cleanup;
        tokio::time::timeout(Duration::from_secs(2), async {
            while !ports.cancelled.load(Ordering::SeqCst) { tokio::task::yield_now().await; }
        }).await.unwrap();
        assert!(cleanup.clone().now_or_never().is_none());
        let mut seal = state.operation_ownership.as_ref().unwrap().seal().unwrap();
        assert!(seal.wait_application().now_or_never().is_none());
        ports.issue.add_permits(1);
        if let Some(result) = result {
            let outcome = tokio::time::timeout(Duration::from_secs(2), result).await.unwrap();
            assert_eq!(outcome.failure, Some(FailureKind::Cancelled));
            assert!(outcome.response.is_none() && outcome.cleanup_confirmed);
        }
        assert_eq!(tokio::time::timeout(Duration::from_secs(2), cleanup).await.unwrap(), Ok(()));
        let _drained = seal.wait_application().await.unwrap();
        let scope = ports.scope.lock().unwrap().clone().unwrap();
        assert_eq!(scope.closed().await, Ok(()));
        assert_eq!(ports.builds.load(Ordering::SeqCst), 1);
    }
}

#[tokio::test]
async fn dropped_stream_keeps_actual_issuance_owned_until_cleanup() {
    use futures::StreamExt;
    for polled in [false, true] {
        let (state, ports, spec, sessions) = fixture();
        ports.issue.try_acquire().unwrap().forget();
        let (sender, receiver) = tokio::sync::mpsc::channel(1);
        let run = launch_with_source(state.clone(), spec, Arc::new(Builder(ports.clone())),
            sessions, Some(sender)).unwrap();
        wait(&ports.issuing).await;
        let mut body = crate::service::stream::response(run.result, receiver,
            FreeRoute::Responses, "gpt-4.1".into(), run.diagnostic_id).into_body().into_data_stream();
        if polled { body.next().await.unwrap().unwrap(); }
        drop(body);
        tokio::time::timeout(Duration::from_secs(2), async {
            while !ports.cancelled.load(Ordering::SeqCst) { tokio::task::yield_now().await; }
        }).await.unwrap();
        let mut seal = state.operation_ownership.as_ref().unwrap().seal().unwrap();
        assert!(run.cleanup.clone().now_or_never().is_none());
        assert!(seal.wait_application().now_or_never().is_none());
        ports.issue.add_permits(1);
        assert_eq!(tokio::time::timeout(Duration::from_secs(2), run.cleanup).await.unwrap(), Ok(()));
        let _drained = seal.wait_application().await.unwrap();
        assert_eq!(ports.scope.lock().unwrap().clone().unwrap().closed().await, Ok(()));
    }
}

#[tokio::test]
async fn unconfirmed_issuance_keeps_account_fenced_after_local_task_join() {
    let (state, ports, spec, sessions) = fixture();
    let identity = spec.identity.clone();
    ports.uncertain.store(true, Ordering::SeqCst);
    let outcome = finish(start(state.clone(), &ports, spec, &sessions)).await;
    assert_eq!(outcome.failure, Some(FailureKind::Cleanup));
    assert!(!outcome.cleanup_confirmed && outcome.response.is_none());
    let admission = native_control::admit(&state, identity, "gpt-4.1",
        Instant::now() + Duration::from_secs(1)).await;
    assert!(admission.is_err());
    assert_eq!(*ports.events.lock().unwrap(), vec!["load", "loaded"]);
}

#[tokio::test]
async fn changed_scope_after_session_read_rejects_before_source_build() {
    let (state, ports, spec, sessions) = fixture();
    ports.read.try_acquire().unwrap().forget();
    let run = start(state, &ports, spec, &sessions);
    wait(&ports.loading).await;
    ports.account.lock().unwrap().current_project = Some(ProjectId("changed-project".into()));
    ports.read.add_permits(1);
    let outcome = finish(run).await;
    assert_eq!(outcome.failure, Some(FailureKind::InvalidScope));
    assert!(outcome.cleanup_confirmed && outcome.response.is_none());
    assert_eq!(ports.builds.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn deadline_during_private_load_never_starts_issuance() {
    let (state, ports, mut spec, sessions) = fixture();
    spec.deadline = Instant::now() + Duration::from_millis(100);
    let deadline = spec.deadline;
    ports.read.try_acquire().unwrap().forget();
    let run = start(state, &ports, spec, &sessions);
    wait(&ports.loading).await;
    tokio::time::sleep_until(deadline).await;
    assert!(run.cleanup.clone().now_or_never().is_none());
    ports.read.add_permits(1);
    let outcome = finish(run).await;
    assert_eq!(outcome.failure, Some(FailureKind::Deadline));
    assert!(outcome.cleanup_confirmed && outcome.response.is_none());
    assert_eq!(ports.builds.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn explicit_proxy_and_authorization_failure_keep_scope_and_health() {
    let (state, ports, spec, sessions) = fixture();
    ports.account.lock().unwrap().proxy = Some("http://127.0.0.1:9".into());
    let outcome = finish(start(state, &ports, spec, &sessions)).await;
    assert_eq!(outcome.failure, Some(FailureKind::Authorization));
    assert!(outcome.cleanup_confirmed && outcome.response.is_none());
    assert_eq!(ports.builds.load(Ordering::SeqCst), 1);
    assert_eq!(ports.account.lock().unwrap().consecutive_failures, 0);
    let scope = ports.scope.lock().unwrap().clone().unwrap();
    assert_eq!(scope.closed().await, Ok(()));
}

#[tokio::test]
async fn source_builder_panic_cannot_release_the_account_as_clean() {
    let (state, ports, spec, sessions) = fixture();
    let identity = spec.identity.clone();
    ports.panic_build.store(true, Ordering::SeqCst);
    let outcome = finish(start(state.clone(), &ports, spec, &sessions)).await;
    assert_eq!(outcome.failure, Some(FailureKind::Cleanup));
    assert!(!outcome.cleanup_confirmed && outcome.response.is_none());
    assert!(native_control::admit(&state, identity, "gpt-4.1",
        Instant::now() + Duration::from_secs(1)).await.is_err());
}

struct Ports {
    account: Mutex<ManagedAccount>, events: Mutex<Vec<&'static str>>,
    others: Mutex<std::collections::HashMap<AccountId, ManagedAccount>>,
    stored: Mutex<Session>, other_id: AccountId,
    load_failure: AtomicUsize,
    loading: Semaphore, read: Semaphore, issuing: Semaphore, issue: Semaphore,
    builds: AtomicUsize, cancelled: AtomicBool, uncertain: AtomicBool,
    panic_build: AtomicBool, scope: Mutex<Option<AccountScope>>,
    device_mode: AtomicUsize, device_users: AtomicUsize,
    device_closing: Semaphore, device_finish: Semaphore,
}

fn fixture() -> (ServiceState, Arc<Ports>, Specification, Sessions) {
    fixture_browser(None)
}

fn fixture_browser(browser: Option<Arc<dyn crate::transport::browser::BrowserTransport>>)
    -> (ServiceState, Arc<Ports>, Specification, Sessions) {
    let id = AccountId::new();
    let other_id = AccountId::new();
    let account = ManagedAccount { id, alias: "fixture".into(), email: None,
        framer_user_id: FramerUserId("fixture-user".into()), castle_device_id: "fixture-device".into(),
        proxy: None, current_team: Some(TeamId("fixture-team".into())),
        current_project: Some(ProjectId("fixture-project".into())), extra_workspaces: vec![],
        state: AccountState::Ready, credit: None, consecutive_failures: 0,
        cooldown_step: 0, cooling_until_ms: None, cooldown_cause: None };
    let identity = port::Identity { account: id, user: account.framer_user_id.clone(),
        team: account.current_team.clone().unwrap(), project: account.current_project.clone().unwrap(),
        chat_session: "fixture-conversation".into() };
    let ports = Arc::new(Ports { account: Mutex::new(account), events: Mutex::new(vec![]),
        others: Mutex::new(std::collections::HashMap::new()),
        stored: Mutex::new(Session { active_user: FramerUserId("fixture-user".into()),
            cookie_header: "session=fixture".into() }),
        other_id,
        load_failure: AtomicUsize::new(0),
        loading: Semaphore::new(0), read: Semaphore::new(1),
        issuing: Semaphore::new(0), issue: Semaphore::new(1), builds: AtomicUsize::new(0),
        cancelled: AtomicBool::new(false), uncertain: AtomicBool::new(false),
        panic_build: AtomicBool::new(false), scope: Mutex::new(None),
        device_mode: AtomicUsize::new(0), device_users: AtomicUsize::new(0),
        device_closing: Semaphore::new(0), device_finish: Semaphore::new(1) });
    let mut state = ServiceState::new(ports.clone(), ports.clone(), [id, other_id]).unwrap();
    state.gate = Arc::new(framer_core::gate::ConcurrencyGate::with_config(framer_core::gate::GateConfig {
            initial: 1, min: 1, max: 1, recover_ms: 60_000, relax_ms: 60_000, min_interval_ms: 0,
        }));
    let sessions = Sessions::with_browser(state.sessions.clone(),
        state.operation_ownership.clone().unwrap(), browser,
        state.session_authorization.clone()).unwrap();
    state.sessions = sessions.store();
    let spec = Specification { identity, route: FreeRoute::Responses, token_limit: 32,
        payload: serde_json::json!({"model":"gpt-4.1", "input":"fixture", "stream":false}),
        context: AttemptContext { feature_category: "fixture".into(), feature_name: "fixture".into(),
            step: "fixture-step".into(), turn_id: "fixture-turn".into(), user_prompt: "fixture".into() },
        deadline: Instant::now() + Duration::from_secs(3) };
    (state, ports, spec, sessions)
}

struct Builder(Arc<Ports>);
impl SourceBuilder for Builder {
    fn build(&self, scope: AccountScope, proxy: Option<String>)
        -> Result<Arc<dyn CredentialSource>, Failure> {
        assert!(!self.0.panic_build.load(Ordering::SeqCst), "fixture builder failure");
        assert_eq!(proxy, self.0.account.lock().unwrap().proxy);
        assert!(scope.matches_egress(&proxy.as_deref().map(NativeEgress::Proxy)
            .unwrap_or(NativeEgress::Direct)));
        self.0.builds.fetch_add(1, Ordering::SeqCst);
        *self.0.scope.lock().unwrap() = Some(scope);
        Ok(self.0.clone())
    }
}

impl CredentialSource for Ports {
    fn issue<'a>(&'a self, _: &'a AccountScope, _: &'a Session, control: &'a PreparationControl)
        -> BoxFuture<'a, PreparationResult<CredentialMaterial>> {
        async move {
            self.issuing.add_permits(1);
            loop {
                if control.is_cancelled() { self.cancelled.store(true, Ordering::SeqCst); }
                tokio::select! {
                    value = self.issue.acquire() => { value.unwrap().forget(); break; }
                    _ = tokio::time::sleep(Duration::from_millis(1)) => {}
                }
            }
            if self.uncertain.load(Ordering::SeqCst) { PreparationResult::unconfirmed() }
            else { PreparationResult::finished(Err(PreparationFailure::Rejected)) }
        }.boxed()
    }
}

#[async_trait::async_trait]
impl StateStore for Ports {
    async fn get(&self, id: AccountId) -> Result<Option<ManagedAccount>, String> {
        let account = self.account.lock().unwrap();
        Ok(if account.id == id { Some(account.clone()) }
            else { self.others.lock().unwrap().get(&id).cloned() })
    }
    async fn list(&self) -> Result<Vec<ManagedAccount>, String> {
        let mut accounts = vec![self.account.lock().unwrap().clone()];
        accounts.extend(self.others.lock().unwrap().values().cloned());
        Ok(accounts)
    }
    async fn upsert(&self, account: &ManagedAccount) -> Result<(), String> {
        self.events.lock().unwrap().push("publish");
        let mut primary = self.account.lock().unwrap();
        if primary.id == account.id { *primary = account.clone(); }
        else { self.others.lock().unwrap().insert(account.id, account.clone()); }
        Ok(())
    }
    async fn remove(&self, _: AccountId) -> Result<(), String> { panic!("unexpected removal") }
}

#[async_trait::async_trait]
impl SessionStore for Ports {
    async fn load(&self, user: &FramerUserId) -> Result<Option<Session>, String> {
        let captured = self.stored.lock().unwrap().clone();
        assert_eq!(&captured.active_user, user);
        self.events.lock().unwrap().push("load");
        self.loading.add_permits(1);
        match self.load_failure.swap(0, Ordering::SeqCst) {
            1 => panic!("fixture private store failure"),
            2 => return Err("fixture store rejected".into()),
            3 => return Ok(None),
            _ => {}
        }
        self.read.acquire().await.unwrap().forget();
        self.events.lock().unwrap().push("loaded");
        Ok(Some(captured))
    }
    async fn save(&self, _: &Session) -> Result<(), String> { panic!("unexpected save") }
    async fn delete(&self, _: &FramerUserId) -> Result<(), String> { panic!("unexpected delete") }
}

#[async_trait::async_trait]
impl DeviceOwner for Ports {
    async fn open_native_device(&self, account: AccountId)
        -> Result<crate::account::device::DeviceGrant, crate::account::device::DeviceFailure> {
        use crate::account::device::DeviceFailure;
        use crate::account::device::DeviceGrant;
        self.events.lock().unwrap().push("device_open");
        assert_eq!(account, self.account.lock().unwrap().id);
        let mode = self.device_mode.load(Ordering::SeqCst);
        if mode == 1 { return Err(DeviceFailure::Rejected); }
        self.device_users.fetch_add(1, Ordering::SeqCst);
        if mode == 2 { return Err(DeviceFailure::Unconfirmed); }
        Ok(DeviceGrant { account, lease: uuid::Uuid::new_v4(),
            authority: crate::account::device::DeviceAuthority::Ledger,
            host_generation: "a".repeat(32), device: "fixture-device".into() })
    }
    async fn close_native_device(&self, grant: &crate::account::device::DeviceGrant,
        joined: bool, _risk: bool) -> Result<bool, crate::account::device::DeviceFailure> {
        assert_eq!(grant.account, self.account.lock().unwrap().id);
        if joined {
            let scope = self.scope.lock().unwrap().clone();
            if let Some(scope) = scope { assert_eq!(scope.closed().now_or_never(), Some(Ok(()))); }
        }
        self.device_closing.add_permits(1);
        self.device_finish.acquire().await.unwrap().forget();
        self.events.lock().unwrap().push("device_close");
        assert_ne!(self.device_mode.load(Ordering::SeqCst), 4, "fixture device close panic");
        if self.device_mode.load(Ordering::SeqCst) == 3 {
            return Err(crate::account::device::DeviceFailure::Unconfirmed);
        }
        if joined { assert_eq!(self.device_users.fetch_sub(1, Ordering::SeqCst), 1); }
        Ok(joined)
    }
}

use super::*;
use crate::authorization::control::{browser_control_fixture as shared, BrowserCredentialControl};
use crate::transport::browser::{BrowserOutcome, BrowserRequest, BrowserResponse, BrowserTransport};
use crate::transport::http::NativeCancellation;
use framer_core::{ids::{AccountId, FramerUserId, ProjectId, TeamId}, port::{Session, SessionStore, StateStore}};
use std::sync::Mutex;
use std::time::Duration;
use tokio::sync::Semaphore;

struct Stored(Session);
#[async_trait::async_trait]
impl SessionStore for Stored {
    async fn load(&self, user: &FramerUserId) -> Result<Option<Session>, String> {
        assert_eq!(user, &self.0.active_user); Ok(Some(self.0.clone()))
    }
    async fn save(&self, _: &Session) -> Result<(), String> { panic!("unexpected save") }
    async fn delete(&self, _: &FramerUserId) -> Result<(), String> { panic!("unexpected delete") }
}

struct Remote {
    ports: Arc<shared::Ports>, entered: Semaphore, release: Semaphore,
    calls: Mutex<Vec<BrowserRequest>>,
}
impl BrowserTransport for Arc<Remote> {
    fn send(&self, request: BrowserRequest, _: NativeCancellation) -> BoxFuture<'static, BrowserOutcome> {
        let this = self.clone();
        async move {
            this.calls.lock().unwrap().push(request);
            this.ports.event("transport_started"); this.entered.add_permits(1);
            shared::wait(&this.release).await; this.ports.event("transport_settled");
            BrowserOutcome { cleanup_confirmed: true, response: Ok(reply()) }
        }.boxed()
    }
}
fn reply() -> BrowserResponse {
    BrowserResponse { status: 200, usage: None, content_type: Some("application/json".into()),
        body: serde_json::to_vec(&serde_json::json!({
            "id":"fixture-result", "created_at":123, "status":"completed",
            "output":[{"type":"message", "role":"assistant", "content":[
                {"type":"output_text", "text":"fixture reply", "annotations":[]}]}],
            "usage":{"input_tokens":1, "output_tokens":1, "total_tokens":2},
            "incomplete_details":null, "service_tier":"default"
        })).unwrap() }
}

struct Case {
    state: ServiceState, sessions: Sessions, spec: Specification,
    source: Arc<shared::Builder>, remote: Arc<Remote>, ports: Arc<shared::Ports>,
}
async fn case() -> (Case, tokio::task::JoinHandle<String>) {
    let account = AccountId::new(); let ports = shared::Ports::new();
    let store = Arc::new(framer_store::SqliteStateStore::open(":memory:").unwrap());
    store.upsert(&serde_json::from_value(serde_json::json!({
        "id":account, "alias":"fixture", "framer_user_id":"fixture-user",
        "castle_device_id":"fixture-device", "current_team":"fixture-team",
        "current_project":"fixture-project", "state":framer_core::account::AccountState::Ready,
        "credit":null, "consecutive_failures":0, "cooldown_step":0
    })).unwrap()).await.unwrap();
    let stored = Arc::new(Stored(Session { active_user: FramerUserId("fixture-user".into()),
        cookie_header: "session=fixture-only-cookie".into() }));
    let mut state = ServiceState::new(stored, store, [account]).unwrap();
    state.gate = Arc::new(framer_core::gate::ConcurrencyGate::with_config(framer_core::gate::GateConfig {
        initial: 1, min: 1, max: 1, recover_ms: 60_000, relax_ms: 60_000, min_interval_ms: 0,
    }));
    let remote = Arc::new(Remote { ports: ports.clone(), entered: Semaphore::new(0),
        release: Semaphore::new(0), calls: Mutex::new(vec![]) });
    let sessions = Sessions::with_browser(state.sessions.clone(),
        state.operation_ownership.clone().unwrap(), Some(Arc::new(remote.clone())),
        state.session_authorization.clone()).unwrap();
    state.sessions = sessions.store();
    let spec = Specification { identity: port::Identity { account, user: FramerUserId("fixture-user".into()),
        team: TeamId("fixture-team".into()), project: ProjectId("fixture-project".into()),
        chat_session: "fixture-conversation".into() }, route: FreeRoute::Responses, token_limit: 32,
        payload: serde_json::json!({"model":"gpt-4.1", "input":"fixture", "stream":false}),
        context: crate::request::invocation::AttemptContext { feature_category:"fixture".into(),
            feature_name:"fixture".into(), step:"fixture".into(), turn_id:"fixture-turn".into(),
            user_prompt:"fixture".into() }, deadline: Instant::now() + Duration::from_secs(3) };
    let listener = shared::fixture::listener().await;
    let source = Arc::new(shared::Builder { owner: shared::owner(&ports),
        address: listener.local_addr().unwrap() });
    let server = shared::fixture::serve_once(listener, shared::fixture::response(200,
        &shared::fixture::token().to_string(), ""));
    (Case { state, sessions, spec, source, remote, ports }, server)
}
impl Case {
    fn specification(&self) -> Specification {
        Specification { identity: self.spec.identity.clone(), route: FreeRoute::Responses,
            payload: self.spec.payload.clone(), context: self.spec.context.clone(),
            deadline: self.spec.deadline, token_limit: self.spec.token_limit }
    }
    fn start(&self) -> Run {
        launch_with_source(self.state.clone(), self.specification(), self.source.clone(),
            self.sessions.clone(), None).unwrap()
    }
}
async fn finish(run: Run) -> Outcome {
    let result = tokio::time::timeout(Duration::from_secs(3), run.result).await.unwrap();
    assert_eq!(run.cleanup.await.is_ok(), result.cleanup_confirmed); result
}
#[tokio::test]
async fn successful_request_holds_control_until_transport_and_control_settle() {
    let (case, server) = case().await;
    case.ports.finish_release.try_acquire().unwrap().forget();
    let run = case.start(); shared::wait(&case.remote.entered).await;
    assert!(case.ports.finished.lock().unwrap().is_empty());
    assert!(run.cleanup.clone().now_or_never().is_none());
    case.remote.release.add_permits(1); shared::wait(&case.ports.finish_entered).await;
    assert!(case.ports.closed.lock().unwrap().is_empty());
    assert!(run.cleanup.clone().now_or_never().is_none());
    case.ports.finish_release.add_permits(1);
    let outcome = finish(run).await;
    assert!(outcome.response.is_some() && outcome.failure.is_none() && outcome.cleanup_confirmed);
    shared::fixture::result(server).await;
    assert_eq!(*case.ports.events.lock().unwrap(), ["open", "admit", "mint", "mint_settled",
        "rate_joined", "transport_started", "transport_settled", "control_finish", "control_closed", "device_close"]);
    {
        let calls = case.remote.calls.lock().unwrap(); assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].device, "fixture-device");
        assert_eq!(calls[0].bearer, "fixture-bearer");
        let wire: serde_json::Value = serde_json::from_slice(&calls[0].wire).unwrap();
        assert_eq!(wire["castleToken"], "fixture-browser-token");
    }
    let second = finish(case.start()).await;
    assert_eq!(second.failure, Some(FailureKind::Ineligible));
    assert_eq!(case.remote.calls.lock().unwrap().len(), 1);
    case.sessions.shutdown().await.unwrap();
}
#[tokio::test]
async fn cancelled_request_joins_transport_before_browser_control_and_device() {
    let (case, server) = case().await; let run = case.start();
    shared::wait(&case.remote.entered).await; run.cancel();
    assert!(run.cleanup.clone().now_or_never().is_none());
    assert!(case.ports.finished.lock().unwrap().is_empty());
    let mut seal = case.state.operation_ownership.as_ref().unwrap().seal().unwrap();
    assert!(seal.wait_application().now_or_never().is_none());
    case.remote.release.add_permits(1);
    let outcome = finish(run).await;
    assert_eq!(outcome.failure, Some(FailureKind::Cancelled));
    assert!(outcome.response.is_none() && outcome.cleanup_confirmed);
    shared::fixture::result(server).await;
    assert_eq!(*case.ports.closed.lock().unwrap(), [(true, false)]);
    let drained = seal.wait_application().await.unwrap();
    case.state.operation_ownership.as_ref().unwrap().check_application_drained(&drained).unwrap();
    case.sessions.bridge().shutdown().await.unwrap();
    case.sessions.native().shutdown_idle().await.unwrap();
}
#[tokio::test]
async fn unconfirmed_control_finish_suppresses_success_and_fences_account() {
    let (case, server) = case().await;
    case.ports.mode.store(3, std::sync::atomic::Ordering::SeqCst);
    case.remote.release.add_permits(1);
    let outcome = finish(case.start()).await;
    assert_eq!(outcome.failure, Some(FailureKind::Cleanup));
    assert!(outcome.response.is_none() && !outcome.cleanup_confirmed);
    shared::fixture::result(server).await;
    assert_eq!(*case.ports.closed.lock().unwrap(), [(false, false)]);
    assert!(native_control::admit(&case.state, case.spec.identity.clone(), "gpt-4.1",
        Instant::now() + Duration::from_secs(1)).await.is_err());
}

#[tokio::test]
async fn preparation_is_joined_when_startup_is_stopped() {
    let ports = shared::Ports::new();
    ports.prepare_release.try_acquire().unwrap().forget();
    let (stop, stopped) = tokio::sync::oneshot::channel();
    let preparing = crate::service::startup::prepare_control(ports.as_ref(), async { stopped.await.unwrap() });
    tokio::pin!(preparing);
    assert!(preparing.as_mut().now_or_never().is_none());
    shared::wait(&ports.prepare_entered).await;
    stop.send(()).unwrap();
    assert!(preparing.as_mut().now_or_never().is_none());
    assert_eq!(*ports.events.lock().unwrap(), ["prepare"]);
    ports.prepare_release.add_permits(1);
    assert_eq!(preparing.await, Ok(false));
    assert!(ports.shutdown().await);
    assert_eq!(*ports.events.lock().unwrap(), ["prepare", "prepared", "shutdown"]);
}

#[tokio::test]
async fn already_stopped_startup_wins_over_ready_preparation() {
    let ports = shared::Ports::new();
    assert_eq!(crate::service::startup::prepare_control(ports.as_ref(), async {}).await, Ok(false));
    assert_eq!(*ports.events.lock().unwrap(), ["prepare", "prepared"]);
    assert!(ports.shutdown().await);
}

#[tokio::test]
async fn explicit_control_entry_rejects_multiple_accounts_before_preparation() {
    let ports = shared::Ports::new();
    let config = crate::ServeConfig {
        database: "/must-not-open".into(), profiles: "/must-not-open".into(),
        python: "/must-not-execute".into(), runtime_script: "/must-not-execute".into(),
        worker_config: "/must-not-read".into(), selected_accounts: vec![
            uuid::Uuid::new_v4().to_string(), uuid::Uuid::new_v4().to_string()],
        address: "127.0.0.1:0".parse().unwrap(), expire_legacy_from: None,
    };
    let result = crate::serve_with_browser_control(config, ports.clone(), None, std::future::pending()).await;
    assert_eq!(result.unwrap_err(), "browser credential control requires one selected account");
    assert!(ports.events.lock().unwrap().is_empty());
}

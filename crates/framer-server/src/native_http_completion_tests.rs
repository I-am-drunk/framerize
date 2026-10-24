//! Inert native routes retain HTTP and independent cleanup ownership.
use super::*;
use axum::{body::{to_bytes, Body, Bytes}, http::Request};
use crate::operation_ownership::ServingOwnership;
use framer_core::{native_dispatch as n, operation_gate::OperationGate};
use framer_core::browser_lifecycle::{BrowserAccountState, BrowserAccountStatus,
    IdleCloseOutcome, IdleCloseResult};
use framer_core::ids::AccountId;
use framer_core::native_dispatch::InvocationProvider;
use framer_core::port::{BrowserBridge, CastleToken};
use futures::{future::BoxFuture, FutureExt};
use http_body::Body as _;
use serde_json::json;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::Duration;
use tokio::sync::{oneshot, OwnedSemaphorePermit};
use tower::ServiceExt;

#[path = "native_dispatch_fixture.rs"]
mod fixture;

const WAIT: Duration = Duration::from_secs(3);
const RAW: &str = "/paid/openai/v1/responses";
const CHAIN: &str = "/openai/v1/responses";
const SESSION: &str = "native-http-completion-fixture";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Acquisition { BeforeCloseBusy, CloseError, CloseUnknown, CloseDeadline, StatusUnknown }

struct Lifecycle {
    mode: Acquisition,
    observed: Mutex<Vec<AccountId>>,
    closes: AtomicUsize,
}

struct HeldProvider {
    inner: Arc<fixture::FakeProvider>,
    cleanup: n::Cleanup,
}
struct HeldCall {
    inner: Box<dyn n::OwnedInvocation>,
    cleanup: n::Cleanup,
}

fn hold_cleanup<T: Send + 'static>(mut run: InvocationRun<T>, proof: n::Cleanup) -> InvocationRun<T> {
    let cleanup = run.cleanup;
    run.cleanup = async move { proof.await?; cleanup.await }.boxed().shared();
    run
}

impl n::OwnedInvocation for HeldCall {
    fn raw(self: Box<Self>, payload: Value) -> InvocationRun<n::RawResult> {
        hold_cleanup(self.inner.raw(payload), self.cleanup)
    }
    fn messages(self: Box<Self>, payload: Value)
        -> InvocationRun<n::Completed<convert::AssembledResponse>> {
        hold_cleanup(self.inner.messages(payload), self.cleanup)
    }
    fn responses(self: Box<Self>, payload: Value)
        -> InvocationRun<n::Completed<convert::AssembledResponses>> {
        hold_cleanup(self.inner.responses(payload), self.cleanup)
    }
}

impl n::InvocationProvider for HeldProvider {
    fn policy(&self) -> Policy { self.inner.policy.clone() }
    fn contains_account(&self, id: AccountId) -> bool { self.inner.contains_account(id) }
    fn supports_pool_admission(&self) -> bool { true }
    fn prepare(&self, _: Preparation, _: OwnedSemaphorePermit)
        -> BoxFuture<'_, Result<Box<dyn n::OwnedInvocation>, DispatchError>> {
        panic!("native ownership must use prepare_in_pool")
    }
    fn prepare_in_pool(&self, request: Preparation, permit: OwnedSemaphorePermit,
        admission: n::PoolAdmission)
        -> BoxFuture<'_, Result<Box<dyn n::OwnedInvocation>, DispatchError>> {
        assert_eq!(request.identity.account, admission.account());
        async move {
            let inner = self.inner.prepare(request, permit).await?;
            let call = Box::new(HeldCall { inner, cleanup: self.cleanup.clone() });
            Ok(pool_fixture::hold(call, admission))
        }.boxed()
    }
}

#[async_trait::async_trait]
impl BrowserBridge for Lifecycle {
    fn supports_browser_lifecycle(&self) -> bool { true }
    async fn browser_status(&self, account: &AccountId) -> Result<BrowserAccountStatus, String> {
        self.observed.lock().unwrap().push(*account);
        Ok(BrowserAccountStatus {
            account_id: *account, host_generation: Some("native-http-fixture-host".into()),
            account_incarnation: 1,
            state: if self.mode == Acquisition::StatusUnknown { BrowserAccountState::Unconfirmed }
                else { BrowserAccountState::Resident },
            active_commands: usize::from(self.mode == Acquisition::BeforeCloseBusy),
        })
    }
    async fn close_idle_browser(&self, account: &AccountId, expected: &BrowserAccountStatus)
        -> Result<IdleCloseResult, String> {
        assert_eq!(*account, AccountId(uuid::Uuid::from_u128(2)));
        assert_eq!(expected.account_id, *account);
        self.closes.fetch_add(1, Ordering::SeqCst);
        match self.mode {
            Acquisition::CloseError => Err("fixture close failure".into()),
            Acquisition::CloseUnknown => Ok(IdleCloseResult {
                outcome: IdleCloseOutcome::Unconfirmed, status: expected.clone(),
            }),
            Acquisition::CloseDeadline => std::future::pending().await,
            _ => panic!("a local refusal must not close a browser"),
        }
    }
    async fn castle_token(&self, _: &AccountId) -> Result<CastleToken, String> {
        panic!("native fixture must not mint through the browser")
    }
    async fn auth_begin(&self, _: &AccountId, _: &str) -> Result<(), String> {
        panic!("unexpected authentication")
    }
    async fn auth_complete(&self, _: &AccountId, _: &str) -> Result<(String, Value), String> {
        panic!("unexpected authentication")
    }
    async fn seed_session(&self, _: &AccountId, _: &str) -> Result<(), String> {
        panic!("unexpected session mutation")
    }
    async fn ai_post(&self, _: &AccountId, _: &str, _: &str, _: &Value)
        -> Result<(u16, Option<String>, String), String> {
        panic!("native fixture must not use browser inference")
    }
}

async fn request(state: &AppState, path: &str, body: Value, session: Option<&str>) -> Response {
    let mut request = Request::builder().method("POST").uri(path)
        .header("content-type", "application/json")
        .header("x-framerize-account", fixture::account().id.0.to_string());
    if let Some(session) = session { request = request.header("x-framerize-session", session); }
    tokio::time::timeout(WAIT, crate::router(state.clone(), "/fixture-no-web")
        .oneshot(request.body(Body::from(body.to_string())).unwrap())).await.unwrap().unwrap()
}

async fn dispose(response: Response, consume: bool) -> Option<Bytes> {
    assert!(!response.body().is_end_stream());
    if consume {
        Some(tokio::time::timeout(WAIT, to_bytes(response.into_body(), 65536)).await.unwrap().unwrap())
    } else { drop(response); None }
}

async fn until(mut condition: impl FnMut() -> bool) {
    tokio::time::timeout(WAIT, async {
        while !condition() { tokio::task::yield_now().await; }
    }).await.unwrap();
}

async fn next_admission(state: &AppState, gate: &OperationGate, uncertain: bool) {
    assert_eq!(gate.stats().unwrap().active, 1); // Unmanaged host remains.
    assert_eq!(gate.stats().unwrap().unconfirmed, uncertain);
    let response = crate::router(state.clone(), "/fixture-no-web").oneshot(
        Request::builder().uri("/api/hello").body(Body::empty()).unwrap()).await.unwrap();
    assert_eq!(response.status(), if uncertain { StatusCode::SERVICE_UNAVAILABLE } else { StatusCode::OK });
    let body = to_bytes(response.into_body(), 1024).await.unwrap();
    assert_eq!(body.as_ref(), if uncertain { b"operation admission closed".as_slice() } else { b"ok".as_slice() });
    assert_eq!(gate.stats().unwrap().active, 1);
    assert_eq!(gate.stats().unwrap().unconfirmed, uncertain);
}

fn owned() -> (AppState, Arc<fixture::Spy>, Arc<fixture::FakeProvider>, OperationGate) {
    let (mut state, spy, provider) = fixture::fixture(true, None);
    let (ownership, gate) = ServingOwnership::new_with_host_factory(
        state.wave.clone(), 32, |gate| gate).unwrap();
    state.operation_ownership = Some(ownership);
    (state, spy, provider, gate)
}

fn payload(path: &str) -> Value {
    if path.ends_with("/responses") {
        json!({"model":"gpt-4.1", "stream":false, "input":"fixture", "max_output_tokens":32})
    } else {
        json!({"model":"claude-sonnet-5-5", "stream":false, "max_tokens":32,
            "messages":[{"role":"user", "content":"fixture"}]})
    }
}

fn acquisition_fixture(mode: Acquisition)
    -> (AppState, Arc<fixture::FakeProvider>, OperationGate, Arc<Lifecycle>, AccountId) {
    let (mut state, _, mut provider, gate) = owned();
    state.serving_accounts = Arc::new(crate::ServingAccountPolicy::from_config(
        crate::PoolConfig { active_capacity: 1, ..Default::default() }).unwrap());
    let retained = if mode == Acquisition::StatusUnknown { fixture::account().id }
        else { AccountId(uuid::Uuid::from_u128(2)) };
    if mode != Acquisition::StatusUnknown {
        match state.serving_accounts.residents.try_acquire(retained, None).unwrap() {
            framer_core::resident_pool::Acquire::Lease(lease) => drop(lease),
            _ => panic!("empty fixture pool must admit the retained account"),
        }
    }
    if mode == Acquisition::CloseDeadline {
        state.native_dispatch = None;
        Arc::get_mut(&mut provider).unwrap().policy.timeout = Duration::from_millis(250);
        state.native_dispatch = Some(provider.clone());
    }
    let browser = Arc::new(Lifecycle { mode, observed: Mutex::new(vec![]), closes: AtomicUsize::new(0) });
    state.browser = browser.clone();
    (state, provider, gate, browser, retained)
}

fn held_fixture(fail_result: bool, timeout: Duration) -> (AppState, Arc<fixture::Spy>, Arc<fixture::FakeProvider>,
    OperationGate, oneshot::Sender<Result<(), FailureKind>>) {
    let (mut state, spy, mut provider, gate) = owned();
    state.native_dispatch = None;
    let inner = Arc::get_mut(&mut provider).unwrap();
    inner.failure = fail_result.then_some(403);
    inner.policy.timeout = timeout;
    let (release, receive) = oneshot::channel();
    let cleanup = async move { receive.await.unwrap_or(Err(FailureKind::Cleanup)) }.boxed().shared();
    state.native_dispatch = Some(Arc::new(HeldProvider { inner: provider.clone(), cleanup }));
    (state, spy, provider, gate, release)
}

#[tokio::test]
async fn native_nonstream_waiter_disconnect_keeps_cleanup_and_completes_http() {
    for path in [RAW, CHAIN] {
        let (state, spy, provider, gate, release) = held_fixture(false, WAIT);
        let request_state = state.clone();
        let task = tokio::spawn(async move {
            request(&request_state, path, payload(path), Some(SESSION)).await
        });
        until(|| provider.records.lock().unwrap().len() == 1).await;
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        assert_eq!(gate.stats().unwrap().active, 3); // Host, HTTP, native cleanup.
        assert_eq!(state.serving_accounts.residents.snapshot().unwrap().residents[0].users, 1);
        release.send(Ok(())).unwrap();
        until(|| gate.stats().unwrap().active == 1).await;
        let snapshot = state.serving_accounts.residents.snapshot().unwrap();
        assert_eq!(snapshot.residents[0].users, 0);
        assert!(!snapshot.residents[0].uncertain);
        assert_eq!(*provider.preparations.lock().unwrap(), 1);
        assert_eq!(provider.records.lock().unwrap().len(), 1);
        assert!(spy.calls.lock().unwrap().is_empty());
        next_admission(&state, &gate, false).await;
    }
}

#[tokio::test]
async fn native_http_errors_and_deadlines_leave_cleanup_to_the_admitted_guardian() {
    for path in [RAW, CHAIN] { for fail_result in [false, true] {
        for confirmed in [false, true] { for consume in [false, true] {
            held_cleanup_case(path, fail_result, confirmed, consume).await;
        } }
    } }
}

async fn held_cleanup_case(path: &str, fail_result: bool, confirmed: bool, consume: bool) {
    let (state, spy, provider, gate, release) = held_fixture(fail_result, Duration::from_millis(250));
    let response = request(&state, path, payload(path), Some(SESSION)).await;
    let kind = if fail_result { FailureKind::Upstream } else { FailureKind::Deadline };
    assert_eq!(response.status(), if fail_result { StatusCode::FORBIDDEN } else { StatusCode::GATEWAY_TIMEOUT });
    assert_eq!(response.extensions().get::<FailureKind>(), Some(&kind));
    if fail_result {
        let attempts = response.extensions().get::<Arc<Vec<AttemptMetadata>>>().unwrap();
        assert_eq!(attempts.len(), 1);
        assert_eq!(attempts[0].status, 403);
    }
    if let Some(bytes) = dispose(response, consume).await {
        if fail_result { assert_eq!(bytes.as_ref(), b"fixture upstream rejection\nexact bytes"); }
        else {
            let value: Value = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(value["error"]["code"], "Deadline");
        }
    }
    assert_eq!(gate.stats().unwrap().active, 2); // Host and native cleanup.
    assert!(!gate.stats().unwrap().unconfirmed);
    let snapshot = state.serving_accounts.residents.snapshot().unwrap();
    assert_eq!(snapshot.residents[0].users, 1);
    assert!(!snapshot.residents[0].uncertain);
    release.send(if confirmed { Ok(()) } else { Err(FailureKind::Cleanup) }).unwrap();
    until(|| gate.stats().unwrap().active == 1).await;
    let snapshot = state.serving_accounts.residents.snapshot().unwrap();
    assert_eq!(snapshot.residents[0].users, usize::from(!confirmed));
    assert_eq!(snapshot.residents[0].uncertain, !confirmed);
    assert_eq!(*provider.preparations.lock().unwrap(), 1);
    assert_eq!(provider.records.lock().unwrap().len(), 1);
    assert!(spy.calls.lock().unwrap().is_empty());
    next_admission(&state, &gate, !confirmed).await;
}

#[tokio::test]
async fn native_acquisition_keeps_typed_cleanup_and_timeout_uncertainty() {
    for mode in [Acquisition::BeforeCloseBusy, Acquisition::CloseError,
        Acquisition::CloseUnknown, Acquisition::CloseDeadline, Acquisition::StatusUnknown] {
        for path in [RAW, CHAIN] { for consume in [false, true] {
            acquisition_case(mode, path, consume).await;
        } }
    }
}

async fn acquisition_case(mode: Acquisition, path: &str, consume: bool) {
    let (state, provider, gate, browser, retained) = acquisition_fixture(mode);
    let response = request(&state, path, payload(path), Some(SESSION)).await;
    let deadline = mode == Acquisition::CloseDeadline;
    let kind = if deadline { FailureKind::Deadline } else { FailureKind::Ineligible };
    assert_eq!(response.status(), if deadline { StatusCode::GATEWAY_TIMEOUT }
        else { StatusCode::SERVICE_UNAVAILABLE });
    assert_eq!(response.extensions().get::<FailureKind>(), Some(&kind));
    if let Some(bytes) = dispose(response, consume).await {
        let value: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(value["error"]["code"], format!("{kind:?}"));
    }
    assert_eq!(*provider.preparations.lock().unwrap(), 0);
    assert!(provider.records.lock().unwrap().is_empty());
    assert_eq!(*browser.observed.lock().unwrap(), vec![retained]);
    let closed = matches!(mode, Acquisition::CloseError | Acquisition::CloseUnknown | Acquisition::CloseDeadline);
    assert_eq!(browser.closes.load(Ordering::SeqCst), usize::from(closed));
    let snapshot = state.serving_accounts.residents.snapshot().unwrap();
    assert_eq!(snapshot.occupied, 1);
    assert_eq!(snapshot.residents.len(), 1);
    let resident = &snapshot.residents[0];
    assert_eq!(resident.account, retained);
    assert_eq!(resident.users, 0);
    assert_eq!(resident.replacement, None);
    let uncertain = mode != Acquisition::BeforeCloseBusy;
    assert_eq!(resident.uncertain, uncertain);
    next_admission(&state, &gate, uncertain).await;
}

#[tokio::test]
async fn native_nonstream_completion_releases_http_after_body_consumption_or_discard() {
    for path in [RAW, CHAIN, "/anthropic/v1/messages", "/openai/v1/chat/completions"] {
        for consume in [false, true] {
            let (state, spy, provider, gate) = owned();
            let response = request(&state, path, payload(path), Some(SESSION)).await;
            assert_eq!(response.status(), StatusCode::OK);
            assert_eq!(response.headers()["x-framerize-session"], SESSION);
            let attempts = response.extensions().get::<Arc<Vec<AttemptMetadata>>>().unwrap();
            assert_eq!(attempts.len(), 1);
            assert_eq!(attempts[0].usage_header.as_deref(), Some("fixture-usage"));
            if let Some(bytes) = dispose(response, consume).await {
                let value: Value = serde_json::from_slice(&bytes).unwrap();
                if path == RAW { assert_eq!(value, json!({"fixture":true})); }
                else { assert!(std::str::from_utf8(&bytes).unwrap().contains("fixture")); }
            }
            assert_eq!(*provider.preparations.lock().unwrap(), 1);
            assert_eq!(provider.records.lock().unwrap().len(), 1);
            assert!(spy.calls.lock().unwrap().is_empty());
            let snapshot = state.serving_accounts.residents.snapshot().unwrap();
            assert_eq!(snapshot.residents.len(), 1);
            assert_eq!(snapshot.residents[0].users, 0);
            assert!(!snapshot.residents[0].uncertain);
            next_admission(&state, &gate, false).await;
        }
    }
}

#[tokio::test]
async fn native_local_refusals_complete_http_for_consumed_and_discarded_bodies() {
    for path in [RAW, CHAIN] { for case in 0..4 { for consume in [false, true] {
        let (mut state, spy, mut provider, gate) = owned();
        let mut body = payload(path);
        if case == 1 { body["stream"] = json!(true); }
        if case == 2 { body["model"] = json!(""); }
        if case == 3 {
            state.native_dispatch = None;
            Arc::get_mut(&mut provider).unwrap().prepare_failure = Some(FailureKind::ScopeClosed);
            state.native_dispatch = Some(provider.clone());
        }
        let session = if case == 0 { None } else { Some(SESSION) };
        let response = request(&state, path, body, session).await;
        let (kind, status) = match case {
            0 => (FailureKind::MissingContext, StatusCode::BAD_REQUEST),
            1 => (FailureKind::UnsupportedStreaming, StatusCode::NOT_IMPLEMENTED),
            2 => (FailureKind::InvalidRequest, StatusCode::BAD_REQUEST),
            _ => (FailureKind::ScopeClosed, StatusCode::CONFLICT),
        };
        assert_eq!(response.status(), status);
        assert_eq!(response.extensions().get::<FailureKind>(), Some(&kind));
        if let Some(bytes) = dispose(response, consume).await {
            let value: Value = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(value["error"]["code"], format!("{kind:?}"));
        }
        assert_eq!(*provider.preparations.lock().unwrap(), usize::from(case == 3));
        assert!(provider.records.lock().unwrap().is_empty());
        assert!(spy.calls.lock().unwrap().is_empty());
        assert!(state.serving_accounts.residents.snapshot().unwrap().residents
            .iter().all(|slot| slot.users == 0 && !slot.uncertain));
        next_admission(&state, &gate, false).await;
    } } }
}

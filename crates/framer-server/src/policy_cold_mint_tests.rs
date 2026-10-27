//! Real worker/router/HTTP authorization composition. SOURCE ONLY, UNRUN.
use super::router_fixture::{self as fixture, Fixture, BOUND};
use framer_api::{HttpFramerClient, authorization_fixture};
use framer_protocol::forward::PolicyDiag;
use framer_core::{ids::*, port::*, pipeline::{InferenceRequest, InferenceResult},
    workspace::CreditSnapshot};
use futures::{FutureExt, task::AtomicWaker};
use serde_json::{Value, json};
use std::{future::{poll_fn, Future}, panic::AssertUnwindSafe, sync::{Arc, Mutex,
    atomic::{AtomicBool, AtomicUsize, Ordering::SeqCst}}};
use tokio::sync::{Notify, oneshot};

const COOKIE: &str = "session=fixture-only";
const BEARER: &str = "fixture-combined-cold-mint";
const MODELS: [&str; 2] = ["gpt-6-astra", "gpt-4.1"];
const CONTENT: [&str; 2] = ["first cold mint café", "second cold mint 日本語"];

#[derive(Default)]
struct Observation {
    mints: AtomicUsize, invalid_mints: AtomicUsize,
    held: AtomicBool, released: AtomicBool,
    started: AtomicUsize, joined: AtomicUsize, active: AtomicUsize,
    first_pending: [AtomicBool; 2], held_pending: [AtomicBool; 2],
    wake: [AtomicWaker; 2], changed: Notify, release: Notify,
    bindings: Mutex<Vec<(AccountId, Session, ProjectId)>>,
}
impl Observation {
    fn release(&self) {
        self.released.store(true, SeqCst);
        self.release.notify_waiters();
    }
}

async fn mint(axum::extract::State(observed): axum::extract::State<Arc<Observation>>,
    method: axum::http::Method, uri: axum::http::Uri, headers: axum::http::HeaderMap)
    -> axum::response::Response {
    use axum::{http::StatusCode, response::IntoResponse};
    let number = observed.mints.fetch_add(1, SeqCst);
    if method != axum::http::Method::GET || uri != "/auth/ai/access-token"
        || headers.get("cookie").and_then(|value| value.to_str().ok()) != Some(COOKIE) {
        observed.invalid_mints.fetch_add(1, SeqCst);
        return StatusCode::BAD_REQUEST.into_response();
    }
    if number == 0 {
        observed.held.store(true, SeqCst);
        // Re-poll both real futures after the first HTTP request has arrived.
        // Counting an outer worker task, or merely creating a future, is insufficient.
        for wake in &observed.wake { wake.wake(); }
        observed.changed.notify_one();
        loop {
            let released = observed.release.notified();
            if observed.released.load(SeqCst) { break; }
            released.await;
        }
    }
    ([(axum::http::header::CONNECTION, "close")],
        axum::Json(json!({"accessToken":BEARER, "expiresAt":"2099-01-01T00:00:00Z"})))
        .into_response()
}

struct Active<'a>(&'a Observation);
impl Drop for Active<'_> {
    fn drop(&mut self) {
        self.0.active.fetch_sub(1, SeqCst);
    }
}

struct ObservedBinding<'a> {
    inner: Box<dyn AuthorizedForward + 'a>, observed: &'a Observation,
}
impl ChainControl for ObservedBinding<'_> {
    fn is_cancelled(&self) -> bool { self.inner.is_cancelled() }
    fn on_429(&self) { self.inner.on_429(); }
    fn diag(&self, event: PolicyDiag) { self.inner.diag(event); }
}
#[async_trait::async_trait]
impl AuthorizedForward for ObservedBinding<'_> {
    fn category(&self) -> &str { self.inner.category() }
    async fn saturated(&self) -> Result<(), framer_protocol::forward_error::ControlError> {
        self.inner.saturated().await
    }
    async fn forward(&self, step: &str, payload: Value, chunk: u32)
        -> Result<ForwardReply, String> {
        self.forward_typed(step, payload, chunk).await.map_err(|error| error.to_string())
    }
    async fn forward_typed(&self, step: &str, payload: Value, chunk: u32)
        -> Result<ForwardReply, PolicyError> {
        let index = self.observed.started.fetch_add(1, SeqCst);
        self.observed.active.fetch_add(1, SeqCst);
        let _active = Active(self.observed);
        assert!(index < 2, "unexpected repeated Forward");
        assert_eq!(step, "translate");
        assert_eq!(chunk, 0);
        let mut inner = self.inner.forward_typed(step, payload, chunk);
        let mut first = true;
        let result = poll_fn(|cx| {
            self.observed.wake[index].register(cx.waker());
            let result = inner.as_mut().poll(cx);
            if first {
                first = false;
                self.observed.first_pending[index].store(result.is_pending(), SeqCst);
            }
            if result.is_pending() && self.observed.held.load(SeqCst)
                && !self.observed.released.load(SeqCst)
                && !self.observed.held_pending[index].swap(true, SeqCst) {
                self.observed.changed.notify_one();
            }
            result
        }).await;
        self.observed.joined.fetch_add(1, SeqCst);
        result
    }
}

#[derive(Default)]
struct Browser {
    seeds: Mutex<Vec<(AccountId, String)>>,
    posts: Mutex<Vec<(AccountId, String, String, Value)>>,
}
#[async_trait::async_trait]
impl BrowserBridge for Browser {
    async fn castle_token(&self, _: &AccountId) -> Result<CastleToken, String> {
        panic!("unexpected Castle request")
    }
    async fn auth_begin(&self, _: &AccountId, _: &str) -> Result<(), String> {
        panic!("unexpected login")
    }
    async fn auth_complete(&self, _: &AccountId, _: &str) -> Result<(String, Value), String> {
        panic!("unexpected login")
    }
    async fn seed_session(&self, account: &AccountId, cookie: &str) -> Result<(), String> {
        self.seeds.lock().unwrap().push((*account, cookie.into()));
        Ok(())
    }
    async fn ai_post(&self, account: &AccountId, path: &str, bearer: &str, body: &Value)
        -> Result<(u16, Option<String>, String), String> {
        self.posts.lock().unwrap().push((*account, path.into(), bearer.into(), body.clone()));
        assert_eq!(path, "/ai/v3/responses/");
        assert_eq!(bearer, BEARER);
        assert_eq!(body["request"]["stream"], false);
        let index = MODELS.iter().position(|model|
            body["request"]["model"] == framer_core::convert::to_openrouter_id(model))
            .expect("unexpected model");
        let reply = fixture::reply(CONTENT[index]);
        Ok((reply.status, None, String::from_utf8(reply.body.to_vec()).unwrap()))
    }
}
struct Client { http: HttpFramerClient, observed: Arc<Observation> }

async fn release_after_overlap(f: &Fixture, client: &Client, session: &Session) {
    let observed = &client.observed;
    loop {
        let changed = observed.changed.notified();
        if observed.held.load(SeqCst)
            && observed.held_pending.iter().all(|pending| pending.load(SeqCst)) { break; }
        changed.await;
    }
    assert!(!observed.released.load(SeqCst));
    assert!(observed.first_pending.iter().all(|pending| pending.load(SeqCst)));
    assert_eq!(observed.started.load(SeqCst), 2);
    assert_eq!(observed.active.load(SeqCst), 2);
    assert_eq!(observed.joined.load(SeqCst), 0);
    assert_eq!(observed.mints.load(SeqCst), 1);
    assert_eq!(authorization_fixture::cached(&client.http, f.accounts[0].id, session), [false; 3]);
    let team = &f.accounts[0].current_team.as_ref().unwrap().0;
    let keys = MODELS.map(|model| format!("{team}:{}", framer_core::convert::to_openrouter_id(model)));
    assert_ne!(keys[0], keys[1]);
    for key in &keys {
        assert_eq!(f.state.gate.limit_of(key).await, 1);
        assert!(f.state.gate.acquire_now(key).await.is_none(), "both original permits are held");
    }
    observed.release();
}

async fn response(f: &Fixture, index: usize) -> String {
    let input = json!({"model":MODELS[index], "stream":true, "input":CONTENT[index]});
    fixture::consume(f.post(Some(0), input).await).await
}

fn check_forwards(f: &Fixture, client: &Client, browser: &Browser, session: &Session) {
    let account = &f.accounts[0];
    let bindings = client.observed.bindings.lock().unwrap();
    assert_eq!(bindings.len(), 2);
    for (id, bound, project) in bindings.iter() {
        assert_eq!(*id, account.id);
        assert_eq!(bound.active_user, session.active_user);
        assert_eq!(bound.cookie_header, session.cookie_header);
        assert_eq!(Some(project), account.current_project.as_ref());
    }
    let seeds = browser.seeds.lock().unwrap();
    assert_eq!(seeds.len(), 2);
    assert!(seeds.iter().all(|(id, cookie)| *id == account.id && cookie == COOKIE));
    let posts = browser.posts.lock().unwrap();
    assert_eq!(posts.len(), 2);
    for (index, model) in MODELS.iter().enumerate() {
        let matching: Vec<_> = posts.iter().filter(|(_, _, _, body)|
            body["request"]["model"] == framer_core::convert::to_openrouter_id(model)).collect();
        assert_eq!(matching.len(), 1);
        let (id, path, bearer, body) = matching[0];
        assert_eq!(*id, account.id);
        assert_eq!(path, "/ai/v3/responses/");
        assert_eq!(bearer, BEARER);
        assert_eq!(body["request"]["input"][0]["content"][0]["text"], CONTENT[index]);
        assert_eq!(body["sessionId"], "worker-fixture-session");
        assert_eq!(body["stepIndex"], 0);
    }
}

async fn shutdown(f: &Fixture, passed: bool) -> Result<(), String> {
    let manager = f.manager();
    if passed {
        match tokio::time::timeout(BOUND, manager.shutdown(false)).await {
            Ok(Ok(())) => return Ok(()),
            _ => (),
        }
    }
    // A failed assertion, panic or deadline still reaps the child. Do not
    // time out and detach this final join. Main's process bound covers a hang.
    let forced = manager.shutdown(true).await;
    Err(format!("proof or graceful shutdown failed; forced join: {forced:?}"))
}

async fn settle_after_shutdown(f: &Fixture, observed: &Observation) {
    // The loopback peer remains alive until every real call and associated
    // owner has settled, even after forced child shutdown. Retain this await;
    // main's outer process deadline handles a broken cleanup implementation.
    loop {
        if observed.active.load(SeqCst) == 0
            && f.state.response_output.counts() == (0, 0, 0)
            && f.manager().snapshot().is_ok_and(|slots| slots.iter().all(|slot| slot.pins == 0)) {
            return;
        }
        tokio::task::yield_now().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "main supplies the real worker binary; launch alone with empty relay configuration"]
async fn real_worker_concurrent_cold_forwards_share_one_mint() {
    for key in ["FRAMERIZE_JWT_RELAY_URL", "FRAMERIZE_JWT_RELAY_TOKEN"] {
        assert!(std::env::var_os(key).is_none_or(|value| value.is_empty()), "{key} must be empty");
    }
    assert!(fixture::memory_points()["children"].as_array().unwrap().is_empty());
    let observed = Arc::new(Observation::default());
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let client = Arc::new(Client {
        http: authorization_fixture::cold_loopback(listener.local_addr().unwrap()).unwrap(),
        observed: observed.clone(),
    });
    let browser = Arc::new(Browser::default());
    let f = Fixture::with_client_browser(client.clone(), browser.clone());
    let app = axum::Router::new().fallback(mint).with_state(observed.clone());
    let (stop, closed) = oneshot::channel::<()>();
    let server = tokio::spawn(async move {
        axum::serve(listener, app).with_graceful_shutdown(async move {
            let _ = closed.await;
        }).await
    });
    let outcome = AssertUnwindSafe(tokio::time::timeout(BOUND * 4,
        exercise(&f, &client, &browser))).catch_unwind().await;
    observed.release(); // Also release a held HTTP response on panic or timeout.
    let cleanup = shutdown(&f, matches!(&outcome, Ok(Ok(_)))).await;
    settle_after_shutdown(&f, &observed).await;
    let _ = stop.send(());
    // Graceful server completion joins its connection handlers, including errors.
    let served = server.await;
    let _ticket = match outcome {
        Ok(Ok(ticket)) => ticket,
        Ok(Err(error)) => panic!("cold mint proof deadline after cleanup: {error}"),
        Err(error) => std::panic::resume_unwind(error),
    };
    cleanup.unwrap();
    served.unwrap().unwrap();
    assert_eq!(observed.mints.load(SeqCst), 1);
    assert_eq!(observed.invalid_mints.load(SeqCst), 0);
    assert_eq!(observed.started.load(SeqCst), 2);
    assert_eq!(observed.joined.load(SeqCst), 2);
    assert_eq!(observed.active.load(SeqCst), 0);
    assert_eq!(f.state.response_output.counts(), (0, 0, 0));
    assert_eq!(f.state.response_output.output_counts(), (0, 0));
    assert!(f.manager().snapshot().unwrap().is_empty());
    assert!(fixture::memory_points()["children"].as_array().unwrap().is_empty());
    println!("cold_mint: held_pending=2 mints=1 forwards_joined=2 application_owners=0 output=0 child=reaped");
}

async fn exercise(f: &Fixture, client: &Client, browser: &Browser)
    -> framer_core::operation_gate::DrainTicket {
    let account = &f.accounts[0];
    let session = f.state.sessions.load(&account.framer_user_id).await.unwrap().unwrap();
    assert_eq!(session.cookie_header, COOKIE);
    assert_eq!(session.active_user, account.framer_user_id);
    assert_eq!(authorization_fixture::cached(&client.http, account.id, &session), [false; 3]);
    assert_eq!(client.observed.mints.load(SeqCst), 0);
    let generation = f.stage().await;
    f.manager().activate(generation).unwrap();
    assert_eq!(fixture::memory_points()["children"].as_array().unwrap().len(), 1);
    let (first, second, ()) = tokio::join!(
        response(f, 0), response(f, 1), release_after_overlap(f, client, &session));
    let first_id = fixture::trace(&first, CONTENT[0]);
    let second_id = fixture::trace(&second, CONTENT[1]);
    assert_ne!(first_id, second_id);
    assert_eq!(authorization_fixture::cached(&client.http, account.id, &session), [true, true, false]);
    check_forwards(f, client, browser, &session);
    f.settle().await;
    let slots = f.manager().snapshot().unwrap();
    assert_eq!(slots.len(), 1);
    assert_eq!(slots[0].generation, generation);
    assert!(slots[0].alive && slots[0].active && !slots[0].unconfirmed);
    assert_eq!(slots[0].pins, 0);
    assert_eq!(f.state.response_output.counts(), (0, 0, 0));
    assert_eq!(f.state.response_output.output_counts(), (0, 0));
    assert!(f.state.serving_accounts.residents.snapshot().unwrap().residents.iter()
        .all(|resident| resident.users == 0 && !resident.uncertain));
    let scope = f.state.operation_ownership.as_ref().unwrap();
    let mut ticket = scope.seal().unwrap();
    scope.check_application_drained(&ticket.wait_application().await.unwrap()).unwrap();
    ticket // Keep the seal through child shutdown. The legacy host marker is retained.
}
#[async_trait::async_trait]
impl FramerClient for Client {
    fn bind_responses<'a>(&'a self, session: &'a Session, browser: &'a dyn BrowserBridge,
        account: &'a AccountId, project: &'a ProjectId, cfg: &'a ChainConfig, chat: &'a str)
        -> Option<Box<dyn AuthorizedForward + 'a>> {
        self.observed.bindings.lock().unwrap().push((*account, session.clone(), project.clone()));
        Some(Box::new(ObservedBinding {
            inner: self.http.bind_responses(session, browser, account, project, cfg, chat)?,
            observed: &self.observed,
        }))
    }
    async fn request_magic_link(&self, _: &str, _: &CastleToken) -> Result<(), String> {
        panic!("unexpected login")
    }
    async fn complete_magic_link(&self, _: &str, _: &CastleToken) -> Result<Session, String> {
        panic!("unexpected login")
    }
    async fn me(&self, _: &Session) -> Result<Value, String> { panic!("unexpected profile") }
    async fn run_inference(&self, _: &Session, _: &dyn BrowserBridge, _: &AccountId,
        _: &ProjectId, _: &InferenceRequest) -> Result<InferenceResult, String> {
        panic!("unexpected inference path")
    }
    async fn proxy(&self, _: &Session, _: &dyn BrowserBridge, _: &AccountId,
        _: &ProjectId, _: &str, _: Value, _: &str) -> Result<ProxyResponse, String> {
        panic!("unexpected proxy path")
    }
    async fn freechat_long(&self, _: &Session, _: &dyn BrowserBridge, _: &AccountId,
        _: &ProjectId, _: Value, _: &ChainConfig, _: &str,
        _: Option<&(dyn Fn(framer_core::convert::ChainEvent) + Send + Sync)>)
        -> Result<framer_core::convert::AssembledResponse, String> {
        panic!("unexpected legacy path")
    }
    async fn native_long(&self, _: &Session, _: &dyn BrowserBridge, _: &AccountId,
        _: &ProjectId, _: Value, _: &ChainConfig, _: &str,
        _: Option<&(dyn Fn(framer_core::convert::ChainEvent) + Send + Sync)>)
        -> Result<framer_core::convert::AssembledResponses, String> {
        panic!("unexpected legacy path")
    }
    async fn usage(&self, _: &Session, _: &TeamId) -> Result<CreditSnapshot, String> {
        panic!("unexpected credit request")
    }
    async fn create_workspace(&self, _: &Session, _: &WorkspaceSpec) -> Result<TeamId, String> {
        panic!("unexpected workspace request")
    }
}

//! Both production retry paths with inert browser and token-mint fixtures.
#[path = "auth_session_scope_tests.rs"]
mod scope_tests;

use super::AiToken;
use crate::{inference, proxy, session_auth::Egress, transport::Transport};
use async_trait::async_trait;
use framer_core::{AccountId, FramerUserId, ProjectId};
use framer_core::pipeline::InferenceRequest;
use framer_core::port::{BrowserBridge, CastleToken, Session};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

const AI: &str = "/auth/ai/access-token";
const WEB: &str = "/auth/web/access-token";
const REJECTED: &str = "fixture-rejected";
const FRESH: &str = "fixture-refreshed";

struct Mint { result: Result<AiToken, String>, calls: usize, delay: std::time::Duration }

fn mints() -> &'static Mutex<HashMap<String, Mint>> {
    static MINTS: OnceLock<Mutex<HashMap<String, Mint>>> = OnceLock::new();
    MINTS.get_or_init(|| Mutex::new(HashMap::new()))
}

pub(super) async fn mint_fixture(session: &Session, path: &str) -> Option<Result<AiToken, String>> {
    let (result, delay) = {
        let mut entries = mints().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let entry = entries.get_mut(&session.cookie_header)?;
        entry.calls += 1;
        (if path == AI { entry.result.clone() }
            else { Err("unexpected fixture token endpoint".into()) }, entry.delay)
    };
    if !delay.is_zero() {
        tokio::time::sleep(delay).await;
    }
    Some(result)
}

fn token(session: &Session, bearer: &str) -> AiToken {
    AiToken::for_session(session, bearer.into(), i64::MAX)
}

fn store(t: &Transport, session: &Session, path: &str, value: AiToken) {
    t.authorization.sessions.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
        .insert((session.cookie_header.clone(), path.into()), value);
}

#[derive(Clone, Copy)]
enum Mode { Proxy, Inference }

struct Browser {
    account: AccountId, cookie: String, statuses: Vec<u16>, bearers: Mutex<Vec<String>>,
}

struct Fixture {
    account: AccountId, session: Session, browser: Browser, transport: Transport,
}

impl Fixture {
    fn new(shared: &str, mint: Result<&str, String>, statuses: Vec<u16>) -> Self {
        Self::with_authorization(shared, mint, statuses, Default::default())
    }

    fn with_authorization(shared: &str, mint: Result<&str, String>, statuses: Vec<u16>,
        authorization: std::sync::Arc<crate::authorization::AuthorizationState>) -> Self {
        let account = AccountId(uuid::Uuid::new_v4());
        let session = Session { active_user: FramerUserId(format!("fixture-{}", account.0)),
            cookie_header: format!("auth-retry-fixture={}", account.0) };
        mints().lock().unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(session.cookie_header.clone(), Mint {
                result: mint.map(|bearer| token(&session, bearer)), calls: 0,
                delay: std::time::Duration::ZERO });
        let browser = Browser { account, cookie: session.cookie_header.clone(),
            statuses, bearers: Mutex::new(Vec::new()) };
        let mut transport = Transport::control_for_loopback(
            "127.0.0.1:9".parse().unwrap(), Egress::Direct).unwrap();
        transport.authorization = authorization;
        store(&transport, &session, AI, token(&session, shared));
        transport.authorization.accounts.lock().unwrap().push((account, token(&session, REJECTED)));
        Self { account, session, browser, transport }
    }

    fn mint_calls(&self) -> usize {
        mints().lock().unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(&self.session.cookie_header).map_or(usize::MAX, |entry| entry.calls)
    }

    fn cached(&self, path: &str) -> Option<String> {
        self.transport.authorization.sessions.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(&(self.session.cookie_header.clone(), path.into()))
            .map(|value| value.bearer.clone())
    }

    fn sent(&self) -> Vec<String> {
        self.browser.bearers.lock().unwrap().clone()
    }

    async fn run(&self, mode: Mode) -> Result<(), String> {
        let project = ProjectId("fixture-project".into());
        match mode {
            Mode::Proxy => proxy::forward(&self.transport, &self.transport.authorization.accounts, &self.session,
                &self.browser, &self.account, &project, proxy::Dialect::OpenAiResponses,
                "message", json!({"model":"fixture","stream":false}), "fixture-chat", 0)
                .await.map(|_| ()),
            Mode::Inference => inference::run(&self.transport, &self.transport.authorization.accounts, &self.session,
                &self.browser, &self.account, &project, &InferenceRequest {
                    prompt: "fixture".into(), model: Some("fixture".into()),
                    chat_session: Some("fixture-chat".into()),
                }).await.map(|_| ()),
        }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        crate::SessionAuthorization(self.transport.authorization.clone()).invalidate(&self.session);
        mints().lock().unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&self.session.cookie_header);
    }
}

#[async_trait]
impl BrowserBridge for Browser {
    async fn castle_token(&self, _: &AccountId) -> Result<CastleToken, String> {
        panic!("unexpected standalone Castle mint")
    }
    async fn auth_begin(&self, _: &AccountId, _: &str) -> Result<(), String> {
        panic!("unexpected login")
    }
    async fn auth_complete(&self, _: &AccountId, _: &str) -> Result<(String, Value), String> {
        panic!("unexpected login")
    }
    async fn seed_session(&self, account: &AccountId, cookie: &str) -> Result<(), String> {
        assert_eq!(account, &self.account);
        assert_eq!(cookie, self.cookie);
        Ok(())
    }
    async fn ai_post(&self, account: &AccountId, path: &str, bearer: &str, _: &Value)
        -> Result<(u16, Option<String>, String), String> {
        assert_eq!(account, &self.account);
        assert_eq!(path, "/ai/v3/openai/responses/");
        let mut sent = self.bearers.lock().unwrap();
        let status = self.statuses.get(sent.len()).copied().unwrap_or(599);
        sent.push(bearer.to_owned());
        Ok((status, None, concat!("data: {\"type\":\"response.completed\",\"response\":{",
            "\"id\":\"fixture\",\"status\":\"completed\",\"output\":[],\"usage\":{}}}\n\n").into()))
    }
}

async fn refreshes(mode: Mode) {
    let fixture = Fixture::new(REJECTED, Ok(FRESH), vec![401, 200]);
    let unrelated = Fixture::with_authorization("fixture-peer", Err("unused".into()), vec![],
        fixture.transport.authorization.clone());
    store(&fixture.transport, &fixture.session, WEB, token(&fixture.session, "fixture-web"));
    fixture.run(mode).await.unwrap();
    assert_eq!(fixture.sent(), [REJECTED, FRESH]);
    assert_eq!(fixture.mint_calls(), 1);
    assert_eq!(fixture.cached(AI).as_deref(), Some(FRESH));
    assert_eq!(fixture.cached(WEB).as_deref(), Some("fixture-web"));
    assert_eq!(unrelated.cached(AI).as_deref(), Some("fixture-peer"));
    assert_eq!(unrelated.mint_calls(), 0);
    assert_eq!(fixture.transport.authorization.accounts.lock().unwrap().iter()
        .find(|(id, _)| *id == fixture.account).unwrap().1.bearer, FRESH);
}

#[tokio::test]
async fn proxy_401_replaces_rejected_session_token() { refreshes(Mode::Proxy).await; }

#[tokio::test]
async fn inference_401_replaces_rejected_session_token() { refreshes(Mode::Inference).await; }

#[tokio::test]
async fn late_rejection_keeps_a_newer_shared_token() {
    for mode in [Mode::Proxy, Mode::Inference] {
        let fixture = Fixture::new(FRESH, Err("unexpected remint".into()), vec![401, 200]);
        fixture.run(mode).await.unwrap();
        assert_eq!(fixture.sent(), [REJECTED, FRESH]);
        assert_eq!(fixture.mint_calls(), 0);
        assert_eq!(fixture.cached(AI).as_deref(), Some(FRESH));
    }
}

#[tokio::test]
async fn refresh_failure_returns_without_another_ai_request() {
    for mode in [Mode::Proxy, Mode::Inference] {
        let fixture = Fixture::new(REJECTED, Err("unauthorized".into()), vec![401]);
        assert_eq!(fixture.run(mode).await.unwrap_err(), "unauthorized");
        assert_eq!(fixture.sent(), [REJECTED]);
        assert_eq!(fixture.mint_calls(), 1);
        assert!(fixture.cached(AI).is_none());
        assert!(fixture.transport.authorization.accounts.lock().unwrap().is_empty());
    }
}

#[tokio::test]
async fn repeated_401_stops_after_one_refresh() {
    for mode in [Mode::Proxy, Mode::Inference] {
        let fixture = Fixture::new(REJECTED, Ok(FRESH), vec![401, 401]);
        let expected = match mode {
            Mode::Proxy => "unauthorized",
            Mode::Inference => "unauthorized after token refresh",
        };
        assert_eq!(fixture.run(mode).await.unwrap_err(), expected);
        assert_eq!(fixture.sent(), [REJECTED, FRESH]);
        assert_eq!(fixture.mint_calls(), 1);
    }
}

#[tokio::test]
async fn successful_calls_reuse_cached_tokens_without_minting() {
    for mode in [Mode::Proxy, Mode::Inference] {
        let fixture = Fixture::new(REJECTED, Err("unexpected remint".into()), vec![200]);
        fixture.run(mode).await.unwrap();
        assert_eq!(fixture.sent(), [REJECTED]);
        assert_eq!(fixture.mint_calls(), 0);
        assert_eq!(fixture.cached(AI).as_deref(), Some(REJECTED));
    }
}

#[tokio::test]
async fn concurrent_token_cache_misses_share_one_session_mint() {
    let session = Session {
        active_user: FramerUserId("fixture-concurrent-mint-user".into()),
        cookie_header: "auth-retry-fixture=concurrent-mint".into(),
    };
    mints().lock().unwrap_or_else(std::sync::PoisonError::into_inner)
        .insert(session.cookie_header.clone(), Mint {
            result: Ok(token(&session, FRESH)), calls: 0,
            delay: std::time::Duration::from_millis(30),
        });
    let transport = std::sync::Arc::new(Transport::new());
    let tasks: Vec<_> = (0..8).map(|_| {
        let transport = transport.clone();
        let session = session.clone();
        tokio::spawn(async move { crate::auth::ai_access_token(&transport, &session).await })
    }).collect();
    for task in tasks {
        assert_eq!(task.await.unwrap().unwrap().bearer, FRESH);
    }
    let calls = mints().lock().unwrap_or_else(std::sync::PoisonError::into_inner)
        .get(&session.cookie_header).unwrap().calls;
    assert_eq!(calls, 1);
    mints().lock().unwrap_or_else(std::sync::PoisonError::into_inner)
        .remove(&session.cookie_header);
}

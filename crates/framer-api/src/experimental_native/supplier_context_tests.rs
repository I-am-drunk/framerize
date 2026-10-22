use super::*;
use crate::experimental_native::{
    native_account::{CloseObserver, RequestContext},
    native_http::{NativeHttp, NativeHttpLimits},
    observed_authorization::{ObservedCredentialSource, ObservedSourceFactory, OpenedSource,
        OpenFailure, SourceCleanup, SourceControl, SourceIdentity},
};
use crate::proxy::Dialect;
use framer_core::{ids::{AccountId, FramerUserId}, port::CastleToken};
use futures::StreamExt;
use serde_json::json;
use std::sync::{Mutex, atomic::{AtomicUsize, Ordering}};
use tokio::{sync::Semaphore, time::{timeout, Duration, Instant}};

const BOUND: Duration = Duration::from_secs(2);
const CURRENT: &str = "session=fixture-login; browser-context=current";
fn session() -> Session {
    Session { active_user: FramerUserId("supplier-fixture".into()),
        cookie_header: "session=fixture-login".into() }
}
fn binding() -> BoundAttemptContext {
    BoundAttemptContext { project: ProjectId("fixture-project".into()),
        chat_session: "actual-fixture-conversation".into(), step_index: 4,
        context: AttemptContext { feature_category: "fixture-category".into(),
            feature_name: "fixture-feature".into(), step: "fixture-step".into(),
            turn_id: "actual-fixture-turn".into(), user_prompt: "full supplied prompt".into() } }
}
fn prepared<'a>(scope: &'a AccountScope, project: &'a ProjectId, payload: &'a Value,
    dialect: Dialect) -> PreparedAttempt<'a> {
    PreparedAttempt { scope, project, payload, dialect,
        chat_session: "actual-fixture-conversation", step_index: 4,
        deadline: Instant::now() + BOUND }
}

struct Probe {
    targets: Mutex<Vec<(String, String, String)>>,
    entered: Semaphore, source_release: Semaphore, sealed: Semaphore,
    cleanup_release: Semaphore, stops: AtomicUsize,
}
impl Probe {
    fn new() -> Arc<Self> {
        Arc::new(Self { targets: Mutex::new(Vec::new()), entered: Semaphore::new(0),
            source_release: Semaphore::new(0), sealed: Semaphore::new(0),
            cleanup_release: Semaphore::new(0), stops: AtomicUsize::new(0) })
    }
}
impl SourceControl for Probe {
    fn request_stop(&self) { self.stops.fetch_add(1, Ordering::SeqCst); }
    fn seal(&self) { self.sealed.add_permits(1); }
}
impl CredentialSource for Probe {
    fn issue<'a>(&'a self, _: &'a AccountScope, _: &'a Session,
        _: &'a PreparationControl) -> BoxFuture<'a, PreparationResult<CredentialMaterial>> {
        async { panic!("contextual target was lost by a wrapper") }.boxed()
    }
    fn issue_for_target<'a>(&'a self, scope: &'a AccountScope, session: &'a Session,
        target: &'a CredentialTarget, _: &'a PreparationControl)
        -> BoxFuture<'a, PreparationResult<PreparedCredentials>> {
        async move {
            self.targets.lock().unwrap().push((target.project.0.clone(),
                target.path.into(), session.cookie_header.clone()));
            self.entered.add_permits(1);
            self.source_release.acquire().await.unwrap().forget();
            PreparationResult::finished(Ok(PreparedCredentials {
                material: CredentialMaterial { bearer: "fixture-bearer".into(),
                    castle: FreshCastleToken::from_browser(scope.account(), CastleToken("fixture-token".into())) },
                current_cookies: Some(CurrentRequestCookies::from_current_jar(session, CURRENT.into()).unwrap()),
            }))
        }.boxed()
    }
}

struct Factory(Arc<Probe>);
impl ObservedSourceFactory for Factory {
    fn open(&self, scope: &AccountScope) -> Result<OpenedSource, OpenFailure> {
        let probe = self.0.clone();
        let identity = SourceIdentity::from_scope(scope);
        Ok(OpenedSource { scope: scope.clone(), source: probe.clone(), control: probe.clone(),
            cleanup: async move {
                probe.cleanup_release.acquire().await.unwrap().forget();
                SourceCleanup::Closed(identity)
            }.boxed() })
    }
}
fn authorization(client: &NativeAccountClient, scope: &AccountScope, probe: Arc<Probe>)
    -> GenerationAuthorization {
    let source = ObservedCredentialSource::new(scope.clone(), Arc::new(Factory(probe)));
    GenerationAuthorization::for_attempt(client.clone(), scope.clone(), Arc::new(source), binding()).unwrap()
}
async fn gate(value: &Semaphore) {
    timeout(BOUND, value.acquire()).await.unwrap().unwrap().forget();
}
fn open(address: std::net::SocketAddr) -> (NativeAccountClient, AccountScope) {
    let client = NativeAccountClient::new().unwrap();
    let account = AccountId(uuid::Uuid::new_v4());
    let session = session();
    let user = session.active_user.clone();
    let scope = client.open_account_for_test(account, session, ||
        NativeHttp::for_loopback(account, user, address, NativeHttpLimits::default())).unwrap();
    (client, scope)
}

fn request(dialect: Dialect, payload: Value) -> RequestContext {
    let b = binding();
    RequestContext { project: b.project, dialect, chat_session: b.chat_session,
        step_index: b.step_index, feature_category: b.context.feature_category,
        feature_name: b.context.feature_name, step: b.context.step,
        turn_id: b.context.turn_id, user_prompt: b.context.user_prompt, payload }
}
async fn serve(listener: tokio::net::TcpListener) -> (String, Value) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    timeout(BOUND, async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut bytes = Vec::new();
        let (head, end, length) = loop {
            let mut part = [0u8; 2048];
            let n = socket.read(&mut part).await.unwrap();
            assert!(n > 0 && bytes.len() + n <= 65536);
            bytes.extend_from_slice(&part[..n]);
            if let Some(at) = bytes.windows(4).position(|x| x == b"\r\n\r\n") {
                let head = String::from_utf8(bytes[..at].to_vec()).unwrap();
                let length: usize = head.lines().find_map(|line| line.to_ascii_lowercase()
                    .strip_prefix("content-length: ").map(|v| v.parse().unwrap())).unwrap();
                break (head, at + 4, length);
            }
        };
        while bytes.len() < end + length {
            let mut part = [0u8; 2048];
            let n = socket.read(&mut part).await.unwrap();
            assert!(n > 0 && bytes.len() + n <= 65536);
            bytes.extend_from_slice(&part[..n]);
        }
        socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}").await.unwrap();
        socket.shutdown().await.unwrap();
        (head, serde_json::from_slice(&bytes[end..end + length]).unwrap())
    }).await.unwrap()
}

async fn issue(client: &NativeAccountClient, scope: &AccountScope,
    probe: &Arc<Probe>, dialect: Dialect, payload: &Value) -> AuthorizedAttempt {
    let source = authorization(client, scope, probe.clone());
    let project = binding().project;
    let mut pending = source.authorize(prepared(scope, &project, payload, dialect));
    assert!(pending.as_mut().now_or_never().is_none());
    gate(&probe.entered).await;
    probe.source_release.add_permits(1);
    gate(&probe.sealed).await;
    assert!(pending.as_mut().now_or_never().is_none(), "original cleanup still pending");
    probe.cleanup_release.add_permits(1);
    timeout(BOUND, pending).await.unwrap().unwrap()
}
async fn deliver(client: &NativeAccountClient, scope: &AccountScope,
    dialect: Dialect, payload: Value, authorized: AuthorizedAttempt) {
    assert_eq!(authorized.context.turn_id, binding().context.turn_id);
    assert_eq!(authorized.context.user_prompt, binding().context.user_prompt);
    let pending = client.start(scope, request(dialect, payload), authorized.auth,
        Instant::now() + BOUND, "application/json".into()).unwrap();
    let mut response = timeout(BOUND, pending.response).await.unwrap().unwrap();
    assert_eq!(response.proxy.status, 200);
    let mut bytes = Vec::new();
    while let Some(part) = timeout(BOUND, response.proxy.stream.next()).await.unwrap() {
        bytes.extend_from_slice(&part.unwrap());
    }
    assert_eq!(bytes, b"{}");
    timeout(BOUND, pending.cleanup).await.unwrap().unwrap();
}

#[tokio::test]
async fn current_cookies_reach_both_free_routes_without_mutating_private_session() {
    for dialect in [Dialect::OpenRouterResponses, Dialect::OpenRouterMessages] {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let (client, scope) = open(listener.local_addr().unwrap());
        let probe = Probe::new();
        let payload = json!({"model":"fixture-model", "stream":false,
            "input":[{"role":"user", "content":"preserved history"}],
            "tools":[{"name":"fixture-tool"}], "reasoning":{"effort":"max"}});
        let authorized = issue(&client, &scope, &probe, dialect, &payload).await;
        let worker = tokio::spawn(serve(listener));
        deliver(&client, &scope, dialect, payload.clone(), authorized).await;
        let (headers, body) = timeout(BOUND, worker).await.unwrap().unwrap();
        assert!(headers.starts_with(&format!("POST {} HTTP/1.1\r\n", dialect.path())));
        assert!(headers.lines().any(|line| line == format!("cookie: {CURRENT}")));
        assert_eq!(body["request"], payload);
        assert_eq!(body["projectId"], binding().project.0);
        assert_eq!(body["sessionId"], binding().chat_session);
        assert_eq!(body["turnId"], binding().context.turn_id);
        assert_eq!(*probe.targets.lock().unwrap(), vec![(binding().project.0,
            dialect.path().into(), session().cookie_header)]);
        let original = client.prepare_owned(&scope, Instant::now() + BOUND,
            |session, _| async move { PreparationResult::finished(Ok(session.cookie_header)) }).unwrap();
        assert_eq!(timeout(BOUND, original).await.unwrap().unwrap(), session().cookie_header);
        timeout(BOUND, client.close_account(&scope).unwrap()).await.unwrap().unwrap();
    }
}

#[tokio::test]
async fn observed_target_cancellation_keeps_both_original_work_and_cleanup() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let (client, scope) = open(listener.local_addr().unwrap());
    let probe = Probe::new();
    let source = authorization(&client, &scope, probe.clone());
    let project = binding().project;
    let payload = json!({"model":"fixture-model", "stream":false});
    let observed = Mutex::new(None);
    let observer = |done: CloseObserver, stop: PreparationControl| {
        *observed.lock().unwrap() = Some((done, stop));
    };
    let mut waiting = source.authorize_observed(
        prepared(&scope, &project, &payload, Dialect::OpenRouterResponses), &observer);
    assert!(waiting.as_mut().now_or_never().is_none());
    gate(&probe.entered).await;
    drop(waiting);
    let (done, stop) = observed.into_inner().unwrap().unwrap();
    assert!(stop.is_cancelled());
    let closed = client.close_account(&scope).unwrap();
    assert!(done.clone().now_or_never().is_none());
    assert!(closed.clone().now_or_never().is_none());
    probe.source_release.add_permits(1);
    gate(&probe.sealed).await;
    assert!(done.clone().now_or_never().is_none());
    assert!(closed.clone().now_or_never().is_none());
    probe.cleanup_release.add_permits(1);
    timeout(BOUND, done).await.unwrap().unwrap();
    timeout(BOUND, closed).await.unwrap().unwrap();
    assert!(probe.stops.load(Ordering::SeqCst) > 0);
    assert!(listener.accept().now_or_never().is_none());
}

#[tokio::test]
async fn foreign_current_jar_is_rejected_before_native_transport() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let (client, scope) = open(listener.local_addr().unwrap());
    for case in 0..2 {
        let mut other = session();
        if case == 0 { other.active_user.0 = "other-private-user".into(); }
        else { other.cookie_header = "session=other-private-login".into(); }
        let jar = CurrentRequestCookies::from_current_jar(&other,
            format!("{}; context=foreign", other.cookie_header)).unwrap();
        let castle = FreshCastleToken::from_browser(scope.account(), CastleToken("fixture-token".into()));
        let auth = SuppliedAuth::new(&scope, "fixture-bearer".into(), castle)
            .with_current_cookies(Some(jar));
        let result = client.start(&scope,
            request(Dialect::OpenRouterResponses, json!({"model":"fixture", "stream":false})),
            auth, Instant::now() + BOUND, "application/json".into());
        assert!(matches!(result, Err(crate::experimental_native::native_account::ClientError::InvalidContext)));
        assert!(listener.accept().now_or_never().is_none());
    }
    timeout(BOUND, client.close_account(&scope).unwrap()).await.unwrap().unwrap();
}

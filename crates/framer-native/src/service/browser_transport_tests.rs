use super::*;
use crate::transport::browser::{BrowserOutcome, BrowserRequest, BrowserResponse, BrowserTransport};
use crate::transport::http::{FreshCastleToken, NativeCancellation};
use framer_core::port::CastleToken;

struct Remote {
    calls: Mutex<Vec<BrowserRequest>>, entered: Semaphore,
    release: Semaphore, cancelled: Semaphore,
}
impl Remote {
    fn new(held: bool) -> Arc<Self> {
        Arc::new(Self { calls: Mutex::new(Vec::new()), entered: Semaphore::new(0),
            release: Semaphore::new(usize::from(!held)), cancelled: Semaphore::new(0) })
    }
}

impl BrowserTransport for Arc<Remote> {
    fn send(&self, request: BrowserRequest, cancellation: NativeCancellation)
        -> BoxFuture<'static, BrowserOutcome> {
        let remote = self.clone();
        async move {
            remote.calls.lock().unwrap().push(request);
            remote.entered.add_permits(1);
            tokio::select! {
                permit = remote.release.acquire() => permit.unwrap().forget(),
                _ = cancellation.cancelled() => {
                    remote.cancelled.add_permits(1);
                    wait(&remote.release).await;
                }
            }
            BrowserOutcome { cleanup_confirmed: true, response: Ok(BrowserResponse {
                status: 200, usage: None, content_type: Some("application/json".into()),
                body: serde_json::to_vec(&serde_json::json!({
                    "id":"fixture-result", "created_at":123, "status":"completed",
                    "output":[{"type":"message", "role":"assistant", "content":[
                        {"type":"output_text", "text":"fixture reply", "annotations":[]}]}],
                    "usage":{"input_tokens":1, "output_tokens":1, "total_tokens":2},
                    "incomplete_details":null, "service_tier":"default"
                })).unwrap(),
            }) }
        }.boxed()
    }
}

struct ReadySource(AccountScope);
impl CredentialSource for ReadySource {
    fn issue<'a>(&'a self, scope: &'a AccountScope, session: &'a Session,
        _: &'a PreparationControl) -> BoxFuture<'a, PreparationResult<CredentialMaterial>> {
        assert!(self.0.same_generation(scope));
        assert_eq!(scope.active_user(), &session.active_user);
        async move { PreparationResult::finished(Ok(CredentialMaterial {
            bearer: "fixture-bearer".into(),
            castle: FreshCastleToken::from_sdk_output(scope.account(),
                CastleToken("fixture-native-output".into()))
                .with_device(Some("fixture-minted-device".into())),
        })) }.boxed()
    }
}

struct ReadyBuilder(Arc<Ports>);
impl SourceBuilder for ReadyBuilder {
    fn build(&self, scope: AccountScope, _: Option<String>)
        -> Result<Arc<dyn CredentialSource>, Failure> {
        *self.0.scope.lock().unwrap() = Some(scope.clone());
        self.0.builds.fetch_add(1, Ordering::SeqCst);
        Ok(Arc::new(ReadySource(scope)))
    }
}

#[tokio::test]
async fn service_routes_owned_request_to_browser_and_retains_only_completed_generation() {
    let remote = Remote::new(false);
    let (state, ports, spec, sessions) = fixture_browser(Some(Arc::new(remote.clone())));
    let account = spec.identity.account;
    let outcome = finish(launch_with_source(state, spec, Arc::new(ReadyBuilder(ports.clone())),
        sessions.clone(), None).unwrap()).await;
    assert!(outcome.cleanup_confirmed && outcome.failure.is_none());
    assert!(outcome.response.is_some());
    assert_eq!(ports.builds.load(Ordering::SeqCst), 1);
    let evidence = outcome.evidence.unwrap();
    assert!(evidence.accepted && evidence.complete_json);
    assert_eq!((evidence.authorization_calls, evidence.transport_starts, evidence.transport_finishes), (1, 1, 1));
    let scope = ports.scope.lock().unwrap().clone().unwrap();
    assert!(scope.closed().now_or_never().is_none());
    {
        let calls = remote.calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        let request = &calls[0];
        assert_eq!(request.account, account);
        assert_eq!(request.user, "fixture-user");
        assert_eq!(request.path, "/ai/v3/responses/");
        assert_eq!(request.device, "fixture-minted-device");
        assert_eq!(request.cookie_header, "session=fixture; __cuid=fixture-minted-device");
        assert_eq!(request.bearer, "fixture-bearer");
        let wire: serde_json::Value = serde_json::from_slice(&request.wire).unwrap();
        assert_eq!(wire["castleToken"], "fixture-native-output");
        assert_eq!(wire["request"]["input"], serde_json::json!([{
            "type":"message", "role":"user", "content":[{"type":"input_text", "text":"fixture"}]
        }]));
    }
    sessions.shutdown().await.unwrap();
    assert_eq!(scope.closed().await, Ok(()));
}

#[tokio::test]
async fn cancelled_service_waits_for_browser_cleanup_before_closing_the_generation() {
    let remote = Remote::new(true);
    let (state, ports, spec, sessions) = fixture_browser(Some(Arc::new(remote.clone())));
    let run = launch_with_source(state, spec, Arc::new(ReadyBuilder(ports.clone())),
        sessions.clone(), None).unwrap();
    wait(&remote.entered).await;
    run.cancel();
    wait(&remote.cancelled).await;
    let scope = ports.scope.lock().unwrap().clone().unwrap();
    assert!(run.cleanup.clone().now_or_never().is_none());
    assert!(scope.closed().now_or_never().is_none());
    remote.release.add_permits(1);
    let outcome = finish(run).await;
    assert!(outcome.cleanup_confirmed && outcome.response.is_none());
    assert_eq!(outcome.failure, Some(FailureKind::Cancelled));
    assert_eq!(remote.calls.lock().unwrap().len(), 1);
    assert_eq!(scope.closed().await, Ok(()));
    sessions.shutdown().await.unwrap();
}

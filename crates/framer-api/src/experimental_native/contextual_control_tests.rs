use super::*;
#[path = "control_authorization_fixture.rs"]
mod fixture;
use fixture::*;
use framer_core::{ids::{AccountId, ProjectId}, port::{BrowserAuthorization, BrowserAuthorizationRequest, CastleToken}};
use std::sync::Mutex;

fn target() -> CredentialTarget {
    CredentialTarget { project: ProjectId("fixture-project".into()), path: "/ai/v3/responses/" }
}
fn supplied() -> BrowserAuthorization {
    BrowserAuthorization { project: target().project, path: target().path.into(),
        page_url: "https://framer.com/projects/fixture-project".into(),
        castle: CastleToken("fixture-token".into()),
        cookie_header: "session=fixture-only-cookie; current=fresh".into() }
}
struct Browser {
    calls: Mutex<Vec<(AccountId, String, String, String)>>,
    entered: tokio::sync::Semaphore,
    release: tokio::sync::Semaphore,
}
impl Browser {
    fn new(held: bool) -> Arc<Self> {
        Arc::new(Self { calls: Mutex::new(Vec::new()),
            entered: tokio::sync::Semaphore::new(0),
            release: tokio::sync::Semaphore::new(usize::from(!held)) })
    }
}

#[async_trait::async_trait]
impl BrowserBridge for Browser {
    async fn prepare_inference_authorization(&self, account: &AccountId,
        request: BrowserAuthorizationRequest<'_>) -> Result<BrowserAuthorization, String> {
        self.calls.lock().unwrap().push((*account, request.project.0.clone(),
            request.path.into(), request.cookie_header.into()));
        self.entered.add_permits(1);
        self.release.acquire().await.unwrap().forget();
        Ok(supplied())
    }
    async fn castle_token(&self, _: &AccountId) -> Result<CastleToken, String> {
        panic!("contextual source must not use legacy mint")
    }
    async fn seed_session(&self, _: &AccountId, _: &str) -> Result<(), String> {
        panic!("contextual source must not seed through legacy initialization")
    }
    async fn auth_begin(&self, _: &AccountId, _: &str) -> Result<(), String> {
        panic!("unexpected login")
    }
    async fn auth_complete(&self, _: &AccountId, _: &str) -> Result<(String, serde_json::Value), String> {
        panic!("unexpected login completion")
    }
}

#[tokio::test]
async fn contextual_source_forwards_target_and_current_cookies_without_legacy_mint() {
    let (client, account) = scope(NativeEgress::Direct);
    let server = listener().await;
    let api = Arc::new(bound(&account, server.local_addr().unwrap(), NativeEgress::Direct));
    let worker = serve_once(server, response(200, &token().to_string(), ""));
    let browser = Browser::new(false);
    let source = BoundBrowserSessionSource::new(api, browser.clone());
    let expected = account.clone();
    let pending = client.prepare_owned(&account, tokio::time::Instant::now() + BOUND,
        move |session, control| async move {
            let result = source.issue_for_target(&expected, &session, &target(), &control).await;
            assert!(result.cleanup_confirmed);
            let prepared = result.value.unwrap();
            assert_eq!(prepared.material.bearer, "fixture-bearer");
            let current = prepared.current_cookies.unwrap().into_session(&session).unwrap();
            assert_eq!(session.cookie_header, "session=fixture-only-cookie");
            PreparationResult::finished(Ok(current.cookie_header))
        }).unwrap();
    assert_eq!(tokio::time::timeout(BOUND, pending).await.unwrap().unwrap(),
        "session=fixture-only-cookie; current=fresh");
    result(worker).await;
    assert_eq!(*browser.calls.lock().unwrap(), vec![(account.account(),
        target().project.0, target().path.into(), session().cookie_header)]);
    close(&client, &account).await;
}

#[tokio::test]
async fn contextual_material_rejects_mismatched_route_page_and_login() {
    let (client, account) = scope(NativeEgress::Direct);
    let server = listener().await;
    let api = Arc::new(bound(&account, server.local_addr().unwrap(), NativeEgress::Direct));
    let source = BoundBrowserSessionSource::new(api, Browser::new(false));
    let requested = account.clone();
    let pending = client.prepare_owned(&account, tokio::time::Instant::now() + BOUND,
        move |session, control| async move {
            for case in 0..7 {
                let mut value = supplied();
                match case {
                    0 => value.project.0 = "other-project".into(),
                    1 => value.path = "/ai/v3/messages/".into(),
                    2 => value.page_url = "https://framer.com/projects/other-project".into(),
                    3 => value.castle.0.clear(),
                    4 => value.castle.0 = "x".repeat(16 * 1024 + 1),
                    5 => value.cookie_header = "session=other-login".into(),
                    _ => value.cookie_header = "session=fixture-only-cookie; session=fixture-only-cookie".into(),
                }
                let result = source.finish_contextual(requested.account(), &session,
                    &target(), &control, "fixture-bearer".into(), value);
                assert!(result.cleanup_confirmed);
                assert!(matches!(result.value, Err(PreparationFailure::Rejected)));
            }
            PreparationResult::finished(Ok(()))
        }).unwrap();
    tokio::time::timeout(BOUND, pending).await.unwrap().unwrap();
    assert!(server.accept().now_or_never().is_none());
    close(&client, &account).await;
}

#[tokio::test]
async fn invalid_contextual_login_and_target_fail_before_private_auth_or_browser() {
    let (client, account) = scope(NativeEgress::Direct);
    let server = listener().await;
    let api = Arc::new(bound(&account, server.local_addr().unwrap(), NativeEgress::Direct));
    let browser = Browser::new(false);
    let source = BoundBrowserSessionSource::new(api, browser.clone());
    let expected = account.clone();
    let pending = client.prepare_owned(&account, tokio::time::Instant::now() + BOUND,
        move |session, control| async move {
            for case in 0..3 {
                let mut login = session.clone();
                let mut target = target();
                match case {
                    0 => login.cookie_header = "other=value".into(),
                    1 => target.path = "/ai/v3/openai/responses/",
                    _ => login.active_user.0 = "other-user".into(),
                }
                let result = source.issue_for_target(&expected, &login, &target, &control).await;
                assert!(result.cleanup_confirmed);
                assert!(matches!(result.value, Err(PreparationFailure::Rejected)));
            }
            PreparationResult::finished(Ok(()))
        }).unwrap();
    tokio::time::timeout(BOUND, pending).await.unwrap().unwrap();
    assert!(browser.calls.lock().unwrap().is_empty());
    assert!(server.accept().now_or_never().is_none());
    close(&client, &account).await;
}

#[tokio::test]
async fn cancelled_contextual_waiter_retains_original_preparation_until_browser_returns() {
    let (client, account) = scope(NativeEgress::Direct);
    let server = listener().await;
    let api = Arc::new(bound(&account, server.local_addr().unwrap(), NativeEgress::Direct));
    let worker = serve_once(server, response(200, &token().to_string(), ""));
    let browser = Browser::new(true);
    let source = BoundBrowserSessionSource::new(api, browser.clone());
    let expected = account.clone();
    let pending = client.prepare_owned(&account, tokio::time::Instant::now() + BOUND,
        move |session, control| async move {
            source.issue_for_target(&expected, &session, &target(), &control).await
        }).unwrap();
    tokio::time::timeout(BOUND, browser.entered.acquire()).await.unwrap().unwrap().forget();
    result(worker).await;
    let completion = pending.completion();
    drop(pending);
    let closing = client.close_account(&account).unwrap();
    assert!(completion.clone().now_or_never().is_none());
    assert!(closing.clone().now_or_never().is_none());
    browser.release.add_permits(1);
    tokio::time::timeout(BOUND, completion).await.unwrap().unwrap();
    tokio::time::timeout(BOUND, closing).await.unwrap().unwrap();
    assert_eq!(browser.calls.lock().unwrap().len(), 1);
}

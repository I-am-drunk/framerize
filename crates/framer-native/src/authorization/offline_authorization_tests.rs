use super::*;
use crate::account::client::NativeAccountClient;
use crate::account::client::PendingPreparation;
use std::sync::Mutex;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use tokio::sync::Notify;

#[path = "control_authorization_fixture.rs"]
mod fixture;
use fixture::BOUND;
use fixture::close;
use fixture::listener;
use fixture::scope;

#[path = "offline_scope_tests.rs"]
mod scope_tests;

struct Minter {
    scope: AccountScope,
    device: Option<String>,
    calls: AtomicUsize,
    joined: AtomicUsize,
    outcome: Mutex<Option<PreparationResult<MintedCastle>>>,
    started: Notify,
    release: Option<Arc<Notify>>,
}
impl Minter {
    fn new(scope: &AccountScope, outcome: PreparationResult<MintedCastle>) -> Self {
        let device = outcome.value.as_ref().ok().and_then(|minted| minted.device.clone());
        Self { scope: scope.clone(), device, calls: AtomicUsize::new(0), joined: AtomicUsize::new(0),
            outcome: Mutex::new(Some(outcome)), started: Notify::new(), release: None }
    }
}
impl OfflineMinter for Minter {
    fn scope(&self) -> &AccountScope { &self.scope }
    fn saved_device(&self) -> Option<&str> { self.device.as_deref() }
    fn mint<'a>(&'a self, target: &'a OfflineMintTarget, _: &'a PreparationControl)
        -> BoxFuture<'a, PreparationResult<MintedCastle>> {
        async move {
            assert_eq!(target.project().0, "fixture-project");
            assert_eq!(target.path(), "/ai/v3/responses/");
            assert_eq!(target.page_url(), "https://framer.com/projects/fixture-project");
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.started.notify_one();
            if let Some(release) = &self.release { release.notified().await; }
            self.joined.fetch_add(1, Ordering::SeqCst);
            self.outcome.lock().unwrap().take().unwrap()
        }.boxed()
    }
}

fn target() -> CredentialTarget {
    CredentialTarget { project: ProjectId("fixture-project".into()), path: "/ai/v3/responses/" }
}
fn source(scope: &AccountScope, address: std::net::SocketAddr,
    minter: Arc<Minter>) -> OfflineSessionSource {
    OfflineSessionSource::new(
        Arc::new(fixture::bound(scope, address, NativeEgress::Direct)),
        OfflineMintTarget::new(target(), "https://framer.com/projects/fixture-project".into())
            .unwrap(), minter).unwrap()
}
fn prepare(client: &NativeAccountClient, scope: &AccountScope,
    source: OfflineSessionSource, requested: CredentialTarget)
    -> PendingPreparation<(String, bool)> {
    let expected = scope.clone();
    client.prepare_owned(scope, tokio::time::Instant::now() + BOUND,
        move |session, control| async move {
            let issued = source.issue_for_target(&expected, &session, &requested, &control).await;
            PreparationResult { value: issued.value.map(|result|
                (result.material.bearer, result.current_cookies.is_some())),
                cleanup_confirmed: issued.cleanup_confirmed }
        }).unwrap()
}
fn good(scope: &AccountScope) -> Arc<Minter> {
    Arc::new(Minter::new(scope, PreparationResult::finished(Ok(
        minted(scope, "fixture-token".into())))))
}
fn minted(scope: &AccountScope, value: String) -> MintedCastle {
    MintedCastle::new(scope.clone(), CastleToken(value), std::time::Instant::now())
}
fn auth(listener: tokio::net::TcpListener) -> tokio::task::JoinHandle<String> {
    fixture::serve_once(listener, fixture::response(200, &fixture::token().to_string(), ""))
}

#[test]
fn actual_project_page_and_supported_route_are_required() {
    assert!(OfflineMintTarget::new(target(), "https://fixture.invalid/".into()).is_err());
    assert!(OfflineMintTarget::new(target(), "https://framer.com/projects/another".into()).is_err());
    let mut unsupported = target(); unsupported.path = "/ai/v3/unsupported/";
    assert!(OfflineMintTarget::new(unsupported,
        "https://framer.com/projects/fixture-project".into()).is_err());
}

#[tokio::test]
async fn saved_device_follows_the_joined_mint_into_request_cookies() {
    let (client, account) = scope(NativeEgress::Direct);
    let listener = listener().await;
    let minted = minted(&account, "fixture-token".into()).with_saved_device("saved-device".into());
    let minter = Arc::new(Minter::new(&account, PreparationResult::finished(Ok(minted))));
    let source = source(&account, listener.local_addr().unwrap(), minter.clone());
    let server = auth(listener);
    let expected = account.clone();
    let pending = client.prepare_owned(&account, tokio::time::Instant::now() + BOUND,
        move |session, control| async move {
            let issued = source.issue_for_target(&expected, &session, &target(), &control).await;
            PreparationResult { cleanup_confirmed: issued.cleanup_confirmed,
                value: issued.value.map(|prepared| {
                    let current = prepared.current_cookies.expect("saved device context")
                        .into_session(&session).unwrap();
                    (current.cookie_header, session.cookie_header)
                }) }
        }).unwrap();
    let result = tokio::time::timeout(BOUND, pending).await.unwrap().unwrap();
    assert_eq!(result, ("session=fixture-only-cookie; __cuid=saved-device".into(),
        "session=fixture-only-cookie".into()));
    assert_eq!(minter.calls.load(Ordering::SeqCst), 1);
    assert_eq!(minter.joined.load(Ordering::SeqCst), 1);
    assert!(fixture::result(server).await
        .contains("cookie: session=fixture-only-cookie; __cuid=saved-device\r\n"));
    close(&client, &account).await;
}

#[tokio::test]
async fn conflicting_device_refuses_before_authentication_or_mint() {
    let (client, original) = scope(NativeEgress::Direct);
    close(&client, &original).await;
    let mut session = fixture::session();
    session.cookie_header.push_str("; __cuid=other-device");
    let account = client.open_account(original.account(), session,
        crate::transport::http::NativeHttpLimits::default(), NativeEgress::Direct).unwrap();
    let listener = listener().await;
    let result = minted(&account, "fixture-token".into()).with_saved_device("saved-device".into());
    let minter = Arc::new(Minter::new(&account, PreparationResult::finished(Ok(result))));
    let source = source(&account, listener.local_addr().unwrap(), minter.clone());
    assert_eq!(prepare(&client, &account, source, target()).await.unwrap_err(),
        PreparationFailure::Rejected);
    assert_eq!(minter.calls.load(Ordering::SeqCst), 0);
    assert!(tokio::time::timeout(std::time::Duration::from_millis(20),
        listener.accept()).await.is_err());
    close(&client, &account).await;
}

#[tokio::test]
async fn declared_device_must_match_the_joined_mint() {
    for returned in [None, Some("changed-device")] {
        let (client, account) = scope(NativeEgress::Direct);
        let listener = listener().await;
        let mut result = minted(&account, "fixture-token".into());
        result.device = returned.map(str::to_owned);
        let mut fake = Minter::new(&account, PreparationResult::finished(Ok(result)));
        fake.device = Some("saved-device".into());
        let minter = Arc::new(fake);
        let source = source(&account, listener.local_addr().unwrap(), minter.clone());
        let server = auth(listener);
        assert_eq!(prepare(&client, &account, source, target()).await.unwrap_err(),
            PreparationFailure::Rejected);
        assert_eq!(minter.calls.load(Ordering::SeqCst), 1);
        assert_eq!(minter.joined.load(Ordering::SeqCst), 1);
        assert!(fixture::result(server).await.contains("__cuid=saved-device"));
        close(&client, &account).await;
    }
}

#[tokio::test]
async fn exact_target_issues_once_without_browser_cookie_state() {
    let (client, account) = scope(NativeEgress::Direct);
    let listener = listener().await;
    let minter = good(&account);
    let source = source(&account, listener.local_addr().unwrap(), minter.clone());
    assert_eq!(minter.calls.load(Ordering::SeqCst), 0);
    let server = auth(listener);
    let result = tokio::time::timeout(BOUND, prepare(&client, &account, source, target()))
        .await.unwrap().unwrap();
    assert_eq!(result, ("fixture-bearer".into(), false));
    assert_eq!(minter.calls.load(Ordering::SeqCst), 1);
    assert_eq!(minter.joined.load(Ordering::SeqCst), 1);
    let request = fixture::result(server).await;
    assert!(request.contains("/auth/ai/access-token"));
    close(&client, &account).await;
}

#[tokio::test]
async fn different_project_is_rejected_before_authentication_or_mint() {
    let (client, account) = scope(NativeEgress::Direct);
    let listener = listener().await;
    let minter = good(&account);
    let source = source(&account, listener.local_addr().unwrap(), minter.clone());
    let mut requested = target(); requested.project = ProjectId("different".into());
    assert_eq!(prepare(&client, &account, source, requested).await.unwrap_err(),
        PreparationFailure::Rejected);
    assert_eq!(minter.calls.load(Ordering::SeqCst), 0);
    assert!(tokio::time::timeout(std::time::Duration::from_millis(20),
        listener.accept()).await.is_err());
    close(&client, &account).await;
}

#[tokio::test]
async fn another_account_generation_cannot_use_the_source() {
    let (first, original) = scope(NativeEgress::Direct);
    let (second, other) = scope(NativeEgress::Direct);
    let listener = listener().await;
    let minter = good(&original);
    let source = source(&original, listener.local_addr().unwrap(), minter.clone());
    assert_eq!(prepare(&second, &other, source, target()).await.unwrap_err(),
        PreparationFailure::Rejected);
    assert_eq!(minter.calls.load(Ordering::SeqCst), 0);
    assert!(tokio::time::timeout(std::time::Duration::from_millis(20),
        listener.accept()).await.is_err());
    close(&first, &original).await;
    close(&second, &other).await;
}

#[tokio::test]
async fn targetless_entry_never_falls_back_to_browser_or_default_target() {
    let (client, account) = scope(NativeEgress::Direct);
    let listener = listener().await;
    let minter = good(&account);
    let source = source(&account, listener.local_addr().unwrap(), minter.clone());
    let expected = account.clone();
    let pending = client.prepare_owned(&account, tokio::time::Instant::now() + BOUND,
        move |session, control| async move {
            let result = source.issue(&expected, &session, &control).await;
            PreparationResult { value: result.value.map(|_| ()),
                cleanup_confirmed: result.cleanup_confirmed }
        }).unwrap();
    assert_eq!(pending.await.unwrap_err(), PreparationFailure::Rejected);
    assert_eq!(minter.calls.load(Ordering::SeqCst), 0);
    close(&client, &account).await;
}

#[tokio::test]
async fn dropped_waiter_keeps_mint_owned_until_join_then_suppresses_token() {
    let (client, account) = scope(NativeEgress::Direct);
    let listener = listener().await;
    let release = Arc::new(Notify::new());
    let mut fake = Minter::new(&account, PreparationResult::finished(Ok(minted(&account, "fixture".into()))));
    fake.release = Some(release.clone());
    let minter = Arc::new(fake);
    let source = source(&account, listener.local_addr().unwrap(), minter.clone());
    let server = auth(listener);
    let pending = prepare(&client, &account, source, target());
    let completion = pending.completion();
    let cancellation = pending.cancellation();
    tokio::time::timeout(BOUND, minter.started.notified()).await.unwrap();
    drop(pending);
    assert!(cancellation.is_cancelled());
    assert_eq!(minter.joined.load(Ordering::SeqCst), 0);
    assert!(tokio::time::timeout(std::time::Duration::from_millis(20),
        completion.clone()).await.is_err());
    release.notify_one();
    tokio::time::timeout(BOUND, completion).await.unwrap().unwrap();
    assert_eq!(minter.joined.load(Ordering::SeqCst), 1);
    fixture::result(server).await;
    close(&client, &account).await;
}

#[tokio::test]
async fn token_with_unconfirmed_cleanup_keeps_account_closed() {
    let (client, account) = scope(NativeEgress::Direct);
    let listener = listener().await;
    let minter = Arc::new(Minter::new(&account, PreparationResult {
        value: Ok(minted(&account, "fixture-token".into())), cleanup_confirmed: false }));
    let source = source(&account, listener.local_addr().unwrap(), minter.clone());
    let server = auth(listener);
    assert_eq!(prepare(&client, &account, source, target()).await.unwrap_err(),
        PreparationFailure::CleanupUnconfirmed);
    assert!(tokio::time::timeout(BOUND, client.close_account(&account).unwrap())
        .await.unwrap().is_err());
    assert_eq!(minter.joined.load(Ordering::SeqCst), 1);
    fixture::result(server).await;
}

#[tokio::test]
async fn empty_and_oversize_tokens_never_become_credentials() {
    for value in [String::new(), "x".repeat(16 * 1024 + 1)] {
        let (client, account) = scope(NativeEgress::Direct);
        let listener = listener().await;
        let minter = Arc::new(Minter::new(&account, PreparationResult::finished(Ok(minted(&account, value)))));
        let source = source(&account, listener.local_addr().unwrap(), minter);
        let server = auth(listener);
        assert_eq!(prepare(&client, &account, source, target()).await.unwrap_err(),
            PreparationFailure::Rejected);
        fixture::result(server).await;
        close(&client, &account).await;
    }
}

#[tokio::test]
async fn authentication_failure_does_not_mint_or_fall_back() {
    let (client, account) = scope(NativeEgress::Direct);
    let listener = listener().await;
    let minter = good(&account);
    let source = source(&account, listener.local_addr().unwrap(), minter.clone());
    let server = fixture::serve_once(listener, fixture::response(403, "{}", ""));
    assert_eq!(prepare(&client, &account, source, target()).await.unwrap_err(),
        PreparationFailure::CleanupUnconfirmed);
    assert_eq!(minter.calls.load(Ordering::SeqCst), 0);
    fixture::result(server).await;
    assert!(tokio::time::timeout(BOUND, client.close_account(&account).unwrap())
        .await.unwrap().is_err());
}

#[tokio::test]
async fn future_mint_timestamp_is_rejected_after_join() {
    let (client, account) = scope(NativeEgress::Direct);
    let listener = listener().await;
    let mut value = minted(&account, "fixture-token".into());
    value.received_at += std::time::Duration::from_secs(30);
    let minter = Arc::new(Minter::new(&account, PreparationResult::finished(Ok(value))));
    let source = source(&account, listener.local_addr().unwrap(), minter.clone());
    let server = auth(listener);
    assert_eq!(prepare(&client, &account, source, target()).await.unwrap_err(),
        PreparationFailure::Rejected);
    assert_eq!(minter.joined.load(Ordering::SeqCst), 1);
    fixture::result(server).await;
    close(&client, &account).await;
}

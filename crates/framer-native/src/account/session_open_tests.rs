use super::*;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use tokio::time::timeout;
use tokio::time::Duration;

struct Store { panic: AtomicBool }
fn session(user: &FramerUserId) -> Session {
    Session { active_user: user.clone(), cookie_header: "session=fixture".into() }
}
#[async_trait::async_trait]
impl SessionStore for Store {
    async fn load(&self, user: &FramerUserId) -> Result<Option<Session>, String> {
        assert!(!self.panic.swap(false, Ordering::SeqCst), "fixture store panic");
        Ok(Some(session(user)))
    }
    async fn save(&self, _: &Session) -> Result<(), String> { panic!("unexpected save") }
    async fn delete(&self, _: &FramerUserId) -> Result<(), String> { panic!("unexpected delete") }
}

#[tokio::test]
async fn buffered_open_after_sibling_panic_joins_scope_and_reports_unconfirmed() {
    buffered_open(true).await;
}

#[tokio::test]
async fn buffered_open_after_orderly_shutdown_joins_scope_before_clean_rejection() {
    buffered_open(false).await;
}

async fn buffered_open(panic_worker: bool) {
    let native = NativeAccountClient::new().unwrap();
    let store = Arc::new(Store { panic: AtomicBool::new(false) });
    let bridge = SessionStoreBridge::new(native.clone(), store.clone()).unwrap();
    let account = AccountId::new();
    let user = FramerUserId("buffered-fixture".into());
    let mut opening = Box::pin(bridge.open_account_observed(account, user.clone(),
        NativeHttpLimits::default(), NativeEgress::Direct));
    assert!(futures::poll!(opening.as_mut()).is_pending());
    timeout(Duration::from_secs(2), async {
        while bridge.jobs_in_flight_for_test() != 0 { tokio::task::yield_now().await; }
    }).await.unwrap();
    assert!(matches!(native.open_account(account, session(&user),
        NativeHttpLimits::default(), NativeEgress::Direct),
        Err(crate::account::client::ClientError::AlreadyOpen)));
    if panic_worker {
        store.panic.store(true, Ordering::SeqCst);
        assert!(matches!(bridge.open_account_observed(AccountId::new(), user.clone(),
            NativeHttpLimits::default(), NativeEgress::Direct).await,
            Err(OpenAccountError::CleanupUnconfirmed)));
    } else {
        bridge.shutdown().await.unwrap();
    }
    finish_buffered(opening.await, &bridge, &native, account, &user, panic_worker).await;
}

async fn finish_buffered(result: Result<AccountScope, OpenAccountError>,
    bridge: &SessionStoreBridge, native: &NativeAccountClient,
    account: AccountId, user: &FramerUserId, panic_worker: bool) {
    if panic_worker {
        assert!(matches!(result, Err(OpenAccountError::CleanupUnconfirmed)));
    } else {
        assert!(matches!(result, Err(OpenAccountError::Rejected(_))));
    }
    // No yield after receiving the error: a fire-and-forget close still leaves
    // AlreadyOpen here on this single-threaded fixture runtime.
    let next = native.open_account(account, session(user),
        NativeHttpLimits::default(), NativeEgress::Direct)
        .expect("unpublished scope close must finish before returning rejection");
    native.close_account(&next).unwrap().await.unwrap();
    assert_eq!(bridge.shutdown().await.is_err(), panic_worker);
    native.shutdown_idle().await.unwrap();
}

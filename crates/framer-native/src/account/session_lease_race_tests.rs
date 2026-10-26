use super::*;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::time::{timeout, Duration};

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
async fn buffered_lease_cannot_escape_orderly_shutdown() { buffered(false).await; }

#[tokio::test]
async fn buffered_lease_cannot_escape_store_worker_loss() { buffered(true).await; }

async fn buffered(panic_worker: bool) {
    let native = NativeAccountClient::new().unwrap();
    let store = Arc::new(Store { panic: AtomicBool::new(false) });
    let bridge = SessionStoreBridge::new(native.clone(), store.clone()).unwrap();
    let account = AccountId::new();
    let user = FramerUserId("buffered-fixture".into());
    let mut opening = Box::pin(bridge.lease_account_observed(account, user.clone(),
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
        assert!(matches!(bridge.lease_account_observed(AccountId::new(), user.clone(),
            NativeHttpLimits::default(), NativeEgress::Direct).await,
            Err(OpenAccountError::CleanupUnconfirmed)));
    } else {
        bridge.shutdown().await.unwrap();
    }
    check_buffered(opening.await, &bridge, &native, account, &user, panic_worker).await;
}

async fn check_buffered(result: Result<leases::AccountLease, OpenAccountError>,
    bridge: &SessionStoreBridge, native: &NativeAccountClient, account: AccountId,
    user: &FramerUserId, panic_worker: bool) {
    if panic_worker { assert!(matches!(result, Err(OpenAccountError::CleanupUnconfirmed))); }
    else { assert!(matches!(result, Err(OpenAccountError::Rejected(_)))); }
    let next = native.open_account(account, session(user),
        NativeHttpLimits::default(), NativeEgress::Direct)
        .expect("buffered lease rejection must join the old generation");
    native.close_account(&next).unwrap().await.unwrap();
    assert_eq!(bridge.shutdown().await.is_err(), panic_worker);
    native.shutdown_idle().await.unwrap();
}

#[tokio::test]
async fn dropping_bridge_closes_its_idle_generations() {
    let native = NativeAccountClient::new().unwrap();
    let store = Arc::new(Store { panic: AtomicBool::new(false) });
    let bridge = SessionStoreBridge::new(native.clone(), store).unwrap();
    let lease = bridge.lease_account_observed(AccountId::new(), FramerUserId("fixture".into()),
        NativeHttpLimits::default(), NativeEgress::Direct).await.unwrap();
    let scope = lease.scope().clone();
    assert!(lease.finish(true).await);
    drop(bridge);
    timeout(Duration::from_secs(2), scope.closed()).await.unwrap().unwrap();
    native.shutdown_idle().await.unwrap();
}

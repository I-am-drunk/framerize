//! Shared legacy cache invalidation through the real native mutation barrier.
#[tokio::test]
async fn abandoned_session_change_invalidates_only_after_drain() {
    mutation_waits_for_authorization(false).await;
}

#[tokio::test]
async fn abandoned_account_change_invalidates_only_after_drain() {
    mutation_waits_for_authorization(true).await;
}

use super::*;
use framer_api::authorization_fixture::{cached, reuse, seed};

#[tokio::test]
async fn unconfirmed_generation_refuses_invalidation_and_store_write() {
    let f = Fixture::new();
    let scope = f.native.open_account(f.account, f.target.clone(),
        NativeHttpLimits::default(), NativeEgress::Direct).unwrap();
    let failed = f.native.prepare_owned(&scope, Instant::now() + Duration::from_secs(2),
        |_, _| async { PreparationResult::<()>::unconfirmed() }).unwrap();
    assert!(failed.await.is_err());
    assert!(f.bridge.save(&session("user-a", "replacement")).await.is_err());
    f.assert_cache([true; 3]);
    assert_eq!(f.store.saves.load(Ordering::SeqCst), 0);
    f.bridge.shutdown().await.unwrap();
}

async fn mutation_waits_for_authorization(account_change: bool) {
    let f = Fixture::new();
    reuse(&f.client, &f.target).await;
    let scope = f.native.open_account(f.account, f.target.clone(),
        NativeHttpLimits::default(), NativeEgress::Direct).unwrap();
    let (work, release) = held(&f.native, &scope).await;
    let replacement = session("user-a", "replacement");
    let mut change: futures::future::BoxFuture<'_, Result<(), String>> = if account_change {
        let replacement = replacement.clone();
        f.bridge.change_account(f.account, move |access| async move {
            let result = async {
                access.session(replacement.active_user.clone()).await?.save(&replacement).await
            }.await;
            ChangeOutcome::finished(result)
        }.boxed()).boxed()
    } else {
        f.bridge.save(&replacement).boxed()
    };
    assert!(futures::poll!(change.as_mut()).is_pending());
    finish_mutation(&f, work, release, change).await;
}

async fn finish_mutation(f: &Fixture, work: PendingPreparation<()>, release: oneshot::Sender<()>,
    change: futures::future::BoxFuture<'_, Result<(), String>>) {
    tokio::time::timeout(Duration::from_secs(2), async {
        while !work.cancellation().is_cancelled() { tokio::task::yield_now().await; }
    }).await.unwrap();
    f.assert_cache([true; 3]);
    assert_eq!(f.store.saves.load(Ordering::SeqCst), 0);
    drop(change); // Losing the caller must not lose mutation or invalidation.
    release.send(()).unwrap();
    assert_eq!(work.await, Err(PreparationFailure::Cancelled));
    f.bridge.shutdown().await.unwrap();
    f.assert_cache([true, false, false]);
    assert_eq!(f.store.saves.load(Ordering::SeqCst), 1);
    reuse(&f.client, &f.peer).await;
}

struct Fixture {
    native: NativeAccountClient,
    bridge: SessionStoreBridge,
    client: Arc<framer_api::HttpFramerClient>,
    store: Arc<MultiStore>,
    target: Session,
    peer: Session,
    account: AccountId,
    peer_account: AccountId,
}

impl Fixture {
    fn new() -> Self {
        let native = NativeAccountClient::new().unwrap();
        let store = MultiStore::new();
        let target = session("user-a", "target");
        let peer = session("user-b", "peer");
        let account = AccountId(uuid::Uuid::new_v4());
        let peer_account = AccountId(uuid::Uuid::new_v4());
        let client = Arc::new(framer_api::HttpFramerClient::new());
        for session in [&target, &peer] {
            store.values.lock().unwrap().insert(session.active_user.0.clone(), session.clone());
        }
        seed(&client, account, &target);
        seed(&client, peer_account, &peer);
        let bridge = SessionStoreBridge::with_authorization(native.clone(), store.clone(),
            client.session_authorization()).unwrap();
        Self { native, bridge, client, store, target, peer, account, peer_account }
    }

    fn assert_cache(&self, expected: [bool; 3]) {
        assert_eq!(cached(&self.client, self.account, &self.target), expected);
        assert_eq!(cached(&self.client, self.peer_account, &self.peer), [true; 3]);
    }

}

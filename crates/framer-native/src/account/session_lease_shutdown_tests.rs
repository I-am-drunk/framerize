// Included by account::sessions as lease_shutdown_tests. Source-only; unrun.
use super::{AccountLease, SessionStoreBridge};
use crate::account::client::{AccountScope, CloseError, NativeAccountClient};
use crate::account::client::{PendingPreparation, PreparationControl};
use crate::account::client::{PreparationFailure, PreparationResult};
use crate::transport::http::{NativeEgress, NativeHttpLimits};
use framer_core::ids::{AccountId, FramerUserId};
use framer_core::port::{Session, SessionStore};
use futures::FutureExt;
use std::future::Future;
use std::sync::Arc;
use tokio::sync::oneshot;
use tokio::time::{timeout, Duration, Instant};

async fn bounded<F: Future>(future: F) -> F::Output {
    timeout(Duration::from_secs(2), future).await.expect("fixture deadline exceeded")
}

struct Store;
#[async_trait::async_trait]
impl SessionStore for Store {
    async fn load(&self, user: &FramerUserId) -> Result<Option<Session>, String> {
        Ok(Some(Session { active_user: user.clone(),
            cookie_header: format!("session=fixture-{}", user.0) }))
    }
    async fn save(&self, _: &Session) -> Result<(), String> {
        panic!("shutdown fixture must not save sessions")
    }
    async fn delete(&self, _: &FramerUserId) -> Result<(), String> {
        panic!("shutdown fixture must not delete sessions")
    }
}

struct Fixture { native: NativeAccountClient, bridge: SessionStoreBridge }
impl Fixture {
    fn new() -> Self {
        let native = NativeAccountClient::new().unwrap();
        let bridge = SessionStoreBridge::new(native.clone(), Arc::new(Store)).unwrap();
        Self { native, bridge }
    }
    async fn lease(&self, account: AccountId, name: &str) -> AccountLease {
        bounded(self.bridge.lease_account_observed(account, FramerUserId(name.to_owned()),
            NativeHttpLimits::default(), NativeEgress::Direct)).await.unwrap()
    }
}

async fn held_work(f: &Fixture, scope: &AccountScope)
    -> (PendingPreparation<()>, oneshot::Sender<bool>) {
    let (entered, started) = oneshot::channel();
    let (release, released) = oneshot::channel();
    let pending = f.native.prepare_owned(scope, Instant::now() + Duration::from_secs(30),
        move |_, _| async move {
            entered.send(()).unwrap();
            if released.await.unwrap() { PreparationResult::finished(Ok(())) }
            else { PreparationResult::unconfirmed() }
        }).unwrap();
    bounded(started).await.unwrap();
    (pending, release)
}

async fn cancelled(control: PreparationControl) {
    while !control.is_cancelled() { tokio::task::yield_now().await; }
}

#[tokio::test]
async fn concurrent_shutdown_waiters_join_the_same_held_work() {
    let f = Fixture::new();
    let lease = f.lease(AccountId::new(), "user-a").await;
    let scope = lease.scope().clone();
    let (pending, release) = held_work(&f, &scope).await;
    assert!(bounded(lease.finish(true)).await);
    let mut first = Box::pin(f.bridge.shutdown());
    let mut second = Box::pin(f.bridge.shutdown());
    bounded(async {
        tokio::select! {
            biased;
            result = first.as_mut() => panic!("first shutdown returned before join: {result:?}"),
            result = second.as_mut() => panic!("second shutdown returned before join: {result:?}"),
            _ = cancelled(pending.cancellation()) => {},
        }
    }).await;
    assert!(futures::poll!(first.as_mut()).is_pending());
    assert!(futures::poll!(second.as_mut()).is_pending());
    assert!(pending.completion().now_or_never().is_none());
    assert!(scope.closed().now_or_never().is_none());
    release.send(true).unwrap();
    let (first, second, work) = bounded(async {
        tokio::join!(first, second, pending)
    }).await;
    assert_eq!(first, Ok(()));
    assert_eq!(second, Ok(()));
    assert_eq!(work, Err(PreparationFailure::Cancelled));
    assert_eq!(bounded(scope.closed()).await, Ok(()));
    bounded(f.native.shutdown_idle()).await.unwrap();
}

#[tokio::test]
async fn dropped_shutdown_waiter_does_not_release_the_retry_before_join() {
    let f = Fixture::new();
    let lease = f.lease(AccountId::new(), "user-a").await;
    let scope = lease.scope().clone();
    let (pending, release) = held_work(&f, &scope).await;
    assert!(bounded(lease.finish(true)).await);
    let mut first = Box::pin(f.bridge.shutdown());
    bounded(async {
        tokio::select! {
            biased;
            result = first.as_mut() => panic!("shutdown returned before cancellation: {result:?}"),
            _ = cancelled(pending.cancellation()) => {},
        }
    }).await;
    assert!(futures::poll!(first.as_mut()).is_pending());
    assert!(pending.completion().now_or_never().is_none());
    drop(first);
    let mut retry = Box::pin(f.bridge.shutdown());
    assert!(futures::poll!(retry.as_mut()).is_pending());
    assert!(pending.completion().now_or_never().is_none());
    assert!(scope.closed().now_or_never().is_none());
    release.send(true).unwrap();
    let (result, work) = bounded(async { tokio::join!(retry, pending) }).await;
    assert_eq!(result, Ok(()));
    assert_eq!(work, Err(PreparationFailure::Cancelled));
    assert_eq!(bounded(scope.closed()).await, Ok(()));
    bounded(f.native.shutdown_idle()).await.unwrap();
}

#[tokio::test]
async fn unconfirmed_work_keeps_concurrent_and_repeated_shutdown_failed() {
    let f = Fixture::new();
    let lease = f.lease(AccountId::new(), "user-a").await;
    let scope = lease.scope().clone();
    let (pending, release) = held_work(&f, &scope).await;
    assert!(bounded(lease.finish(true)).await);
    let mut first = Box::pin(f.bridge.shutdown());
    let mut second = Box::pin(f.bridge.shutdown());
    bounded(async {
        tokio::select! {
            biased;
            result = first.as_mut() => panic!("first shutdown outran held work: {result:?}"),
            result = second.as_mut() => panic!("second shutdown outran held work: {result:?}"),
            _ = cancelled(pending.cancellation()) => {},
        }
    }).await;
    assert!(pending.completion().now_or_never().is_none());
    release.send(false).unwrap();
    let (first, second, work) = bounded(async {
        tokio::join!(first, second, pending)
    }).await;
    assert_eq!(work, Err(PreparationFailure::CleanupUnconfirmed));
    assert_eq!(bounded(scope.closed()).await, Err(CloseError::PreparationUnconfirmed));
    assert!(first.is_err(), "unconfirmed native work must fail shutdown");
    assert_eq!(second, first);
    assert_eq!(bounded(f.bridge.shutdown()).await, first);
    assert_eq!(bounded(f.bridge.shutdown()).await, first);
    // No success is inferred for this intentionally failed, synthetic generation.
    drop(f);
}

#[tokio::test]
async fn unfinished_lease_drop_closes_without_idle_expiry_and_keeps_a_peer_usable() {
    let f = Fixture::new();
    let unfinished = f.lease(AccountId::new(), "user-a").await;
    let peer = f.lease(AccountId::new(), "user-b").await;
    let abandoned = unfinished.scope().clone();
    let other = peer.scope().clone();
    drop(unfinished);
    // The two-second bound is below the fifteen-second idle cache expiry.
    assert_eq!(bounded(abandoned.closed()).await, Ok(()));
    assert!(other.closed().now_or_never().is_none());
    let prepared = f.native.prepare_owned(&other,
        Instant::now() + Duration::from_secs(30), |session, _| async move {
            PreparationResult::finished(Ok(session.active_user))
        }).unwrap();
    assert_eq!(bounded(prepared).await.unwrap().0, "user-b");
    let reused = f.lease(other.account(), "user-b").await;
    assert!(other.same_generation(reused.scope()));
    assert!(bounded(reused.finish(true)).await);
    assert!(other.closed().now_or_never().is_none());
    let replacement = f.lease(abandoned.account(), "user-a").await;
    assert!(!abandoned.same_generation(replacement.scope()));
    assert!(bounded(replacement.finish(true)).await);
    assert!(bounded(peer.finish(true)).await);
    bounded(f.bridge.shutdown()).await.unwrap();
    assert_eq!(bounded(other.closed()).await, Ok(()));
    bounded(f.native.shutdown_idle()).await.unwrap();
}

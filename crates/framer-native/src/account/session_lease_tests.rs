// Included by account::sessions. Source-only; execution belongs to the owner.
// Paused-time cases require Tokio's test-util feature.
use super::{AccountLease, ChangeOutcome, OpenAccountError, SessionStoreBridge};
use crate::account::client::{AccountScope, NativeAccountClient, PendingPreparation};
use crate::account::client::{PreparationControl, PreparationFailure, PreparationResult};
use crate::transport::http::{NativeEgress, NativeHttpLimits};
use framer_core::ids::{AccountId, FramerUserId};
use framer_core::port::{Session, SessionStore};
use futures::FutureExt;
use std::future::Future;
use std::sync::{Arc, Mutex};
use tokio::sync::oneshot;
use tokio::time::{advance, timeout, Duration, Instant};

const PROXY_A: &str = "http://127.0.0.1:19001";
const PROXY_B: &str = "http://127.0.0.1:19002";

fn user(name: &str) -> FramerUserId { FramerUserId(name.to_owned()) }
fn session(name: &str, cookie: &str) -> Session {
    Session { active_user: user(name), cookie_header: cookie.to_owned() }
}
fn egress(proxy: Option<&str>) -> NativeEgress<'_> {
    match proxy { Some(value) => NativeEgress::Proxy(value), None => NativeEgress::Direct }
}
async fn bounded<F: Future>(future: F) -> F::Output {
    timeout(Duration::from_secs(2), future).await.expect("fixture deadline exceeded")
}
fn assert_open(scope: &AccountScope) { assert!(scope.closed().now_or_never().is_none()); }
fn assert_closed(scope: &AccountScope) {
    assert_eq!(scope.closed().now_or_never(), Some(Ok(())));
}

struct Store { values: Mutex<Vec<Session>> }
impl Store {
    fn new() -> Arc<Self> {
        Arc::new(Self { values: Mutex::new(vec![
            session("user-a", "session=fixture-a"),
            session("user-b", "session=fixture-b"),
        ]) })
    }
    fn put(&self, value: Session) {
        let mut values = self.values.lock().unwrap();
        values.retain(|old| old.active_user != value.active_user);
        values.push(value);
    }
}
#[async_trait::async_trait]
impl SessionStore for Store {
    async fn load(&self, user: &FramerUserId) -> Result<Option<Session>, String> {
        Ok(self.values.lock().unwrap().iter().find(|value| &value.active_user == user).cloned())
    }
    async fn save(&self, value: &Session) -> Result<(), String> {
        self.put(value.clone());
        Ok(())
    }
    async fn delete(&self, user: &FramerUserId) -> Result<(), String> {
        self.values.lock().unwrap().retain(|value| &value.active_user != user);
        Ok(())
    }
}

struct Fixture {
    native: NativeAccountClient,
    store: Arc<Store>,
    bridge: SessionStoreBridge,
}
impl Fixture {
    fn new() -> Self {
        let native = NativeAccountClient::new().unwrap();
        let store = Store::new();
        let bridge = SessionStoreBridge::new(native.clone(), store.clone()).unwrap();
        Self { native, store, bridge }
    }
    async fn lease(&self, account: AccountId, name: &str) -> AccountLease {
        bounded(self.bridge.lease_account_observed(account, user(name),
            NativeHttpLimits::default(), NativeEgress::Direct)).await.unwrap()
    }
    async fn close(self) {
        bounded(self.bridge.shutdown()).await.unwrap();
        bounded(self.native.shutdown_idle()).await.unwrap();
    }
}

async fn observed_session(f: &Fixture, scope: &AccountScope) -> Session {
    let pending = f.native.prepare_owned(scope, Instant::now() + Duration::from_secs(10),
        |value, _| async move { PreparationResult::finished(Ok(value)) }).unwrap();
    bounded(pending).await.unwrap()
}

async fn held_work(f: &Fixture, scope: &AccountScope)
    -> (PendingPreparation<()>, oneshot::Sender<()>) {
    let (entered, started) = oneshot::channel();
    let (release, released) = oneshot::channel();
    let pending = f.native.prepare_owned(scope, Instant::now() + Duration::from_secs(30),
        move |_, _| async move {
            entered.send(()).unwrap();
            released.await.unwrap();
            PreparationResult::finished(Ok(()))
        }).unwrap();
    bounded(started).await.unwrap();
    (pending, release)
}

async fn cancelled(control: PreparationControl) {
    bounded(async {
        while !control.is_cancelled() { tokio::task::yield_now().await; }
    }).await;
}

#[tokio::test]
async fn consecutive_matching_leases_reuse_one_generation() {
    let f = Fixture::new();
    let account = AccountId::new();
    let first = f.lease(account, "user-a").await;
    let scope = first.scope().clone();
    assert!(bounded(first.finish(true)).await);
    assert_open(&scope);
    let second = f.lease(account, "user-a").await;
    assert!(scope.same_generation(second.scope()));
    assert!(bounded(second.finish(true)).await);
    f.close().await;
    assert_closed(&scope);
}

#[tokio::test]
async fn same_user_accounts_are_isolated_and_one_release_keeps_its_peer_open() {
    let f = Fixture::new();
    let left = f.lease(AccountId::new(), "user-a").await;
    let right = f.lease(AccountId::new(), "user-a").await;
    let a = left.scope().clone();
    let b = right.scope().clone();
    assert!(!a.same_generation(&b));
    let peer = f.lease(b.account(), "user-a").await;
    assert!(b.same_generation(peer.scope()));
    // The bool for unconfirmed callers is unspecified; observe actual closure.
    let _ = bounded(left.finish(false)).await;
    assert_closed(&a);
    assert_open(&b);
    assert!(bounded(right.finish(true)).await);
    assert_open(peer.scope());
    assert_eq!(observed_session(&f, peer.scope()).await.active_user.0, "user-a");
    assert!(bounded(peer.finish(true)).await);
    f.close().await;
    assert_closed(&b);
}

#[tokio::test(start_paused = true)]
async fn only_the_last_release_starts_the_fifteen_second_idle_period() {
    let f = Fixture::new();
    let account = AccountId::new();
    let first = f.lease(account, "user-a").await;
    let second = f.lease(account, "user-a").await;
    let scope = first.scope().clone();
    assert!(scope.same_generation(second.scope()));
    assert!(bounded(first.finish(true)).await);
    advance(Duration::from_secs(40)).await;
    tokio::task::yield_now().await;
    assert_open(&scope);
    assert!(bounded(second.finish(true)).await);
    advance(Duration::from_millis(14_999)).await;
    tokio::task::yield_now().await;
    assert_open(&scope);
    timeout(Duration::from_millis(1_001), scope.closed()).await
        .expect("idle generation survived the next one-second sweep").unwrap();
    let next = f.lease(account, "user-a").await;
    assert!(!scope.same_generation(next.scope()));
    assert!(bounded(next.finish(true)).await);
    f.close().await;
}

#[tokio::test(start_paused = true)]
async fn reacquiring_an_idle_generation_resets_its_expiry() {
    let f = Fixture::new();
    let account = AccountId::new();
    let first = f.lease(account, "user-a").await;
    let scope = first.scope().clone();
    assert!(bounded(first.finish(true)).await);
    advance(Duration::from_secs(10)).await;
    let second = f.lease(account, "user-a").await;
    assert!(scope.same_generation(second.scope()));
    assert!(bounded(second.finish(true)).await);
    advance(Duration::from_millis(14_999)).await;
    tokio::task::yield_now().await;
    assert_open(&scope);
    timeout(Duration::from_millis(1_001), scope.closed()).await
        .expect("renewed idle deadline was not swept").unwrap();
    f.close().await;
}

#[derive(Clone, Copy, Debug)]
enum Difference { Credentials, User, DirectToProxy, ProxyToDirect, ProxyUrl, Limit(usize) }

#[derive(Clone, Copy)]
struct Config { name: &'static str, limits: NativeHttpLimits, proxy: Option<&'static str> }
impl Default for Config {
    fn default() -> Self {
        Self { name: "user-a", limits: NativeHttpLimits::default(), proxy: None }
    }
}
fn limit_change(index: usize) -> NativeHttpLimits {
    let mut limits = NativeHttpLimits::default();
    match index {
        0 => limits.request_bytes -= 1,
        1 => limits.buffered_response_bytes -= 1,
        2 => limits.streamed_response_bytes -= 1,
        3 => limits.incoming_chunk_bytes -= 1,
        4 => limits.emitted_chunk_bytes -= 1,
        5 => limits.credential_bytes -= 1,
        6 => limits.in_flight += 1,
        7 => limits.connect_timeout -= Duration::from_secs(1),
        8 => limits.request_timeout -= Duration::from_secs(1),
        9 => limits.token_max_age -= Duration::from_millis(1),
        _ => unreachable!("fixture limit field"),
    }
    limits
}

fn configurations(difference: Difference) -> (Config, Config) {
    let mut before = Config::default();
    let mut after = before;
    match difference {
        Difference::Credentials => {},
        Difference::User => after.name = "user-b",
        Difference::DirectToProxy => after.proxy = Some(PROXY_A),
        Difference::ProxyToDirect => before.proxy = Some(PROXY_A),
        Difference::ProxyUrl => { before.proxy = Some(PROXY_A); after.proxy = Some(PROXY_B); },
        Difference::Limit(index) => after.limits = limit_change(index),
    }
    (before, after)
}
async fn configured(f: &Fixture, account: AccountId, config: Config)
    -> Result<AccountLease, OpenAccountError> {
    bounded(f.bridge.lease_account_observed(account, user(config.name),
        config.limits, egress(config.proxy))).await
}

async fn configuration_change(difference: Difference, held: bool) {
    let f = Fixture::new();
    let account = AccountId::new();
    let (before, after) = configurations(difference);
    let first = configured(&f, account, before).await.unwrap();
    let scope = first.scope().clone();
    let held = if held { Some(first) } else {
        assert!(bounded(first.finish(true)).await);
        None
    };
    if matches!(difference, Difference::Credentials) {
        f.store.put(session("user-a", "session=fixture-replaced"));
    }
    let result = configured(&f, account, after).await;
    if let Some(first) = held {
        assert!(matches!(result, Err(OpenAccountError::Rejected(_))), "{difference:?}");
        assert_open(&scope);
        f.store.put(session("user-a", "session=fixture-a"));
        let peer = configured(&f, account, before).await.unwrap();
        assert!(scope.same_generation(peer.scope()), "{difference:?}");
        assert!(bounded(first.finish(true)).await);
        assert_open(peer.scope());
        assert!(bounded(peer.finish(true)).await);
    } else {
        let next = result.expect("idle mismatched generation must be replaceable");
        assert_closed(&scope);
        assert!(!scope.same_generation(next.scope()), "{difference:?}");
        assert!(next.scope().matches_egress(&egress(after.proxy)));
        let expected = bounded(f.store.load(&user(after.name))).await.unwrap().unwrap();
        let observed = observed_session(&f, next.scope()).await;
        assert_eq!(observed.active_user.0, expected.active_user.0, "{difference:?}");
        assert_eq!(observed.cookie_header, expected.cookie_header, "{difference:?}");
        let replacement = next.scope().clone();
        assert!(bounded(next.finish(true)).await);
        let reused = configured(&f, account, after).await.unwrap();
        assert!(replacement.same_generation(reused.scope()), "{difference:?}");
        assert!(bounded(reused.finish(true)).await);
    }
    f.close().await;
}

const IDENTITY_CHANGES: [Difference; 5] = [Difference::Credentials, Difference::User,
    Difference::DirectToProxy, Difference::ProxyToDirect, Difference::ProxyUrl];

#[tokio::test]
async fn held_identity_mismatches_reject_without_closing_the_generation() {
    for difference in IDENTITY_CHANGES { configuration_change(difference, true).await; }
}

#[tokio::test]
async fn idle_identity_mismatches_join_and_replace_the_generation() {
    for difference in IDENTITY_CHANGES { configuration_change(difference, false).await; }
}

#[tokio::test]
async fn every_changed_limit_rejects_reuse_while_held() {
    for field in 0..10 { configuration_change(Difference::Limit(field), true).await; }
}

#[tokio::test]
async fn every_changed_limit_replaces_an_idle_generation() {
    for field in 0..10 { configuration_change(Difference::Limit(field), false).await; }
}

#[tokio::test]
async fn idle_replacement_waits_for_the_old_native_worker_to_join() {
    let f = Fixture::new();
    let account = AccountId::new();
    let lease = f.lease(account, "user-a").await;
    let old = lease.scope().clone();
    let (pending, release) = held_work(&f, &old).await;
    assert!(bounded(lease.finish(true)).await);
    f.store.put(session("user-a", "session=fixture-new"));
    let mut replacement = Box::pin(f.bridge.lease_account_observed(account, user("user-a"),
        NativeHttpLimits::default(), NativeEgress::Direct));
    assert!(futures::poll!(replacement.as_mut()).is_pending());
    tokio::select! {
        _ = replacement.as_mut() => panic!("replacement returned before old work joined"),
        _ = cancelled(pending.cancellation()) => {},
    }
    assert!(futures::poll!(replacement.as_mut()).is_pending());
    assert_open(&old);
    release.send(()).unwrap();
    assert_eq!(bounded(pending).await, Err(PreparationFailure::Cancelled));
    let next = bounded(replacement).await.unwrap();
    assert_closed(&old);
    assert!(!old.same_generation(next.scope()));
    assert_eq!(observed_session(&f, next.scope()).await.cookie_header, "session=fixture-new");
    assert!(bounded(next.finish(true)).await);
    f.close().await;
}

#[tokio::test]
async fn unconfirmed_finish_joins_its_worker_and_preserves_another_account() {
    let f = Fixture::new();
    let lease = f.lease(AccountId::new(), "user-a").await;
    let other = f.lease(AccountId::new(), "user-b").await;
    let scope = lease.scope().clone();
    let (pending, release) = held_work(&f, &scope).await;
    let mut finishing = Box::pin(lease.finish(false));
    assert!(futures::poll!(finishing.as_mut()).is_pending());
    tokio::select! {
        _ = finishing.as_mut() => panic!("finish returned before owned work joined"),
        _ = cancelled(pending.cancellation()) => {},
    }
    assert!(futures::poll!(finishing.as_mut()).is_pending());
    assert_open(&scope);
    assert_open(other.scope());
    release.send(()).unwrap();
    assert_eq!(bounded(pending).await, Err(PreparationFailure::Cancelled));
    let _ = bounded(finishing).await;
    assert_closed(&scope);
    assert_open(other.scope());
    assert_eq!(observed_session(&f, other.scope()).await.active_user.0, "user-b");
    assert!(bounded(other.finish(true)).await);
    f.close().await;
}

#[tokio::test]
async fn native_session_barrier_blocks_cached_and_new_alias_leases() {
    let f = Fixture::new();
    let first = f.lease(AccountId::new(), "user-a").await;
    let alias = f.lease(AccountId::new(), "user-a").await;
    let other = f.lease(AccountId::new(), "user-b").await;
    let old = first.scope().clone();
    let alias_scope = alias.scope().clone();
    let mut ticket = f.native.begin_session_change(&user("user-a")).unwrap();
    bounded(ticket.ready()).await.unwrap();
    assert_closed(&old);
    assert_closed(&alias_scope);
    for account in [old.account(), alias_scope.account(), AccountId::new()] {
        assert!(matches!(configured(&f, account, Config::default()).await,
            Err(OpenAccountError::Rejected(_))));
    }
    let other_peer = f.lease(other.scope().account(), "user-b").await;
    assert!(other.scope().same_generation(other_peer.scope()));
    let _ = bounded(first.finish(true)).await;
    let _ = bounded(alias.finish(true)).await;
    assert_open(other_peer.scope());
    f.native.finish_session_change(&mut ticket).unwrap();
    let next = f.lease(old.account(), "user-a").await;
    assert!(!old.same_generation(next.scope()));
    assert_open(next.scope());
    assert!(bounded(next.finish(true)).await);
    assert!(bounded(other.finish(true)).await);
    assert!(bounded(other_peer.finish(true)).await);
    f.close().await;
}

#[tokio::test]
async fn native_account_barrier_blocks_every_user_without_closing_a_peer() {
    let f = Fixture::new();
    let first = f.lease(AccountId::new(), "user-a").await;
    let other = f.lease(AccountId::new(), "user-a").await;
    let old = first.scope().clone();
    let mut ticket = f.native.begin_account_change(old.account()).unwrap();
    bounded(ticket.ready()).await.unwrap();
    assert_closed(&old);
    for name in ["user-a", "user-b"] {
        let config = Config { name, ..Config::default() };
        assert!(matches!(configured(&f, old.account(), config).await,
            Err(OpenAccountError::Rejected(_))));
    }
    let peer = f.lease(other.scope().account(), "user-a").await;
    assert!(other.scope().same_generation(peer.scope()));
    let _ = bounded(first.finish(false)).await;
    assert_open(peer.scope());
    f.native.finish_account_change(&mut ticket).unwrap();
    let next = f.lease(old.account(), "user-b").await;
    assert!(!old.same_generation(next.scope()));
    assert_open(next.scope());
    assert!(bounded(next.finish(true)).await);
    assert!(bounded(other.finish(true)).await);
    assert!(bounded(peer.finish(true)).await);
    f.close().await;
}

async fn bridge_mutation_barrier(session_change: bool) {
    let f = Fixture::new();
    let first = f.lease(AccountId::new(), "user-a").await;
    let alias = f.lease(AccountId::new(), "user-a").await;
    let other = f.lease(AccountId::new(), "user-b").await;
    let old = first.scope().clone();
    let alias_scope = alias.scope().clone();
    assert!(bounded(first.finish(true)).await);
    assert!(bounded(alias.finish(true)).await);
    let (entered, started) = oneshot::channel();
    let (release, released) = oneshot::channel();
    let mut mutation: futures::future::BoxFuture<'_, Result<(), String>> = if session_change {
        f.bridge.change(user("user-a"), move |access| async move {
            entered.send(()).unwrap();
            released.await.unwrap();
            ChangeOutcome::finished(access.save(&session("user-a", "session=fixture-new")).await)
        }.boxed()).boxed()
    } else {
        f.bridge.change_account(old.account(), move |_| async move {
            entered.send(()).unwrap();
            released.await.unwrap();
            ChangeOutcome::finished(Ok(()))
        }.boxed()).boxed()
    };
    assert!(futures::poll!(mutation.as_mut()).is_pending());
    bounded(started).await.unwrap();
    assert_closed(&old);
    let forbidden = if session_change {
        vec![(old.account(), "user-a"), (alias_scope.account(), "user-a"),
            (AccountId::new(), "user-a")]
    } else {
        vec![(old.account(), "user-a"), (old.account(), "user-b")]
    };
    for (account, name) in forbidden {
        assert!(matches!(configured(&f, account, Config { name, ..Config::default() }).await,
            Err(OpenAccountError::Rejected(_))));
    }
    if session_change { assert_closed(&alias_scope); } else {
        let alias = f.lease(alias_scope.account(), "user-a").await;
        assert!(alias_scope.same_generation(alias.scope()));
        assert!(bounded(alias.finish(true)).await);
    }
    let other_peer = f.lease(other.scope().account(), "user-b").await;
    assert!(other.scope().same_generation(other_peer.scope()));
    release.send(()).unwrap();
    bounded(mutation).await.unwrap();
    let next = f.lease(old.account(), "user-a").await;
    assert!(!old.same_generation(next.scope()));
    let expected = if session_change { "session=fixture-new" } else { "session=fixture-a" };
    assert_eq!(observed_session(&f, next.scope()).await.cookie_header, expected);
    assert!(bounded(next.finish(true)).await);
    assert!(bounded(other.finish(true)).await);
    assert!(bounded(other_peer.finish(true)).await);
    f.close().await;
}

#[tokio::test]
async fn bridge_session_change_fences_cached_aliases_through_the_store_commit() {
    bridge_mutation_barrier(true).await;
}

#[tokio::test]
async fn bridge_account_change_fences_cached_admission_through_completion() {
    bridge_mutation_barrier(false).await;
}

#[tokio::test]
async fn bridge_shutdown_joins_cached_generations_and_closes_lease_admission() {
    let f = Fixture::new();
    let first = f.lease(AccountId::new(), "user-a").await;
    let second = f.lease(AccountId::new(), "user-b").await;
    let a = first.scope().clone();
    let b = second.scope().clone();
    let (pending, release) = held_work(&f, &a).await;
    assert!(bounded(first.finish(true)).await);
    assert!(bounded(second.finish(true)).await);
    let mut shutdown = Box::pin(f.bridge.shutdown());
    assert!(futures::poll!(shutdown.as_mut()).is_pending());
    tokio::select! {
        _ = shutdown.as_mut() => panic!("shutdown returned before owned work joined"),
        _ = cancelled(pending.cancellation()) => {},
    }
    assert!(futures::poll!(shutdown.as_mut()).is_pending());
    assert_open(&a);
    assert!(matches!(configured(&f, AccountId::new(), Config::default()).await,
        Err(OpenAccountError::Rejected(_))));
    release.send(()).unwrap();
    assert_eq!(bounded(pending).await, Err(PreparationFailure::Cancelled));
    bounded(shutdown).await.unwrap();
    assert_closed(&a);
    assert_closed(&b);
    assert!(matches!(configured(&f, b.account(), Config { name: "user-b", ..Config::default() }).await,
        Err(OpenAccountError::Rejected(_))));
    bounded(f.native.shutdown_idle()).await.unwrap();
}

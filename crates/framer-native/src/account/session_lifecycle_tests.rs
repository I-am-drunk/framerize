use crate::account::client::*;
use crate::transport::http::NativeEgress;
use crate::transport::http::NativeHttpLimits;
use crate::account::sessions::ChangeOutcome;
use crate::account::sessions::SessionStoreBridge;
use framer_core::ids::AccountId;
use framer_core::ids::FramerUserId;
use framer_core::port::Session;
use framer_core::port::SessionStore;
use futures::FutureExt;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use tokio::sync::oneshot;
use tokio::sync::Semaphore;
use tokio::time::Duration;
use tokio::time::Instant;

#[tokio::test]
async fn cancelled_session_private_load_poisons_account_and_user() {
    let native = NativeAccountClient::new().unwrap();
    let store = MultiStore::new();
    let loading = store.pause("user-b");
    let bridge = SessionStoreBridge::new(native.clone(), store).unwrap();
    let account = AccountId(uuid::Uuid::new_v4());
    let result = bridge.change_account(account, move |access| async move {
        let mut binding = Box::pin(access.session(FramerUserId("user-b".to_owned())));
        tokio::select! {
            result = &mut binding => panic!("private load unexpectedly finished: {}", result.is_ok()),
            _ = loading.entered() => {}
        }
        drop(binding);
        ChangeOutcome::finished(Ok(()))
    }.boxed()).await;
    assert!(result.is_err());
    let mut account_ticket = native.begin_account_change(account).unwrap();
    let mut user_ticket = native.begin_session_change(&FramerUserId("user-b".to_owned())).unwrap();
    assert_eq!(account_ticket.ready().await, Err(CloseError::SessionMutationUnconfirmed));
    assert_eq!(user_ticket.ready().await, Err(CloseError::SessionMutationUnconfirmed));
    assert!(native.finish_account_change(&mut account_ticket).is_err());
    assert!(native.finish_session_change(&mut user_ticket).is_err());
    bridge.shutdown().await.unwrap();
}

#[tokio::test]
async fn cancelled_session_preparation_before_drain_keeps_retryable_fences() {
    let native = NativeAccountClient::new().unwrap();
    let target = open(&native, "user-b");
    let (work, release) = held(&native, &target).await;
    let account = AccountId(uuid::Uuid::new_v4());
    let bridge = SessionStoreBridge::new(native.clone(), MultiStore::new()).unwrap();
    let result = bridge.change_account(account, move |access| async move {
        let mut binding = Box::pin(access.session(FramerUserId("user-b".to_owned())));
        assert!(futures::poll!(binding.as_mut()).is_pending());
        drop(binding);
        release.send(()).unwrap();
        assert_eq!(work.await, Err(PreparationFailure::Cancelled));
        target.closed().await.unwrap();
        ChangeOutcome::finished(Ok(()))
    }.boxed()).await;
    assert!(result.is_err());
    assert!(bridge.load(&FramerUserId("user-b".to_owned())).await.is_err());
    assert!(matches!(native.open_account(account, session("user-b", "old"),
        NativeHttpLimits::default(), NativeEgress::Direct), Err(ClientError::SessionChanging)));
    bridge.change_account(account, |access| async move {
        let result = access.session(FramerUserId("user-b".to_owned())).await.map(|_| ());
        ChangeOutcome::finished(result)
    }.boxed()).await.unwrap();
    let reopened = bridge.open_account(account, FramerUserId("user-b".to_owned()),
        NativeHttpLimits::default(), NativeEgress::Direct).await.unwrap();
    native.close_account(&reopened).unwrap().await.unwrap();
    bridge.shutdown().await.unwrap();
}

#[tokio::test]
async fn unrelated_user_replacement_does_not_reject_delayed_read() {
    let store = MultiStore::new();
    let gate = store.pause("user-a");
    let bridge = SessionStoreBridge::new(NativeAccountClient::new().unwrap(), store).unwrap();
    let reader = bridge.clone();
    let read = tokio::spawn(async move { reader.load(&FramerUserId("user-a".to_owned())).await });
    gate.entered().await;
    bridge.save(&session("user-b", "new")).await.unwrap();
    gate.finish();
    assert_eq!(read.await.unwrap().unwrap().unwrap().active_user.0, "user-a");
    bridge.shutdown().await.unwrap();
}

#[tokio::test]
async fn unrelated_user_replacement_does_not_reject_delayed_open() {
    let native = NativeAccountClient::new().unwrap();
    let store = MultiStore::new();
    let gate = store.pause("user-a");
    let bridge = SessionStoreBridge::new(native.clone(), store).unwrap();
    let reader = bridge.clone();
    let opening = tokio::spawn(async move {
        reader.open_account(AccountId(uuid::Uuid::new_v4()), FramerUserId("user-a".to_owned()),
            NativeHttpLimits::default(), NativeEgress::Direct).await
    });
    gate.entered().await;
    bridge.save(&session("user-b", "new")).await.unwrap();
    gate.finish();
    let scope = opening.await.unwrap().unwrap();
    native.close_account(&scope).unwrap().await.unwrap();
    bridge.shutdown().await.unwrap();
}

#[tokio::test]
async fn completed_account_change_rejects_an_older_open_for_that_account() {
    let native = NativeAccountClient::new().unwrap();
    let store = MultiStore::new();
    let gate = store.pause("user-a");
    let bridge = SessionStoreBridge::new(native.clone(), store).unwrap();
    let reader = bridge.clone();
    let account = AccountId(uuid::Uuid::new_v4());
    let opening = tokio::spawn(async move {
        reader.open_account(account, FramerUserId("user-a".to_owned()),
            NativeHttpLimits::default(), NativeEgress::Direct).await
    });
    gate.entered().await;
    bridge.change_account(account, |_| async { ChangeOutcome::finished(Ok(())) }.boxed()).await.unwrap();
    gate.finish();
    assert!(opening.await.unwrap().is_err());
    let next = bridge.open_account(account, FramerUserId("user-a".to_owned()),
        NativeHttpLimits::default(), NativeEgress::Direct).await.unwrap();
    native.close_account(&next).unwrap().await.unwrap();
    bridge.shutdown().await.unwrap();
}

#[tokio::test]
async fn account_fence_drains_old_generation_and_excludes_every_new_user() {
    let native = NativeAccountClient::new().unwrap();
    let scope = open(&native, "user-a");
    let other = open(&native, "user-a");
    let (pending, release) = held(&native, &scope).await;
    let mut ticket = native.begin_account_change(scope.account()).unwrap();
    assert!(ticket.ready().now_or_never().is_none());
    assert!(matches!(native.begin_account_change(scope.account()), Err(ClientError::SessionChanging)));
    assert!(matches!(native.open_account(scope.account(), session("user-b", "new"),
        NativeHttpLimits::default(), NativeEgress::Direct), Err(ClientError::SessionChanging)));
    assert!(other.closed().now_or_never().is_none());
    release.send(()).unwrap();
    assert_eq!(pending.await, Err(PreparationFailure::Cancelled));
    ticket.ready().await.unwrap();
    native.finish_account_change(&mut ticket).unwrap();
    let next = native.open_account(scope.account(), session("user-b", "new"),
        NativeHttpLimits::default(), NativeEgress::Direct).unwrap();
    native.close_account(&scope).unwrap().await.unwrap();
    assert!(next.closed().now_or_never().is_none());
    native.close_account(&next).unwrap().await.unwrap();
    native.close_account(&other).unwrap().await.unwrap();
}

#[tokio::test]
async fn abandoned_login_fences_old_account_then_target_user_through_state_commit() {
    let native = NativeAccountClient::new().unwrap();
    let old = open(&native, "user-a");
    let target = open(&native, "user-b");
    let other = open(&native, "user-c");
    let (old_work, old_release) = held(&native, &old).await;
    let (target_work, target_release) = held(&native, &target).await;
    let store = MultiStore::new();
    let bridge = SessionStoreBridge::new(native.clone(), store.clone()).unwrap();
    let (browser_tx, mut browser_rx) = oneshot::channel();
    let (fenced_tx, fenced_rx) = oneshot::channel();
    let commit_gate = Gate::new();
    let held_commit = commit_gate.clone();
    let published = Arc::new(AtomicBool::new(false));
    let publishing = published.clone();
    let request = bridge.change_account(old.account(), move |account| async move {
        let result = async {
            let _ = browser_tx.send(());
            let mut binding = Box::pin(account.session(FramerUserId("user-b".to_owned())));
            assert!(futures::poll!(binding.as_mut()).is_pending());
            let _ = fenced_tx.send(());
            let access = binding.await?;
            access.save(&session("user-b", "new")).await?;
            held_commit.wait().await;
            publishing.store(true, Ordering::SeqCst);
            Ok(())
        }.await;
        ChangeOutcome::finished(result)
    }.boxed());
    assert!(request.now_or_never().is_none());
    assert!(old_work.cancellation().is_cancelled());
    assert!(matches!(browser_rx.try_recv(), Err(oneshot::error::TryRecvError::Empty)));
    old_release.send(()).unwrap();
    assert_eq!(old_work.await, Err(PreparationFailure::Cancelled));
    browser_rx.await.unwrap();
    fenced_rx.await.unwrap();
    assert!(target_work.cancellation().is_cancelled());
    assert_eq!(store.saves.load(Ordering::SeqCst), 0);
    target_release.send(()).unwrap();
    assert_eq!(target_work.await, Err(PreparationFailure::Cancelled));
    commit_gate.entered().await;
    assert_eq!(store.saves.load(Ordering::SeqCst), 1);
    assert!(!published.load(Ordering::SeqCst));
    assert!(matches!(native.begin_account_change(old.account()), Err(ClientError::SessionChanging)));
    assert!(matches!(native.begin_session_change(target.active_user()), Err(ClientError::SessionChanging)));
    assert!(other.closed().now_or_never().is_none());
    commit_gate.finish();
    bridge.shutdown().await.unwrap();
    assert!(published.load(Ordering::SeqCst));
    let next = native.open_account(old.account(), session("user-b", "new"),
        NativeHttpLimits::default(), NativeEgress::Direct).unwrap();
    native.close_account(&next).unwrap().await.unwrap();
    native.close_account(&other).unwrap().await.unwrap();
}

#[tokio::test]
async fn unconfirmed_login_cleanup_poisons_account_and_discovered_user() {
    let native = NativeAccountClient::new().unwrap();
    let scope = open(&native, "user-a");
    let bridge = SessionStoreBridge::new(native.clone(), MultiStore::new()).unwrap();
    let result = bridge.change_account(scope.account(), |account| async move {
        account.session(FramerUserId("user-b".to_owned())).await.unwrap();
        ChangeOutcome::<()>::unconfirmed()
    }.boxed()).await;
    assert!(result.is_err());
    let mut account = native.begin_account_change(scope.account()).unwrap();
    let mut user = native.begin_session_change(&FramerUserId("user-b".to_owned())).unwrap();
    assert_eq!(account.ready().await, Err(CloseError::SessionMutationUnconfirmed));
    assert_eq!(user.ready().await, Err(CloseError::SessionMutationUnconfirmed));
    assert!(native.finish_account_change(&mut account).is_err());
    assert!(native.finish_session_change(&mut user).is_err());
    bridge.shutdown().await.unwrap();
}

#[tokio::test]
async fn panicked_login_actor_does_not_leave_recoverable_clean_barriers() {
    let native = NativeAccountClient::new().unwrap();
    let account = AccountId(uuid::Uuid::new_v4());
    let bridge = SessionStoreBridge::new(native.clone(), MultiStore::new()).unwrap();
    let result: Result<(), String> = bridge.change_account(account, |access| async move {
        access.session(FramerUserId("user-b".to_owned())).await.unwrap();
        panic!("fixture login panic");
        #[allow(unreachable_code)] ChangeOutcome::finished(Ok(()))
    }.boxed()).await;
    assert!(result.is_err());
    assert!(bridge.load(&FramerUserId("user-b".to_owned())).await.is_err());
    assert!(bridge.load(&FramerUserId("user-a".to_owned())).await.unwrap().is_some());
    bridge.shutdown().await.unwrap();
    let account_ticket = native.begin_account_change(account).unwrap();
    let user_ticket = native.begin_session_change(&FramerUserId("user-b".to_owned())).unwrap();
    assert_eq!(account_ticket.ready().await, Err(CloseError::SessionMutationUnconfirmed));
    assert_eq!(user_ticket.ready().await, Err(CloseError::SessionMutationUnconfirmed));
}

#[tokio::test]
async fn failed_state_commit_keeps_both_fences_until_explicit_retry() {
    let native = NativeAccountClient::new().unwrap();
    let account = AccountId(uuid::Uuid::new_v4());
    let bridge = SessionStoreBridge::new(native.clone(), MultiStore::new()).unwrap();
    let failed: Result<(), String> = bridge.change_account(account, |access| async move {
        let session_access = access.session(FramerUserId("user-b".to_owned())).await.unwrap();
        session_access.save(&session("user-b", "new")).await.unwrap();
        ChangeOutcome::finished(Err("fixture StateStore failure".to_owned()))
    }.boxed()).await;
    assert!(failed.is_err());
    assert!(bridge.load(&FramerUserId("user-b".to_owned())).await.is_err());
    assert!(matches!(native.open_account(account, session("user-a", "old"),
        NativeHttpLimits::default(), NativeEgress::Direct), Err(ClientError::SessionChanging)));
    bridge.change_account(account, |access| async move {
        let result = async {
            access.session(FramerUserId("user-b".to_owned())).await?
                .save(&session("user-b", "recovered")).await
        }.await;
        ChangeOutcome::finished(result)
    }.boxed()).await.unwrap();
    let next = bridge.open_account(account, FramerUserId("user-b".to_owned()),
        NativeHttpLimits::default(), NativeEgress::Direct).await.unwrap();
    native.close_account(&next).unwrap().await.unwrap();
    bridge.shutdown().await.unwrap();
}

#[tokio::test]
async fn ignored_target_fence_error_cannot_publish_a_successful_account_change() {
    let native = NativeAccountClient::new().unwrap();
    let account = AccountId(uuid::Uuid::new_v4());
    let user = FramerUserId("user-b".to_owned());
    let mut competing = native.begin_session_change(&user).unwrap();
    competing.ready().await.unwrap();
    let bridge = SessionStoreBridge::new(native.clone(), MultiStore::new()).unwrap();
    let result = bridge.change_account(account, |access| async move {
        assert!(access.session(FramerUserId("user-b".to_owned())).await.is_err());
        ChangeOutcome::finished(Ok(()))
    }.boxed()).await;
    assert!(result.is_err());
    assert!(matches!(native.open_account(account, session("user-b", "old"),
        NativeHttpLimits::default(), NativeEgress::Direct), Err(ClientError::SessionChanging)));
    native.finish_session_change(&mut competing).unwrap();
    bridge.change_account(account, |_| async { ChangeOutcome::finished(Ok(())) }.boxed()).await.unwrap();
    bridge.shutdown().await.unwrap();
}

#[tokio::test]
async fn failed_old_account_cleanup_prevents_login_callback() {
    let native = NativeAccountClient::new().unwrap();
    let scope = open(&native, "user-a");
    let pending = native.prepare_owned(&scope, Instant::now() + Duration::from_secs(30),
        |_, _| async { PreparationResult::<()>::unconfirmed() }).unwrap();
    assert_eq!(pending.await, Err(PreparationFailure::CleanupUnconfirmed));
    let invoked = Arc::new(AtomicBool::new(false));
    let observed = invoked.clone();
    let bridge = SessionStoreBridge::new(native, MultiStore::new()).unwrap();
    assert!(bridge.change_account(scope.account(), move |_| async move {
        observed.store(true, Ordering::SeqCst);
        ChangeOutcome::finished(Ok(()))
    }.boxed()).await.is_err());
    assert!(!invoked.load(Ordering::SeqCst));
    bridge.shutdown().await.unwrap();
}

#[tokio::test]
async fn ten_account_changes_can_each_hold_a_target_user_without_nested_jobs() {
    let native = NativeAccountClient::new().unwrap();
    let bridge = SessionStoreBridge::new(native.clone(), MultiStore::new()).unwrap();
    let gate = Gate::new();
    let mut jobs = Vec::new();
    for index in 0..CAPACITY {
        let bridge = bridge.clone();
        let gate = gate.clone();
        jobs.push(tokio::spawn(async move {
            bridge.change_account(AccountId(uuid::Uuid::new_v4()), move |access| async move {
                let result = async {
                    access.session(FramerUserId(format!("user-{index}"))).await?;
                    gate.wait().await;
                    Ok(())
                }.await;
                ChangeOutcome::finished(result)
            }.boxed()).await
        }));
    }
    for _ in 0..CAPACITY { gate.entered().await; }
    assert_eq!(bridge.load(&FramerUserId("unrelated".to_owned())).await.unwrap_err(),
        "native session store busy");
    assert!(matches!(native.begin_account_change(AccountId(uuid::Uuid::new_v4())), Err(ClientError::Full)));
    assert!(matches!(native.begin_session_change(&FramerUserId("extra".to_owned())), Err(ClientError::Full)));
    for _ in 0..CAPACITY { gate.finish(); }
    for job in jobs { job.await.unwrap().unwrap(); }
    bridge.shutdown().await.unwrap();
}

#[tokio::test]
async fn completed_read_queued_for_waiter_rechecks_after_replacement() {
    let bridge = SessionStoreBridge::new(NativeAccountClient::new().unwrap(), MultiStore::new()).unwrap();
    let user = FramerUserId("user-a".to_owned());
    let mut read = Box::pin(bridge.load(&user));
    assert!(futures::poll!(read.as_mut()).is_pending());
    tokio::time::timeout(Duration::from_secs(2), async {
        while bridge.jobs_in_flight_for_test() != 0 { tokio::task::yield_now().await; }
    }).await.unwrap();
    bridge.save(&session("user-a", "new")).await.unwrap();
    assert!(read.await.is_err());
    bridge.shutdown().await.unwrap();
}

struct MultiStore {
    values: Mutex<std::collections::BTreeMap<String, Session>>,
    paused: Mutex<Option<(String, Gate)>>,
    saves: AtomicUsize,
}
impl MultiStore {
    fn new() -> Arc<Self> {
        let values = ["user-a", "user-b", "user-c"].into_iter()
            .map(|user| (user.to_owned(), session(user, "old"))).collect();
        Arc::new(Self { values: Mutex::new(values), paused: Mutex::new(None),
            saves: AtomicUsize::new(0) })
    }
    fn pause(&self, user: &str) -> Gate {
        let gate = Gate::new();
        *self.paused.lock().unwrap() = Some((user.to_owned(), gate.clone()));
        gate
    }
}
#[async_trait::async_trait]
impl SessionStore for MultiStore {
    async fn load(&self, user: &FramerUserId) -> Result<Option<Session>, String> {
        let value = self.values.lock().unwrap().get(&user.0).cloned();
        let gate = {
            let mut paused = self.paused.lock().unwrap();
            if paused.as_ref().is_some_and(|(key, _)| key == &user.0) {
                paused.take().map(|(_, gate)| gate)
            } else { None }
        };
        if let Some(gate) = gate { gate.wait().await; }
        Ok(value)
    }
    async fn save(&self, value: &Session) -> Result<(), String> {
        self.saves.fetch_add(1, Ordering::SeqCst);
        self.values.lock().unwrap().insert(value.active_user.0.clone(), value.clone());
        Ok(())
    }
    async fn delete(&self, user: &FramerUserId) -> Result<(), String> {
        self.values.lock().unwrap().remove(&user.0);
        Ok(())
    }
}

fn session(user: &str, version: &str) -> Session {
    Session { active_user: FramerUserId(user.to_owned()), cookie_header: format!("session=fixture-{version}") }
}
fn open(client: &NativeAccountClient, user: &str) -> AccountScope {
    client.open_account(AccountId(uuid::Uuid::new_v4()), session(user, "old"),
        NativeHttpLimits::default(), NativeEgress::Direct).unwrap()
}
async fn held(client: &NativeAccountClient, scope: &AccountScope)
    -> (PendingPreparation<()>, oneshot::Sender<()>) {
    let (started_tx, started_rx) = oneshot::channel();
    let (release_tx, release_rx) = oneshot::channel();
    let pending = client.prepare_owned(scope, Instant::now() + Duration::from_secs(30),
        move |_, _| async move {
            let _ = started_tx.send(());
            let _ = release_rx.await;
            PreparationResult::finished(Ok(()))
        }).unwrap();
    started_rx.await.unwrap();
    (pending, release_tx)
}

#[tokio::test]
async fn pending_authorization_survives_waiter_drop_and_delays_replacement() {
    let client = NativeAccountClient::new().unwrap();
    let scope = open(&client, "user-a");
    let (pending, release) = held(&client, &scope).await;
    drop(pending);
    let mut change = client.begin_session_change(scope.active_user()).unwrap();
    assert!(change.ready().now_or_never().is_none());
    release.send(()).unwrap();
    change.ready().await.unwrap();
    client.finish_session_change(&mut change).unwrap();
    let next = open(&client, "user-a");
    assert!(!scope.same_generation(&next));
    client.close_account(&next).unwrap().await.unwrap();
}
#[tokio::test]
async fn pending_authorizations_share_the_ten_operation_bound() {
    let client = NativeAccountClient::new().unwrap();
    let scope = open(&client, "user-a");
    let mut releases = Vec::new();
    for _ in 0..CAPACITY {
        let (pending, release) = held(&client, &scope).await;
        drop(pending);
        releases.push(release);
    }
    let extra = client.prepare_owned(&scope, Instant::now() + Duration::from_secs(30),
        |_, _| async { PreparationResult::finished(Ok(())) });
    assert!(matches!(extra, Err(ClientError::Full)));
    let closed = client.close_account(&scope).unwrap();
    assert!(closed.clone().now_or_never().is_none());
    for release in releases { release.send(()).unwrap(); }
    closed.await.unwrap();
}

#[tokio::test]
async fn successful_preparation_releases_capacity_before_publishing() {
    let client = NativeAccountClient::new().unwrap();
    let scope = open(&client, "user-a");
    for _ in 0..(CAPACITY * 2) {
        client.prepare_owned(&scope, Instant::now() + Duration::from_secs(30),
            |_, _| async { PreparationResult::finished(Ok(())) }).unwrap().await.unwrap();
    }
    client.close_account(&scope).unwrap().await.unwrap();
}

#[tokio::test]
async fn unconfirmed_authorization_blocks_session_mutation() {
    let client = NativeAccountClient::new().unwrap();
    let scope = open(&client, "user-a");
    let pending = client.prepare_owned(&scope, Instant::now() + Duration::from_secs(30),
        |_, _| async { PreparationResult::<()>::unconfirmed() }).unwrap();
    assert_eq!(pending.await, Err(PreparationFailure::CleanupUnconfirmed));
    let mut ticket = client.begin_session_change(scope.active_user()).unwrap();
    assert_eq!(ticket.ready().await, Err(CloseError::PreparationUnconfirmed));
    assert!(matches!(client.finish_session_change(&mut ticket), Err(ClientError::CleanupFailed(_))));
}
#[tokio::test]
async fn panicked_authorization_is_not_reported_as_clean() {
    let client = NativeAccountClient::new().unwrap();
    let scope = open(&client, "user-a");
    let pending = client.prepare_owned(&scope, Instant::now() + Duration::from_secs(30),
        |_, _| async { panic!("fixture panic"); #[allow(unreachable_code)] PreparationResult::finished(Ok(())) }).unwrap();
    assert_eq!(pending.await, Err(PreparationFailure::CleanupUnconfirmed));
    assert_eq!(client.close_account(&scope).unwrap().await, Err(CloseError::PreparationUnconfirmed));
}

#[tokio::test]
async fn wrong_client_and_closed_generation_never_invoke_source() {
    let client = NativeAccountClient::new().unwrap();
    let other = NativeAccountClient::new().unwrap();
    let scope = open(&client, "user-a");
    let invoked = Arc::new(AtomicBool::new(false));
    let seen = invoked.clone();
    let result = other.prepare_owned(&scope, Instant::now() + Duration::from_secs(30),
        move |_, _| async move { seen.store(true, Ordering::SeqCst); PreparationResult::finished(Ok(())) });
    assert!(matches!(result, Err(ClientError::WrongClient)));
    client.close_account(&scope).unwrap().await.unwrap();
    let seen = invoked.clone();
    let result = client.prepare_owned(&scope, Instant::now() + Duration::from_secs(30),
        move |_, _| async move { seen.store(true, Ordering::SeqCst); PreparationResult::finished(Ok(())) });
    assert!(matches!(result, Err(ClientError::StaleScope)));
    assert!(!invoked.load(Ordering::SeqCst));
}

#[tokio::test]
async fn same_user_close_drains_all_accounts_without_closing_other_users() {
    let client = NativeAccountClient::new().unwrap();
    let a = open(&client, "user-a");
    let b = open(&client, "user-a");
    let c = open(&client, "user-c");
    let (pending, release) = held(&client, &b).await;
    let mut ticket = client.begin_session_change(a.active_user()).unwrap();
    assert!(ticket.ready().now_or_never().is_none());
    assert!(c.closed().now_or_never().is_none());
    release.send(()).unwrap();
    assert_eq!(pending.await, Err(PreparationFailure::Cancelled));
    ticket.ready().await.unwrap();
    client.finish_session_change(&mut ticket).unwrap();
    client.close_account(&c).unwrap().await.unwrap();
}
#[derive(Clone)]
struct Gate { entered: Arc<Semaphore>, release: Arc<Semaphore> }
impl Gate {
    fn new() -> Self { Self { entered: Arc::new(Semaphore::new(0)), release: Arc::new(Semaphore::new(0)) } }
    async fn wait(&self) {
        self.entered.add_permits(1);
        self.release.acquire().await.unwrap().forget();
    }
    async fn entered(&self) { self.entered.acquire().await.unwrap().forget(); }
    fn finish(&self) { self.release.add_permits(1); }
}
struct FixtureStore {
    value: Mutex<Option<Session>>,
    load_gate: Mutex<Option<Gate>>,
    all_loads_gate: Mutex<Option<Gate>>,
    mutation_gate: Mutex<Option<Gate>>,
    saves: AtomicUsize,
    deletes: AtomicUsize,
    fail: AtomicBool,
    changed: Semaphore,
}
impl FixtureStore {
    fn new() -> Arc<Self> {
        Arc::new(Self { value: Mutex::new(Some(session("user-a", "old"))),
            load_gate: Mutex::new(None), all_loads_gate: Mutex::new(None),
            mutation_gate: Mutex::new(None),
            saves: AtomicUsize::new(0), deletes: AtomicUsize::new(0),
            fail: AtomicBool::new(false), changed: Semaphore::new(0) })
    }
    fn pause_load(&self) -> Gate {
        let gate = Gate::new(); *self.load_gate.lock().unwrap() = Some(gate.clone()); gate
    }
    fn pause_mutation(&self) -> Gate {
        let gate = Gate::new(); *self.mutation_gate.lock().unwrap() = Some(gate.clone()); gate
    }
    async fn mutated(&self) { self.changed.acquire().await.unwrap().forget(); }
}
#[async_trait::async_trait]
impl SessionStore for FixtureStore {
    async fn load(&self, _: &FramerUserId) -> Result<Option<Session>, String> {
        let value = self.value.lock().unwrap().clone();
        let gate = self.all_loads_gate.lock().unwrap().clone()
            .or_else(|| self.load_gate.lock().unwrap().take());
        if let Some(gate) = gate { gate.wait().await; }
        Ok(value)
    }
    async fn save(&self, session: &Session) -> Result<(), String> {
        self.saves.fetch_add(1, Ordering::SeqCst);
        let gate = self.mutation_gate.lock().unwrap().take();
        if let Some(gate) = gate { gate.wait().await; }
        if self.fail.load(Ordering::SeqCst) { return Err("fixture save failed".to_owned()); }
        *self.value.lock().unwrap() = Some(session.clone());
        self.changed.add_permits(1);
        Ok(())
    }
    async fn delete(&self, _: &FramerUserId) -> Result<(), String> {
        self.deletes.fetch_add(1, Ordering::SeqCst);
        let gate = self.mutation_gate.lock().unwrap().take();
        if let Some(gate) = gate { gate.wait().await; }
        if self.fail.load(Ordering::SeqCst) { return Err("fixture delete failed".to_owned()); }
        *self.value.lock().unwrap() = None;
        self.changed.add_permits(1);
        Ok(())
    }
}

#[tokio::test]
async fn cancelled_save_waiter_preserves_auth_drain_and_owned_write() {
    let native = NativeAccountClient::new().unwrap();
    let scope = open(&native, "user-a");
    let (pending, release) = held(&native, &scope).await;
    let store = FixtureStore::new();
    let bridge = SessionStoreBridge::new(native, store.clone()).unwrap();
    let replacement = session("user-a", "new");
    assert!(bridge.save(&replacement).now_or_never().is_none());
    assert_eq!(store.saves.load(Ordering::SeqCst), 0);
    release.send(()).unwrap();
    assert_eq!(pending.await, Err(PreparationFailure::Cancelled));
    store.mutated().await;
    bridge.shutdown().await.unwrap();
    assert_eq!(store.value.lock().unwrap().as_ref().unwrap().cookie_header, replacement.cookie_header);
}

#[tokio::test]
async fn cancelled_delete_waiter_preserves_write_completion() {
    let native = NativeAccountClient::new().unwrap();
    let scope = open(&native, "user-a");
    let store = FixtureStore::new();
    let gate = store.pause_mutation();
    let bridge = SessionStoreBridge::new(native.clone(), store.clone()).unwrap();
    assert!(bridge.delete(scope.active_user()).now_or_never().is_none());
    gate.entered().await;
    assert!(matches!(native.begin_session_change(scope.active_user()), Err(ClientError::SessionChanging)));
    gate.finish();
    store.mutated().await;
    bridge.shutdown().await.unwrap();
    assert!(store.value.lock().unwrap().is_none());
}
#[tokio::test]
async fn failed_write_keeps_generation_closed_until_explicit_recovery() {
    let native = NativeAccountClient::new().unwrap();
    let scope = open(&native, "user-a");
    let store = FixtureStore::new();
    store.fail.store(true, Ordering::SeqCst);
    let bridge = SessionStoreBridge::new(native.clone(), store.clone()).unwrap();
    assert!(bridge.delete(scope.active_user()).await.is_err());
    assert!(bridge.load(scope.active_user()).await.is_err());
    assert!(bridge.open_account(scope.account(), scope.active_user().clone(),
        NativeHttpLimits::default(), NativeEgress::Direct).await.is_err());
    store.fail.store(false, Ordering::SeqCst);
    bridge.save(&session("user-a", "new")).await.unwrap();
    let next = bridge.open_account(scope.account(), scope.active_user().clone(),
        NativeHttpLimits::default(), NativeEgress::Direct).await.unwrap();
    assert!(!scope.same_generation(&next));
    native.close_account(&next).unwrap().await.unwrap();
    bridge.shutdown().await.unwrap();
}

#[tokio::test]
async fn stale_open_cannot_publish_a_session_read_before_replacement() {
    let native = NativeAccountClient::new().unwrap();
    let store = FixtureStore::new();
    let gate = store.pause_load();
    let bridge = SessionStoreBridge::new(native.clone(), store).unwrap();
    let opening = bridge.clone();
    let user = FramerUserId("user-a".to_owned());
    let task = tokio::spawn(async move {
        opening.open_account(AccountId(uuid::Uuid::new_v4()), user,
            NativeHttpLimits::default(), NativeEgress::Direct).await
    });
    gate.entered().await;
    bridge.save(&session("user-a", "new")).await.unwrap();
    gate.finish();
    assert!(task.await.unwrap().is_err());
    bridge.shutdown().await.unwrap();
}

#[tokio::test]
async fn stale_load_cannot_return_old_credentials_after_replacement() {
    let native = NativeAccountClient::new().unwrap();
    let store = FixtureStore::new();
    let gate = store.pause_load();
    let bridge = SessionStoreBridge::new(native, store).unwrap();
    let loading = bridge.clone();
    let task = tokio::spawn(async move { loading.load(&FramerUserId("user-a".to_owned())).await });
    gate.entered().await;
    bridge.save(&session("user-a", "new")).await.unwrap();
    gate.finish();
    assert!(task.await.unwrap().is_err());
    bridge.shutdown().await.unwrap();
}

#[tokio::test]
async fn reads_started_during_replacement_cannot_observe_old_credentials() {
    let native = NativeAccountClient::new().unwrap();
    let store = FixtureStore::new();
    let gate = store.pause_mutation();
    let bridge = SessionStoreBridge::new(native, store.clone()).unwrap();
    let user = FramerUserId("user-a".to_owned());
    assert!(bridge.save(&session("user-a", "new")).now_or_never().is_none());
    gate.entered().await;
    assert!(bridge.load(&user).await.is_err());
    assert!(bridge.open_account(AccountId(uuid::Uuid::new_v4()), user.clone(),
        NativeHttpLimits::default(), NativeEgress::Direct).await.is_err());
    gate.finish();
    store.mutated().await;
    assert_eq!(bridge.load(&user).await.unwrap().unwrap().cookie_header,
        "session=fixture-new");
    bridge.shutdown().await.unwrap();
}

#[tokio::test]
async fn abandoned_store_waiters_still_count_toward_the_shared_job_limit() {
    let native = NativeAccountClient::new().unwrap();
    let store = FixtureStore::new();
    let gate = Gate::new();
    *store.all_loads_gate.lock().unwrap() = Some(gate.clone());
    let bridge = SessionStoreBridge::new(native, store).unwrap();
    let user = FramerUserId("user-a".to_owned());
    for _ in 0..CAPACITY { assert!(bridge.load(&user).now_or_never().is_none()); }
    for _ in 0..CAPACITY { gate.entered().await; }
    assert_eq!(bridge.load(&user).await.unwrap_err(), "native session store busy");
    for _ in 0..CAPACITY { gate.finish(); }
    bridge.shutdown().await.unwrap();
}

#[tokio::test]
async fn unconfirmed_preparation_keeps_its_shared_reservation() {
    let client = NativeAccountClient::new().unwrap();
    let failed = open(&client, "user-a");
    let other = open(&client, "user-b");
    let pending = client.prepare_owned(&failed, Instant::now() + Duration::from_secs(30),
        |_, _| async { PreparationResult::<()>::unconfirmed() }).unwrap();
    assert_eq!(pending.await, Err(PreparationFailure::CleanupUnconfirmed));
    let mut releases = Vec::new();
    for _ in 0..(CAPACITY - 1) {
        let (pending, release) = held(&client, &other).await;
        drop(pending);
        releases.push(release);
    }
    assert!(matches!(client.prepare_owned(&other, Instant::now() + Duration::from_secs(30),
        |_, _| async { PreparationResult::finished(Ok(())) }), Err(ClientError::Full)));
    for release in releases { release.send(()).unwrap(); }
    client.close_account(&other).unwrap().await.unwrap();
}

#[tokio::test]
async fn supervisor_loss_reports_unconfirmed_preparation_cleanup() {
    let client = NativeAccountClient::new().unwrap();
    let scope = open(&client, "user-a");
    let (pending, release) = held(&client, &scope).await;
    let settled = pending.completion();
    client.supervisor_for_test().abort();
    assert_eq!(settled.await, Err(CloseError::SupervisorLost));
    assert_eq!(pending.await, Err(PreparationFailure::CleanupUnconfirmed));
    assert_eq!(scope.closed().await, Err(CloseError::SupervisorLost));
    let _ = release.send(());
}

#[tokio::test]
async fn closing_during_preparation_registration_still_returns_its_completion() {
    let client = NativeAccountClient::new().unwrap();
    let scope = open(&client, "user-a");
    let pending = client.prepare_at_registration_for_test(&scope,
        Instant::now() + Duration::from_secs(30),
        |_, _| async { PreparationResult::finished(Ok(())) },
        || { let _ = client.close_account(&scope).unwrap(); }).unwrap();
    pending.completion().await.unwrap();
    assert_eq!(pending.await, Err(PreparationFailure::Cancelled));
    scope.closed().await.unwrap();
}

// Proposal only, uncompiled and unrun. Append in a NEW integration revision
// to experimental_native/session_lifecycle_tests.rs, preserving all old bodies.
// Reuses that module's MultiStore, session(), imports and public bridge API.

async fn queued_open_survives_replacement_without_closing_new_scope(user_change: bool) {
    let native = NativeAccountClient::new().unwrap();
    let bridge = SessionStoreBridge::new(native.clone(), MultiStore::new()).unwrap();
    let account = AccountId(uuid::Uuid::new_v4());
    let user = FramerUserId("user-a".to_owned());
    let mut opening = Box::pin(bridge.open_account(account, user.clone(),
        NativeHttpLimits::default(), NativeEgress::Direct));
    assert!(futures::poll!(opening.as_mut()).is_pending());
    // Wait for the owned actor job to finish without polling its receiver.
    tokio::time::timeout(Duration::from_secs(2), async {
        while bridge.jobs_in_flight_for_test() != 0 { tokio::task::yield_now().await; }
    }).await.unwrap();
    // Prove the queued result actually opened a generation, rather than
    // accepting an unrelated queued error as a successful race setup.
    assert!(matches!(native.open_account(account, session("user-a", "old"),
        NativeHttpLimits::default(), NativeEgress::Direct), Err(ClientError::AlreadyOpen)));

    if user_change {
        bridge.save(&session("user-a", "new")).await.unwrap();
    } else {
        bridge.change_account(account, |_| async {
            ChangeOutcome::finished(Ok(()))
        }.boxed()).await.unwrap();
    }
    let next = bridge.open_account(account, user,
        NativeHttpLimits::default(), NativeEgress::Direct).await.unwrap();
    assert_eq!(opening.await.err().as_deref(), Some("native session changed during read"));
    // Dropping the stale queued Opened value calls close_account(old_scope).
    // That cleanup must not close the newer generation at the same account ID.
    assert!(next.closed().now_or_never().is_none());
    let observed = native.prepare_owned(&next, Instant::now() + Duration::from_secs(2),
        |current, _| async move { PreparationResult::finished(Ok(current.cookie_header)) })
        .unwrap().await.unwrap();
    assert_eq!(observed, if user_change { "session=fixture-new" } else { "session=fixture-old" });
    native.close_account(&next).unwrap().await.unwrap();
    bridge.shutdown().await.unwrap();
}

#[tokio::test]
async fn completed_open_queued_before_account_replacement_cannot_close_new_generation() {
    queued_open_survives_replacement_without_closing_new_scope(false).await;
}

#[tokio::test]
async fn completed_open_queued_before_user_replacement_cannot_close_new_generation() {
    queued_open_survives_replacement_without_closing_new_scope(true).await;
}

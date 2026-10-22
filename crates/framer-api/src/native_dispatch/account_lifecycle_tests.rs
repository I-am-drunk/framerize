use super::*;
use crate::experimental_native::native_account::NativeAccountClient;
use framer_core::account_lease::{LeaseError, LeaseLimits};
use framer_core::port::{Session, SessionStore};
use std::{collections::HashMap, sync::atomic::{AtomicBool, Ordering}};
use tokio::time::{timeout, Duration};

struct Store {
    values: Mutex<HashMap<String, Session>>,
    pause_save: AtomicBool,
    pause_load: AtomicBool,
    load_entered: Semaphore,
    load_release: Semaphore,
    entered: Semaphore,
    release: Semaphore,
}

#[async_trait::async_trait]
impl SessionStore for Store {
    async fn load(&self, user: &FramerUserId) -> Result<Option<Session>, String> {
        if self.pause_load.load(Ordering::SeqCst) {
            self.load_entered.add_permits(1);
            self.load_release.acquire().await.unwrap().forget();
        }
        Ok(lock(&self.values).get(&user.0).cloned())
    }
    async fn save(&self, value: &Session) -> Result<(), String> {
        if self.pause_save.load(Ordering::SeqCst) {
            self.entered.add_permits(1);
            self.release.acquire().await.unwrap().forget();
        }
        lock(&self.values).insert(value.active_user.0.clone(), value.clone());
        Ok(())
    }
    async fn delete(&self, user: &FramerUserId) -> Result<(), String> {
        lock(&self.values).remove(&user.0);
        Ok(())
    }
}

struct Fixture {
    native: NativeAccountClient,
    store: Arc<Store>,
    bridge: SessionStoreBridge,
    leases: AccountLeaseCoordinator,
    runtime: AccountLifecycle,
    invalidated: Arc<Mutex<Vec<AccountId>>>,
}

fn fixture(capacity: usize) -> Fixture {
    let native = NativeAccountClient::new().unwrap();
    let store = Arc::new(Store { values: Mutex::new(HashMap::new()),
        pause_save: AtomicBool::new(false), pause_load: AtomicBool::new(false),
        load_entered: Semaphore::new(0), load_release: Semaphore::new(0), entered: Semaphore::new(0),
        release: Semaphore::new(0) });
    let bridge = SessionStoreBridge::new(native.clone(), store.clone()).unwrap();
    let leases = AccountLeaseCoordinator::new(LeaseLimits { accounts: 8,
        users: 8, waiters: 16, user_id_bytes: 256, total_user_bytes: 2048 }).unwrap();
    let invalidated = Arc::new(Mutex::new(Vec::new()));
    let recorded = invalidated.clone();
    let runtime = AccountLifecycle::new(leases.clone(), bridge.clone(),
        Arc::new(move |account| { lock(&recorded).push(*account); Ok(()) }), capacity).unwrap();
    Fixture { native, store, bridge, leases, runtime, invalidated }
}

fn session(user: &str) -> Session {
    Session { active_user: FramerUserId(user.into()), cookie_header: "session=fixture".into() }
}

async fn bind(f: &Fixture, account: AccountId, user: &str) {
    let mut change = f.leases.begin_change(account).unwrap().await.unwrap();
    change.fence_user(user).unwrap().await.unwrap();
    change.stage_binding(account, Some(user)).unwrap();
    change.finish(ChangeResult::Completed).unwrap();
    lock(&f.store.values).insert(user.into(), session(user));
}

async fn stopped(f: &Fixture) {
    f.runtime.shutdown().await.unwrap();
    f.bridge.shutdown().await.unwrap();
}

async fn until(mut condition: impl FnMut() -> bool) {
    timeout(Duration::from_secs(2), async {
        while !condition() { tokio::task::yield_now().await; }
    }).await.expect("fixture did not reach barrier");
}

fn account() -> AccountId { AccountId(uuid::Uuid::new_v4()) }

fn assert_unavailable(leases: &AccountLeaseCoordinator, account: AccountId, user: &str) {
    if let Ok(waiting) = leases.operation(account, user) {
        assert!(!matches!(waiting.now_or_never(), Some(Ok(_))), "account granted while changing");
    }
}

async fn save_user(access: &mut Mutation<'_>, user: &'static str) -> MutationOutcome<()> {
    let result = async {
        let value = session(user);
        access.session(value.active_user.clone()).await?.save(&value).await?;
        access.stage_binding()
    }.await;
    match result {
        Ok(()) => MutationOutcome::Completed(()),
        Err(error) => MutationOutcome::Rejected(error),
    }
}

#[tokio::test]
async fn callback_waits_for_old_shared_operation() {
    let f = fixture(2);
    let a = account();
    bind(&f, a, "old").await;
    let operation = f.leases.operation(a, "old").unwrap().await.unwrap();
    let (entered, mut reached) = oneshot::channel();
    let mut changing = Box::pin(f.runtime.change(a, move |access| async move {
        let _ = entered.send(());
        save_user(access, "new").await
    }.boxed()));
    assert!(futures::poll!(changing.as_mut()).is_pending());
    until(|| f.leases.stats().unwrap().lifecycle_active).await;
    assert!(matches!(reached.try_recv(), Err(oneshot::error::TryRecvError::Empty)));
    operation.complete().unwrap();
    changing.await.unwrap();
    reached.await.unwrap();
    f.leases.operation(a, "new").unwrap().await.unwrap().complete().unwrap();
    assert!(lock(&f.invalidated).contains(&a));
    stopped(&f).await;
}

#[tokio::test]
async fn waiter_cancellation_keeps_private_save_and_shared_fence_owned() {
    let f = fixture(1);
    let a = account();
    bind(&f, a, "old").await;
    f.store.pause_save.store(true, Ordering::SeqCst);
    let runtime = f.runtime.clone();
    let waiter = tokio::spawn(async move {
        runtime.change(a, |access| save_user(access, "new").boxed()).await
    });
    timeout(Duration::from_secs(2), f.store.entered.acquire()).await.unwrap().unwrap().forget();
    waiter.abort();
    assert!(waiter.await.unwrap_err().is_cancelled());
    assert_unavailable(&f.leases, a, "old");
    assert_unavailable(&f.leases, a, "new");
    assert!(lock(&f.store.values).get("new").is_none());
    f.store.release.add_permits(1);
    until(|| !f.leases.stats().unwrap().lifecycle_active).await;
    assert!(lock(&f.store.values).get("new").is_some());
    f.leases.operation(a, "new").unwrap().await.unwrap().complete().unwrap();
    stopped(&f).await;
}

#[tokio::test]
async fn target_user_drains_before_save_while_unrelated_accounts_progress() {
    let f = fixture(2);
    let (a, b, c) = (account(), account(), account());
    bind(&f, a, "old").await;
    bind(&f, b, "target").await;
    bind(&f, c, "unrelated").await;
    let busy = f.leases.operation(b, "target").unwrap().await.unwrap();
    f.store.pause_save.store(true, Ordering::SeqCst);
    let (started, reached) = oneshot::channel();
    let mut changing = Box::pin(f.runtime.change(a, move |access| async move {
        let value = session("target");
        let mut binding = Box::pin(access.session(value.active_user.clone()));
        assert!(futures::poll!(binding.as_mut()).is_pending());
        let _ = started.send(());
        let saved = match binding.await {
            Ok(prepared) => prepared.save(&value).await,
            Err(error) => Err(error),
        };
        match saved.and_then(|_| access.stage_binding()) {
            Ok(()) => MutationOutcome::Completed(()),
            Err(error) => MutationOutcome::Rejected(error),
        }
    }.boxed()));
    assert!(futures::poll!(changing.as_mut()).is_pending());
    reached.await.unwrap();
    assert_eq!(f.store.entered.available_permits(), 0);
    assert_unavailable(&f.leases, a, "old");
    f.leases.operation(c, "unrelated").unwrap().await.unwrap().complete().unwrap();
    busy.complete().unwrap();
    timeout(Duration::from_secs(2), f.store.entered.acquire()).await.unwrap().unwrap().forget();
    assert_unavailable(&f.leases, b, "target");
    f.store.release.add_permits(1);
    changing.await.unwrap();
    f.leases.operation(a, "target").unwrap().await.unwrap().complete().unwrap();
    f.leases.operation(b, "target").unwrap().await.unwrap().complete().unwrap();
    assert!(lock(&f.invalidated).contains(&b));
    stopped(&f).await;
}

#[tokio::test]
async fn unconfirmed_mutation_poisons_only_its_account() {
    let f = fixture(2);
    let (a, b) = (account(), account());
    bind(&f, a, "old").await;
    bind(&f, b, "other").await;
    let result = f.runtime.change(a, |_| async { MutationOutcome::<()>::Unconfirmed }.boxed()).await;
    assert!(result.is_err());
    let error = match f.leases.operation(a, "old") {
        Err(error) => error,
        Ok(waiting) => waiting.await.err().expect("poisoned account granted"),
    };
    assert_eq!(error, LeaseError::Poisoned);
    f.leases.operation(b, "other").unwrap().await.unwrap().complete().unwrap();
    let mut ticket = f.native.begin_account_change(a).unwrap();
    assert!(ticket.ready().await.is_err());
    stopped(&f).await;
}

#[tokio::test]
async fn incomplete_publication_keeps_account_closed() {
    let f = fixture(1);
    let a = account();
    bind(&f, a, "old").await;
    let result = f.runtime.change(a, |_| async { MutationOutcome::Completed(()) }.boxed()).await;
    assert!(result.is_err());
    assert_unavailable(&f.leases, a, "old");
    assert_eq!(f.leases.stats().unwrap().poisoned_accounts, 0);
    f.runtime.change(a, |access| save_user(access, "new").boxed()).await.unwrap();
    f.leases.operation(a, "new").unwrap().await.unwrap().complete().unwrap();
    stopped(&f).await;
}

#[tokio::test]
async fn capacity_and_shutdown_do_not_abandon_an_admitted_save() {
    let f = fixture(1);
    let (a, b) = (account(), account());
    bind(&f, a, "old").await;
    bind(&f, b, "other").await;
    f.store.pause_save.store(true, Ordering::SeqCst);
    let runtime = f.runtime.clone();
    let first = tokio::spawn(async move {
        runtime.change(a, |access| save_user(access, "new").boxed()).await
    });
    timeout(Duration::from_secs(2), f.store.entered.acquire()).await.unwrap().unwrap().forget();
    let before = f.leases.stats().unwrap();
    assert!(f.runtime.change(b, |_| async { panic!("capacity was bypassed");
        #[allow(unreachable_code)] MutationOutcome::Completed(()) }.boxed()).await.is_err());
    assert_eq!(before, f.leases.stats().unwrap());
    let mut closing = Box::pin(f.runtime.shutdown());
    assert!(futures::poll!(closing.as_mut()).is_pending());
    f.store.release.add_permits(1);
    first.await.unwrap().unwrap();
    closing.await.unwrap();
    assert!(f.runtime.change(b, |_| async { MutationOutcome::Completed(()) }.boxed()).await.is_err());
    f.bridge.shutdown().await.unwrap();
}

#[tokio::test]
async fn swallowed_binding_error_cannot_reopen_account() {
    let f = fixture(1);
    let a = account();
    bind(&f, a, "old").await;
    let result = f.runtime.change(a, |access| async move {
        assert!(access.session(FramerUserId(String::new())).await.is_err());
        access.stage_removal().unwrap();
        MutationOutcome::Completed(())
    }.boxed()).await;
    assert!(result.is_err());
    assert_unavailable(&f.leases, a, "old");
    assert_eq!(f.leases.stats().unwrap().accounts, 1);
    stopped(&f).await;
}

#[tokio::test]
async fn unpolled_change_has_no_admission_or_side_effect() {
    let f = fixture(1);
    let a = account();
    let before = f.leases.stats().unwrap();
    let never_polled = f.runtime.change(a, |access| save_user(access, "new").boxed());
    drop(never_polled);
    assert_eq!(f.leases.stats().unwrap(), before);
    assert!(lock(&f.store.values).is_empty());
    assert!(lock(&f.invalidated).is_empty());
    stopped(&f).await;
}

#[tokio::test]
async fn closed_bridge_preserves_existing_binding_and_queued_operation() {
    let f = fixture(1);
    let a = account();
    bind(&f, a, "old").await;
    let waiting = f.leases.operation(a, "old").unwrap();
    f.bridge.shutdown().await.unwrap();
    let before = f.leases.stats().unwrap();
    assert!(f.runtime.change(a, |_| async {
        panic!("closed bridge invoked callback");
        #[allow(unreachable_code)] MutationOutcome::Completed(())
    }.boxed()).await.is_err());
    assert_eq!(f.leases.stats().unwrap(), before);
    assert!(lock(&f.invalidated).is_empty());
    waiting.await.unwrap().complete().unwrap();
    stopped(&f).await;
}

#[tokio::test]
async fn unstarted_change_does_not_reopen_previously_closed_account() {
    let f = fixture(1);
    let a = account();
    bind(&f, a, "old").await;
    assert!(f.runtime.change(a, |_| async {
        MutationOutcome::<()>::Rejected("reconcile later".into())
    }.boxed()).await.is_err());
    f.bridge.shutdown().await.unwrap();
    let before = f.leases.stats().unwrap();
    assert_eq!(before.closed_accounts, 1);
    assert!(f.runtime.change(a, |_| async { MutationOutcome::Completed(()) }.boxed()).await.is_err());
    assert_eq!(before, f.leases.stats().unwrap());
    assert_unavailable(&f.leases, a, "old");
    stopped(&f).await;
}

#[tokio::test]
async fn full_bridge_rejects_without_poisoning_or_invalidating_binding() {
    let f = fixture(1);
    let a = account();
    bind(&f, a, "old").await;
    f.store.pause_save.store(true, Ordering::SeqCst);
    let mut jobs = Vec::new();
    for index in 0..10 {
        let bridge = f.bridge.clone();
        jobs.push(tokio::spawn(async move {
            bridge.save(&session(&format!("busy-{index}"))).await
        }));
    }
    timeout(Duration::from_secs(2), f.store.entered.acquire_many(10))
        .await.unwrap().unwrap().forget();
    assert_eq!(f.bridge.jobs_in_flight_for_test(), 10);
    let waiting = f.leases.operation(a, "old").unwrap();
    let before = f.leases.stats().unwrap();
    assert!(f.runtime.change(a, |_| async {
        panic!("full bridge invoked callback");
        #[allow(unreachable_code)] MutationOutcome::Completed(())
    }.boxed()).await.is_err());
    assert_eq!(before, f.leases.stats().unwrap());
    assert!(lock(&f.invalidated).is_empty());
    waiting.await.unwrap().complete().unwrap();
    f.store.release.add_permits(10);
    for job in jobs { job.await.unwrap().unwrap(); }
    stopped(&f).await;
}

#[test]
fn constructor_without_current_runtime_returns_error() {
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    let f = runtime.block_on(async { fixture(1) });
    assert!(AccountLifecycle::new(f.leases.clone(), f.bridge.clone(),
        Arc::new(|_| Ok(())), 1).is_err());
    runtime.block_on(stopped(&f));
}

#[tokio::test]
async fn dropped_private_load_keeps_shared_and_native_barriers_poisoned() {
    let f = fixture(1);
    let (a, b) = (account(), account());
    bind(&f, a, "old").await;
    bind(&f, b, "other").await;
    f.store.pause_load.store(true, Ordering::SeqCst);
    let store = f.store.clone();
    let result = f.runtime.change(a, move |access| async move {
        let mut preparation = Box::pin(access.session(FramerUserId("target".into())));
        tokio::select! {
            entered = store.load_entered.acquire() => entered.unwrap().forget(),
            _ = &mut preparation => panic!("private load unexpectedly completed"),
        }
        drop(preparation);
        MutationOutcome::<()>::Rejected("caller discarded preparation".into())
    }.boxed()).await;
    assert!(result.is_err());
    assert_eq!(f.leases.stats().unwrap().poisoned_accounts, 1);
    assert_eq!(f.leases.stats().unwrap().poisoned_users, 1);
    let ticket = f.native.begin_account_change(a).unwrap();
    assert!(ticket.ready().await.is_err());
    f.leases.operation(b, "other").unwrap().await.unwrap().complete().unwrap();
    stopped(&f).await;
}

#[tokio::test]
async fn callback_panic_does_not_abort_unrelated_private_save() {
    let f = fixture(1);
    let a = account();
    bind(&f, a, "old").await;
    f.store.pause_save.store(true, Ordering::SeqCst);
    let bridge = f.bridge.clone();
    let saved = tokio::spawn(async move { bridge.save(&session("other")).await });
    timeout(Duration::from_secs(2), f.store.entered.acquire()).await.unwrap().unwrap().forget();
    assert!(f.runtime.change(a, |_| async {
        panic!("fixture callback panic");
        #[allow(unreachable_code)] MutationOutcome::Completed(())
    }.boxed()).await.is_err());
    assert_eq!(f.leases.stats().unwrap().poisoned_accounts, 1);
    f.store.release.add_permits(1);
    saved.await.unwrap().unwrap();
    assert!(f.bridge.load(&FramerUserId("other".into())).await.unwrap().is_some());
    stopped(&f).await;
}

#[tokio::test]
async fn queued_retry_observes_native_claims_released_after_rejection() {
    let f = fixture(2);
    let a = account();
    bind(&f, a, "old").await;
    let pause = Arc::new(Semaphore::new(0));
    let release = pause.clone();
    let (entered, reached) = oneshot::channel();
    let runtime = f.runtime.clone();
    let first = tokio::spawn(async move {
        runtime.change(a, move |_| async move {
            let _ = entered.send(());
            pause.acquire().await.unwrap().forget();
            MutationOutcome::<()>::Rejected("known clean rejection".into())
        }.boxed()).await
    });
    reached.await.unwrap();
    let runtime = f.runtime.clone();
    let retry = tokio::spawn(async move {
        runtime.change(a, |access| save_user(access, "new").boxed()).await
    });
    until(|| f.leases.stats().unwrap().waiters == 1).await;
    release.add_permits(1);
    assert!(first.await.unwrap().is_err());
    retry.await.unwrap().unwrap();
    f.leases.operation(a, "new").unwrap().await.unwrap().complete().unwrap();
    stopped(&f).await;
}

#[tokio::test]
async fn invalidator_panic_is_contained_by_native_job() {
    let f = fixture(1);
    let a = account();
    bind(&f, a, "old").await;
    let lifecycle = AccountLifecycle::new(f.leases.clone(), f.bridge.clone(),
        Arc::new(|_| panic!("fixture invalidation panic")), 1).unwrap();
    assert!(lifecycle.change(a, |_| async {
        MutationOutcome::Completed(())
    }.boxed()).await.is_err());
    assert_eq!(f.leases.stats().unwrap().poisoned_accounts, 1);
    f.bridge.save(&session("other")).await.unwrap();
    lifecycle.shutdown().await.unwrap();
    stopped(&f).await;
}

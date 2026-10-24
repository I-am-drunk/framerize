//! Inert ownership checks. The sidecar entry must never be launched.
use super::*;
use framer_api::experimental_native::{native_account::NativeAccountClient,
    native_http::{NativeEgress, NativeHttpLimits}};
use framer_core::{ids::{AccountId, FramerUserId}, port::Session};
use framer_host::sidecar::{SidecarBridge, observer::{ApplicationGeneration,
    CleanupExtent, CleanupReceipt, ObservedAccountScope, ObserverError, ObserverLimits}};
use std::{sync::{Arc, atomic::{AtomicUsize, Ordering}}, time::Duration};
use tokio::sync::{oneshot, Semaphore};

const BOUND: Duration = Duration::from_secs(3);

#[tokio::test]
async fn closed_receipt_without_matching_scope_completion_is_unconfirmed() {
    let account = AccountId::new();
    let (_root, opened) = unopened(account, 7);
    let receipt = CleanupReceipt {
        account, application_generation: ApplicationGeneration(7),
        sidecar_incarnation: uuid::Uuid::new_v4(), scope: opened.control.scope_id(),
        extent: CleanupExtent::AccountBrowserClosed,
    };
    assert_eq!(Verdict::observed(&opened.control, CleanupOutcome::Closed(receipt)),
        Verdict::Unconfirmed);
    assert_eq!(Verdict::observed(&opened.control,
        CleanupOutcome::Unconfirmed(ObserverError::Transport)), Verdict::Unconfirmed);
    opened.control.request_stop();
    assert!(matches!(tokio::time::timeout(BOUND, opened.cleanup.wait()).await.unwrap(),
        CleanupOutcome::NotStarted { .. }));
}

struct Temp(std::path::PathBuf);
impl Drop for Temp {
    fn drop(&mut self) { std::fs::remove_dir_all(&self.0).unwrap(); }
}

fn unopened(account: AccountId, generation: u64) -> (Temp, ObservedAccountScope) {
    let root = std::env::temp_dir().join(format!("comparison-no-launch-{}", uuid::Uuid::new_v4()));
    let mut directory = std::fs::DirBuilder::new();
    #[cfg(unix)] {
        use std::os::unix::fs::DirBuilderExt;
        directory.mode(0o700);
    }
    directory.create(&root).unwrap();
    // The constructor requires this sibling even though no process may start.
    let script = root.join("observed_sidecar.py");
    std::fs::write(&script, "raise AssertionError('must not start')\n").unwrap();
    let bridge = Arc::new(SidecarBridge::new_observed_one_shot(&root, script).unwrap());
    let factory = bridge.observer_factory(ObserverLimits {
        scopes: 1, commands: 2, cleanup_timeout: BOUND,
    }).unwrap();
    let opened = factory.one_shot_scope(account, ApplicationGeneration(generation)).unwrap();
    (Temp(root), opened)
}

#[tokio::test]
async fn not_started_remains_distinct_and_reuses_cached_settlement() {
    let (_root, opened) = unopened(AccountId::new(), 7);
    let completion = Completion::new(opened.control, opened.cleanup);
    let finalizer = completion.clone();
    let failed = tokio::time::timeout(BOUND, completion.failed_prepare::<()>()).await.unwrap();
    assert_eq!(failed.value, Err(PreparationFailure::Rejected));
    assert!(failed.cleanup_confirmed);
    assert_eq!(completion.verdict.clone().now_or_never(), Some(Verdict::NotStarted));
    assert!(tokio::time::timeout(BOUND, finalizer.finish(true)).await.unwrap());
    assert_eq!(finalizer.verdict.clone().now_or_never(), Some(Verdict::NotStarted));
}

#[tokio::test]
async fn dropped_native_prepare_waiter_keeps_shared_completion_owned() {
    let native = NativeAccountClient::new().unwrap();
    let scope = native.open_account(AccountId::new(), Session {
        active_user: FramerUserId("fixture-user".into()),
        cookie_header: "session=fixture-secret".into(),
    }, NativeHttpLimits::default(), NativeEgress::Direct).unwrap();
    let (_root, opened) = unopened(scope.account(), scope.generation());
    let Completion { control, verdict } = Completion::new(opened.control, opened.cleanup);
    let (release, gate) = oneshot::channel::<()>();
    let entered = Arc::new(Semaphore::new(0));
    let calls = Arc::new(AtomicUsize::new(0));
    let observed = calls.clone();
    let started = entered.clone();
    // Hold delivery of the sole real observer's outcome. This does not replace
    // the observer, fabricate a receipt, or start a sidecar.
    let completion = Completion { control, verdict: async move {
        observed.fetch_add(1, Ordering::SeqCst);
        started.add_permits(1);
        gate.await.unwrap();
        verdict.await
    }.boxed().shared() };
    let finalizer = completion.clone();
    let pending = native.prepare_owned(&scope, tokio::time::Instant::now() + BOUND,
        move |_, _| async move { completion.failed_prepare::<()>().await }).unwrap();
    tokio::time::timeout(BOUND, entered.acquire()).await.unwrap().unwrap().forget();
    drop(pending);
    let mut closed = native.close_account(&scope).unwrap();
    assert!((&mut closed).now_or_never().is_none());
    assert!(scope.closed().now_or_never().is_none());
    release.send(()).unwrap();
    tokio::time::timeout(BOUND, closed).await.unwrap().unwrap();
    assert!(tokio::time::timeout(BOUND, finalizer.finish(true)).await.unwrap());
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    native.shutdown_idle().await.unwrap();
}

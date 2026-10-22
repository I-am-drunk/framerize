//! Offline real-pipe integration. The browser and account work remain explicit fakes.
use super::*;
use futures::FutureExt;
use std::future::Future;
use std::panic::AssertUnwindSafe;

async fn exercise<F, Fut>(cleanup_timeout: Duration, body: F)
where F: FnOnce(Arc<SidecarBridge>, ObserverFactory, PathBuf) -> Fut,
      Fut: Future<Output = ()> {
    let root = std::env::temp_dir().join(format!("observer-pipe-fixture-{}", Uuid::new_v4()));
    let mut builder = std::fs::DirBuilder::new();
    #[cfg(unix)] {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(&root).unwrap();
    let script = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../scripts/observer_pipe_fixture.py");
    let bridge = Arc::new(SidecarBridge::new_observed_one_shot(&root, script).unwrap());
    let factory = bridge.observer_factory(ObserverLimits {
        scopes: 3, commands: 4, cleanup_timeout,
    }).unwrap();
    let trace = bridge.state_root.clone();
    let result = AssertUnwindSafe(tokio::time::timeout(Duration::from_secs(8),
        body(bridge.clone(), factory, trace))).catch_unwind().await;
    let cleanup = terminate_fixture(&bridge).await;
    if cleanup.is_ok() { std::fs::remove_dir_all(&root).unwrap(); }
    assert!(cleanup.is_ok(), "fixture child teardown failed: {cleanup:?}");
    match result {
        Ok(value) => value.expect("pipe fixture exceeded its deadline"),
        Err(panic) => std::panic::resume_unwind(panic),
    }
}

async fn terminate_fixture(bridge: &SidecarBridge) -> Result<(), String> {
    // This is test teardown after assertions, never evidence of clean observation.
    let mut guard = tokio::time::timeout(Duration::from_secs(1), bridge.inner.lock())
        .await.map_err(|_| "fixture lifecycle lock remained held")?;
    let Some(mut worker) = guard.take() else { return Ok(()); };
    drop(guard);
    if worker.child.try_wait().map_err(|e| e.to_string())?.is_none() {
        worker.child.start_kill().map_err(|e| e.to_string())?;
    }
    tokio::time::timeout(Duration::from_secs(2), worker.child.wait())
        .await.map_err(|_| "fixture child did not exit")?
        .map_err(|e| e.to_string())?;
    drop(worker);
    tokio::task::yield_now().await;
    Ok(())
}

async fn event(trace: &Path, name: &str, account: AccountId) {
    let path = trace.join(format!("{name}.{}", account.0));
    tokio::time::timeout(Duration::from_secs(2), async {
        while !path.is_file() { tokio::time::sleep(Duration::from_millis(5)).await; }
    }).await.expect("expected pipe fixture event");
}

fn release(trace: &Path, account: AccountId) {
    std::fs::write(trace.join(format!("release.{}", account.0)), b"continue").unwrap();
}

type Reply = Result<(u16, Option<String>, String), String>;
fn request(scope: &ObservedAccountScope, account: AccountId, mode: &str,
    reply: String) -> tokio::task::JoinHandle<Reply> {
    let browser = scope.browser.clone();
    let body = json!({"fixture_mode": mode, "reply": reply,
        "status": 201, "usage": "fixture-usage", "queue_wait_ms": 17});
    tokio::spawn(async move {
        browser.ai_post(&account, "/ai/v3/responses/", "fixture-only", &body).await
    })
}

fn scope(factory: &ObserverFactory, account: AccountId, generation: u64)
    -> ObservedAccountScope {
    factory.one_shot_scope(account, ApplicationGeneration(generation)).unwrap()
}

async fn outcome(cleanup: CleanupObserver) -> CleanupOutcome {
    tokio::time::timeout(Duration::from_secs(2), cleanup.wait())
        .await.expect("observer did not publish bounded outcome")
}

#[tokio::test]
async fn pipe_roundtrip_preserves_large_payload_usage_and_identity() {
    exercise(Duration::from_secs(2), |bridge, factory, trace| async move {
        let account = AccountId(Uuid::new_v4());
        let owned = scope(&factory, account, 71);
        let expected = "quoted \"body\"\nUnicode λ🙂\\".repeat(6000);
        let reply = request(&owned, account, "echo", expected.clone()).await.unwrap().unwrap();
        assert_eq!(reply, (201, Some("fixture-usage".into()), expected));
        assert_eq!(owned.browser.last_queue_wait_ms(&account).await, 17);
        assert!(owned.control.state.lock_phase().outcome.is_none());
        assert!(matches!(factory.one_shot_scope(account, ApplicationGeneration(72)),
            Err(ObserverError::AccountBusy)));
        let control = owned.control.clone();
        let incarnation = bridge.inner.lock().await.as_ref().unwrap().incarnation;
        control.seal_for_account_teardown();
        let CleanupOutcome::Closed(receipt) = outcome(owned.cleanup).await else {
            panic!("valid pipe cleanup was not confirmed");
        };
        assert_eq!(receipt.account, account);
        assert_eq!(receipt.application_generation, ApplicationGeneration(71));
        assert_eq!(receipt.sidecar_incarnation, incarnation);
        assert_eq!(receipt.scope, control.scope_id());
        assert!(control.matches_closed_receipt(&receipt));
        event(&trace, "forgotten", account).await;
        let next = scope(&factory, account, 72);
        next.control.seal_for_account_teardown();
        assert_eq!(outcome(next.cleanup).await, CleanupOutcome::NotStarted { reason: None });
    }).await;
}

async fn stopped_command(cancel_caller: bool) {
    exercise(Duration::from_secs(2), move |_, factory, trace| async move {
        let account = AccountId(Uuid::new_v4());
        let owned = scope(&factory, account, 81);
        let call = request(&owned, account, "wait", "joined response".into());
        event(&trace, "command_started", account).await;
        let call = if cancel_caller {
            call.abort();
            assert!(call.await.unwrap_err().is_cancelled());
            None
        } else {
            owned.control.request_stop();
            Some(call)
        };
        event(&trace, "stop_seen", account).await;
        let mut cleanup = Box::pin(owned.cleanup.wait());
        assert!(tokio::time::timeout(Duration::from_millis(40), cleanup.as_mut()).await.is_err());
        assert_eq!(owned.control.state.lock_phase().active, 1);
        assert!(!trace.join(format!("browser_close.{}", account.0)).exists());
        assert!(matches!(factory.one_shot_scope(account, ApplicationGeneration(82)),
            Err(ObserverError::AccountBusy)));
        release(&trace, account);
        if let Some(call) = call { assert_eq!(call.await.unwrap().unwrap().2, "joined response"); }
        assert!(matches!(cleanup.await, CleanupOutcome::Closed(_)));
        assert_eq!(owned.control.state.lock_phase().active, 0);
        event(&trace, "forgotten", account).await;
    }).await;
}

#[tokio::test]
async fn pipe_stop_waits_for_command_reply_and_cleanup() {
    stopped_command(false).await;
}

#[tokio::test]
async fn pipe_cancelled_caller_keeps_remote_work_owned() {
    stopped_command(true).await;
}

#[tokio::test]
async fn pipe_deadline_uncertainty_survives_late_remote_cleanup() {
    exercise(Duration::from_millis(150), |_, factory, trace| async move {
        let account = AccountId(Uuid::new_v4());
        let owned = scope(&factory, account, 91);
        let call = request(&owned, account, "wait", "late reply".into());
        event(&trace, "command_started", account).await;
        owned.control.request_stop();
        assert_eq!(outcome(owned.cleanup).await,
            CleanupOutcome::Unconfirmed(ObserverError::CleanupTimeout));
        release(&trace, account);
        assert_eq!(call.await.unwrap().unwrap().2, "late reply");
        // Late remote teardown cannot upgrade the host's bounded-wait failure.
        // Completing the command invalidates the uncertain connection before
        // finish_scope can pass quiet() and request scope_forget.
        event(&trace, "browser_close", account).await;
        event(&trace, "cleanup_sent", account).await;
        {
            let phase = owned.control.state.lock_phase();
            assert_eq!(phase.active, 0);
            assert!(phase.sealed && phase.stop);
            assert!(!phase.connection.as_ref().unwrap().alive());
        }
        assert!(!trace.join(format!("forgotten.{}", account.0)).exists());
        assert_eq!(owned.control.state.lock_phase().outcome,
            Some(CleanupOutcome::Unconfirmed(ObserverError::CleanupTimeout)));
        assert!(matches!(factory.one_shot_scope(account, ApplicationGeneration(92)),
            Err(ObserverError::AccountUnconfirmed)));
    }).await;
}

#[tokio::test]
async fn pipe_process_death_does_not_respawn_or_reopen_the_old_account() {
    exercise(Duration::from_secs(2), |bridge, factory, _| async move {
        let account = AccountId(Uuid::new_v4());
        let owned = scope(&factory, account, 101);
        assert!(request(&owned, account, "die", "unused".into()).await.unwrap().is_err());
        assert!(matches!(outcome(owned.cleanup).await, CleanupOutcome::Unconfirmed(_)));
        let incarnation = {
            let mut guard = bridge.inner.lock().await;
            let worker = guard.as_mut().unwrap();
            let status = tokio::time::timeout(Duration::from_secs(1), worker.child.wait())
                .await.unwrap().unwrap();
            assert_eq!(status.code(), Some(73));
            worker.incarnation
        };
        assert!(matches!(factory.one_shot_scope(account, ApplicationGeneration(102)),
            Err(ObserverError::AccountUnconfirmed)));
        let other_account = AccountId(Uuid::new_v4());
        let other = scope(&factory, other_account, 1);
        assert!(request(&other, other_account, "echo", "unused".into()).await.unwrap().is_err());
        assert!(matches!(outcome(other.cleanup).await, CleanupOutcome::NotStarted { .. }));
        assert_eq!(bridge.inner.lock().await.as_ref().unwrap().incarnation, incarnation);
    }).await;
}

#[tokio::test]
async fn pipe_wrong_generation_or_incarnation_cannot_confirm_cleanup() {
    for mode in ["wrong_generation", "wrong_incarnation"] {
        exercise(Duration::from_secs(2), move |_, factory, _| async move {
            let account = AccountId(Uuid::new_v4());
            let owned = scope(&factory, account, 111);
            assert!(request(&owned, account, mode, "reply".into()).await.unwrap().is_ok());
            owned.control.seal_for_account_teardown();
            assert!(matches!(outcome(owned.cleanup).await, CleanupOutcome::Unconfirmed(_)));
            assert!(matches!(factory.one_shot_scope(account, ApplicationGeneration(112)),
                Err(ObserverError::AccountUnconfirmed)));
        }).await;
    }
}

#[tokio::test]
async fn pipe_stopping_one_account_preserves_another_accounts_reply() {
    exercise(Duration::from_secs(2), |_, factory, trace| async move {
        let slow_id = AccountId(Uuid::new_v4());
        let fast_id = AccountId(Uuid::new_v4());
        let slow = scope(&factory, slow_id, 121);
        let fast = scope(&factory, fast_id, 122);
        let pending = request(&slow, slow_id, "wait", "slow".into());
        event(&trace, "command_started", slow_id).await;
        slow.control.request_stop();
        event(&trace, "stop_seen", slow_id).await;
        let reply = request(&fast, fast_id, "echo", "fast independent reply".into())
            .await.unwrap().unwrap();
        assert_eq!(reply.2, "fast independent reply");
        fast.control.seal_for_account_teardown();
        let CleanupOutcome::Closed(fast_receipt) = outcome(fast.cleanup).await else {
            panic!("unrelated account cleanup was not confirmed");
        };
        assert_eq!(fast_receipt.account, fast_id);
        assert_eq!(slow.control.state.lock_phase().active, 1);
        let mut cleanup = Box::pin(slow.cleanup.wait());
        assert!(tokio::time::timeout(Duration::from_millis(40), cleanup.as_mut()).await.is_err());
        release(&trace, slow_id);
        assert_eq!(pending.await.unwrap().unwrap().2, "slow");
        let CleanupOutcome::Closed(slow_receipt) = cleanup.await else {
            panic!("joined slow account cleanup was not confirmed");
        };
        assert_eq!(slow_receipt.account, slow_id);
        assert_eq!(slow_receipt.sidecar_incarnation, fast_receipt.sidecar_incarnation);
        assert_ne!(slow_receipt.scope, fast_receipt.scope);
    }).await;
}

use super::*;

#[tokio::test]
async fn proxy_registration_never_respawns_an_exited_host() {
    let (bridge, mut frames, reader) = echo_bridge(None).await;
    {
        let mut guard = bridge.inner.lock().await;
        let child = &mut guard.as_mut().unwrap().child;
        child.kill().await.unwrap();
        child.wait().await.unwrap();
    }
    tokio::time::timeout(std::time::Duration::from_secs(2), reader).await.unwrap().unwrap();
    let account = AccountId::new();
    let error = bridge.register_account_proxy(&account, Some("http://saved.invalid:1234".into()))
        .await.unwrap_err();
    assert!(error.starts_with("host-lifecycle-unconfirmed:"));
    assert!(bridge.inner.lock().await.is_some());
    let cell = bridge.route_state(&account).await;
    let state = cell.lock().await;
    assert!(state.current.proxy.is_none());
    assert!(state.pending.is_some());
    assert!(frames.try_recv().is_err());
}

#[tokio::test]
async fn invalid_proxy_registration_does_not_create_route_or_host_state() {
    let account = AccountId::new();
    let root = std::env::temp_dir().join(format!("uncreated-empty-route-{}", account.0));
    let bridge = SidecarBridge::new(&root);
    assert!(bridge.register_account_proxy(&account, Some(String::new())).await.is_err());
    assert!(bridge.routes.lock().await.is_empty());
    assert!(bridge.inner.lock().await.is_none());
    assert!(!root.exists());
}

#[tokio::test]
async fn interrupted_registration_reconciles_before_applying_a_different_route() {
    let (bridge, mut frames, reader) = echo_bridge_mode(None, "first_silent").await;
    let account = AccountId::new();
    let mut pending = Box::pin(bridge.register_account_proxy(&account, Some("http://first.invalid:1".into())));
    tokio::select! {
        value = &mut pending => panic!("registration unexpectedly finished: {value:?}"),
        frame = frames.recv() => assert_eq!(frame.unwrap()["revision"], 1),
        _ = tokio::time::sleep(std::time::Duration::from_secs(2)) => panic!("no registration frame"),
    }
    drop(pending);
    bridge.register_account_proxy(&account, Some("http://second.invalid:2".into())).await.unwrap();
    assert_eq!(frames.recv().await.unwrap()["revision"], 1);
    assert_eq!(frames.recv().await.unwrap()["revision"], 2);
    let route = bridge.route_snapshot(&account).await.unwrap();
    assert_eq!(route.proxy.as_deref(), Some("http://second.invalid:2"));
    assert_eq!(route.revision, 2);
    finish(bridge, reader).await;
}

pub(super) async fn echo_bridge(close_error: Option<&'static str>) -> (SidecarBridge, mpsc::UnboundedReceiver<Value>, tokio::task::JoinHandle<()>) {
    echo_bridge_mode(close_error, "registered").await
}

async fn echo_bridge_mode(close_error: Option<&'static str>, mode: &'static str)
    -> (SidecarBridge, mpsc::UnboundedReceiver<Value>, tokio::task::JoinHandle<()>) {
    let mut child = Command::new("python3").args(["-u", "-c",
        "import sys\nfor line in sys.stdin:\n sys.stdout.write(line)\n sys.stdout.flush()"])
        .stdin(Stdio::piped()).stdout(Stdio::piped()).kill_on_drop(true).spawn().unwrap();
    let stdin = Arc::new(Mutex::new(child.stdin.take().unwrap()));
    let stdout = child.stdout.take().unwrap();
    let pending: PendingReplies = Arc::new(StdMutex::new(HashMap::new()));
    let observed = pending.clone();
    let (capture, frames) = mpsc::unbounded_channel();
    let reader = tokio::spawn(async move {
        let mut lines = BufReader::new(stdout).lines();
        let mut registrations = 0;
        while let Ok(Some(line)) = lines.next_line().await {
            let value: Value = serde_json::from_str(&line).unwrap();
            let id = value["id"].as_u64().unwrap();
            let result = match value["cmd"].as_str().unwrap() {
                "close" if close_error.is_some() => Err(close_error.unwrap().to_owned()),
                "register_proxy" if mode == "busy" => Ok(json!({"outcome":"busy",
                    "route":{"revision":value["expected_revision"],"proxy":null},
                    "status":{"account_id":value["account"],"host_generation":"fixture-host",
                        "account_incarnation":1,"state":"resident","active_commands":0}})),
                "register_proxy" if mode == "malformed" => Ok(json!(true)),
                "register_proxy" => Ok(json!({"outcome":"registered",
                    "route":{"revision":value["revision"],"proxy":
                        if mode == "wrong_route" { Value::Null } else { value["proxy"].clone() }},
                    "status":{"account_id":if mode == "wrong_account" {
                            json!(uuid::Uuid::nil()) } else { value["account"].clone() },
                        "host_generation":if mode == "no_host" { "" } else { "fixture-host" },
                        "account_incarnation":1,
                        "state":if mode == "live_ack" { "resident" } else { "absent" },
                        "active_commands":if mode == "active_ack" { 1 } else { 0 }}})),
                "mint" => Ok(json!("fixture-token")),
                "complete" => Ok(json!({"cookie_header":"session=fixture-only","me":{}})),
                "ai_post" => Ok(json!({"status":200,"body":"{}"})),
                _ => Ok(json!(true)),
            };
            let registration = value["cmd"] == "register_proxy";
            let hold_ai = value["cmd"] == "ai_post" && mode == "hold_ai";
            if capture.send(value).is_err() { break; }
            if hold_ai { continue; }
            if registration {
                registrations += 1;
                if mode == "first_silent" && registrations == 1 { continue; }
            }
            if let Some(tx) = observed.lock().unwrap().remove(&id) {
                let _ = tx.send(result);
            }
        }
    });
    let (cancel, _rx) = mpsc::channel(1);
    let bridge = SidecarBridge::new("unused-proxy-fixture");
    *bridge.inner.lock().await = Some(Sidecar {
        child, stdin, pending, cancel, next_id: Arc::new(AtomicU64::new(0)),
        healthy: Arc::new(std::sync::atomic::AtomicBool::new(true)),
        failure: Arc::new(StdMutex::new(None)),
        incarnation: uuid::Uuid::new_v4(),
    });
    (bridge, frames, reader)
}

pub(super) async fn finish(bridge: SidecarBridge, reader: tokio::task::JoinHandle<()>) {
    let mut state = bridge.inner.lock().await;
    let worker = state.as_mut().unwrap();
    worker.child.kill().await.unwrap();
    worker.child.wait().await.unwrap();
    drop(state);
    tokio::time::timeout(std::time::Duration::from_secs(2), reader).await.unwrap().unwrap();
}

async fn seed_route(bridge: &SidecarBridge, account: &AccountId, proxy: &str) {
    let cell = bridge.route_state(account).await;
    cell.lock().await.current.proxy = Some(proxy.into());
}

#[tokio::test]
async fn browser_refusal_waits_for_current_reply_without_changing_devices() {
    use std::sync::atomic::AtomicBool;
    use std::time::Duration;
    for cancel in [true, false] {
        let (bridge, mut frames, reader) = echo_bridge_mode(None, "hold_ai").await;
        let account = AccountId::new();
        let body = json!({});
        let cancelled = AtomicBool::new(false);
        let check = || cancelled.load(Ordering::SeqCst);
        let mut request = Box::pin(bridge.ai_post_cancellable(
            &account, "/fixture", "fixture-only", &body, Some(&check)));
        let first = tokio::select! {
            frame = frames.recv() => frame.unwrap(),
            result = &mut request => panic!("early reply: {result:?}"),
            _ = tokio::time::sleep(Duration::from_secs(2)) => panic!("no command"),
        };
        assert_eq!(first["defer_retries"], true);
        assert_eq!(first["attempt_index"], 0);
        cancelled.store(cancel, Ordering::SeqCst);
        assert!(tokio::time::timeout(Duration::from_millis(5), &mut request).await.is_err());
        let send = |frame: &Value, result: Value| {
            let inner = bridge.inner.try_lock().unwrap();
            let tx = inner.as_ref().unwrap().pending.lock().unwrap()
                .remove(&frame["id"].as_u64().unwrap()).unwrap();
            assert!(tx.send(Ok(result)).is_ok());
        };
        send(&first, json!({"status":403,"body":"fixture","_retry_device":true}));
        let result = tokio::time::timeout(Duration::from_secs(2), &mut request)
            .await.unwrap().unwrap();
        assert_eq!(result.0, 403);
        drop(request);
        assert!(frames.try_recv().is_err());
        finish(bridge, reader).await;
    }
}

#[tokio::test]
async fn launch_capable_commands_carry_the_saved_account_proxy() {
    let (bridge, mut frames, reader) = echo_bridge(None).await;
    let account = AccountId::new();
    let proxy = "http://account-proxy.invalid:1234";
    seed_route(&bridge, &account, proxy).await;
    bridge.castle_token(&account).await.unwrap();
    bridge.seed_session(&account, "session=fixture-only").await.unwrap();
    bridge.ai_post(&account, "/fixture", "fixture-only", &json!({})).await.unwrap();
    bridge.auth_begin(&account, "fixture@example.invalid").await.unwrap();
    bridge.auth_complete(&account, "fixture-only").await.unwrap();
    bridge.call_account(&account, "eval", json!({"js":"null"})).await.unwrap();
    for command in ["mint", "set_cookie", "ai_post", "begin", "complete", "eval"] {
        let frame = frames.recv().await.unwrap();
        assert_eq!(frame["cmd"], command);
        assert_eq!(frame["proxy"], proxy);
        assert_eq!(frame["account"], account.0.to_string());
    }
    finish(bridge, reader).await;
}

#[tokio::test]
async fn failed_close_preserves_the_previous_proxy() {
    let (bridge, mut frames, reader) = echo_bridge(Some("fixture close refused")).await;
    let account = AccountId::new();
    let previous = "http://previous.invalid:1234";
    seed_route(&bridge, &account, previous).await;
    let result = bridge.set_proxy(&account, Some("http://new.invalid:4321".into())).await;
    assert_eq!(result.unwrap_err(), "fixture close refused");
    bridge.castle_token(&account).await.unwrap();
    let close = frames.recv().await.unwrap();
    assert_eq!(close["cmd"], "close");
    assert!(close.get("proxy").is_none());
    assert_eq!(frames.recv().await.unwrap()["proxy"], previous);
    assert_eq!(bridge.route_snapshot(&account).await.unwrap().proxy.as_deref(), Some(previous));
    finish(bridge, reader).await;
}

#[tokio::test]
async fn acknowledged_close_clears_the_proxy_on_the_next_launch() {
    let (bridge, mut frames, reader) = echo_bridge(None).await;
    let account = AccountId::new();
    seed_route(&bridge, &account, "http://previous.invalid:1234").await;
    bridge.set_proxy(&account, None).await.unwrap();
    assert!(bridge.route_snapshot(&account).await.unwrap().proxy.is_none());
    bridge.castle_token(&account).await.unwrap();
    assert_eq!(frames.recv().await.unwrap()["cmd"], "close");
    assert_eq!(frames.recv().await.unwrap()["cmd"], "register_proxy");
    let next = frames.recv().await.unwrap();
    assert_eq!(next["cmd"], "mint");
    assert_eq!(next.get("proxy"), Some(&Value::Null));
    finish(bridge, reader).await;
}

#[tokio::test]
async fn concurrent_accounts_keep_separate_proxy_settings() {
    let (bridge, mut frames, reader) = echo_bridge(None).await;
    let first = AccountId::new();
    let second = AccountId::new();
    seed_route(&bridge, &first, "http://first.invalid:1234").await;
    seed_route(&bridge, &second, "http://second.invalid:4321").await;
    let (left, right) = tokio::join!(bridge.castle_token(&first), bridge.castle_token(&second));
    left.unwrap();
    right.unwrap();
    let mut expected = HashMap::from([
        (first.0.to_string(), "http://first.invalid:1234"),
        (second.0.to_string(), "http://second.invalid:4321"),
    ]);
    for _ in 0..2 {
        let frame = frames.recv().await.unwrap();
        let proxy = expected.remove(frame["account"].as_str().unwrap()).unwrap();
        assert_eq!(frame["proxy"], proxy);
    }
    assert!(expected.is_empty());
    finish(bridge, reader).await;
}

#[tokio::test]
async fn registration_before_startup_never_starts_a_host_or_creates_state() {
    let account = AccountId::new();
    let root = std::env::temp_dir().join(format!("uncreated-proxy-{}", account.0));
    let bridge = SidecarBridge::new(&root);
    bridge.register_account_proxy(&account, Some("http://saved.invalid:1234".into())).await.unwrap();
    let route = bridge.route_snapshot(&account).await.unwrap();
    assert_eq!(route.proxy.as_deref(), Some("http://saved.invalid:1234"));
    assert_eq!(route.revision, 1);
    bridge.register_account_proxy(&account, route.proxy.clone()).await.unwrap();
    assert_eq!(bridge.route_snapshot(&account).await.unwrap().revision, 1);
    assert!(bridge.inner.lock().await.is_none());
    assert!(!root.exists());
}

#[tokio::test]
async fn live_registration_refusal_preserves_the_old_route_without_closing() {
    let (bridge, mut frames, reader) = echo_bridge_mode(None, "busy").await;
    let account = AccountId::new();
    assert!(bridge.register_account_proxy(&account, Some("http://new.invalid:4321".into())).await.is_err());
    let route = bridge.route_snapshot(&account).await.unwrap();
    assert_eq!(route.revision, 0);
    assert!(route.proxy.is_none());
    assert_eq!(frames.recv().await.unwrap()["cmd"], "register_proxy");
    assert!(frames.try_recv().is_err());
    finish(bridge, reader).await;
}

#[tokio::test]
async fn acknowledged_registration_fences_the_route_and_leaves_other_accounts_alone() {
    let (bridge, mut frames, reader) = echo_bridge(None).await;
    let account = AccountId::new();
    let other = AccountId::new();
    let proxy = Some("http://saved.invalid:1234".into());
    bridge.register_account_proxy(&account, proxy.clone()).await.unwrap();
    let registered = frames.recv().await.unwrap();
    assert_eq!(registered["cmd"], "register_proxy");
    assert_eq!(registered["expected_revision"], 0);
    assert_eq!(registered["revision"], 1);
    bridge.register_account_proxy(&account, proxy).await.unwrap();
    assert!(frames.try_recv().is_err(), "unchanged registration performed I/O");
    bridge.castle_token(&account).await.unwrap();
    let first = frames.recv().await.unwrap();
    assert_eq!(first["proxy_revision"], 1);
    assert_eq!(first["proxy"], "http://saved.invalid:1234");
    bridge.castle_token(&other).await.unwrap();
    let second = frames.recv().await.unwrap();
    assert_eq!(second["proxy_revision"], 0);
    assert_eq!(second["proxy"], Value::Null);
    finish(bridge, reader).await;
}

#[tokio::test]
async fn malformed_registration_keeps_the_old_proxy_and_blocks_uncertain_commands() {
    let (bridge, mut frames, reader) = echo_bridge_mode(None, "malformed").await;
    let account = AccountId::new();
    assert!(bridge.register_account_proxy(&account, Some("http://new.invalid:4321".into())).await.is_err());
    let cell = bridge.route_state(&account).await;
    let state = cell.lock().await;
    assert!(state.current.proxy.is_none());
    assert_eq!(state.current.revision, 0);
    assert!(state.pending.is_some());
    drop(state);
    assert_eq!(frames.recv().await.unwrap()["cmd"], "register_proxy");
    assert!(bridge.castle_token(&account).await.err().expect("expected rejection").contains("unconfirmed"));
    assert!(frames.try_recv().is_err());
    finish(bridge, reader).await;
}

#[tokio::test]
async fn cancelled_registration_is_reconciled_without_blocking_another_account() {
    let (bridge, mut frames, reader) = echo_bridge_mode(None, "first_silent").await;
    let account = AccountId::new();
    let other = AccountId::new();
    let proxy = Some("http://saved.invalid:1234".into());
    let mut registration = Box::pin(bridge.register_account_proxy(&account, proxy.clone()));
    let first = tokio::select! {
        result = &mut registration => panic!("registration finished early: {result:?}"),
        frame = frames.recv() => frame.unwrap(),
        _ = tokio::time::sleep(std::time::Duration::from_secs(2)) => panic!("no registration frame"),
    };
    tokio::time::timeout(std::time::Duration::from_secs(2), bridge.castle_token(&other))
        .await.unwrap().unwrap();
    assert_eq!(frames.recv().await.unwrap()["account"], other.0.to_string());
    drop(registration);
    assert!(bridge.route_snapshot(&account).await.is_err());
    bridge.register_account_proxy(&account, proxy).await.unwrap();
    let retry = frames.recv().await.unwrap();
    assert_eq!(retry["revision"], first["revision"]);
    assert_eq!(retry["expected_revision"], first["expected_revision"]);
    assert_eq!(bridge.route_snapshot(&account).await.unwrap().revision, 1);
    finish(bridge, reader).await;
}

#[tokio::test]
async fn registration_cannot_overtake_a_command_preparing_its_first_host() {
    let account = AccountId::new();
    let root = std::env::temp_dir().join(format!("uncreated-route-race-{}", account.0));
    let bridge = SidecarBridge::new(&root);
    let host = bridge.inner.lock().await;
    let mut command = Box::pin(bridge.castle_token(&account));
    assert!(futures::poll!(command.as_mut()).is_pending());
    let mut registration = Box::pin(bridge.register_account_proxy(
        &account, Some("http://saved.invalid:1234".into())));
    assert!(futures::poll!(registration.as_mut()).is_pending());
    drop(command);
    drop(host);
    registration.await.unwrap();
    assert!(bridge.inner.lock().await.is_none());
    assert!(!root.exists());
}

#[tokio::test]
async fn unsafe_registration_receipts_never_commit_or_start_work() {
    for mode in ["wrong_route", "wrong_account", "live_ack", "active_ack", "no_host"] {
        let (bridge, mut frames, reader) = echo_bridge_mode(None, mode).await;
        let account = AccountId::new();
        assert!(bridge.register_account_proxy(&account, Some("http://new.invalid:4321".into()))
            .await.is_err(), "accepted {mode}");
        assert_eq!(frames.recv().await.unwrap()["cmd"], "register_proxy");
        assert!(bridge.castle_token(&account).await.err().expect("expected rejection").contains("unconfirmed"));
        let cell = bridge.route_state(&account).await;
        let state = cell.lock().await;
        assert_eq!(state.current.revision, 0);
        assert!(state.current.proxy.is_none());
        assert!(state.pending.is_some());
        drop(state);
        assert!(frames.try_recv().is_err());
        finish(bridge, reader).await;
    }
}

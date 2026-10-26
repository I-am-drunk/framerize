//! Public startup errors must retain uncertainty until child cleanup is joined.
use super::*;

#[tokio::test]
async fn rejected_device_seed_reports_uncertainty_until_joined_shutdown() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let fixture = Fixture::new(Some(listener.local_addr().unwrap().port()));
    let peer = tokio::spawn(reject_seed(listener));
    let bridge = fixture.bridge(EngineKind::FullChrome);
    let account = AccountId::new();
    let result = timeout(Duration::from_secs(5), bridge.castle_token(&account)).await;
    let retry = timeout(Duration::from_secs(2), bridge.seed_session(&account, "session=fixture")).await;
    let closed = timeout(Duration::from_secs(3), bridge.shutdown()).await;
    let joined = timeout(Duration::from_secs(3), peer).await;
    joined.unwrap().unwrap();
    closed.unwrap().unwrap();
    fixture.assert_exited(account);
    assert_eq!(result.unwrap().err().unwrap(), OWNED_STARTUP_UNCONFIRMED);
    assert_eq!(retry.unwrap().unwrap_err(), OWNED_STARTUP_UNCONFIRMED);
    assert!(!bridge.route_cell(&account).await.lock().await.may_have_browser);
    assert!(bridge.instances.lock().await.is_empty());
}

#[tokio::test]
async fn pre_spawn_failures_keep_original_errors_and_clean_shutdown() {
    let fixture = Fixture::new(None);
    let bridge = CdpBridge::new(fixture.root.join("missing-executable"), &fixture.profiles);
    let invalid_record = AccountId::new();
    let profile = bridge.profile_dir(&invalid_record);
    std::fs::create_dir_all(&profile).unwrap();
    std::fs::write(DeviceRecord::path(&profile), "{broken").unwrap();
    assert_eq!(bridge.castle_token(&invalid_record).await.err().unwrap(),
        "invalid browser device record JSON");
    let failed_spawn = AccountId::new();
    let error = bridge.castle_token(&failed_spawn).await.err().unwrap();
    assert!(error.starts_with("launch chrome: "), "{error}");
    for account in [invalid_record, failed_spawn] {
        assert!(!bridge.route_cell(&account).await.lock().await.may_have_browser);
    }
    bridge.shutdown().await.unwrap();
    assert!(bridge.instances.lock().await.is_empty());
}

#[tokio::test]
async fn pending_commands_and_attach_keep_their_existing_errors() {
    for engine in [EngineKind::FullChrome, EngineKind::HeadlessShell, EngineKind::Attach] {
        let bridge = CdpBridge::with_engine("unused", "/fixture-uncreated", engine);
        let account = AccountId::new();
        let cell = bridge.route_cell(&account).await;
        cell.lock().await.may_have_browser = true;
        if engine == EngineKind::Attach {
            assert_eq!(bridge.castle_token(&account).await.err().unwrap(),
                "cdp-lifecycle-unconfirmed: browser cleanup is unconfirmed");
        }
        cell.lock().await.pending.record(1).unwrap();
        assert_eq!(bridge.castle_token(&account).await.err().unwrap(),
            cdp_pending_commands::UNCONFIRMED);
        assert!(bridge.instances.lock().await.is_empty());
    }
}

#[tokio::test]
async fn failed_owned_route_marks_every_public_entry_unconfirmed() {
    for engine in [EngineKind::FullChrome, EngineKind::HeadlessShell] {
        let bridge = CdpBridge::with_engine("unused", "/fixture-uncreated", engine);
        let account = AccountId::new();
        bridge.route_cell(&account).await.lock().await.may_have_browser = true;
        let errors = [
            bridge.castle_token(&account).await.err().unwrap(),
            bridge.seed_session(&account, "session=fixture").await.unwrap_err(),
            bridge.auth_begin(&account, "fixture@example.invalid").await.unwrap_err(),
            bridge.auth_complete(&account, "fixture-token").await.unwrap_err(),
            bridge.ai_post(&account, "/fixture", "fixture-bearer", &json!({})).await.unwrap_err(),
        ];
        for error in errors { assert_eq!(error, OWNED_STARTUP_UNCONFIRMED); }
        assert!(bridge.instances.lock().await.is_empty());
    }
}

type Peer = tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>;

async fn reject_seed(listener: tokio::net::TcpListener) {
    let mut peer = startup_peer(listener).await;
    let ready = reply_to_eval(&mut peer, json!({"result":{"type":"boolean", "value":true}})).await;
    assert!(ready["params"]["expression"].as_str().unwrap().contains("document.readyState"));
    let seed = reply_to_eval(&mut peer, json!({
        "exceptionDetails":{"text":"fixture seed payload"},
        "result":{"type":"undefined"}
    })).await;
    assert!(seed["params"]["expression"].as_str().unwrap().contains("localStorage.setItem"));
    if let Some(Ok(frame)) = timeout(Duration::from_secs(3), peer.next()).await.unwrap() {
        assert!(frame.is_close(), "failed startup kept its CDP transport");
    }
}

async fn reply_to_eval(peer: &mut Peer, result: Value) -> Value {
    let frame = timeout(Duration::from_secs(3), peer.next()).await.unwrap().unwrap().unwrap();
    let command: Value = serde_json::from_str(frame.to_text().unwrap()).unwrap();
    assert_eq!(command["method"], "Runtime.evaluate");
    let reply = json!({"id":command["id"], "result":result});
    peer.send(Message::Text(reply.to_string().into())).await.unwrap();
    command
}

async fn startup_peer(listener: tokio::net::TcpListener) -> Peer {
    let address = listener.local_addr().unwrap();
    let (mut http, _) = timeout(Duration::from_secs(3), listener.accept()).await.unwrap().unwrap();
    let mut request = [0; 1024];
    timeout(Duration::from_secs(2), http.read(&mut request)).await.unwrap().unwrap();
    let body = json!([{"type":"page", "url":FRAMER_HOME,
        "webSocketDebuggerUrl":format!("ws://{address}")}])
        .to_string();
    let reply = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
    http.write_all(reply.as_bytes()).await.unwrap();
    drop(http);
    let (socket, _) = timeout(Duration::from_secs(3), listener.accept()).await.unwrap().unwrap();
    timeout(Duration::from_secs(2), tokio_tungstenite::accept_async(socket))
        .await.unwrap().unwrap()
}

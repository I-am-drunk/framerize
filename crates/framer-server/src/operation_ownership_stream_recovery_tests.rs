// Post-attempt account replay is forbidden. The explicit_account_retry tests
// cover original-result delivery, disconnection, permits and cleanup fencing.
use super::*;

#[tokio::test]
async fn discarded_first_permit_cooldown_keeps_stream_admission_open() {
    first_permit_cooldown(true).await;
}

#[tokio::test]
async fn discarded_first_permit_cooldown_keeps_nonstream_admission_open() {
    first_permit_cooldown(false).await;
}

async fn first_permit_cooldown(stream: bool) {
    for messages in [true, false] {
        let mut f = fixture::Fixture::new();
        f.state = with_owned_operations(f.state, 32).unwrap();
        let ownership = f.state.operation_ownership.clone().unwrap();
        let model = if messages { "claude-opus-5-5" } else { "gpt-6-astra" };
        let first_permit = f.state.gate.acquire(&f.key(0, model)).await;
        let app = crate::router(f.state.clone(), "/nonexistent-fixture-web");
        let task = tokio::spawn(app.clone().oneshot(cooldown_request(&f, messages, stream, 0)));
        // Session load follows the initial Ready check. The first permit is still held.
        // Require the next notification so an unexpected identity cannot be skipped.
        let loaded = tokio::time::timeout(WAIT, f.loads.recv()).await.unwrap().unwrap();
        assert_eq!(loaded, f.accounts[0].framer_user_id);
        f.no_call();
        let mut cooling = f.accounts[0].clone();
        cooling.cooling_until_ms = Some(framer_core::rotation::now_ms() + 60_000);
        f.state.state.upsert(&cooling).await.unwrap();
        let before = serde_json::to_value(f.state.state.list().await.unwrap()).unwrap();
        drop(first_permit);
        let response = tokio::time::timeout(WAIT, task).await.unwrap().unwrap().unwrap();
        if messages && stream {
            assert_eq!(response.status(), StatusCode::OK);
            assert_eq!(response.headers()["content-type"], "text/event-stream");
        } else {
            assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
            let retry: u64 = response.headers()[axum::http::header::RETRY_AFTER]
                .to_str().unwrap().parse().unwrap();
            assert!((1..=30).contains(&retry));
        }
        assert!(!response.body().is_end_stream(), "exercise an unread refusal body");
        f.no_call();
        assert!(f.loads.try_recv().is_err());
        drop(response);
        tokio::time::timeout(WAIT, async {
            while ownership.gate.stats().unwrap().active > 1 { tokio::task::yield_now().await; }
        }).await.unwrap();
        assert_eq!(ownership.gate.stats().unwrap().active, 1); // Retained host only.
        assert!(!ownership.gate.stats().unwrap().unconfirmed);
        assert_eq!(serde_json::to_value(f.state.state.list().await.unwrap()).unwrap(), before);
        let snapshot = f.state.serving_accounts.residents.snapshot().unwrap();
        assert_eq!(snapshot.residents.len(), 1);
        assert_eq!(snapshot.residents[0].account, cooling.id);
        assert_eq!(snapshot.residents[0].users, 0);
        assert!(!snapshot.residents[0].uncertain);
        drop(f.state.gate.acquire_now(&f.key(0, model)).await.expect("first permit leaked"));
        let task = tokio::spawn(app.oneshot(cooldown_request(&f, messages, stream, 1)));
        let call = f.next().await;
        assert_eq!(call.account, f.accounts[1].id);
        assert_eq!(call.kind, if messages { "messages" } else { "responses" });
        let loaded = tokio::time::timeout(WAIT, f.loads.recv()).await.unwrap().unwrap();
        assert_eq!(loaded, f.accounts[1].framer_user_id);
        call.reply.send(Ok(fixture::success(call.kind))).ok().unwrap();
        tokio::time::timeout(WAIT, call.dropped).await.unwrap().unwrap();
        let response = tokio::time::timeout(WAIT, task).await.unwrap().unwrap().unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = tokio::time::timeout(WAIT, to_bytes(response.into_body(), 65536))
            .await.unwrap().unwrap();
        if stream { assert_recovery_output(&body, messages, false); }
        else { assert_cooldown_peer_output(&body, messages); }
        tokio::time::timeout(WAIT, async {
            while ownership.gate.stats().unwrap().active > 1 { tokio::task::yield_now().await; }
        }).await.unwrap();
        assert_eq!(ownership.gate.stats().unwrap().active, 1);
        assert!(!ownership.gate.stats().unwrap().unconfirmed);
        assert_eq!(serde_json::to_value(f.state.state.list().await.unwrap()).unwrap(), before);
        let snapshot = f.state.serving_accounts.residents.snapshot().unwrap();
        assert_eq!(snapshot.residents.len(), 2);
        assert!(snapshot.residents.iter().all(|slot| slot.users == 0 && !slot.uncertain));
        drop(f.state.gate.acquire_now(&f.key(1, model)).await.expect("peer permit leaked"));
        f.no_call();
        assert!(f.loads.try_recv().is_err());
    }
}

fn cooldown_request(f: &fixture::Fixture, messages: bool, stream: bool, index: usize)
    -> HttpRequest<Body> {
    let (path, payload) = if messages {
        ("/anthropic/v1/messages", serde_json::json!({"model":"claude-opus-5-5",
            "messages":[{"role":"user","content":"fixture"}],"max_tokens":16,"stream":stream}))
    } else {
        ("/openai/v1/responses", serde_json::json!({"model":"gpt-6-astra",
            "input":"fixture","stream":stream}))
    };
    HttpRequest::builder().method("POST").uri(path)
        .header("content-type", "application/json")
        .header("x-framerize-account", f.accounts[index].id.0.to_string())
        .header("x-framerize-session", "fixture-conversation")
        .body(Body::from(payload.to_string())).unwrap()
}

fn assert_cooldown_peer_output(body: &[u8], messages: bool) {
    let value: serde_json::Value = serde_json::from_slice(body).unwrap();
    if messages {
        assert_eq!(value["content"][0]["text"], "fixture");
        assert_eq!(value["stop_reason"], "end_turn");
    } else {
        assert_eq!(value["output"][0]["content"][0]["text"], "fixture");
        assert_eq!(value["status"], "completed");
    }
    assert_eq!(value["usage"]["output_tokens"], 1);
}






fn assert_recovery_output(body: &[u8], messages: bool, failed: bool) {
    let text = std::str::from_utf8(body).unwrap();
    let events: Vec<serde_json::Value> = text.lines()
        .filter_map(|line| line.strip_prefix("data: "))
        .map(|data| serde_json::from_str(data).unwrap()).collect();
    assert!(!text.contains("host-lifecycle-unconfirmed"));
    if failed {
        let kind = if messages { "error" } else { "response.failed" };
        let errors: Vec<_> = events.iter().filter(|event| event["type"] == kind).collect();
        assert_eq!(errors.len(), 1);
        let error = if messages { &errors[0]["error"] } else { &errors[0]["response"]["error"] };
        assert_eq!(error["message"], "inert-final-error");
        if !messages { assert_eq!(errors[0]["response"]["status"], "failed"); }
        assert!(!events.iter().any(|event|
            event["type"] == "message_stop" || event["type"] == "response.completed"));
        return;
    }
    assert!(!events.iter().any(|event|
        event["type"] == "error" || event["type"] == "response.failed"));
    if messages {
        let text: String = events.iter().filter(|event| event["type"] == "content_block_delta")
            .map(|event| event["delta"]["text"].as_str().unwrap()).collect();
        assert_eq!(text, "fixture");
        let endings: Vec<_> = events.iter().filter(|event| event["type"] == "message_delta").collect();
        assert_eq!(endings.len(), 1);
        assert_eq!(endings[0]["delta"]["stop_reason"], "end_turn");
        assert_eq!(endings[0]["usage"]["output_tokens"], 1);
        assert_eq!(events.iter().filter(|event| event["type"] == "message_stop").count(), 1);
    } else {
        let endings: Vec<_> = events.iter().filter(|event| event["type"] == "response.completed").collect();
        assert_eq!(endings.len(), 1);
        assert_eq!(endings[0]["response"]["status"], "completed");
        assert_eq!(endings[0]["response"]["output"][0]["content"][0]["text"], "fixture");
        assert_eq!(endings[0]["response"]["usage"]["output_tokens"], 1);
    }
}


#[tokio::test]
async fn read_only_heads_and_discarded_gets_preserve_running_stream_and_admission() {
    let mut f = fixture::Fixture::new();
    f.state = with_owned_operations(f.state, 32).unwrap();
    let ownership = f.state.operation_ownership.clone().unwrap();
    let app = crate::router(f.state.clone(), "/nonexistent-fixture-web");
    let stream = app.clone().oneshot(fixture_request(&f, true)).await.unwrap();
    let call = f.next().await;
    for (path, content_type, length) in [
        ("/anthropic/api/hello", "text/plain; charset=utf-8", Some("2")),
        ("/openai/api/hello", "text/plain; charset=utf-8", Some("2")),
        ("/api/hello", "text/plain; charset=utf-8", Some("2")),
        ("/api/accounts", "application/json", None),
        ("/api/pool", "application/json", None),
        ("/api/pool/config", "application/json", None),
    ] {
        for method in ["HEAD", "GET"] {
            let response = app.clone().oneshot(HttpRequest::builder().method(method)
                .uri(path).body(Body::empty()).unwrap()).await.unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            assert_eq!(response.headers()[axum::http::header::CONTENT_TYPE], content_type);
            if let Some(length) = length {
                assert_eq!(response.headers()[axum::http::header::CONTENT_LENGTH], length);
            }
            if method == "HEAD" {
                assert!(to_bytes(response.into_body(), 16).await.unwrap().is_empty());
            } else { drop(response); }
            assert!(!ownership.gate.stats().unwrap().unconfirmed, "{method} {path}");
            assert_eq!(ownership.gate.stats().unwrap().active, 3);
            assert!(!call.reply.is_closed());
        }
    }
    f.no_call();
    call.reply.send(Ok(fixture::success(call.kind))).ok().unwrap();
    let body = tokio::time::timeout(WAIT, to_bytes(stream.into_body(), 65536))
        .await.unwrap().unwrap();
    assert!(String::from_utf8_lossy(&body).contains("response.completed"));
    drop(body);
    // Channel EOF does not wait for the owned producer's disposal join.
    tokio::time::timeout(WAIT, async {
        while ownership.gate.stats().unwrap().active > 1 { tokio::task::yield_now().await; }
    }).await.expect("owned stream did not settle after body disposal");
    assert_eq!(ownership.gate.stats().unwrap().active, 1);
    assert!(!ownership.gate.stats().unwrap().unconfirmed);
    let next = app.oneshot(fixture_request(&f, true)).await.unwrap();
    assert_eq!(next.status(), StatusCode::OK);
    let call = f.next().await;
    call.reply.send(Ok(fixture::success(call.kind))).ok().unwrap();
    to_bytes(next.into_body(), 65536).await.unwrap();
}

#[tokio::test]
async fn dropped_stream_preflight_refusal_does_not_seal_admission() {
    let mut f = fixture::Fixture::new();
    f.state = with_owned_operations(f.state, 32).unwrap();
    let ownership = f.state.operation_ownership.clone().unwrap();
    let mut cooling = f.accounts[0].clone();
    cooling.cooling_until_ms = Some(framer_core::rotation::now_ms() + 60_000);
    f.state.state.upsert(&cooling).await.unwrap();
    let app = crate::router(f.state.clone(), "/nonexistent-fixture-web");
    let response = app.clone().oneshot(fixture_request(&f, true)).await.unwrap();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert!(response.headers().contains_key(axum::http::header::RETRY_AFTER));
    f.no_call();
    drop(response);
    assert!(!ownership.gate.stats().unwrap().unconfirmed);
    assert_eq!(ownership.gate.stats().unwrap().active, 1);

    let mut request = fixture_request(&f, true);
    request.headers_mut().insert("x-framerize-account",
        f.accounts[1].id.0.to_string().parse().unwrap());
    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let call = f.next().await;
    call.reply.send(Ok(fixture::success(call.kind))).ok().unwrap();
    let body = tokio::time::timeout(WAIT, to_bytes(response.into_body(), 65536))
        .await.unwrap().unwrap();
    assert!(String::from_utf8_lossy(&body).contains("response.completed"));
    assert!(!ownership.gate.stats().unwrap().unconfirmed);
}

#[tokio::test(flavor = "current_thread")]
async fn disconnected_first_permit_wait_releases_stream_request_before_permit() {
    first_permit_disconnect(true).await;
}

#[tokio::test(flavor = "current_thread")]
async fn disconnected_first_permit_wait_releases_nonstream_request_before_permit() {
    first_permit_disconnect(false).await;
}

async fn first_permit_disconnect(stream: bool) {
    for messages in [true, false] {
        let mut f = fixture::Fixture::new();
        f.state = with_owned_operations(f.state, 32).unwrap();
        let ownership = f.state.operation_ownership.clone().unwrap();
        let model = if messages { "claude-opus-5-5" } else { "gpt-6-astra" };
        let first_key = f.key(0, model);
        let peer_key = f.key(1, model);
        let first_permit = f.state.gate.acquire(&first_key).await;
        let before = serde_json::to_value(f.state.state.list().await.unwrap()).unwrap();
        let app = crate::router(f.state.clone(), "/nonexistent-fixture-web");
        let task = tokio::spawn(app.clone().oneshot(cooldown_request(&f, messages, stream, 0)));
        // On this current-thread fixture, the load returns before the handler
        // yields on the occupied first permit. Do not skip another identity.
        let loaded = tokio::time::timeout(WAIT, f.loads.recv()).await.unwrap().unwrap();
        assert_eq!(loaded, f.accounts[0].framer_user_id);
        f.no_call();
        assert_eq!(ownership.gate.stats().unwrap().active, if messages && stream { 3 } else { 2 });
        let snapshot = f.state.serving_accounts.residents.snapshot().unwrap();
        assert_eq!(snapshot.residents.len(), 1);
        assert_eq!(snapshot.residents[0].account, f.accounts[0].id);
        assert_eq!(snapshot.residents[0].users, 1);
        assert!(!snapshot.residents[0].uncertain);
        if messages && stream {
            let response = tokio::time::timeout(WAIT, task).await.unwrap().unwrap().unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            drop(response);
        } else {
            task.abort();
            let cancelled = tokio::time::timeout(WAIT, task).await.unwrap().unwrap_err();
            assert!(cancelled.is_cancelled());
        }
        // Cancellation must drain without borrowing the test's held permit.
        tokio::time::timeout(WAIT, async {
            while ownership.gate.stats().unwrap().active > 1 { tokio::task::yield_now().await; }
        }).await.expect("disconnected request retained its owner behind the first permit");
        assert_eq!(ownership.gate.stats().unwrap().active, 1);
        assert!(!ownership.gate.stats().unwrap().unconfirmed);
        assert!(f.state.gate.acquire_now(&first_key).await.is_none());
        let snapshot = f.state.serving_accounts.residents.snapshot().unwrap();
        assert_eq!(snapshot.residents.len(), 1);
        assert_eq!(snapshot.residents[0].account, f.accounts[0].id);
        assert_eq!(snapshot.residents[0].users, 0);
        assert!(!snapshot.residents[0].uncertain);
        // Full record equality preserves state; write-attempt count is unobserved.
        // Identical-value upserts cannot be ruled out by these snapshots.
        assert_eq!(serde_json::to_value(f.state.state.list().await.unwrap()).unwrap(), before);
        f.no_call();
        assert!(matches!(f.loads.try_recv(), Err(tokio::sync::mpsc::error::TryRecvError::Empty)));
        assert!(f.state.wave.check().is_none());
        let task = tokio::spawn(app.oneshot(cooldown_request(&f, messages, stream, 1)));
        let call = f.next().await;
        assert_eq!(call.account, f.accounts[1].id);
        assert_eq!(call.kind, if messages { "messages" } else { "responses" });
        let loaded = tokio::time::timeout(WAIT, f.loads.recv()).await.unwrap().unwrap();
        assert_eq!(loaded, f.accounts[1].framer_user_id);
        assert!(f.state.gate.acquire_now(&first_key).await.is_none());
        assert!(f.state.gate.acquire_now(&peer_key).await.is_none());
        f.no_call();
        call.reply.send(Ok(fixture::success(call.kind))).ok().unwrap();
        tokio::time::timeout(WAIT, call.dropped).await.unwrap().unwrap();
        let response = tokio::time::timeout(WAIT, task).await.unwrap().unwrap().unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = tokio::time::timeout(WAIT, to_bytes(response.into_body(), 65536))
            .await.unwrap().unwrap();
        if stream { assert_recovery_output(&body, messages, false); }
        else { assert_cooldown_peer_output(&body, messages); }
        tokio::time::timeout(WAIT, async {
            while ownership.gate.stats().unwrap().active > 1 { tokio::task::yield_now().await; }
        }).await.unwrap();
        assert_eq!(ownership.gate.stats().unwrap().active, 1); // Retained host only.
        assert!(!ownership.gate.stats().unwrap().unconfirmed);
        let snapshot = f.state.serving_accounts.residents.snapshot().unwrap();
        assert_eq!(snapshot.residents.len(), 2);
        for account in &f.accounts {
            let slot = snapshot.residents.iter().find(|slot| slot.account == account.id).unwrap();
            assert_eq!(slot.users, 0);
            assert!(!slot.uncertain);
        }
        assert_eq!(serde_json::to_value(f.state.state.list().await.unwrap()).unwrap(), before);
        f.no_call();
        assert!(matches!(f.loads.try_recv(), Err(tokio::sync::mpsc::error::TryRecvError::Empty)));
        assert!(f.state.wave.check().is_none());
        assert!(f.state.gate.acquire_now(&first_key).await.is_none());
        drop(f.state.gate.acquire_now(&peer_key).await.expect("peer permit leaked"));
        let mut ticket = ownership.seal().unwrap();
        let proof = tokio::time::timeout(WAIT, ticket.wait_application()).await.unwrap().unwrap();
        ownership.check_application_drained(&proof).unwrap();
        drop(proof);
        ticket.cancel_before_control().unwrap();
        // Only release the test permit after both requests have drained.
        drop(first_permit);
        drop(f.state.gate.acquire_now(&first_key).await.expect("cancelled waiter retained permit"));
    }
}




#[tokio::test(flavor = "current_thread")]
async fn discarded_first_permit_lifecycle_refusal_keeps_stream_admission_open() {
    first_permit_lifecycle_refusal(true).await;
}

#[tokio::test(flavor = "current_thread")]
async fn discarded_first_permit_lifecycle_refusal_keeps_nonstream_admission_open() {
    first_permit_lifecycle_refusal(false).await;
}

async fn first_permit_lifecycle_refusal(stream: bool) {
    for messages in [true, false] {
        let mut f = fixture::Fixture::new();
        f.state = with_owned_operations(f.state, 32).unwrap();
        let ownership = f.state.operation_ownership.clone().unwrap();
        let model = if messages { "claude-opus-5-5" } else { "gpt-6-astra" };
        let first_key = f.key(0, model);
        let peer_key = f.key(1, model);
        let first_permit = f.state.gate.acquire(&first_key).await;
        let app = crate::router(f.state.clone(), "/nonexistent-fixture-web");
        let task = tokio::spawn(app.clone().oneshot(cooldown_request(&f, messages, stream, 0)));
        let loaded = tokio::time::timeout(WAIT, f.loads.recv()).await.unwrap().unwrap();
        assert_eq!(loaded, f.accounts[0].framer_user_id);
        f.no_call();
        assert_eq!(ownership.gate.stats().unwrap().active, if messages && stream { 3 } else { 2 });
        assert!(f.state.gate.acquire_now(&first_key).await.is_none());
        let mut unavailable = f.accounts[0].clone();
        unavailable.state = framer_core::account::AccountState::Quarantined;
        f.state.state.upsert(&unavailable).await.unwrap();
        let before = serde_json::to_value(f.state.state.list().await.unwrap()).unwrap();
        drop(first_permit);
        let response = tokio::time::timeout(WAIT, task).await.unwrap().unwrap().unwrap();
        if messages && stream {
            assert_eq!(response.status(), StatusCode::OK);
            assert_eq!(response.headers()["content-type"], "text/event-stream");
        } else {
            assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
            assert_eq!(response.headers()[axum::http::header::CONTENT_TYPE], "text/plain; charset=utf-8");
        }
        assert!(!response.body().is_end_stream(), "exercise an unread lifecycle refusal");
        f.no_call();
        assert!(matches!(f.loads.try_recv(), Err(tokio::sync::mpsc::error::TryRecvError::Empty)));
        drop(response);
        tokio::time::timeout(WAIT, async {
            while ownership.gate.stats().unwrap().active > 1 { tokio::task::yield_now().await; }
        }).await.unwrap();
        assert_eq!(ownership.gate.stats().unwrap().active, 1); // Retained host only.
        assert!(!ownership.gate.stats().unwrap().unconfirmed);
        let snapshot = f.state.serving_accounts.residents.snapshot().unwrap();
        assert_eq!(snapshot.residents.len(), 1);
        assert_eq!(snapshot.residents[0].account, unavailable.id);
        assert_eq!(snapshot.residents[0].users, 0);
        assert!(!snapshot.residents[0].uncertain);
        // Full record equality observes state; write-attempt count is unobserved.
        assert_eq!(serde_json::to_value(f.state.state.list().await.unwrap()).unwrap(), before);
        assert!(f.state.wave.check().is_none());
        drop(f.state.gate.acquire_now(&first_key).await.expect("refused account retained permit"));
        let task = tokio::spawn(app.oneshot(cooldown_request(&f, messages, stream, 1)));
        let call = f.next().await;
        assert_eq!(call.account, f.accounts[1].id);
        assert_eq!(call.kind, if messages { "messages" } else { "responses" });
        let loaded = tokio::time::timeout(WAIT, f.loads.recv()).await.unwrap().unwrap();
        assert_eq!(loaded, f.accounts[1].framer_user_id);
        assert!(f.state.gate.acquire_now(&peer_key).await.is_none());
        f.no_call();
        call.reply.send(Ok(fixture::success(call.kind))).ok().unwrap();
        tokio::time::timeout(WAIT, call.dropped).await.unwrap().unwrap();
        let response = tokio::time::timeout(WAIT, task).await.unwrap().unwrap().unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = tokio::time::timeout(WAIT, to_bytes(response.into_body(), 65536))
            .await.unwrap().unwrap();
        if stream { assert_recovery_output(&body, messages, false); }
        else { assert_cooldown_peer_output(&body, messages); }
        tokio::time::timeout(WAIT, async {
            while ownership.gate.stats().unwrap().active > 1 { tokio::task::yield_now().await; }
        }).await.unwrap();
        assert_eq!(ownership.gate.stats().unwrap().active, 1);
        assert!(!ownership.gate.stats().unwrap().unconfirmed);
        let snapshot = f.state.serving_accounts.residents.snapshot().unwrap();
        assert_eq!(snapshot.residents.len(), 2);
        assert!(snapshot.residents.iter().all(|slot| slot.users == 0 && !slot.uncertain));
        assert_eq!(serde_json::to_value(f.state.state.list().await.unwrap()).unwrap(), before);
        drop(f.state.gate.acquire_now(&peer_key).await.expect("peer permit retained after completion"));
        f.no_call();
        assert!(matches!(f.loads.try_recv(), Err(tokio::sync::mpsc::error::TryRecvError::Empty)));
    }
}

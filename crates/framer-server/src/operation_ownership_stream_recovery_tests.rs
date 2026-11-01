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
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        let retry: u64 = response.headers()[axum::http::header::RETRY_AFTER]
            .to_str().unwrap().parse().unwrap();
        assert!((1..=30).contains(&retry));
        assert!(!response.body().is_end_stream(), "exercise an unread refusal body");
        f.no_call();
        assert!(f.loads.try_recv().is_err());
        drop(response);
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

#[derive(Clone, Copy)]
enum RecoveryEnd { Eof, Disconnect, Error }

#[tokio::test]
async fn repicked_stream_success_keeps_prior_unknown_cleanup_fenced() {
    for messages in [true, false] { recovery(messages, RecoveryEnd::Eof).await; }
}

#[tokio::test]
async fn disconnected_repicked_stream_joins_success_and_keeps_prior_cleanup_fenced() {
    for messages in [true, false] { recovery(messages, RecoveryEnd::Disconnect).await; }
}

#[tokio::test]
async fn repicked_stream_plain_error_preserves_result_and_prior_cleanup_fence() {
    for messages in [true, false] { recovery(messages, RecoveryEnd::Error).await; }
}

fn recovery_request(f: &fixture::Fixture, messages: bool) -> HttpRequest<Body> {
    if !messages {
        let mut request = fixture_request(f, true);
        request.headers_mut().remove("x-framerize-account");
        return request;
    }
    HttpRequest::builder().method("POST").uri("/anthropic/v1/messages")
        .header("content-type", "application/json")
        .header("x-framerize-session", "fixture-conversation")
        .body(Body::from(serde_json::json!({"model":"claude-opus-5-5",
            "messages":[{"role":"user","content":"fixture"}],
            "max_tokens":16,"stream":true}).to_string())).unwrap()
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

async fn recovery(messages: bool, end: RecoveryEnd) {
    let mut f = fixture::Fixture::new();
    f.state = with_owned_operations(f.state, 32).unwrap();
    let ownership = f.state.operation_ownership.clone().unwrap();
    let mut next_account = f.accounts[1].clone();
    next_account.consecutive_failures = 3;
    f.state.state.upsert(&next_account).await.unwrap();
    let model = if messages { "claude-opus-5-5" } else { "gpt-6-astra" };
    let app = crate::router(f.state.clone(), "/nonexistent-fixture-web");
    let response = tokio::time::timeout(WAIT, app.clone().oneshot(recovery_request(&f, messages)))
        .await.unwrap().unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let first = f.next().await;
    assert_eq!(first.account, f.accounts[0].id);
    assert_eq!(first.kind, if messages { "messages" } else { "responses" });
    let loaded = tokio::time::timeout(WAIT, f.loads.recv()).await.unwrap().unwrap();
    assert_eq!(loaded, f.accounts[0].framer_user_id);
    first.reply.send(Err("host-lifecycle-unconfirmed: inert first account".into())).ok().unwrap();
    tokio::time::timeout(WAIT, first.dropped).await.unwrap().unwrap();
    // Keep the response connected until the replacement actually starts.
    let second = f.next().await;
    assert_eq!(second.account, f.accounts[1].id);
    assert_eq!(second.kind, first.kind);
    let loaded = tokio::time::timeout(WAIT, f.loads.recv()).await.unwrap().unwrap();
    assert_eq!(loaded, f.accounts[1].framer_user_id);
    assert_eq!(ownership.gate.stats().unwrap().active, 3); // Host, body, producer.
    assert!(f.state.gate.acquire_now(&f.key(1, model)).await.is_none());
    let snapshot = f.state.serving_accounts.residents.snapshot().unwrap();
    assert!(snapshot.residents.iter().any(|slot|
        slot.account == first.account && slot.uncertain && slot.users == 0));
    assert!(snapshot.residents.iter().any(|slot|
        slot.account == second.account && !slot.uncertain && slot.users == 1));
    let mut response = Some(response);
    if matches!(end, RecoveryEnd::Disconnect) {
        drop(response.take());
        assert!(!second.reply.is_closed(), "disconnect cancelled the in-flight call");
        assert_eq!(ownership.gate.stats().unwrap().active, 3);
        assert!(f.state.gate.acquire_now(&f.key(1, model)).await.is_none());
    }
    let reply = if matches!(end, RecoveryEnd::Error) {
        Err("inert-final-error".to_string())
    } else { Ok(fixture::success(second.kind)) };
    second.reply.send(reply).ok().unwrap();
    tokio::time::timeout(WAIT, second.dropped).await.unwrap().unwrap();
    if let Some(response) = response {
        let body = tokio::time::timeout(WAIT, to_bytes(response.into_body(), 65536))
            .await.unwrap().unwrap();
        assert_recovery_output(&body, messages, matches!(end, RecoveryEnd::Error));
    }
    // Port return alone is insufficient: wait for the owned producer/body to release.
    tokio::time::timeout(WAIT, async {
        while ownership.gate.stats().unwrap().active > 1 { tokio::task::yield_now().await; }
    }).await.unwrap();
    assert_eq!(ownership.gate.stats().unwrap().active, 1); // Retained host only.
    assert!(ownership.gate.stats().unwrap().unconfirmed);
    assert!(matches!(ownership.seal(), Err(GateError::Unconfirmed)));
    f.no_call();
    let snapshot = f.state.serving_accounts.residents.snapshot().unwrap();
    let first_slot = snapshot.residents.iter().find(|slot| slot.account == first.account).unwrap();
    let second_slot = snapshot.residents.iter().find(|slot| slot.account == second.account).unwrap();
    assert!(first_slot.uncertain);
    assert_eq!(first_slot.users, 0);
    assert!(!second_slot.uncertain);
    assert_eq!(second_slot.users, 0);
    assert_eq!(second_slot.status, framer_core::resident_pool::ResidentStatus::Idle);
    for index in [0, 1] {
        let permit = f.state.gate.acquire_now(&f.key(index, model)).await.expect("attempt slot leaked");
        drop(permit);
    }
    let current = f.state.state.get(second.account).await.unwrap().unwrap();
    // Even after disconnect, only processing B's success can clear its seeded failures.
    // A plain error preserves them, independently of the global cleanup fence.
    let expected_failures = if matches!(end, RecoveryEnd::Error) { 3 } else { 0 };
    assert_eq!(current.consecutive_failures, expected_failures);
    let mut request = recovery_request(&f, messages);
    request.headers_mut().insert("x-framerize-account",
        second.account.0.to_string().parse().unwrap());
    let refusal = tokio::time::timeout(WAIT, app.oneshot(request)).await.unwrap().unwrap();
    assert_eq!(refusal.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(to_bytes(refusal.into_body(), 1024).await.unwrap().as_ref(),
        b"Cannot start this request: earlier work ended without confirmed cleanup; its resources cannot be reused.");
    f.no_call();
    assert!(f.loads.try_recv().is_err());
}

#[tokio::test]
async fn discarded_messages_wave_refusal_keeps_cooldown_without_sealing() {
    let mut f = fixture::Fixture::new();
    f.state = with_owned_operations(f.state, 32).unwrap();
    let ownership = f.state.operation_ownership.clone().unwrap();
    f.state.wave.note_refusal("fixture-wave-a");
    f.state.wave.note_refusal("fixture-wave-b");
    assert!(f.state.wave.check().is_some());
    let request = HttpRequest::builder().method("POST").uri("/anthropic/v1/messages")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::json!({"model":"claude-opus-5-5",
            "messages":[{"role":"user","content":"fixture"}],
            "max_tokens":16,"stream":true}).to_string())).unwrap();
    let response = crate::router(f.state.clone(), "/nonexistent-fixture-web")
        .oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert!(response.headers().contains_key(axum::http::header::RETRY_AFTER));
    drop(response);
    f.no_call();
    assert!(f.loads.try_recv().is_err());
    assert_eq!(f.state.serving_accounts.residents.snapshot().unwrap().occupied, 0);
    assert_eq!(ownership.gate.stats().unwrap().active, 1);
    assert!(!ownership.gate.stats().unwrap().unconfirmed);
    assert!(f.state.wave.check().is_some());
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
        assert_eq!(ownership.gate.stats().unwrap().active, 2);
        let snapshot = f.state.serving_accounts.residents.snapshot().unwrap();
        assert_eq!(snapshot.residents.len(), 1);
        assert_eq!(snapshot.residents[0].account, f.accounts[0].id);
        assert_eq!(snapshot.residents[0].users, 1);
        assert!(!snapshot.residents[0].uncertain);
        task.abort();
        let cancelled = tokio::time::timeout(WAIT, task).await.unwrap().unwrap_err();
        assert!(cancelled.is_cancelled());
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
async fn disconnected_nonstream_repick_wait_preserves_risk_cooldown_and_open_admission() {
    nonstream_repick_disconnect(false).await;
}

#[tokio::test(flavor = "current_thread")]
async fn disconnected_nonstream_repick_wait_preserves_prior_unknown_cleanup_fence() {
    nonstream_repick_disconnect(true).await;
}

async fn nonstream_repick_disconnect(unknown_cleanup: bool) {
    for messages in [true, false] {
        let mut f = fixture::Fixture::new();
        f.accounts[1].consecutive_failures = 3;
        f.state.state.upsert(&f.accounts[1]).await.unwrap();
        f.state = with_owned_operations(f.state, 32).unwrap();
        let ownership = f.state.operation_ownership.clone().unwrap();
        let model = if messages { "claude-opus-5-5" } else { "gpt-6-astra" };
        let first_key = f.key(0, model);
        let peer_key = f.key(1, model);
        let peer_permit = f.state.gate.acquire(&peer_key).await;
        let app = crate::router(f.state.clone(), "/nonexistent-fixture-web");
        let mut request = cooldown_request(&f, messages, false, 0);
        request.headers_mut().remove("x-framerize-account");
        let task = tokio::spawn(app.oneshot(request));
        let first = f.next().await;
        assert_eq!(first.account, f.accounts[0].id);
        assert_eq!(first.kind, if messages { "messages" } else { "responses" });
        let loaded = tokio::time::timeout(WAIT, f.loads.recv()).await.unwrap().unwrap();
        assert_eq!(loaded, f.accounts[0].framer_user_id);
        assert!(f.state.gate.acquire_now(&first_key).await.is_none());
        f.no_call();
        let error = if unknown_cleanup { "host-lifecycle-unconfirmed: inert first account" }
            else { "risk-refused" };
        first.reply.send(Err(error.into())).ok().unwrap();
        tokio::time::timeout(WAIT, first.dropped).await.unwrap().unwrap();
        // The synchronous load reaches the held B permit before this task resumes.
        let loaded = tokio::time::timeout(WAIT, f.loads.recv()).await.unwrap().unwrap();
        assert_eq!(loaded, f.accounts[1].framer_user_id);
        f.no_call();
        assert_eq!(ownership.gate.stats().unwrap().active, 2);
        assert!(!ownership.gate.stats().unwrap().unconfirmed);
        assert!(f.state.gate.acquire_now(&peer_key).await.is_none());
        drop(f.state.gate.acquire_now(&first_key).await.expect("A permit did not release"));
        let snapshot = f.state.serving_accounts.residents.snapshot().unwrap();
        assert_eq!(snapshot.residents.len(), 2);
        let a = snapshot.residents.iter().find(|slot| slot.account == first.account).unwrap();
        let b = snapshot.residents.iter().find(|slot| slot.account == f.accounts[1].id).unwrap();
        assert_eq!(a.users, 0);
        assert_eq!(a.uncertain, unknown_cleanup);
        assert_eq!(b.users, 1);
        assert!(!b.uncertain);
        let records = f.state.state.list().await.unwrap();
        let a = records.iter().find(|account| account.id == first.account).unwrap();
        assert_eq!(a.state, framer_core::account::AccountState::Ready);
        assert_eq!(a.consecutive_failures, u32::from(!unknown_cleanup));
        assert_eq!(a.cooldown_step, u32::from(!unknown_cleanup));
        assert_eq!(a.cooling_until_ms.is_some(), !unknown_cleanup);
        let b = records.iter().find(|account| account.id == f.accounts[1].id).unwrap();
        assert_eq!(serde_json::to_value(b).unwrap(), serde_json::to_value(&f.accounts[1]).unwrap());
        let post_a = serde_json::to_value(&records).unwrap();
        task.abort();
        let cancelled = tokio::time::timeout(WAIT, task).await.unwrap().unwrap_err();
        assert!(cancelled.is_cancelled());
        tokio::time::timeout(WAIT, async {
            while ownership.gate.stats().unwrap().active > 1 { tokio::task::yield_now().await; }
        }).await.expect("repick request retained its owner behind B's held permit");
        assert_eq!(ownership.gate.stats().unwrap().active, 1); // Retained host only.
        assert_eq!(ownership.gate.stats().unwrap().unconfirmed, unknown_cleanup);
        assert!(f.state.gate.acquire_now(&peer_key).await.is_none());
        let snapshot = f.state.serving_accounts.residents.snapshot().unwrap();
        assert_eq!(snapshot.residents.len(), 2);
        let a = snapshot.residents.iter().find(|slot| slot.account == first.account).unwrap();
        let b = snapshot.residents.iter().find(|slot| slot.account == f.accounts[1].id).unwrap();
        assert_eq!(a.users, 0);
        assert_eq!(a.uncertain, unknown_cleanup);
        assert_eq!(b.users, 0);
        assert!(!b.uncertain);
        assert_eq!(b.status, framer_core::resident_pool::ResidentStatus::Idle);
        // Full records preserve the completed A outcome. Write-attempt count is
        // unobserved; equality does not rule out identical-value upserts.
        assert_eq!(serde_json::to_value(f.state.state.list().await.unwrap()).unwrap(), post_a);
        f.no_call();
        assert!(matches!(f.loads.try_recv(), Err(tokio::sync::mpsc::error::TryRecvError::Empty)));
        assert!(f.state.wave.check().is_none());
        if unknown_cleanup {
            assert!(matches!(ownership.seal(), Err(GateError::Unconfirmed)));
            assert!(matches!(ownership.gate.admit(OperationKind::Inference),
                Err(GateError::Unconfirmed)));
        } else {
            let mut ticket = ownership.seal().unwrap();
            let proof = tokio::time::timeout(WAIT, ticket.wait_application()).await.unwrap().unwrap();
            ownership.check_application_drained(&proof).unwrap();
            drop(proof);
            ticket.cancel_before_control().unwrap();
            let admitted = ownership.gate.admit(OperationKind::Inference).unwrap();
            admitted.complete().unwrap();
            assert!(!ownership.gate.stats().unwrap().sealed);
        }
        assert_eq!(ownership.gate.stats().unwrap().active, 1);
        assert_eq!(ownership.gate.stats().unwrap().unconfirmed, unknown_cleanup);
        // The cancelled request drained before releasing this independent permit.
        drop(peer_permit);
        drop(f.state.gate.acquire_now(&peer_key).await.expect("B permit retained after drain"));
        drop(f.state.gate.acquire_now(&first_key).await.expect("A permit retained after drain"));
        f.no_call();
        assert!(matches!(f.loads.try_recv(), Err(tokio::sync::mpsc::error::TryRecvError::Empty)));
        assert_eq!(serde_json::to_value(f.state.state.list().await.unwrap()).unwrap(), post_a);
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
        assert_eq!(ownership.gate.stats().unwrap().active, 2);
        assert!(f.state.gate.acquire_now(&first_key).await.is_none());
        let mut unavailable = f.accounts[0].clone();
        unavailable.state = framer_core::account::AccountState::Quarantined;
        f.state.state.upsert(&unavailable).await.unwrap();
        let before = serde_json::to_value(f.state.state.list().await.unwrap()).unwrap();
        drop(first_permit);
        let response = tokio::time::timeout(WAIT, task).await.unwrap().unwrap().unwrap();
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(response.headers()[axum::http::header::CONTENT_TYPE], "text/plain; charset=utf-8");
        assert!(!response.body().is_end_stream(), "exercise an unread lifecycle refusal");
        f.no_call();
        assert!(matches!(f.loads.try_recv(), Err(tokio::sync::mpsc::error::TryRecvError::Empty)));
        drop(response);
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

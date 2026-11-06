use super::*;
use axum::{body::BodyDataStream, http::Request};
use futures::StreamExt;
use tower::ServiceExt;

#[path = "permit_lifetime_fixture.rs"]
mod fixture;

fn request(account: Option<AccountId>) -> Request<Body> {
    let mut builder = Request::builder().method("POST").uri("/anthropic/v1/messages")
        .header("content-type", "application/json");
    if let Some(account) = account { builder = builder.header("x-framerize-account", account.0.to_string()); }
    builder.body(Body::from(serde_json::json!({"model":"claude-sonnet-4-5",
        "messages":[{"role":"user","content":"fixture"}],"stream":true}).to_string())).unwrap()
}

fn pause(f: &fixture::Fixture) {
    for account in &f.accounts { f.state.wave.note_refusal(&account.id.0.to_string()); }
    assert!(f.state.wave.check().is_some());
}

#[tokio::test(start_paused = true)]
async fn closed_event_channel_waits_for_the_producer_without_ping_spinning() {
    let (tx, rx) = tokio::sync::mpsc::channel(1);
    drop(tx);
    let (handle, _) = operation_ownership::spawn_observed(None,
        framer_core::operation_gate::OperationKind::Inference, async {
            tokio::time::sleep(Duration::from_secs(16)).await;
            (Err("fixture terminal failure".into()), true)
        }).unwrap();
    let stream = render(rx, handle, "claude-fixture".into());
    futures::pin_mut!(stream);
    assert!(std::str::from_utf8(&stream.next().await.unwrap().unwrap()).unwrap()
        .contains("event: message_start"));
    let started = tokio::time::Instant::now();
    assert!(std::str::from_utf8(&stream.next().await.unwrap().unwrap()).unwrap()
        .contains("event: ping"));
    assert_eq!(started.elapsed(), Duration::from_secs(8));
    let mut ending = String::new();
    while let Some(bytes) = stream.next().await {
        ending.push_str(std::str::from_utf8(&bytes.unwrap()).unwrap());
    }
    assert!(ending.contains("event: error"));
    assert!(!ending.contains("event: message_stop"));
}

#[tokio::test]
async fn terminal_drain_keeps_content_queued_after_noncontent_events() {
    let (tx, rx) = tokio::sync::mpsc::channel(4);
    let fixture::Reply::Messages(result) = fixture::success("messages") else { unreachable!() };
    tx.send(convert::ChainEvent::Done(Box::new(result.clone()))).await.unwrap();
    tx.send(convert::ChainEvent::Chunk(result.content.clone())).await.unwrap();
    drop(tx);
    let (finished, ready) = tokio::sync::oneshot::channel();
    let (handle, _) = operation_ownership::spawn_observed(None,
        framer_core::operation_gate::OperationKind::Inference, async move {
            finished.send(()).unwrap();
            (Ok(result), true)
        }).unwrap();
    ready.await.unwrap();
    let stream = render(rx, handle, "claude-fixture".into());
    futures::pin_mut!(stream);
    let mut wire = String::new();
    while let Some(bytes) = stream.next().await {
        wire.push_str(std::str::from_utf8(&bytes.unwrap()).unwrap());
    }
    assert_eq!(wire.matches("\"text\":\"fixture\"").count(), 1);
    assert_eq!(wire.matches("event: message_stop\n").count(), 1);
    assert!(wire.find("\"text\":\"fixture\"").unwrap() < wire.find("event: message_stop").unwrap());
}

#[tokio::test]
async fn admission_records_a_refusal_once_and_uses_the_latest_success_snapshot() {
    use framer_core::port::ChainAdmission;
    let f = fixture::Fixture::new();
    let guard = account_wait::MessagesAdmission::new(f.state.clone(), f.accounts[0].clone(),
        account_wait::MAX_WAIT);
    guard.before_attempt().await.unwrap();
    let error = guard.refused("risk-refused").await.unwrap_err();
    let account = f.state.state.get(f.accounts[0].id).await.unwrap().unwrap();
    assert_eq!(account.cooldown_step, 1);
    guard.finished(Some(&error)).await;
    assert_eq!(serde_json::to_value(f.state.state.get(account.id).await.unwrap().unwrap()).unwrap(),
        serde_json::to_value(&account).unwrap());
    let mut expired = account;
    expired.cooling_until_ms = Some(0);
    f.state.state.upsert(&expired).await.unwrap();
    guard.before_attempt().await.unwrap();
    guard.finished(None).await;
    let recovered = f.state.state.get(expired.id).await.unwrap().unwrap();
    assert_eq!(recovered.consecutive_failures, 0);
    assert_eq!(recovered.cooldown_step, 0);
    assert!(recovered.cooling_until_ms.is_none());
}

async fn heartbeat(body: &mut BodyDataStream) -> String {
    let mut wire = String::new();
    tokio::time::timeout(Duration::from_secs(10), async {
        while !wire.contains("event: ping\n") {
            let frame = body.next().await.expect("stream closed during cooldown").unwrap();
            wire.push_str(std::str::from_utf8(&frame).unwrap());
        }
    }).await.expect("missing cooldown heartbeat");
    assert_eq!(wire.matches("event: message_start\n").count(), 1);
    assert!(!wire.contains("event: error\n"));
    wire
}

async fn finish(f: &mut fixture::Fixture, mut body: BodyDataStream, mut wire: String) {
    let call = f.next().await;
    assert_eq!(call.kind, "messages");
    assert!(call.config.as_ref().unwrap().admission.is_some());
    call.reply.send(Ok(fixture::success("messages"))).ok().unwrap();
    while let Some(frame) = body.next().await {
        wire.push_str(std::str::from_utf8(&frame.unwrap()).unwrap());
    }
    assert_eq!(wire.matches("event: message_stop\n").count(), 1);
    assert_eq!(wire.matches("event: message_start\n").count(), 1);
    assert!(!wire.contains("event: error\n"));
    assert!(wire.contains("fixture"));
    call.dropped.await.unwrap();
    f.no_call();
}

#[tokio::test(start_paused = true)]
async fn refusal_wave_does_not_delay_a_healthy_account_response() {
    let mut f = fixture::Fixture::new();
    pause(&f);
    let response = crate::router(f.state.clone(), "/nonexistent-fixture-web")
        .oneshot(request(None)).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["content-type"], "text/event-stream");
    let body = response.into_body().into_data_stream();
    finish(&mut f, body, String::new()).await;
}

#[tokio::test(start_paused = true)]
async fn pinned_cooldown_waits_without_using_a_peer() {
    let mut f = fixture::Fixture::new();
    let mut account = f.accounts[0].clone();
    account.cooling_until_ms = Some(framer_core::rotation::now_ms() + 600_000);
    f.state.state.upsert(&account).await.unwrap();
    let response = crate::router(f.state.clone(), "/nonexistent-fixture-web")
        .oneshot(request(Some(account.id))).await.unwrap();
    let mut body = response.into_body().into_data_stream();
    let wire = heartbeat(&mut body).await;
    f.no_call();
    assert_eq!(f.state.serving_accounts.residents.snapshot().unwrap().occupied, 0);
    account.cooling_until_ms = None;
    f.state.state.upsert(&account).await.unwrap();
    finish(&mut f, body, wire).await;
    assert_eq!(f.loads.recv().await.unwrap(), account.framer_user_id);
    assert!(f.loads.try_recv().is_err());
}

#[tokio::test(start_paused = true)]
async fn disconnect_during_refusal_wave_cancels_the_started_request() {
    let mut f = fixture::Fixture::new();
    f.state = operation_ownership::with_owned_operations(f.state, 32).unwrap();
    let ownership = f.state.operation_ownership.clone().unwrap();
    pause(&f);
    let response = crate::router(f.state.clone(), "/nonexistent-fixture-web")
        .oneshot(request(None)).await.unwrap();
    let body = response.into_body().into_data_stream();
    let call = f.next().await;
    assert_eq!(call.kind, "messages");
    f.loads.recv().await.unwrap();
    drop(body);
    drop(call);
    let mut ticket = ownership.seal().unwrap();
    let proof = tokio::time::timeout(Duration::from_secs(2), ticket.wait_application())
        .await.unwrap().unwrap();
    ownership.check_application_drained(&proof).unwrap();
    drop(proof);
    ticket.cancel_before_control().unwrap();
    assert!(ownership.accepting_requests());
    f.no_call();
    assert!(f.loads.try_recv().is_err());
}

#[tokio::test]
async fn stream_errors_keep_provider_status_and_never_finish_a_failed_message() {
    for (error, kind, permanent) in [
        ("retryable 400 on /ai/v3/messages/: invalid input", "invalid_request_error", true),
        ("upstream 403 on /ai/v3/messages/: model is forbidden", "permission_error", true),
        ("retryable 429 on /ai/v3/messages/: busy", "rate_limit_error", false),
        ("retryable 503 on /ai/v3/messages/: busy", "overloaded_error", false),
    ] {
        for partial in [false, true] {
            let (tx, rx) = tokio::sync::mpsc::channel(1);
            if partial {
                tx.send(convert::ChainEvent::Chunk(vec![serde_json::json!({
                    "type":"text", "text":"already delivered"
                })])).await.unwrap();
            }
            drop(tx);
            let (handle, _) = operation_ownership::spawn_observed(None,
                framer_core::operation_gate::OperationKind::Inference, async move {
                    (Err(error.to_owned()), true)
                }).unwrap();
            let stream = render(rx, handle, "claude-fixture".into());
            futures::pin_mut!(stream);
            let mut wire = String::new();
            while let Some(bytes) = stream.next().await {
                wire.push_str(std::str::from_utf8(&bytes.unwrap()).unwrap());
            }
            assert_eq!(wire.matches("event: error\n").count(), 1, "{wire}");
            assert!(!wire.contains("event: message_stop\n"), "{wire}");
            assert_eq!(wire.contains("already delivered"), partial);
            let event: Value = serde_json::from_str(wire.split("event: error\ndata: ")
                .nth(1).unwrap().trim()).unwrap();
            assert_eq!(event["error"]["type"], kind);
            assert_eq!(event["error"]["retryable"] == false, partial || permanent);
        }
    }
}

#[tokio::test]
async fn overflow_cancels_generation_and_preserves_unknown_cleanup() {
    for uncertain in [false, true] {
        let mut f = fixture::Fixture::new();
        let response = crate::router(f.state.clone(), "/nonexistent-fixture-web")
            .oneshot(request(Some(f.accounts[0].id))).await.unwrap();
        let call = f.next().await;
        let cause = if uncertain { "host-lifecycle-unconfirmed: fixture cleanup failed" }
            else { "fixture completed generation" };
        call.reply.send(Ok(fixture::Reply::MessagesFailure {
            chunks: (0..40).map(|i| vec![serde_json::json!({
                "type":"text", "text":format!("part {i}")})]).collect(),
            error: cause.into(),
        })).ok().unwrap();
        call.dropped.await.unwrap();
        assert!(call.config.unwrap().is_cancelled());
        let bytes = axum::body::to_bytes(response.into_body(), 100_000).await.unwrap();
        let wire = std::str::from_utf8(&bytes).unwrap();
        assert_eq!(wire.matches("event: error\n").count(), 1);
        assert!(!wire.contains("event: message_stop\n"));
        assert!(wire.contains("\"retryable\":false"));
        assert_eq!(wire.contains("fixture cleanup failed"), uncertain);
        let snapshot = f.state.serving_accounts.residents.snapshot().unwrap();
        assert_eq!(snapshot.residents.iter().any(|slot| slot.uncertain), uncertain);
        f.no_call();
    }
}

#[tokio::test(start_paused = true)]
async fn messages_share_output_capacity_until_body_and_polled_bytes_are_disposed() {
    let mut f = fixture::Fixture::new();
    f.state = operation_ownership::with_owned_operations(f.state, 32).unwrap();
    f.state.runtime_mut().unwrap().response_output = Arc::new(response_output::OutputBudget::new(1));
    let ownership = f.state.operation_ownership.clone().unwrap();
    pause(&f);
    let app = crate::router(f.state.clone(), "/nonexistent-fixture-web");
    let response = app.clone().oneshot(request(None)).await.unwrap();
    let mut body = response.into_body().into_data_stream();
    let call = f.next().await;
    assert_eq!(call.kind, "messages");
    f.loads.recv().await.unwrap();
    call.reply.send(Ok(fixture::success("messages"))).ok().unwrap();
    tokio::time::timeout(Duration::from_secs(3), call.dropped).await.unwrap().unwrap();
    let bytes = body.next().await.unwrap().unwrap();
    let retained = bytes.clone();
    drop(bytes);
    let blocked = app.clone().oneshot(request(None)).await.unwrap();
    assert_eq!(blocked.status(), StatusCode::SERVICE_UNAVAILABLE);
    drop(blocked);
    let responses = Request::builder().method("POST").uri("/openai/v1/responses")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::json!({"model":"gpt-6-astra",
            "input":"fixture", "stream":true}).to_string())).unwrap();
    let blocked = app.clone().oneshot(responses).await.unwrap();
    assert_eq!(blocked.status(), StatusCode::SERVICE_UNAVAILABLE);
    drop(blocked);
    assert_eq!(f.state.response_output.counts().0, 1);
    drop(body);
    let mut ticket = ownership.seal().unwrap();
    let proof = ticket.wait_application().await.unwrap();
    ownership.check_application_drained(&proof).unwrap();
    drop(proof);
    ticket.cancel_before_control().unwrap();
    assert_eq!(f.state.response_output.counts().0, 1,
        "a retained output buffer must keep its capacity reservation");
    drop(retained);
    assert_eq!(f.state.response_output.counts().0, 0);
    let response = app.oneshot(request(None)).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let mut body = response.into_body().into_data_stream();
    let call = f.next().await;
    assert_eq!(call.kind, "messages");
    f.loads.recv().await.unwrap();
    call.reply.send(Ok(fixture::success("messages"))).ok().unwrap();
    tokio::time::timeout(Duration::from_secs(3), call.dropped).await.unwrap().unwrap();
    while body.next().await.is_some() {}
    let mut ticket = ownership.seal().unwrap();
    let proof = ticket.wait_application().await.unwrap();
    ownership.check_application_drained(&proof).unwrap();
    assert_eq!(f.state.response_output.counts().0, 0);
    f.no_call();
}

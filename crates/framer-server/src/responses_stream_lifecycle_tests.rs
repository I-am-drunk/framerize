use super::*;
use serde_json::Value;

fn events(wire: &str) -> Vec<Value> {
    wire.lines().filter_map(|line| line.strip_prefix("data: "))
        .map(|line| serde_json::from_str(line).unwrap()).collect()
}

fn assert_sequence(events: &[Value]) {
    let actual: Vec<_> = events.iter().map(|event| event["sequence_number"].as_u64()).collect();
    let expected: Vec<_> = (0..events.len() as u64).map(Some).collect();
    assert!(!events.is_empty());
    assert_eq!(actual, expected);
}

#[tokio::test]
async fn responses_disconnect_requires_validated_terminal_success() {
    use tower::ServiceExt;
    for valid in [false, true] {
        let mut f = Fixture::new();
        f.accounts[0].consecutive_failures = 3;
        f.state.state.upsert(&f.accounts[0]).await.unwrap();
        f.state = crate::operation_ownership::with_owned_operations(f.state, 3).unwrap();
        let scope = f.state.operation_ownership.clone().unwrap();
        let response = crate::router(f.state.clone(), "/nonexistent-fixture-web")
            .oneshot(owned_request(&f)).await.unwrap();
        let call = f.next().await;
        drop(response);
        assert!(!call.reply.is_closed());
        let Reply::Responses(mut result) = success("responses") else { unreachable!() };
        if !valid { result.status = "failed".into(); }
        call.reply.send(Ok(Reply::Responses(result))).ok().unwrap();
        tokio::time::timeout(WAIT, call.dropped).await.unwrap().unwrap();
        gate_released(&scope).await;
        let account = f.state.state.get(call.account).await.unwrap().unwrap();
        assert_eq!(account.consecutive_failures, if valid { 0 } else { 3 });
        assert_eq!(f.state.response_output.counts(), (0, 0, 0));
        f.no_call();
    }
}

#[tokio::test]
async fn responses_disconnected_success_preserves_newer_health_update() {
    use tower::ServiceExt;
    let mut f = Fixture::new();
    f.accounts[0].consecutive_failures = 3;
    f.state.state.upsert(&f.accounts[0]).await.unwrap();
    f.state = crate::operation_ownership::with_owned_operations(f.state, 3).unwrap();
    let scope = f.state.operation_ownership.clone().unwrap();
    let response = crate::router(f.state.clone(), "/nonexistent-fixture-web")
        .oneshot(owned_request(&f)).await.unwrap();
    let call = f.next().await;
    let mut newer = f.state.state.get(call.account).await.unwrap().unwrap();
    newer.note_risk_refused(framer_core::rotation::now_ms());
    f.state.state.upsert(&newer).await.unwrap();
    drop(response);
    assert!(!call.reply.is_closed());
    call.reply.send(Ok(success("responses"))).ok().unwrap();
    tokio::time::timeout(WAIT, call.dropped).await.unwrap().unwrap();
    gate_released(&scope).await;
    let current = f.state.state.get(call.account).await.unwrap().unwrap();
    assert_eq!(current.consecutive_failures, newer.consecutive_failures);
    assert_eq!(current.cooldown_step, newer.cooldown_step);
    assert_eq!(current.cooling_until_ms, newer.cooling_until_ms);
    f.no_call();
}

async fn run_reply(reply: Result<Reply, String>) -> String {
    let mut f = Fixture::new();
    let response = chain(f.state.clone(), f.headers(0), false, true).await.unwrap();
    let mut body = response.into_body().into_data_stream();
    let first = tokio::time::timeout(WAIT, body.next()).await.unwrap().unwrap().unwrap();
    let mut wire = String::from_utf8(first.to_vec()).unwrap();
    let call = f.next().await;
    assert!(call.reply.send(reply).is_ok());
    tokio::time::timeout(WAIT, call.dropped).await.unwrap().unwrap();
    while let Some(frame) = tokio::time::timeout(WAIT, body.next()).await.unwrap() {
        wire.push_str(&String::from_utf8(frame.unwrap().to_vec()).unwrap());
    }
    available(&f.state.gate, &f.key(0, RESPONSE_MODEL)).await;
    f.no_call();
    wire
}

#[tokio::test]
async fn responses_handler_sequences_prelude_tools_and_final_snapshot() {
    let Reply::Responses(mut result) = success("responses") else { unreachable!() };
    result.output.push(json!({"type":"function_call", "id":"tool-kept", "call_id":"call-one",
        "name":"calculate", "arguments":"{}", "status":"completed"}));
    let wire = run_reply(Ok(Reply::Responses(result))).await;
    let frames = events(&wire);
    assert_sequence(&frames);
    let tools: Vec<_> = frames.iter().filter(|event| matches!(event["type"].as_str(),
        Some("response.function_call_arguments.delta" | "response.function_call_arguments.done"))).collect();
    assert_eq!(tools.len(), 2);
    assert!(tools.iter().all(|event| event["item_id"] == "tool-kept"));
    assert_eq!(frames.last().unwrap()["type"], "response.completed");
    assert_eq!(frames.last().unwrap()["response"]["output"][1]["id"], "tool-kept");
}

#[tokio::test]
async fn responses_handler_error_uses_the_open_stream_sequence() {
    let wire = run_reply(Err("fixture response rejected".into())).await;
    let frames = events(&wire);
    assert_sequence(&frames);
    assert_eq!(frames.last().unwrap()["type"], "response.failed");
    assert!(!wire.contains("response.completed"));
}

#[tokio::test]
async fn responses_handler_join_failure_emits_a_fixed_sequenced_error() {
    let wire = run_reply(Ok(success("messages"))).await;
    let frames = events(&wire);
    assert_sequence(&frames);
    assert_eq!(frames.last().unwrap()["type"], "response.failed");
    assert_eq!(frames.last().unwrap()["response"]["error"]["message"],
        framer_core::port::OutputError::ProducerPanicked.to_string());
    assert!(!wire.contains("wrong fixture reply"));
    assert!(!wire.contains("response.completed"));
}

#[tokio::test(start_paused = true)]
async fn responses_heartbeat_keeps_identity_sequence_and_joined_completion() {
    let mut f = Fixture::new();
    let response = chain(f.state.clone(), f.headers(0), false, true).await.unwrap();
    let mut body = response.into_body().into_data_stream();
    let first = body.next().await.unwrap().unwrap();
    let mut wire = String::from_utf8(first.to_vec()).unwrap();
    drop(first);
    let id = events(&wire)[0]["response"]["id"].clone();
    let call = f.next().await;
    let heartbeat_bytes = body.next().await.unwrap().unwrap();
    let heartbeat = String::from_utf8(heartbeat_bytes.to_vec()).unwrap();
    drop(heartbeat_bytes);
    wire.push_str(&heartbeat);
    assert!(call.reply.send(Ok(success("responses"))).is_ok());
    call.dropped.await.unwrap();
    while let Some(frame) = body.next().await {
        wire.push_str(&String::from_utf8(frame.unwrap().to_vec()).unwrap());
    }
    available(&f.state.gate, &f.key(0, RESPONSE_MODEL)).await;
    f.no_call();
    let pulse = events(&heartbeat);
    assert_eq!(pulse.len(), 1, "heartbeat must reach an SSE event consumer");
    assert_eq!(pulse[0]["type"], "response.in_progress");
    assert_eq!(pulse[0]["response"]["id"], id);
    let all = events(&wire);
    assert_sequence(&all);
    assert_eq!(all.last().unwrap()["type"], "response.completed");
}

fn owned_request(f: &Fixture) -> axum::http::Request<Body> {
    axum::http::Request::builder().method("POST").uri("/openai/v1/responses")
        .header("content-type", "application/json")
        .header("x-framerize-account", f.accounts[0].id.0.to_string())
        .body(Body::from(json!({"model":RESPONSE_MODEL,"input":"fixture","stream":true}).to_string())).unwrap()
}
async fn gate_released(scope: &std::sync::Arc<crate::operation_ownership::ServingOwnership>) {
    tokio::time::timeout(WAIT, async {
        loop {
            if let Ok(owner) = scope.external_cleanup() { owner.complete().unwrap(); return; }
            tokio::task::yield_now().await;
        }
    }).await.unwrap();
}
#[tokio::test]
async fn responses_eof_retains_gate_until_the_last_polled_clone_drops() {
    use tower::ServiceExt;
    let mut f = Fixture::new();
    f.state = crate::operation_ownership::with_owned_operations(f.state, 3).unwrap();
    let scope = f.state.operation_ownership.clone().unwrap();
    let response = crate::router(f.state.clone(), "/nonexistent-fixture-web")
        .oneshot(owned_request(&f)).await.unwrap();
    let call = f.next().await;
    call.reply.send(Ok(success("responses"))).ok().unwrap();
    let mut body = response.into_body().into_data_stream();
    let mut retained = None;
    while let Some(frame) = tokio::time::timeout(WAIT, body.next()).await.unwrap() {
        let bytes = frame.unwrap();
        if String::from_utf8_lossy(&bytes).contains("response.completed") { retained = Some(bytes.clone()); }
    }
    assert!(retained.is_some());
    assert!(scope.external_cleanup().is_err());
    drop(body);
    assert!(scope.external_cleanup().is_err());
    drop(retained);
    gate_released(&scope).await;
    f.no_call();
}
#[tokio::test]
async fn responses_disconnect_joins_pending_forward_and_preserves_cleanup_fence() {
    use tower::ServiceExt;
    let mut f = Fixture::new();
    f.state = crate::operation_ownership::with_owned_operations(f.state, 3).unwrap();
    let scope = f.state.operation_ownership.clone().unwrap();
    let response = crate::router(f.state.clone(), "/nonexistent-fixture-web")
        .oneshot(owned_request(&f)).await.unwrap();
    let call = f.next().await;
    drop(response);
    assert!(scope.external_cleanup().is_err());
    call.reply.send(Err("host-lifecycle-unconfirmed: inert forward".into())).ok().unwrap();
    call.dropped.await.unwrap();
    tokio::time::timeout(WAIT, async {
        while scope.accepting_requests() { tokio::task::yield_now().await; }
    }).await.unwrap();
    assert!(matches!(scope.external_cleanup(), Err(framer_core::operation_gate::GateError::Unconfirmed)));
    f.no_call();
}
#[tokio::test]
async fn responses_encoder_failure_never_changes_health_or_repicks_account() {
    let mut f = Fixture::new();
    for account in &mut f.accounts {
        account.consecutive_failures = 2;
        f.state.state.upsert(account).await.unwrap();
    }
    f.state.response_output.inject(crate::response_output::Fault::BeforeContent);
    let response = chain(f.state.clone(), HeaderMap::new(), false, true).await.unwrap();
    let call = f.next().await;
    let before = f.state.state.get(call.account).await.unwrap().unwrap();
    call.reply.send(Ok(success("responses"))).ok().unwrap();
    let wire = crate::response_output::consume_body(response.into_body(), 65536).await.unwrap();
    assert!(String::from_utf8_lossy(&wire).contains("response.failed"));
    let after = f.state.state.get(call.account).await.unwrap().unwrap();
    assert_eq!(before.consecutive_failures, after.consecutive_failures);
    assert_eq!(before.cooling_until_ms, after.cooling_until_ms);
    f.no_call();
}

#[tokio::test]
async fn responses_failed_producer_registration_releases_output_before_forward() {
    let mut f = Fixture::new();
    f.state = crate::operation_ownership::with_owned_operations(f.state, 1).unwrap();
    let result = chain(f.state.clone(), f.headers(0), false, true).await;
    assert!(matches!(result, Err(ApiError::ServingCapacity(_))));
    assert_eq!(f.state.response_output.counts(), (0, 0, 0));
    available(&f.state.gate, &f.key(0, RESPONSE_MODEL)).await;
    f.no_call();
}

#[tokio::test]
async fn responses_without_operation_gate_still_reserve_before_headers_and_forward() {
    let mut f = Fixture::new();
    f.state.runtime_mut().unwrap().response_output =
        std::sync::Arc::new(crate::response_output::OutputBudget::new(0));
    let result = chain(f.state.clone(), f.headers(0), false, true).await;
    assert!(matches!(result, Err(ApiError::ServingCapacity(_))));
    assert_eq!(f.state.response_output.counts(), (0, 0, 0));
    available(&f.state.gate, &f.key(0, RESPONSE_MODEL)).await;
    f.no_call();
}

#[tokio::test]
async fn responses_each_gate_admitted_stream_gets_output_without_an_earlier_refusal() {
    use tower::ServiceExt;
    for admitted in [1, 2] {
        let mut f = Fixture::new();
        f.state = crate::operation_ownership::with_owned_operations(f.state, 1 + 2 * admitted).unwrap();
        let mut responses = Vec::new();
        let mut calls = Vec::new();
        for account in 0..admitted {
            let mut request = owned_request(&f);
            request.headers_mut().insert("x-framerize-account", f.accounts[account].id.0.to_string().parse().unwrap());
            let response = crate::router(f.state.clone(), "/nonexistent-fixture-web").oneshot(request).await.unwrap();
            assert_eq!(response.status(), axum::http::StatusCode::OK);
            responses.push(response); calls.push(f.next().await);
        }
        assert_eq!(f.state.response_output.counts().0, admitted);
        let refusal = crate::router(f.state.clone(), "/nonexistent-fixture-web")
            .oneshot(owned_request(&f)).await.unwrap();
        assert_eq!(refusal.status(), axum::http::StatusCode::SERVICE_UNAVAILABLE);
        f.no_call();
        drop(responses);
        for call in calls { call.reply.send(Ok(success("responses"))).ok().unwrap(); }
        tokio::time::timeout(WAIT, async {
            while f.state.response_output.counts() != (0, 0, 0) { tokio::task::yield_now().await; }
        }).await.unwrap();
        gate_released(f.state.operation_ownership.as_ref().unwrap()).await;
        f.no_call();
    }
}

#[tokio::test]
async fn responses_stalled_http_body_resumes_with_complete_unicode_output() {
    let mut f = Fixture::new();
    let response = chain(f.state.clone(), f.headers(0), false, true).await.unwrap();
    let call = f.next().await;
    let Reply::Responses(mut result) = success("responses") else { unreachable!() };
    let text = "雪\\\"\n".repeat(16000);
    result.output[0]["content"][0]["text"] = json!(text);
    call.reply.send(Ok(Reply::Responses(result))).ok().unwrap();
    tokio::time::timeout(WAIT, async {
        while f.state.response_output.counts().2 < 5 * 16 * 1024 {
            tokio::task::yield_now().await;
        }
    }).await.unwrap();
    assert_eq!(f.state.response_output.counts().1, 1);
    let wire = tokio::time::timeout(WAIT,
        crate::response_output::consume_body(response.into_body(), 2 * 1024 * 1024))
        .await.unwrap().unwrap();
    let frames = events(std::str::from_utf8(&wire).unwrap());
    assert_sequence(&frames);
    assert_eq!(frames.iter().filter(|e| e["type"] == "response.completed").count(), 1);
    assert_eq!(frames.last().unwrap()["response"]["output"][0]["content"][0]["text"], text);
    available(&f.state.gate, &f.key(0, RESPONSE_MODEL)).await;
    f.no_call();
}

#[test]
fn responses_explicit_library_limit_cannot_replace_owned_output_capacity() {
    for owned in [false, true] {
        let mut f = Fixture::new();
        if owned { f.state = crate::operation_ownership::with_owned_operations(f.state, 3).unwrap(); }
        let runtime = f.state.runtime().clone();
        drop(f.state);
        let runtime = std::sync::Arc::try_unwrap(runtime).ok().unwrap();
        let configured = runtime.with_response_output_limit(0);
        if owned {
            assert!(matches!(configured, Err(crate::operation_ownership::OwnershipConfigError::AlreadyConfigured)));
        } else { assert!(configured.is_ok()); }
    }
}

#[tokio::test]
async fn responses_producer_panic_fences_with_retained_bytes_until_disposal() {
    use tower::ServiceExt;
    let mut f = Fixture::new();
    f.state = crate::operation_ownership::with_owned_operations(f.state, 3).unwrap();
    let scope = f.state.operation_ownership.clone().unwrap();
    let response = crate::router(f.state.clone(), "/nonexistent-fixture-web")
        .oneshot(owned_request(&f)).await.unwrap();
    let call = f.next().await;
    call.reply.send(Ok(success("messages"))).ok().unwrap();
    let mut body = response.into_body().into_data_stream();
    let mut retained = None;
    while let Some(frame) = tokio::time::timeout(WAIT, body.next()).await.unwrap() {
        let bytes = frame.unwrap();
        if String::from_utf8_lossy(&bytes).contains("response.failed") { retained = Some(bytes.clone()); }
    }
    assert!(retained.is_some());
    tokio::time::timeout(WAIT, async {
        while scope.accepting_requests() { tokio::task::yield_now().await; }
    }).await.unwrap();
    assert!(matches!(scope.external_cleanup(), Err(framer_core::operation_gate::GateError::Unconfirmed)));
    assert_eq!(f.state.response_output.counts().0, 1);
    drop(body); drop(retained);
    tokio::time::timeout(WAIT, async {
        while f.state.response_output.counts() != (0, 0, 0) { tokio::task::yield_now().await; }
    }).await.unwrap();
    assert!(!scope.accepting_requests());
    f.no_call();
}

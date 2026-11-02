//! Actual metered routes with inert ports. Runtime qualification is owner-held.
//! Prepared against b1f47cbb; parent wiring/seams: Server 6a6073c7.
//! All compilation and execution remain UNRUN in this source lane.
use super::*;
use axum::{body::Body, http::{Request, StatusCode}, response::Response};
use crate::operation_ownership::ServingOwnership;
use framer_core::operation_gate::{GateError, OperationGate};
use tower::ServiceExt;

const WAIT: Duration = Duration::from_secs(2);
const UNKNOWN: &str = "account browser cleanup is unconfirmed; retry after cleanup";
const DIALECTS: [&str; 3] = ["anthropic-messages", "openai-responses", "openrouter-chat"];

fn request(dialect: &str, header: bool, stream: bool, account: AccountId) -> Request<Body> {
    let route = match dialect {
        "anthropic-messages" => "/anthropic/v1/messages",
        "openai-responses" => "/openai/v1/responses",
        _ => "/openai/v1/chat/completions",
    };
    let path = if header { route.to_owned() } else { format!("/paid{route}") };
    let mut builder = Request::builder().method("POST").uri(path)
        .header("content-type", "application/json")
        .header("x-framerize-account", account.0.to_string())
        .header("x-framerize-session", "paid-fixture-chat");
    if header { builder = builder.header("x-framerize-mode", "paid"); }
    builder.body(Body::from(json!({"model":"fixture", "stream":stream,
        "input":"fixture", "messages":[{"role":"user","content":"fixture"}]}).to_string())).unwrap()
}

// Regression for the uncancellable first paid permit at ff723aaa:lib.rs:498.
// Requires the Server-owned correction; this source lane does not execute it.
#[tokio::test(flavor = "current_thread")]
async fn metered_first_permit_disconnect_drains_before_permit_release() {
    for dialect in DIALECTS { for header in [false, true] { for stream in [false, true] {
        let (mut state, ports, scope, gate) = setup().await;
        state.runtime_mut().expect("unique fixture runtime").gate = Arc::new(framer_core::gate::ConcurrencyGate::with_config(
            framer_core::gate::GateConfig { initial: 1, min: 1, max: 1,
                recover_ms: 100_000, relax_ms: 100_000, min_interval_ms: 0 }));
        let key = format!("team-1:{}", framer_core::convert::to_openrouter_id("fixture"));
        let held = state.gate.acquire(&key).await;
        let before = records(&ports);
        ports.pause_usage.store(true, Ordering::SeqCst);
        let app = crate::router(state.clone(), "/fixture-no-web");
        let waiter = tokio::spawn(app.oneshot(request(dialect, header, stream, id(1))));
        until(|| ports.usage_captured.load(Ordering::SeqCst) == 1).await;
        assert_eq!(ports.request_started.load(Ordering::SeqCst), 0);
        ports.usage_gate.add_permits(1);
        // On this current-thread fixture, the synchronous upsert returns before
        // the handler yields at its occupied model permit. No new port seam.
        until(|| ports.account_writes.load(Ordering::SeqCst) == 1).await;
        assert!(!waiter.is_finished());
        assert_eq!(gate.stats().unwrap().active, 2);
        assert_eq!(slot(&state, id(1)).users, 1);
        assert!(!slot(&state, id(1)).uncertain);
        assert!(state.gate.acquire_now(&key).await.is_none());
        waiter.abort();
        let error = tokio::time::timeout(WAIT, waiter).await.unwrap().unwrap_err();
        assert!(error.is_cancelled());
        // Drain the application owner before releasing the test's A permit.
        finished(&state, &scope, &gate, false, false).await;
        assert_eq!(records(&ports), before);
        assert!(ports.proxy_calls.lock().unwrap().is_empty());
        assert_eq!(ports.request_started.load(Ordering::SeqCst), 0);
        assert_eq!(ports.request_steps.load(Ordering::SeqCst), 0);
        assert_eq!(ports.request_drops.load(Ordering::SeqCst), 0);
        assert_eq!(ports.response_drops.load(Ordering::SeqCst), 0);
        assert_eq!(ports.account_writes.load(Ordering::SeqCst), 1);
        assert_eq!(ports.mint_routes.lock().unwrap().iter().map(|entry| entry.0)
            .collect::<Vec<_>>(), vec![id(1)]);
        assert!(state.gate.acquire_now(&key).await.is_none());
        *ports.proxy_chunks.lock().unwrap() = Some(chunks(dialect));
        ports.request_gate.add_permits(2);
        ports.response_gate.add_permits(2);
        let response = send(&state, request(dialect, header, stream, id(2))).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()["x-framerize-session"], "paid-fixture-chat");
        let bytes = tokio::time::timeout(WAIT, to_bytes(response.into_body(), 65536))
            .await.unwrap().unwrap();
        if stream { assert_eq!(bytes.as_ref(), chunks(dialect).concat().as_slice()); }
        else { assert_eq!(serde_json::from_slice::<Value>(&bytes).unwrap(), outcome(dialect)); }
        until(|| gate.stats().unwrap().active == 1).await;
        assert!(!gate.stats().unwrap().unconfirmed);
        let resident = slot(&state, id(2));
        assert_eq!(resident.users, 0);
        assert!(!resident.uncertain);
        assert_eq!(state.serving_accounts.residents.snapshot().unwrap().occupied, 1);
        let mut expected = before;
        expected[1]["credit"] = json!({"available":1000.0,"funds":1,"grants":1,"allocated":1000.0});
        assert_eq!(records(&ports), expected);
        assert_eq!(ports.account_writes.load(Ordering::SeqCst), 2);
        assert_eq!(*ports.proxy_calls.lock().unwrap(), vec![(FramerUserId("user-2".into()),
            id(2), ProjectId("project-2".into()), dialect.into(), "paid-fixture-chat".into())]);
        assert_eq!(ports.request_started.load(Ordering::SeqCst), 1);
        assert_eq!(ports.request_finished.load(Ordering::SeqCst), 1);
        assert_eq!(ports.request_steps.load(Ordering::SeqCst), 2);
        assert_eq!(ports.request_drops.load(Ordering::SeqCst), 1);
        assert_eq!(ports.response_drops.load(Ordering::SeqCst), 1);
        assert_eq!(ports.setting_writes.load(Ordering::SeqCst), 0);
        assert_eq!(ports.mint_routes.lock().unwrap().iter().map(|entry| entry.0)
            .collect::<Vec<_>>(), vec![id(1), id(2)]);
        assert_eq!(ports.max_live.load(Ordering::SeqCst), 1);
        assert_eq!(ports.live.lock().unwrap().len(), 1);
        assert!(ports.live.lock().unwrap().contains_key(&id(2)));
        assert_eq!(ports.events.lock().unwrap().iter()
            .filter(|event| **event == ("close_confirmed", id(1))).count(), 1);
        let mut ticket = scope.seal().unwrap();
        let proof = tokio::time::timeout(WAIT, ticket.wait_application()).await.unwrap().unwrap();
        scope.check_application_drained(&proof).unwrap();
        drop(proof);
        {
            let waiting = ticket.wait();
            tokio::pin!(waiting);
            assert!(futures::poll!(waiting.as_mut()).is_pending());
        }
        ticket.cancel_before_control().unwrap();
        assert_eq!(gate.stats().unwrap().active, 1);
        assert!(!gate.stats().unwrap().unconfirmed);
        assert!(state.gate.acquire_now(&key).await.is_none());
        drop(held);
    } } }
}

async fn send(state: &AppState, request: Request<Body>) -> Response {
    tokio::time::timeout(WAIT, crate::router(state.clone(), "/fixture-no-web").oneshot(request))
        .await.unwrap().unwrap()
}

async fn setup() -> (AppState, Arc<Ports>, Arc<ServingOwnership>, OperationGate) {
    let (mut state, ports) = fixture(1, 2);
    ports.records.lock().unwrap()[0].credit = Some(CreditSnapshot {
        available: 1000.0, funds: 1, grants: 1, allocated: 1000.0,
    });
    activated(&state, 1).await;
    let (scope, gate) = ServingOwnership::new_with_host_factory(
        state.wave.clone(), 32, |gate| gate).unwrap();
    state.runtime_mut().expect("unique fixture runtime").operation_ownership = Some(scope.clone());
    (state, ports, scope, gate)
}

fn outcome(dialect: &str) -> Value {
    match dialect {
        "anthropic-messages" => json!({"id":"paid-message","type":"message",
            "role":"assistant","content":[{"type":"text","text":"paid result"}]}),
        "openai-responses" => json!({"id":"paid-response","status":"completed",
            "output":[{"type":"message","content":[{"type":"output_text","text":"paid result"}]}]}),
        _ => json!({"id":"paid-chat","object":"chat.completion","created":1,"model":"fixture",
            "choices":[{"index":0,"finish_reason":"stop",
                "message":{"role":"assistant","content":"paid result"}}],
            "usage":{"prompt_tokens":1,"completion_tokens":2}}),
    }
}

fn event(value: Value) -> Bytes { Bytes::from(format!("data: {value}\n\n")) }

fn chunks(dialect: &str) -> [Bytes; 2] {
    match dialect {
        "anthropic-messages" => [event(json!({"type":"message_start",
            "message":{"id":"paid-message","type":"message","role":"assistant","content":[]}})),
            event(json!({"type":"content_block_delta","delta":{"type":"text_delta","text":"paid result"}}))],
        "openai-responses" => [event(json!({"type":"response.output_text.delta","delta":"paid result"})),
            event(json!({"type":"response.completed","response":outcome(dialect)}))],
        _ => [event(json!({"id":"paid-chat","created":1,"model":"fixture",
            "choices":[{"delta":{"content":"paid result"},"finish_reason":null}]})),
            event(json!({"choices":[{"delta":{},"finish_reason":"stop"}],
                "usage":{"prompt_tokens":1,"completion_tokens":2}}))],
    }
}

fn attempt(ports: &Ports, before: &Value, dialect: &str, bodies: usize) {
    assert_eq!(records(ports), *before);
    assert_eq!(ports.request_started.load(Ordering::SeqCst), 1);
    assert_eq!(ports.request_steps.load(Ordering::SeqCst), 2);
    assert_eq!(ports.request_finished.load(Ordering::SeqCst), 1);
    assert_eq!(ports.request_drops.load(Ordering::SeqCst), 1);
    assert_eq!(ports.response_drops.load(Ordering::SeqCst), bodies);
    assert_eq!(*ports.proxy_calls.lock().unwrap(), vec![(FramerUserId("user-1".into()),
        id(1), ProjectId("project-1".into()), dialect.into(), "paid-fixture-chat".into())]);
    assert_eq!(ports.mint_routes.lock().unwrap().iter().map(|entry| entry.0)
        .collect::<Vec<_>>(), vec![id(1), id(1)]);
    assert_eq!(ports.account_writes.load(Ordering::SeqCst), 1); // Refreshed credit snapshot.
    assert_eq!(ports.setting_writes.load(Ordering::SeqCst), 0);
    assert_eq!(ports.max_live.load(Ordering::SeqCst), 1);
}

async fn finished(state: &AppState, scope: &ServingOwnership, gate: &OperationGate,
    global_unknown: bool, resident_unknown: bool) {
    until(|| gate.stats().unwrap().active == 1).await;
    assert_eq!(gate.stats().unwrap().unconfirmed, global_unknown);
    assert_eq!(slot(state, id(1)).users, 0);
    assert_eq!(slot(state, id(1)).uncertain, resident_unknown);
    assert_eq!(state.serving_accounts.residents.snapshot().unwrap().occupied, 1);
    assert!(scope.unmanaged_host_close_required());
    if global_unknown { assert!(matches!(scope.seal(), Err(GateError::Unconfirmed))); }
    else {
        let mut ticket = scope.seal().unwrap();
        let proof = tokio::time::timeout(WAIT, ticket.wait_application()).await.unwrap().unwrap();
        scope.check_application_drained(&proof).unwrap();
        drop(proof);
        {
            let wait = ticket.wait();
            tokio::pin!(wait);
            assert!(futures::poll!(wait.as_mut()).is_pending());
        }
        ticket.cancel_before_control().unwrap();
    }
    let response = send(state, Request::builder().uri("/api/hello").body(Body::empty()).unwrap()).await;
    assert_eq!(response.status(), if global_unknown { StatusCode::SERVICE_UNAVAILABLE } else { StatusCode::OK });
    let expected = if global_unknown { format!("Cannot start this request: {}.", gate.error_explanation(GateError::Unconfirmed)) } else { "ok".into() };
    assert_eq!(to_bytes(response.into_body(), 1024).await.unwrap().as_ref(), expected.as_bytes());
    assert_eq!(gate.stats().unwrap().active, 1);
    assert_eq!(gate.stats().unwrap().unconfirmed, global_unknown);
}

#[tokio::test]
async fn metered_aggregate_success_completes_consumed_and_discarded_output() {
    for dialect in DIALECTS { for header in [false, true] { for consume in [false, true] {
        let (state, ports, scope, gate) = setup().await;
        let before = records(&ports);
        *ports.proxy_chunks.lock().unwrap() = Some(chunks(dialect));
        ports.request_gate.add_permits(2);
        ports.response_gate.add_permits(2);
        let response = send(&state, request(dialect, header, false, id(1))).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()["content-type"], "application/json");
        assert_eq!(response.headers()["x-framerize-session"], "paid-fixture-chat");
        if consume {
            let bytes = to_bytes(response.into_body(), 65536).await.unwrap();
            assert_eq!(serde_json::from_slice::<Value>(&bytes).unwrap(), outcome(dialect));
        } else { drop(response); }
        finished(&state, &scope, &gate, false, false).await;
        attempt(&ports, &before, dialect, 1);
    } } }
}

#[tokio::test]
async fn metered_raw_eof_preserves_bytes_and_releases_owners() {
    for dialect in DIALECTS { for header in [false, true] { for stream in [false, true] {
        let (state, ports, scope, gate) = setup().await;
        let before = records(&ports);
        ports.plain_json.store(!stream, Ordering::SeqCst);
        let expected = if stream { chunks(dialect).concat() } else { br#"{"fixture":true}"#.to_vec() };
        if stream { *ports.proxy_chunks.lock().unwrap() = Some(chunks(dialect)); }
        ports.request_gate.add_permits(2);
        ports.response_gate.add_permits(2);
        let response = send(&state, request(dialect, header, stream, id(1))).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()["content-type"],
            if stream { "text/event-stream" } else { "application/json" });
        let bytes = tokio::time::timeout(WAIT, to_bytes(response.into_body(), 65536))
            .await.unwrap().unwrap();
        assert_eq!(bytes.as_ref(), expected.as_slice());
        finished(&state, &scope, &gate, false, false).await;
        attempt(&ports, &before, dialect, 1);
    } } }
}

#[tokio::test]
async fn metered_raw_disposal_keeps_unknown_completion_fenced() {
    for dialect in DIALECTS { for header in [false, true] {
        for stream in [false, true] { for partial in [false, true] {
            let (state, ports, scope, gate) = setup().await;
            let before = records(&ports);
            ports.plain_json.store(!stream, Ordering::SeqCst);
            if stream { *ports.proxy_chunks.lock().unwrap() = Some(chunks(dialect)); }
            ports.request_gate.add_permits(2);
            let response = send(&state, request(dialect, header, stream, id(1))).await;
            assert_eq!(response.status(), StatusCode::OK);
            until(|| ports.request_drops.load(Ordering::SeqCst) == 1).await;
            assert!(gate.stats().unwrap().active > 1);
            assert!(slot(&state, id(1)).users > 0);
            let mut body = response.into_body().into_data_stream();
            if partial {
                ports.response_gate.add_permits(1);
                let first = tokio::time::timeout(WAIT, body.next()).await.unwrap().unwrap().unwrap();
                let expected = if stream { chunks(dialect)[0].clone() } else { Bytes::from_static(b"{\"fixture\":") };
                assert_eq!(first, expected);
            }
            drop(body);
            until(|| ports.response_drops.load(Ordering::SeqCst) == 1).await;
            // No host cleanup verdict exists merely because the raw body was dropped.
            finished(&state, &scope, &gate, true, false).await;
            attempt(&ports, &before, dialect, 1);
        } }
    } }
}

#[tokio::test]
async fn metered_preheader_errors_preserve_result_and_cleanup_verdict() {
    for dialect in DIALECTS { for header in [false, true] { for stream in [false, true] {
        for unknown in [false, true] { for consume in [false, true] {
            let (state, ports, scope, gate) = setup().await;
            let before = records(&ports);
            let error = if unknown { UNKNOWN } else { "fixture proxy refused" };
            *ports.proxy_error.lock().unwrap() = Some(error.into());
            ports.request_gate.add_permits(2);
            let response = send(&state, request(dialect, header, stream, id(1))).await;
            until(|| ports.request_drops.load(Ordering::SeqCst) == 1).await;
            if stream { until(|| gate.stats().unwrap().active == 2).await; }
            let expected = if stream { StatusCode::OK }
                else if unknown { StatusCode::SERVICE_UNAVAILABLE }
                else { StatusCode::INTERNAL_SERVER_ERROR };
            assert_eq!(response.status(), expected);
            if consume {
                let bytes = tokio::time::timeout(WAIT, to_bytes(response.into_body(), 65536))
                    .await.unwrap().unwrap();
                let wire = std::str::from_utf8(&bytes).unwrap();
                let body: Value = if stream {
                    let data = wire.lines().find_map(|line| line.strip_prefix("data: ")).unwrap();
                    serde_json::from_str(data).unwrap()
                } else { serde_json::from_slice(&bytes).unwrap() };
                let error = body.get("error").or_else(|| body.pointer("/response/error")).unwrap();
                assert_eq!(error["type"], if unknown { "overloaded_error" } else { "api_error" });
                let message = error["message"].as_str().unwrap();
                if unknown { assert!(message.contains("cleanup")); }
                else { assert_eq!(message, "fixture proxy refused"); }
            } else { drop(response); }
            // Discarding the terminal error body retains paid cancellation semantics.
            finished(&state, &scope, &gate, unknown || (stream && !consume), unknown).await;
            attempt(&ports, &before, dialect, 0);
        } }
    } } }
}

#[tokio::test]
async fn metered_body_error_retains_partial_output_and_cleanup_fence() {
    for dialect in DIALECTS { for header in [false, true] { for stream in [false, true] {
        let (state, ports, scope, gate) = setup().await;
        let before = records(&ports);
        *ports.proxy_chunks.lock().unwrap() = Some(chunks(dialect));
        ports.cleanup_after_content.store(true, Ordering::SeqCst);
        ports.request_gate.add_permits(2);
        ports.response_gate.add_permits(2);
        let response = send(&state, request(dialect, header, stream, id(1))).await;
        assert_eq!(response.status(), if stream { StatusCode::OK } else { StatusCode::INTERNAL_SERVER_ERROR });
        if stream {
            let mut body = response.into_body().into_data_stream();
            assert_eq!(tokio::time::timeout(WAIT, body.next()).await.unwrap()
                .unwrap().unwrap(), chunks(dialect)[0]);
            let error = tokio::time::timeout(WAIT, body.next()).await.unwrap().unwrap().unwrap_err();
            assert!(error.to_string().contains(UNKNOWN));
            drop(body);
        } else {
            assert_eq!(to_bytes(response.into_body(), 65536).await.unwrap().as_ref(), UNKNOWN.as_bytes());
        }
        finished(&state, &scope, &gate, true, true).await;
        attempt(&ports, &before, dialect, 1);
    } } }
}

#[tokio::test]
async fn metered_acquisition_refusal_keeps_original_error_and_close_evidence() {
    for dialect in DIALECTS { for header in [false, true] { for stream in [false, true] {
        for mode in [CloseMode::Error, CloseMode::Unconfirmed, CloseMode::Busy] {
            for consume in [false, true] {
                let (state, ports, scope, gate) = setup().await;
                let before = records(&ports);
                let busy = matches!(mode, CloseMode::Busy);
                if busy { *ports.status_mode.lock().unwrap() = StatusMode::Busy; }
                else { *ports.close_mode.lock().unwrap() = mode; }
                let response = send(&state, request(dialect, header, stream, id(2))).await;
                assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
                assert_eq!(response.headers()["retry-after"], "1");
                if consume {
                    let bytes = to_bytes(response.into_body(), 65536).await.unwrap();
                    let message = match mode {
                        CloseMode::Error => "fixture close failure",
                        CloseMode::Unconfirmed => "the browser reported state Absent with 0 active commands; browser close returned Unconfirmed and its occupied slot is retained",
                        _ => "account browser is still busy or its identity changed",
                    };
                    assert_eq!(serde_json::from_slice::<Value>(&bytes).unwrap(), json!({"error":{
                        "code":"framerize_pool_busy","message":message,"retry_after_seconds":1}}));
                } else { drop(response); }
                finished(&state, &scope, &gate, !busy, !busy).await;
                assert_eq!(records(&ports), before);
                assert!(ports.proxy_calls.lock().unwrap().is_empty());
                assert_eq!(ports.request_started.load(Ordering::SeqCst), 0);
                assert_eq!(ports.request_drops.load(Ordering::SeqCst), 0);
                assert_eq!(ports.response_drops.load(Ordering::SeqCst), 0);
                assert_eq!(ports.account_writes.load(Ordering::SeqCst), 0);
                assert_eq!(ports.setting_writes.load(Ordering::SeqCst), 0);
                assert_eq!(ports.mint_routes.lock().unwrap().iter().map(|entry| entry.0)
                    .collect::<Vec<_>>(), vec![id(1)]);
                assert_eq!(ports.events.lock().unwrap().iter()
                    .filter(|event| event.0 == "close_started").count(), usize::from(!busy));
                assert_eq!(ports.max_live.load(Ordering::SeqCst), 1);
            }
        }
    } } }
}

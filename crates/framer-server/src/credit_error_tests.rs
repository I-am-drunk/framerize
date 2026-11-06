use super::*;
use axum::{body::Body, http::Request};
use crate::response_output::consume_body as to_bytes;
use tower::ServiceExt;

#[path = "permit_lifetime_fixture.rs"]
mod fixture;

#[tokio::test]
async fn local_credit_floor_reports_the_balance_without_a_retryable_server_error() {
    for available in [0.0, 12.5] {
        let error = ApiError::InsufficientCredits { available, floor: 25.0 };
        let response = axum::response::IntoResponse::into_response(error);
        assert_eq!(response.status(), StatusCode::PAYMENT_REQUIRED);
        assert_eq!(response.headers()["x-should-retry"], "false");
        let body: Value = serde_json::from_slice(&to_bytes(response.into_body(), 8192).await.unwrap()).unwrap();
        let message = body["error"]["message"].as_str().unwrap();
        assert!(message.contains(&format!("{available:.2}")) && message.contains("25.00"));
        assert!(message.contains("same workspace"));
        assert!(!message.contains("free model"));
    }
}

async fn check_credit_refusal(route: &str, paid: bool, streaming: bool) {
    let mut f = fixture::Fixture::new();
    let model = if route.starts_with("/anthropic") { "claude-fable-5-1" } else { "gpt-4.1" };
    let mut payload = serde_json::json!({"model":model, "stream":streaming,
        "max_tokens":256, "max_output_tokens":256});
    if route.ends_with("responses") { payload["input"] = "fixture".into(); }
    else { payload["messages"] = serde_json::json!([{"role":"user", "content":"fixture"}]); }
    let mut request = Request::builder().method("POST").uri(route)
        .header("content-type", "application/json")
        .header("x-framerize-account", f.accounts[0].id.0.to_string());
    if paid { request = request.header("x-framerize-mode", "paid"); }
    let app = crate::router(f.state.clone(), "/nonexistent-fixture-web");
    let task = tokio::spawn(app.oneshot(request.body(Body::from(payload.to_string())).unwrap()));
    let call = f.next().await;
    call.reply.send(Err("credits-exhausted".into())).ok().unwrap();
    let response = tokio::time::timeout(std::time::Duration::from_secs(3), task)
        .await.unwrap().unwrap().unwrap();
    assert_eq!(response.status().as_u16(), if streaming { 200 } else { 402 }, "{route} paid={paid}");
    if !streaming { assert_eq!(response.headers()["x-should-retry"], "false"); }
    let bytes = tokio::time::timeout(std::time::Duration::from_secs(3),
        to_bytes(response.into_body(), 65536)).await
        .unwrap_or_else(|_| panic!("body stalled: {route} paid={paid} streaming={streaming}"))
        .unwrap();
    let wire = std::str::from_utf8(&bytes).unwrap();
    let error = if streaming {
        wire.lines().filter_map(|line| line.strip_prefix("data: "))
            .filter_map(|data| serde_json::from_str::<Value>(data).ok())
            .find_map(|event| event.get("error").or_else(|| event.pointer("/response/error")).cloned())
            .expect("stream ended without a machine-readable error")
    } else { serde_json::from_slice::<Value>(&bytes).unwrap()["error"].clone() };
    assert_eq!(error["code"], "insufficient_quota");
    assert_eq!(error["retryable"], false);
    assert!(error["message"].as_str().unwrap().contains("workspace has no AI credits"));
    assert!(!wire.contains("response.completed"));
    for original in &f.accounts {
        let after = f.state.state.get(original.id).await.unwrap().unwrap();
        assert_eq!(after.state, original.state);
        assert_eq!(after.framer_user_id, original.framer_user_id);
        assert_eq!(after.castle_device_id, original.castle_device_id);
        assert_eq!(after.proxy, original.proxy);
        assert_eq!(after.cooling_until_ms, original.cooling_until_ms);
    }
    f.no_call();
}

#[tokio::test]
async fn pinned_exhausted_workspace_is_reported_without_retrying_another_account() {
    for route in ["/anthropic/v1/messages", "/openai/v1/responses", "/openai/v1/chat/completions"] {
        for paid in [false, true] {
            for streaming in [false, true] {
                check_credit_refusal(route, paid, streaming).await;
            }
        }
    }
}

#[tokio::test]
async fn buffered_custom_responses_route_retries_credit_exhaustion_on_another_account() {
    let mut f = fixture::Fixture::new();
    let payload = serde_json::json!({"model":"gpt-4.1", "stream":false, "input":"fixture"});
    let app = crate::router(f.state.clone(), "/nonexistent-fixture-web");
    let task = tokio::spawn(app.oneshot(Request::builder().method("POST")
        .uri("/openai/v1/responses").header("content-type", "application/json")
        .body(Body::from(payload.to_string())).unwrap()));

    let first = f.next().await;
    assert_eq!(first.kind, "responses");
    let first_account = first.account.clone();
    first.reply.send(Err("credits-exhausted".into())).ok().unwrap();

    let second = f.next().await;
    assert_eq!(second.kind, "responses");
    assert_ne!(second.account, first_account, "retry must select a different account");
    second.reply.send(Ok(fixture::success("responses"))).ok().unwrap();

    let response = tokio::time::timeout(std::time::Duration::from_secs(3), task)
        .await.unwrap().unwrap().unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), 65536).await.unwrap();
    let body: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(body["status"], "completed");
    assert!(body.to_string().contains("fixture"));
    f.no_call();
}

#[tokio::test]
async fn anthropic_stream_retries_credit_exhaustion_before_publishing_content() {
    let mut f = fixture::Fixture::new();
    let payload = serde_json::json!({"model":"claude-fable-5-1", "stream":true,
        "max_tokens":256, "messages":[{"role":"user", "content":"fixture"}]});
    let app = crate::router(f.state.clone(), "/nonexistent-fixture-web");
    let task = tokio::spawn(app.oneshot(Request::builder().method("POST")
        .uri("/anthropic/v1/messages").header("content-type", "application/json")
        .body(Body::from(payload.to_string())).unwrap()));

    let first = f.next().await;
    assert_eq!(first.kind, "messages");
    let first_account = first.account.clone();
    first.reply.send(Ok(fixture::Reply::MessagesFailure {
        chunks: vec![], error: "credits-exhausted".into(),
    })).ok().unwrap();

    let second = f.next().await;
    assert_eq!(second.kind, "messages");
    assert_ne!(second.account, first_account, "retry must select a different account");
    second.reply.send(Ok(fixture::success("messages"))).ok().unwrap();

    let response = tokio::time::timeout(std::time::Duration::from_secs(3), task)
        .await.unwrap().unwrap().unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), 65536).await.unwrap();
    let wire = std::str::from_utf8(&bytes).unwrap();
    assert!(wire.contains("message_stop"));
    assert!(!wire.contains("insufficient_quota"));
    f.no_call();
}

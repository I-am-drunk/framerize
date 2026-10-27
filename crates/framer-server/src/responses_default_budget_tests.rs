use axum::{body::Body, http::{Request, StatusCode}};
use futures::StreamExt;
use serde_json::{json, Value};
use std::time::Duration;
use tower::ServiceExt;

#[path = "permit_lifetime_fixture.rs"]
mod fixture;
const WAIT: Duration = Duration::from_secs(3);
const RESPONSES: &str = "/openai/v1/responses";

async fn check(path: &str, stream: bool, limits: Value, expected: Option<u32>) {
    let mut f = fixture::Fixture::new();
    f.state = crate::operation_ownership::with_owned_operations(f.state, 8).unwrap();
    let owner = f.state.operation_ownership.clone().unwrap();
    let mut payload = json!({"model":"gpt-6-astra", "input":"fixture",
        "messages":[{"role":"user", "content":"fixture"}], "stream":stream});
    payload.as_object_mut().unwrap().extend(limits.as_object().unwrap().clone());
    let request = Request::builder().method("POST").uri(path)
        .header("content-type", "application/json")
        .header("x-framerize-account", f.accounts[0].id.0.to_string())
        .body(Body::from(payload.to_string())).unwrap();
    let app = crate::router(f.state.clone(), "/nonexistent-fixture-web");
    let work = tokio::spawn(async move {
        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let mut body = response.into_body().into_data_stream();
        while let Some(bytes) = body.next().await { drop(bytes.unwrap()); }
    });
    let call = f.next().await;
    let config = call.config.unwrap();
    call.reply.send(Ok(fixture::success(call.kind))).ok().unwrap();
    tokio::time::timeout(WAIT, call.dropped).await.unwrap().unwrap();
    tokio::time::timeout(WAIT, work).await.unwrap().unwrap();
    f.no_call();
    assert!(owner.accepting_requests());
    let mut ticket = owner.seal().unwrap();
    let proof = tokio::time::timeout(WAIT, ticket.wait_application()).await.unwrap().unwrap();
    owner.check_application_drained(&proof).unwrap();
    drop(proof);
    ticket.cancel_before_control().unwrap();
    assert_eq!(f.state.response_output.counts(), (0, 0, 0));
    assert_eq!(config.max_chunks, 16);
    assert_eq!(config.chunk_tokens, 600);
    assert!(config.cancelled.is_some());
    assert_eq!(config.max_total_tokens, expected);
}

#[tokio::test]
async fn responses_json_without_limit_does_not_invent_output_cap() {
    check(RESPONSES, false, json!({}), None).await;
}

#[tokio::test]
async fn responses_stream_without_limit_does_not_invent_output_cap() {
    check(RESPONSES, true, json!({}), None).await;
}

#[tokio::test]
async fn responses_explicit_limits_keep_alias_precedence() {
    for stream in [false, true] {
        for (limits, cap) in [
            (json!({"max_output_tokens":4096}), 4096),
            (json!({"max_output_tokens":12000}), 12000),
            (json!({"max_completion_tokens":31,"max_output_tokens":47}), 31),
            (json!({"max_tokens":17,"max_completion_tokens":31,"max_output_tokens":47}),17),
            (json!({"max_tokens":null,"max_completion_tokens":"bad","max_output_tokens":53}),53),
        ] { check(RESPONSES, stream, limits, Some(cap)).await; }
    }
}

#[tokio::test]
async fn responses_non_numeric_limits_do_not_invent_output_cap() {
    for value in [Value::Null, json!("32"), json!(-1), json!(12.5), json!({})] {
        for stream in [false, true] {
            check(RESPONSES, stream, json!({"max_tokens":value.clone(),
                "max_completion_tokens":value.clone(),"max_output_tokens":value}),None).await;
        }
    }
}

#[tokio::test]
async fn other_dialects_keep_the_existing_default() {
    for path in ["/openai/v1/chat/completions", "/anthropic/v1/messages"] {
        for stream in [false, true] {
            check(path, stream, json!({}), Some(4096)).await;
        }
    }
}

#[tokio::test]
async fn oversized_responses_limit_is_rejected_before_ports() {
    for stream in [false, true] {
        let mut f = fixture::Fixture::new();
        let body = json!({"model":"gpt-6-astra", "input":"fixture", "stream":stream,
            "max_output_tokens":u64::from(u32::MAX)+1});
        let request = Request::builder().method("POST").uri(RESPONSES)
            .header("content-type", "application/json")
            .body(Body::from(body.to_string())).unwrap();
        let response = crate::router(f.state.clone(), "/nonexistent-fixture-web")
            .oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let bytes = axum::body::to_bytes(response.into_body(), 4096).await.unwrap();
        let value: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(value["error"]["message"], "output token limit exceeds 4294967295");
        f.no_call();
        assert!(f.loads.try_recv().is_err());
    }
}

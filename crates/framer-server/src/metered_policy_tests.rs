use super::*;
use axum::{body::Body, http::Request};
use crate::response_output::consume_body;
use tower::ServiceExt;

#[path = "permit_lifetime_fixture.rs"]
mod fixture;

const ROUTES: [&str; 3] = ["/anthropic/v1/messages",
    "/openai/v1/responses", "/openai/v1/chat/completions"];

#[tokio::test]
async fn disabled_metering_also_blocks_the_direct_agent_endpoint() {
    let mut f = fixture::Fixture::new();
    f.state.runtime_mut().unwrap().metered_access = false;
    let body = serde_json::json!({"account_id":f.accounts[0].id,
        "prompt":"fixture", "model":"gpt-4.1"});
    let request = Request::builder().method("POST").uri("/api/run")
        .header("content-type", "application/json")
        .body(Body::from(body.to_string())).unwrap();
    let response = router(f.state.clone(), "/missing-web").oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert_eq!(response.headers()["x-should-retry"], "false");
    f.no_call();
    assert!(f.loads.try_recv().is_err());
}

#[tokio::test]
async fn custom_route_rejects_a_metered_only_model_without_changing_transport() {
    for allowed in [false, true] {
        let mut f = fixture::Fixture::new();
        f.state.runtime_mut().unwrap().metered_access = allowed;
        let request = Request::builder().method("POST").uri(ROUTES[1])
            .header("content-type", "application/json")
            .body(Body::from(r#"{"model":"gpt-6-sol","input":"fixture"}"#)).unwrap();
        let response = router(f.state.clone(), "/missing-web").oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let bytes = consume_body(response.into_body(), 8192).await.unwrap();
        let body: Value = serde_json::from_slice(&bytes).unwrap();
        assert!(body["error"]["message"].as_str().unwrap().contains("metered upstream route"));
        f.no_call();
        assert!(f.loads.try_recv().is_err());
    }
}

#[tokio::test]
async fn disabled_paid_paths_and_header_overrides_refuse_before_account_access() {
    for route in ROUTES { for streaming in [false, true] { for prefix in [false, true] {
        let mut f = fixture::Fixture::new();
        f.state.runtime_mut().unwrap().metered_access = false;
        let path = if prefix { format!("/paid{route}") } else { route.into() };
        let mut request = Request::builder().method("POST").uri(path)
            .header("content-type", "application/json");
        if !prefix { request = request.header("x-framerize-mode", "paid"); }
        let response = router(f.state.clone(), "/missing-web").oneshot(
            request.body(Body::from(payload(route, streaming).to_string())).unwrap()).await.unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        assert_eq!(response.headers()["x-should-retry"], "false");
        let bytes = consume_body(response.into_body(), 8192).await.unwrap();
        let body: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(body["error"]["code"], "metered_access_disabled");
        f.no_call();
        assert!(f.loads.try_recv().is_err());
    } } }
}

#[tokio::test]
async fn custom_routes_never_enable_paid_fallback_even_when_paid_access_is_allowed() {
    for route in ROUTES { for streaming in [false, true] { for allowed in [false, true] {
        let mut f = fixture::Fixture::new();
        f.state.runtime_mut().unwrap().metered_access = allowed;
        let request = Request::builder().method("POST").uri(route)
            .header("content-type", "application/json")
            .body(Body::from(payload(route, streaming).to_string())).unwrap();
        let task = tokio::spawn(router(f.state.clone(), "/missing-web").oneshot(request));
        let call = f.next().await;
        assert_ne!(call.kind, "proxy", "{route}");
        assert!(!call.config.as_ref().unwrap().metered_fallback, "{route}");
        call.reply.send(Ok(fixture::success(call.kind))).ok().unwrap();
        let response = task.await.unwrap().unwrap();
        assert_eq!(response.status(), StatusCode::OK, "{route}");
        consume_body(response.into_body(), 65536).await.unwrap();
        f.no_call();
    } } }
}

fn payload(route: &str, streaming: bool) -> Value {
    let mut body = serde_json::json!({"model":"gpt-4.1", "stream":streaming,
        "max_tokens":256, "max_output_tokens":256});
    if route.ends_with("responses") { body["input"] = "fixture".into(); }
    else { body["messages"] = serde_json::json!([{"role":"user", "content":"fixture"}]); }
    if route.starts_with("/anthropic") {
        body["model"] = "claude-fable-5-1".into();
        body["thinking"] = serde_json::json!({"type":"adaptive"});
    }
    body
}

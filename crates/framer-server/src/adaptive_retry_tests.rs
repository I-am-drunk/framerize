use super::*;
use axum::body::{to_bytes, Body};
use axum::http::Request;
use tower::ServiceExt;

#[path = "permit_lifetime_fixture.rs"]
mod fixture;

#[tokio::test]
async fn uncertain_adaptive_completion_tells_the_sdk_not_to_replay() {
    for stop_replay in [false, true] {
        let mut f = fixture::Fixture::new();
        let app = crate::router(f.state.clone(), "/nonexistent-fixture-web");
        let request = Request::builder().method("POST").uri("/anthropic/v1/messages")
            .header("content-type", "application/json")
            .header("x-framerize-account", f.accounts[0].id.0.to_string())
            .body(Body::from(r#"{"model":"claude-fable-5-1","stream":false,"max_tokens":64000,"thinking":{"type":"adaptive"},"messages":[{"role":"user","content":"fixture"}]}"#)).unwrap();
        let task = tokio::spawn(app.oneshot(request));
        let call = f.next().await;
        let error = if stop_replay {
            format!("{}fixture upstream connection closed before completion",
                framer_core::error::ADAPTIVE_INCOMPLETE_PREFIX)
        } else { "fixture transient transport error".into() };
        call.reply.send(Err(error)).ok().unwrap();
        let response = tokio::time::timeout(std::time::Duration::from_secs(2), task)
            .await.unwrap().unwrap().unwrap();
        assert_eq!(response.status().as_u16(), if stop_replay { 502 } else { 500 });
        assert_eq!(response.headers().get("x-should-retry").map(|v| v.to_str().unwrap()),
            if stop_replay { Some("false") } else { None });
        let bytes = to_bytes(response.into_body(), 65536).await.unwrap();
        let body: Value = serde_json::from_slice(&bytes).unwrap();
        if stop_replay {
            assert_eq!(body["error"]["retryable"], false);
            assert!(body["error"]["message"].as_str().unwrap().contains("connection closed"));
        }
        f.no_call();
    }
}

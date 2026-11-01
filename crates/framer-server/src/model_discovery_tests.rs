use axum::{body::{to_bytes, Body}, http::Request};
use serde_json::{json, Value};
use tower::ServiceExt;
#[path = "permit_lifetime_fixture.rs"]
mod fixture;

#[tokio::test]
async fn model_routes_use_catalog_without_loading_a_session_or_browser() {
    let mut f = fixture::Fixture::new();
    let app = crate::router(f.state.clone(), "/nonexistent-fixture-web");
    for prefix in ["", "/paid"] {
        for dialect in ["openai", "anthropic"] {
            let request = Request::builder().uri(format!("{prefix}/{dialect}/v1/models"))
                .body(Body::empty()).unwrap();
            let response = app.clone().oneshot(request).await.unwrap();
            assert_eq!(response.status(), 200);
            let bytes = to_bytes(response.into_body(), 4096).await.unwrap();
            let body: Value = serde_json::from_slice(&bytes).unwrap();
            let ids: Vec<_> = body["data"].as_array().unwrap().iter()
                .map(|model| model["id"].clone()).collect();
            if dialect == "openai" {
                assert_eq!(body["object"], "list");
                assert_eq!(ids, vec![json!("gpt-fixture"), json!("claude-fixture")]);
            } else {
                assert_eq!(ids, vec![json!("claude-fixture")]);
                assert_eq!(body["has_more"], false);
            }
        }
    }
    assert!(f.loads.try_recv().is_err());
    f.no_call();
}

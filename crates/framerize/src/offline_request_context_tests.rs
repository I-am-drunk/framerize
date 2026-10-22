use super::*;
use axum::{body::Body, http::{HeaderMap, HeaderValue, Request, StatusCode},
    routing::post, Router};
use framer_core::{ids::{FramerUserId, ProjectId, TeamId}, native_dispatch::{Identity, Route}};
use tower::ServiceExt;

fn headers(turn: &str) -> HeaderMap {
    let body = serde_json::json!({"page_url":"https://framer.com/projects/project",
        "proxy":null,"attempts":[{"feature_category":"category","feature_name":"feature",
        "step":"step","turn_id":turn,"user_prompt":"original display prompt"}]});
    let mut headers = HeaderMap::new();
    headers.insert("x-framerize-context", HeaderValue::from_str(&body.to_string()).unwrap());
    headers
}

fn preparation() -> Preparation {
    Preparation { identity: Identity { account: AccountId(uuid::Uuid::nil()),
        user: FramerUserId("user".into()), team: TeamId("team".into()),
        project: ProjectId("project".into()), chat_session: "conversation".into() },
        route: Route::Responses, deadline: tokio::time::Instant::now() + std::time::Duration::from_secs(2),
        max_total_tokens: 32 }
}

#[tokio::test]
async fn routing_http_context_capture_survives_the_request_scope() {
    let context = decode(&headers("original-turn")).unwrap();
    let captured = CURRENT.scope(context, async { Current.capture().unwrap().unwrap() }).await;
    assert!(Current.capture().is_err());
    let value = tokio::spawn(async move { captured.resolve(&preparation(), &Value::Null).unwrap() })
        .await.unwrap();
    assert_eq!(value.identity, preparation().identity);
    assert_eq!(value.page_url, "https://framer.com/projects/project");
    assert_eq!(value.attempts[0].turn_id, "original-turn");
    assert_eq!(value.attempts[0].user_prompt, "original display prompt");
}

#[tokio::test]
async fn routing_http_concurrent_contexts_do_not_mix() {
    async fn capture(turn: &'static str) -> String {
        CURRENT.scope(decode(&headers(turn)).unwrap(), async {
            tokio::task::yield_now().await;
            Current.capture().unwrap().unwrap().resolve(&preparation(), &Value::Null)
                .unwrap().attempts.remove(0).turn_id
        }).await
    }
    let (a, b) = tokio::join!(capture("turn-a"), capture("turn-b"));
    assert_eq!((a.as_str(), b.as_str()), ("turn-a", "turn-b"));
    assert!(Current.capture().is_err());
}

#[test]
fn routing_http_context_rejects_missing_duplicate_and_oversized_headers() {
    assert!(decode(&HeaderMap::new()).is_err());
    let mut duplicate = headers("turn");
    duplicate.append("x-framerize-context", duplicate["x-framerize-context"].clone());
    assert!(decode(&duplicate).is_err());
    let mut huge = HeaderMap::new();
    huge.insert("x-framerize-context", HeaderValue::from_str(&" ".repeat(65537)).unwrap());
    assert!(decode(&huge).is_err());
}

#[test]
fn routing_http_context_rejects_unknown_or_incomplete_metadata() {
    for body in [r#"{"page_url":"https://framer.com"}"#,
        r#"{"page_url":"https://framer.com","attempts":[],"paid_credit":100}"#,
        r#"{"page_url":"https://framer.com","attempts":[]}"#] {
        let mut value = HeaderMap::new();
        value.insert("x-framerize-context", HeaderValue::from_str(body).unwrap());
        assert!(decode(&value).is_err());
    }
}

#[tokio::test]
async fn routing_http_missing_context_rejects_before_handler() {
    let app = Router::new().route("/openai/v1/responses", post(|| async {
        panic!("handler must not run without context");
        #[allow(unreachable_code)] StatusCode::OK
    })).layer(axum::middleware::from_fn(middleware));
    let response = app.oneshot(Request::post("/openai/v1/responses")
        .body(Body::empty()).unwrap()).await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn routing_http_all_six_routes_preserve_body_and_remove_private_header() {
    async fn echo(headers: HeaderMap, body: String) -> String {
        assert!(!headers.contains_key("x-framerize-context"));
        assert!(Current.capture().unwrap().is_some());
        body
    }
    for prefix in ["", "/paid"] {
        for suffix in ["/openai/v1/responses", "/openai/v1/chat/completions", "/anthropic/v1/messages"] {
            let path = format!("{prefix}{suffix}");
            let app = Router::new().route(&path, post(echo))
                .layer(axum::middleware::from_fn(middleware));
            let mut request = Request::post(path.as_str()).body(Body::from("exact request bytes")).unwrap();
            *request.headers_mut() = headers("turn");
            let response = app.oneshot(request).await.unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            assert_eq!(axum::body::to_bytes(response.into_body(), 128).await.unwrap(), "exact request bytes");
        }
    }
    assert!(Current.capture().is_err());
}

use super::*;
use serde_json::json;
use tower::ServiceExt;

#[path = "native_dispatch_fixture.rs"]
mod fixture;

const ROUTES: [&str; 6] = [
    "/anthropic/v1/messages", "/openai/v1/responses", "/openai/v1/chat/completions",
    "/paid/anthropic/v1/messages", "/paid/openai/v1/responses", "/paid/openai/v1/chat/completions",
];
const THREAD: &str = "fixture-native-thread";

fn headers() -> HeaderMap {
    let mut h = HeaderMap::new();
    h.insert("thread-id", HeaderValue::from_static(THREAD));
    h.insert("session-id", HeaderValue::from_static(THREAD));
    h
}

fn payload(path: &str) -> Value {
    if path.ends_with("/responses") {
        json!({"model":"gpt-4.1", "stream":false, "input":"fixture"})
    } else {
        json!({"model":"gpt-4.1", "stream":false,
            "messages":[{"role":"user", "content":"fixture"}]})
    }
}

async fn request(state: AppState, path: &str, mut h: HeaderMap, body: Value) -> Response {
    h.insert("content-type", HeaderValue::from_static("application/json"));
    let mut request = axum::http::Request::builder().method("POST").uri(path)
        .body(axum::body::Body::from(body.to_string())).unwrap();
    *request.headers_mut() = h;
    tokio::time::timeout(std::time::Duration::from_secs(3),
        crate::router(state, "/fixture-no-web").oneshot(request)).await.unwrap().unwrap()
}

async fn rejected(path: &str, h: HeaderMap, kind: FailureKind) {
    let (state, spy, provider) = fixture::fixture(true, None);
    let response = request(state, path, h, payload(path)).await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(response.extensions().get::<FailureKind>(), Some(&kind));
    assert_eq!(*provider.preparations.lock().unwrap(), 0);
    assert!(provider.records.lock().unwrap().is_empty());
    assert!(spy.calls.lock().unwrap().is_empty());
}

async fn forwarded(path: &str, h: HeaderMap, body: Value, expected: &str, raw: bool) {
    let (state, spy, provider) = fixture::fixture(true, None);
    let response = request(state, path, h, body).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["x-framerize-session"].to_str().unwrap(), expected);
    assert_eq!(*provider.preparations.lock().unwrap(), 1);
    let records = provider.records.lock().unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].identity.chat_session, expected);
    assert_eq!(matches!(records[0].route, Route::Raw(_)), raw);
    assert!(spy.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn native_context_reaches_all_six_handlers_paid_override_and_direct_model() {
    for path in ROUTES {
        forwarded(path, headers(), payload(path), THREAD, path.starts_with("/paid/")).await;
    }
    for path in &ROUTES[..3] {
        let mut h = headers();
        h.insert("x-framerize-mode", HeaderValue::from_static("paid"));
        forwarded(path, h, payload(path), THREAD, true).await;
    }
    let path = "/openai/v1/responses";
    let mut body = payload(path);
    body["model"] = json!("gpt-6-sol");
    forwarded(path, headers(), body, THREAD, true).await;
}

#[tokio::test]
async fn ambiguous_native_context_is_rejected_before_all_provider_preparations() {
    for path in ROUTES {
        for name in ["thread-id", "session-id"] {
            let mut h = headers();
            h.append(name, HeaderValue::from_static(THREAD));
            rejected(path, h, FailureKind::InvalidRequest).await;
            let mut h = headers();
            h.insert(name, HeaderValue::from_static("fixture-other-thread"));
            rejected(path, h, FailureKind::InvalidRequest).await;
            let mut h = headers();
            h.insert(name, HeaderValue::from_static("thread-one,thread-two"));
            rejected(path, h, FailureKind::InvalidRequest).await;
        }
    }
}

#[tokio::test]
async fn explicit_context_wins_over_malformed_native_headers_on_all_routes() {
    const EXPLICIT: &str = "Explicit:Case-Sensitive/Conversation";
    for path in ROUTES {
        let mut h = headers();
        h.insert("x-framerize-session", HeaderValue::from_static(EXPLICIT));
        h.append("thread-id", HeaderValue::from_bytes(&[255]).unwrap());
        h.insert("session-id", HeaderValue::from_static("different-thread"));
        forwarded(path, h, payload(path), EXPLICIT, path.starts_with("/paid/")).await;
    }
}

#[tokio::test]
async fn invalid_explicit_context_never_falls_back_to_native_or_legacy() {
    for path in ROUTES {
        for invalid in [HeaderValue::from_static(""), HeaderValue::from_bytes(&[255]).unwrap()] {
            let mut h = headers();
            h.insert("x-framerize-session", invalid);
            rejected(path, h, FailureKind::InvalidRequest).await;
        }
        for extra in [THREAD, "different-explicit-context"] {
            let mut h = headers();
            h.append("x-framerize-session", HeaderValue::from_static(THREAD));
            h.append("x-framerize-session", HeaderValue::from_static(extra));
            rejected(path, h, FailureKind::InvalidRequest).await;
        }
    }
}

#[tokio::test]
async fn invalid_native_values_fail_before_provider_preparation() {
    for path in ROUTES {
        for name in ["thread-id", "session-id"] {
            for invalid in [HeaderValue::from_static(""), HeaderValue::from_static("two ids"),
                HeaderValue::from_static("tab\tvalue"), HeaderValue::from_bytes(&[255]).unwrap()] {
                let mut h = headers();
                h.insert(name, invalid);
                rejected(path, h, FailureKind::InvalidRequest).await;
            }
        }
    }
}

#[tokio::test]
async fn missing_context_never_uses_credentials_request_ids_or_body_metadata() {
    for path in ROUTES {
        rejected(path, HeaderMap::new(), FailureKind::MissingContext).await;
        let mut h = HeaderMap::new();
        for name in ["authorization", "cookie", "x-framerize-account", "x-request-id",
            "x-client-request-id", "session-id", "session_id", "conversation_id"] {
            h.insert(name, HeaderValue::from_static(THREAD));
        }
        let mut body = payload(path);
        body["client_metadata"] = json!({"thread_id":THREAD, "session_id":THREAD});
        body["prompt_cache_key"] = json!(THREAD);
        let (state, spy, provider) = fixture::fixture(true, None);
        let response = request(state, path, h, body).await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert_eq!(response.extensions().get::<FailureKind>(), Some(&FailureKind::MissingContext));
        assert_eq!(*provider.preparations.lock().unwrap(), 0);
        assert!(provider.records.lock().unwrap().is_empty());
        assert!(spy.calls.lock().unwrap().is_empty());
    }
}

#[tokio::test]
async fn native_header_reader_is_not_enabled_on_legacy_requests() {
    for path in ROUTES {
        let (state, spy, provider) = fixture::fixture(false, None);
        let mut h = headers();
        h.append("thread-id", HeaderValue::from_static("conflicting-native-thread"));
        h.insert("x-framerize-session", HeaderValue::from_static("legacy-explicit"));
        let response = request(state, path, h, payload(path)).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()["x-framerize-session"], "legacy-explicit");
        assert_eq!(*provider.preparations.lock().unwrap(), 0);
        assert!(provider.records.lock().unwrap().is_empty());
        assert!(!spy.calls.lock().unwrap().is_empty());
    }
}

use super::*;
use serde_json::json;
use tower::ServiceExt;

#[path = "native_http_stream_fixture.rs"]
mod fixture;

const THREAD: &str = "fixture-native-conversation";
const FRAME: &[u8] = b"event: fixture\ndata: {\"fixture\":true}\n\n";
const ROUTES: [(&str, bool, bool); 10] = [
    ("/anthropic/v1/messages", false, false),
    ("/openai/v1/responses", false, false),
    ("/openai/v1/chat/completions", false, false),
    ("/paid/anthropic/v1/messages", false, false),
    ("/paid/openai/v1/responses", false, false),
    ("/paid/openai/v1/chat/completions", false, false),
    ("/anthropic/v1/messages", true, false),
    ("/openai/v1/responses", true, false),
    ("/openai/v1/chat/completions", true, false),
    ("/openai/v1/responses", false, true),
];

fn headers(thread: &str) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert("thread-id", HeaderValue::from_str(thread).unwrap());
    headers.insert("session-id", HeaderValue::from_str(thread).unwrap());
    headers
}

fn payload(path: &str, direct: bool) -> Value {
    let mut body = if path.ends_with("/responses") {
        json!({"model":"gpt-4.1", "stream":true, "input":"fixture"})
    } else {
        json!({"model":"gpt-4.1", "stream":true,
            "messages":[{"role":"user", "content":"fixture"}]})
    };
    if direct { body["model"] = json!("gpt-6-sol"); }
    body
}

async fn request(fixture: &fixture::Fixture, index: usize, path: &str,
    mut headers: HeaderMap, body: Value) -> Response {
    headers.insert("content-type", HeaderValue::from_static("application/json"));
    headers.insert("x-framerize-account", HeaderValue::from_str(
        &fixture.accounts[index].id.0.to_string()).unwrap());
    let mut request = axum::http::Request::builder().method("POST").uri(path)
        .body(axum::body::Body::from(body.to_string())).unwrap();
    *request.headers_mut() = headers;
    tokio::time::timeout(fixture::BOUND,
        crate::router(fixture.state.clone(), "/fixture-no-web").oneshot(request))
        .await.unwrap().unwrap()
}

async fn delivered(response: Response, expected: &str, control: &mut fixture::Control) {
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["x-framerize-session"].to_str().unwrap(), expected);
    let body = tokio::time::timeout(fixture::BOUND,
        axum::body::to_bytes(response.into_body(), 2048)).await.unwrap().unwrap();
    assert_eq!(body.as_ref(), FRAME);
    control.release().await;
}

#[tokio::test]
async fn streamed_context_reaches_every_route_and_downstream_body() {
    for (path, paid, direct) in ROUTES {
        let fixture = fixture::Fixture::new(1, Duration::from_secs(2), None);
        let raw = paid || direct || path.starts_with("/paid/");
        let content_type = if raw { "text/event-stream" } else { "application/json" };
        let mut control = fixture.enqueue(0,
            fixture::headers(200, content_type, None), "text/event-stream");
        control.open(); control.push(FRAME); control.eof(); control.finish(Ok(()));
        let mut h = headers(THREAD);
        if paid { h.insert("x-framerize-mode", HeaderValue::from_static("paid")); }
        let response = request(&fixture, 0, path, h, payload(path, direct)).await;
        delivered(response, THREAD, &mut control).await;
        let records = fixture.records();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].identity.chat_session, THREAD);
        assert_eq!(records[0].method, if raw { "raw_stream" } else { "composed_stream" });
        assert!(fixture.slot_free(0, if direct { "gpt-6-sol" } else { "gpt-4.1" }).await);
        fixture.assert_legacy_unused();
    }
}

#[tokio::test]
async fn fork_is_distinct_and_resumed_context_survives_account_selection() {
    let fixture = fixture::Fixture::new(2, Duration::from_secs(2), None);
    let lifecycle = [(0, THREAD), (0, THREAD), (0, "independent-conversation"),
        (0, "forked-conversation"), (1, THREAD), (0, THREAD)];
    let path = "/openai/v1/responses";
    for (index, session) in lifecycle {
        let mut control = fixture.enqueue(index,
            fixture::headers(200, "application/json", None), "text/event-stream");
        control.open(); control.push(FRAME); control.eof(); control.finish(Ok(()));
        let response = request(&fixture, index, path, headers(session), payload(path, false)).await;
        delivered(response, session, &mut control).await;
        assert!(fixture.slot_free(index, "gpt-4.1").await);
    }
    let records = fixture.records();
    assert_eq!(records.len(), lifecycle.len());
    for (record, (index, session)) in records.iter().zip(lifecycle) {
        assert_eq!(record.identity.chat_session, session);
        assert_eq!(record.identity.account, fixture.accounts[index].id);
        assert_eq!(record.identity.user, fixture.accounts[index].framer_user_id);
        assert_eq!(Some(&record.identity.project), fixture.accounts[index].current_project.as_ref());
    }
    fixture.assert_legacy_unused();
}

#[tokio::test]
async fn invalid_stream_context_never_prepares_or_occupies_a_workspace() {
    for (path, paid, direct) in ROUTES {
        let mut conflicting = headers(THREAD);
        conflicting.insert("session-id", HeaderValue::from_static("another-conversation"));
        let mut duplicate = headers(THREAD);
        duplicate.append("thread-id", HeaderValue::from_static(THREAD));
        let mut explicit_empty = headers(THREAD);
        explicit_empty.insert("x-framerize-session", HeaderValue::from_static(""));
        let mut session_only = HeaderMap::new();
        session_only.insert("session-id", HeaderValue::from_static(THREAD));
        for (mut h, failure) in [
            (HeaderMap::new(), FailureKind::MissingContext),
            (session_only, FailureKind::MissingContext),
            (conflicting, FailureKind::InvalidRequest),
            (duplicate, FailureKind::InvalidRequest),
            (explicit_empty, FailureKind::InvalidRequest),
        ] {
            let fixture = fixture::Fixture::new(1, Duration::from_secs(2), None);
            // No plan is enqueued: any provider preparation would panic.
            if paid { h.insert("x-framerize-mode", HeaderValue::from_static("paid")); }
            let response = request(&fixture, 0, path, h, payload(path, direct)).await;
            assert_eq!(response.status(), StatusCode::BAD_REQUEST);
            assert_eq!(response.extensions().get::<FailureKind>(), Some(&failure));
            assert!(fixture.records().is_empty());
            assert!(fixture.slot_free(0, if direct { "gpt-6-sol" } else { "gpt-4.1" }).await);
            fixture.assert_legacy_unused();
        }
    }
}

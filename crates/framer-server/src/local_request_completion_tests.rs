//! Local router completion fixtures; all execution remains runtime-owner held.
use super::*;
use axum::{body::{to_bytes, Body, Bytes}, extract::FromRequest,
    http::{Request, StatusCode}, response::{IntoResponse, Response}};
use crate::operation_ownership::ServingOwnership;
use framer_core::operation_gate::OperationGate;
use http_body::Body as _;
use std::path::PathBuf;
use std::time::Duration;
use tower::ServiceExt;

const WAIT: Duration = Duration::from_secs(2);
const NO_WEB: &str = "/fixture-no-web";

fn records(ports: &FixturePorts) -> Value {
    serde_json::to_value(&*ports.records.lock().unwrap()).unwrap()
}

fn owned(mut state: AppState) -> (AppState, Arc<ServingOwnership>, OperationGate) {
    let (scope, gate) = ServingOwnership::new_with_host_factory(
        state.wave.clone(), 32, |gate| gate).unwrap();
    state.runtime_mut().expect("unique fixture runtime").operation_ownership = Some(scope.clone());
    (state, scope, gate)
}

async fn send(state: &AppState, web: &str, request: Request<Body>) -> Response {
    tokio::time::timeout(WAIT, crate::router(state.clone(), web).oneshot(request))
        .await.unwrap().unwrap()
}

fn post(path: &str, payload: Value) -> Request<Body> {
    Request::builder().method("POST").uri(path)
        .header("content-type", "application/json")
        .body(Body::from(payload.to_string())).unwrap()
}

fn unchanged(state: &AppState, ports: &FixturePorts, before: &Value,
    reads: usize, sessions: usize) {
    assert_eq!(&records(ports), before);
    assert_eq!(ports.state_reads.load(Ordering::SeqCst), reads);
    assert_eq!(ports.session_calls.load(Ordering::SeqCst), sessions);
    assert_eq!(ports.state_writes.load(Ordering::SeqCst), 0);
    assert_eq!(ports.client_calls.load(Ordering::SeqCst), 0);
    assert_eq!(ports.browser_calls.load(Ordering::SeqCst), 0);
    assert!(ports.run_calls.lock().unwrap().is_empty());
    assert!(ports.saved.lock().unwrap().is_empty());
    assert!(ports.registered_proxies.lock().unwrap().is_empty());
    let residents = state.serving_accounts.residents.snapshot().unwrap();
    assert_eq!(residents.occupied, 0);
    assert!(residents.residents.is_empty());
}

async fn finished(state: &AppState, scope: &ServingOwnership, gate: &OperationGate) {
    let mut ticket = scope.seal().unwrap();
    let proof = tokio::time::timeout(WAIT, ticket.wait_application()).await.unwrap().unwrap();
    scope.check_application_drained(&proof).unwrap();
    drop(proof);
    assert_eq!(gate.stats().unwrap().active, 1);
    assert!(!gate.stats().unwrap().unconfirmed);
    assert!(scope.unmanaged_host_close_required());
    {
        let waiting = ticket.wait();
        tokio::pin!(waiting);
        assert!(futures::poll!(waiting.as_mut()).is_pending());
    }
    ticket.cancel_before_control().unwrap();
    let response = send(state, NO_WEB,
        Request::builder().uri("/api/hello").body(Body::empty()).unwrap()).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(to_bytes(response.into_body(), 1024).await.unwrap().as_ref(), b"ok");
    assert_eq!(gate.stats().unwrap().active, 1);
    assert!(!gate.stats().unwrap().unconfirmed);
    assert!(state.wave.check().is_none());
}

async fn observe(response: Response, status: StatusCode, content_type: &str,
    expected: &[u8], consume: bool) {
    assert_eq!(response.status(), status);
    assert_eq!(response.headers()["content-type"], content_type);
    assert!(!response.body().is_end_stream());
    assert_eq!(response.body().size_hint().exact(), Some(expected.len() as u64));
    if consume {
        assert_eq!(to_bytes(response.into_body(), 65536).await.unwrap().as_ref(), expected);
    } else { drop(response); }
}

#[tokio::test]
async fn count_token_aliases_complete_consumed_and_discarded_output() {
    for path in ["/anthropic/v1/messages/count_tokens",
        "/paid/anthropic/v1/messages/count_tokens"] {
        for consume in [false, true] {
            let (state, ports) = fixture(false);
            let (state, scope, gate) = owned(state);
            let before = records(&ports);
            let request = post(path, json!({"system":"1234","messages":[
                {"role":"user","content":"12345678"},
                {"role":"user","content":[{"type":"text","text":"abcdefgh"}]}]}));
            observe(send(&state, NO_WEB, request).await, StatusCode::OK,
                "application/json", br#"{"input_tokens":5}"#, consume).await;
            unchanged(&state, &ports, &before, 0, 0);
            finished(&state, &scope, &gate).await;
            unchanged(&state, &ports, &before, 0, 0);
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum Rejection { Malformed, MissingType, ReadFailure }

fn rejected(path: &str, paid: bool, kind: Rejection) -> Request<Body> {
    let body = match kind {
        Rejection::Malformed => Body::from("{"),
        Rejection::MissingType => Body::from("{}"),
        Rejection::ReadFailure => Body::from_stream(futures::stream::iter([
            Err::<Bytes, _>(std::io::Error::other("inert local body failure")),
        ])),
    };
    let mut request = Request::builder().method("POST").uri(path);
    if paid { request = request.header("x-framerize-mode", "paid"); }
    if !matches!(kind, Rejection::MissingType) {
        request = request.header("content-type", "application/json");
    }
    request.body(body).unwrap()
}

async fn rejection_cases(routes: &[(&str, bool)]) {
    for &(path, paid) in routes {
        for kind in [Rejection::Malformed, Rejection::MissingType, Rejection::ReadFailure] {
            for consume in [false, true] {
                let rejection = Json::<Value>::from_request(rejected(path, paid, kind), &())
                    .await.unwrap_err();
                let original = rejection.into_response();
                let status = original.status();
                assert_eq!(status, if matches!(kind, Rejection::MissingType) {
                    StatusCode::UNSUPPORTED_MEDIA_TYPE
                } else { StatusCode::BAD_REQUEST });
                let expected = to_bytes(original.into_body(), 65536).await.unwrap();
                let (state, ports) = fixture(false);
                let (state, scope, gate) = owned(state);
                let before = records(&ports);
                let response = send(&state, NO_WEB, rejected(path, paid, kind)).await;
                observe(response, status, "text/plain; charset=utf-8", &expected, consume).await;
                unchanged(&state, &ports, &before, 0, 0);
                finished(&state, &scope, &gate).await;
                unchanged(&state, &ports, &before, 0, 0);
            }
        }
    }
}

#[tokio::test]
async fn local_and_paid_json_rejections_complete_before_ports() {
    rejection_cases(&[
        ("/api/accounts/remove", false), ("/api/accounts/dedupe", false),
        ("/api/accounts/begin", false), ("/api/accounts/complete", false),
        ("/api/accounts/import", false), ("/api/pool/config", false),
        ("/api/pool/activate", false), ("/api/pool/park", false),
        ("/anthropic/v1/messages/count_tokens", false),
        ("/paid/anthropic/v1/messages/count_tokens", false),
        ("/paid/anthropic/v1/messages", false), ("/paid/openai/v1/responses", false),
        ("/paid/openai/v1/chat/completions", false),
        ("/anthropic/v1/messages", true), ("/openai/v1/responses", true),
        ("/openai/v1/chat/completions", true),
    ]).await;
}

#[tokio::test]
async fn local_admin_refusals_complete_without_port_work() {
    let disabled = b"account is disabled by the pool configuration; its saved session is retained";
    let invalid_user = serde_json::to_vec(&json!({"error":{"code":"invalid_request",
        "message":"expected Framer user is invalid"}})).unwrap();
    let invalid_capacity = serde_json::to_vec(&json!({"error":{"code":"invalid_request",
        "message":"active_capacity must be between 1 and 64"}})).unwrap();
    let cases = [
        ("/api/accounts/begin", json!({"account_id":account(2).id,"email":"inert@example.invalid"}),
            StatusCode::SERVICE_UNAVAILABLE, "text/plain; charset=utf-8", disabled.to_vec()),
        ("/api/accounts/complete", json!({"account_id":account(2).id,"email_token":"inert"}),
            StatusCode::SERVICE_UNAVAILABLE, "text/plain; charset=utf-8", disabled.to_vec()),
        ("/api/accounts/import", json!({"account_id":account(1).id,"session_cookie":"inert",
            "expected_framer_user_id":" "}), StatusCode::BAD_REQUEST, "application/json", invalid_user),
        ("/api/pool/config", json!({"active_capacity":65}),
            StatusCode::BAD_REQUEST, "application/json", invalid_capacity),
    ];
    for (path, payload, status, content_type, expected) in cases {
        for consume in [false, true] {
            let (state, ports) = fixture(false);
            let (state, scope, gate) = owned(state);
            let before = records(&ports);
            let config = serde_json::to_value(state.serving_accounts.config()).unwrap();
            let response = send(&state, NO_WEB, post(path, payload.clone())).await;
            observe(response, status, content_type, &expected, consume).await;
            unchanged(&state, &ports, &before, 0, 0);
            finished(&state, &scope, &gate).await;
            unchanged(&state, &ports, &before, 0, 0);
            assert_eq!(serde_json::to_value(state.serving_accounts.config()).unwrap(), config);
        }
    }
}

#[tokio::test]
async fn dedupe_preview_completes_without_mutation() {
    for consume in [false, true] {
        let (state, ports) = fixture(false);
        ports.records.lock().unwrap()[1].castle_device_id = account(1).castle_device_id;
        *ports.native_session.lock().unwrap() = Some(None);
        let before = records(&ports);
        let original = send(&state, NO_WEB, post("/api/accounts/dedupe",
            json!({"apply":false}))).await;
        assert_eq!(original.status(), StatusCode::OK);
        assert_eq!(original.headers()["content-type"], "application/json");
        let expected = to_bytes(original.into_body(), 65536).await.unwrap();
        let preview: Value = serde_json::from_slice(&expected).unwrap();
        assert_eq!(preview["applied"], false);
        assert_eq!(preview["keeping"], json!([{"name":"fixture-1@example.invalid",
            "team":"team-1","device":"device-1"}]));
        assert_eq!(preview["needs_project"], json!([]));
        assert_eq!(preview["removing"], json!([{"name":"fixture-2@example.invalid",
            "reason":"shares a Castle device id with another record"}]));
        assert!(preview["note"].as_str().unwrap().starts_with("Same-login siblings are KEPT:"));
        unchanged(&state, &ports, &before, 1, 1);
        let (state, scope, gate) = owned(state);
        let response = send(&state, NO_WEB, post("/api/accounts/dedupe",
            json!({"apply":false}))).await;
        observe(response, StatusCode::OK, "application/json", &expected, consume).await;
        unchanged(&state, &ports, &before, 2, 2);
        finished(&state, &scope, &gate).await;
        unchanged(&state, &ports, &before, 2, 2);
    }
}

struct LocalWeb(PathBuf);

impl LocalWeb {
    fn new(contents: &[u8]) -> Self {
        let path = std::env::temp_dir().join(format!("framer-local-web-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&path).unwrap();
        let web = Self(path);
        std::fs::write(web.0.join("fixture.txt"), contents).unwrap();
        web
    }
    fn finish(self) {
        std::fs::remove_dir_all(&self.0).unwrap();
        assert!(!self.0.exists());
    }
}

impl Drop for LocalWeb {
    fn drop(&mut self) {
        // Best effort on panic; the success path checks cleanup explicitly.
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[derive(Clone, Copy, Debug)]
enum FileDisposition { Consumed, Discarded, Partial, Head }

#[tokio::test]
async fn static_file_body_completion_preserves_local_ownership() {
    use futures::StreamExt;
    let contents: Vec<u8> = (0..262_144).map(|n| b'a' + (n % 26) as u8).collect();
    for disposition in [FileDisposition::Consumed, FileDisposition::Discarded,
        FileDisposition::Partial, FileDisposition::Head] {
        let web = LocalWeb::new(&contents);
        let (state, ports) = fixture(false);
        let (state, scope, gate) = owned(state);
        let before = records(&ports);
        let head = matches!(disposition, FileDisposition::Head);
        let request = Request::builder().method(if head { "HEAD" } else { "GET" })
            .uri("/fixture.txt").body(Body::empty()).unwrap();
        let response = send(&state, web.0.to_str().unwrap(), request).await;
        assert_eq!(response.status(), StatusCode::OK, "{disposition:?}");
        assert_eq!(response.headers()["content-type"], "text/plain");
        assert_eq!(response.headers()["content-length"], contents.len().to_string());
        unchanged(&state, &ports, &before, 0, 0);
        if head {
            assert!(response.body().is_end_stream());
        } else {
            assert!(!response.body().is_end_stream());
            assert_eq!(gate.stats().unwrap().active, 2);
        }
        match disposition {
            FileDisposition::Consumed => {
                let bytes = tokio::time::timeout(WAIT, to_bytes(response.into_body(), contents.len()))
                    .await.unwrap().unwrap();
                assert_eq!(bytes.as_ref(), contents.as_slice());
            },
            FileDisposition::Discarded | FileDisposition::Head => { drop(response); },
            FileDisposition::Partial => {
                let mut body = response.into_body().into_data_stream();
                let first = tokio::time::timeout(WAIT, body.next()).await.unwrap().unwrap().unwrap();
                assert!(!first.is_empty() && first.len() < contents.len());
                assert_eq!(first.as_ref(), &contents[..first.len()]);
                assert_eq!(gate.stats().unwrap().active, 2);
                drop(body);
            },
        }
        finished(&state, &scope, &gate).await;
        unchanged(&state, &ports, &before, 0, 0);
        // Removing a Unix path is not itself evidence that a descriptor closed.
        web.finish();
    }
}

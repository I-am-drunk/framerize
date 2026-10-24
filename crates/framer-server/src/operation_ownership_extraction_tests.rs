//! Source-only fixtures; execution belongs to the runtime owner.
//! Base: 9cdba88d6a870cfc7da546456669d443b5180065. The server owner wires
//! this child as operation_ownership::tests::extraction with its wrapper fix.
use super::*;
use axum::extract::FromRequest;
use serde_json::Value;

#[derive(Clone, Copy, Debug)]
struct Route { path: &'static str, payload: &'static str, kind: &'static str }

const ROUTES: [Route; 3] = [
    Route { path: "/anthropic/v1/messages", kind: "messages",
        payload: r#"{"model":"claude-opus-5-5","messages":[{"role":"user","content":"fixture"}],"stream":false}"# },
    Route { path: "/openai/v1/responses", kind: "responses",
        payload: r#"{"model":"gpt-6-astra","input":"fixture","stream":false}"# },
    Route { path: "/openai/v1/chat/completions", kind: "responses",
        payload: r#"{"model":"gpt-6-astra","messages":[{"role":"user","content":"fixture"}],"stream":false}"# },
];

#[derive(Clone, Copy, Debug)]
enum Rejection { Malformed, MissingType, ReadFailure }

impl Rejection {
    fn body(self) -> Body {
        match self {
            Self::Malformed => Body::from("{"),
            Self::MissingType => Body::from("{}"),
            Self::ReadFailure => Body::from_stream(futures::stream::iter([
                Err::<Bytes, _>(std::io::Error::other("fixture body read failure")),
            ])),
        }
    }
    fn has_type(self) -> bool { !matches!(self, Self::MissingType) }
    fn status(self) -> StatusCode {
        if self.has_type() { StatusCode::BAD_REQUEST }
        else { StatusCode::UNSUPPORTED_MEDIA_TYPE }
    }
}

fn route_request(route: Route, f: &fixture::Fixture, body: Body,
    content_type: bool) -> HttpRequest<Body> {
    let mut request = HttpRequest::builder().method("POST").uri(route.path)
        .header("x-framerize-account", f.accounts[0].id.0.to_string())
        .header("x-framerize-session", "extraction-fixture");
    if content_type { request = request.header("content-type", "application/json"); }
    request.body(body).unwrap()
}

async fn original_rejection(request: HttpRequest<Body>)
    -> (StatusCode, axum::http::HeaderMap, Bytes) {
    let rejection = axum::Json::<Value>::from_request(request, &()).await.unwrap_err();
    let (parts, body) = rejection.into_response().into_parts();
    let bytes = to_bytes(body, 65536).await.unwrap();
    assert!(!bytes.is_empty());
    (parts.status, parts.headers, bytes)
}

async fn assert_pristine(f: &mut fixture::Fixture, before: &Value) {
    assert!(matches!(f.loads.try_recv(),
        Err(tokio::sync::mpsc::error::TryRecvError::Empty)));
    f.no_call(); // Browser mint and all other forbidden ports panic in the fixture.
    let records = serde_json::to_value(f.state.state.list().await.unwrap()).unwrap();
    assert_eq!(&records, before);
    // Equal full records do not observe identical-value upsert attempts.
    // This fixture has no write-attempt counter; that count remains unobserved.
    let residents = f.state.serving_accounts.residents.snapshot().unwrap();
    assert_eq!(residents.occupied, 0);
    assert!(residents.residents.is_empty());
}

async fn finish_application_drain(ownership: &Arc<ServingOwnership>, mut ticket: DrainTicket) {
    let proof = tokio::time::timeout(WAIT, ticket.wait_application()).await.unwrap().unwrap();
    ownership.check_application_drained(&proof).unwrap();
    drop(proof);
    let stats = ownership.gate.stats().unwrap();
    assert_eq!(stats.active, 1);
    assert!(!stats.unconfirmed);
    assert!(ownership.unmanaged_host_close_required());
    assert!(tokio::time::timeout(Duration::from_millis(5), ticket.wait()).await.is_err());
    // Only application work drained. The unmanaged host never supplied close proof.
    ticket.cancel_before_control().unwrap();
}

async fn subsequent_admission(f: &mut fixture::Fixture, app: Router, route: Route) {
    let request = route_request(route, f, Body::from(route.payload), true);
    let task = tokio::spawn(app.oneshot(request));
    let call = f.next().await;
    assert_eq!(call.kind, route.kind);
    assert_eq!(call.account, f.accounts[0].id);
    assert_eq!(f.loads.try_recv().unwrap(), f.accounts[0].framer_user_id);
    call.reply.send(Ok(fixture::success(call.kind))).ok().unwrap();
    let response = tokio::time::timeout(WAIT, task).await.unwrap().unwrap().unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = tokio::time::timeout(WAIT, to_bytes(response.into_body(), 65536))
        .await.unwrap().unwrap();
    assert!(String::from_utf8_lossy(&bytes).contains("fixture"));
    tokio::time::timeout(WAIT, call.dropped).await.unwrap().unwrap();
    f.no_call();
    let ownership = f.state.operation_ownership.clone().unwrap();
    finish_application_drain(&ownership, ownership.seal().unwrap()).await;
    assert!(matches!(f.loads.try_recv(),
        Err(tokio::sync::mpsc::error::TryRecvError::Empty)));
    f.no_call();
}

async fn rejection_cases(kind: Rejection) {
    for route in ROUTES {
        for consume in [false, true] {
            let mut f = fixture::Fixture::new();
            f.state = with_owned_operations(f.state, 32).unwrap();
            let ownership = f.state.operation_ownership.clone().unwrap();
            let before = serde_json::to_value(f.state.state.list().await.unwrap()).unwrap();
            let expected = original_rejection(route_request(route, &f,
                kind.body(), kind.has_type())).await;
            assert_eq!(expected.0, kind.status());
            let app = crate::router(f.state.clone(), "/nonexistent-fixture-web");
            let request = route_request(route, &f, kind.body(), kind.has_type());
            let response = tokio::time::timeout(WAIT, app.clone().oneshot(request))
                .await.unwrap().unwrap();
            assert_eq!(response.status(), expected.0, "{route:?} {kind:?}");
            assert_eq!(response.headers().get("content-type"), expected.1.get("content-type"));
            assert!(!response.body().is_end_stream());
            assert_eq!(response.body().size_hint().exact(), Some(expected.2.len() as u64));
            assert_eq!(ownership.gate.stats().unwrap().active, 2);
            assert_pristine(&mut f, &before).await;
            if consume {
                let body = tokio::time::timeout(WAIT, to_bytes(response.into_body(), 65536))
                    .await.unwrap().unwrap();
                assert_eq!(body, expected.2);
            } else { drop(response); }
            finish_application_drain(&ownership, ownership.seal().unwrap()).await;
            assert_pristine(&mut f, &before).await;
            subsequent_admission(&mut f, app, route).await;
        }
    }
}

#[tokio::test]
async fn malformed_json_keeps_original_rejection_and_releases_local_ownership() {
    rejection_cases(Rejection::Malformed).await;
}
#[tokio::test]
async fn missing_content_type_keeps_original_rejection_and_releases_local_ownership() {
    rejection_cases(Rejection::MissingType).await;
}
#[tokio::test]
async fn body_read_failure_keeps_original_rejection_and_releases_local_ownership() {
    rejection_cases(Rejection::ReadFailure).await;
}

#[tokio::test]
async fn aborted_http_waiter_keeps_gated_extraction_owned_until_read_failure() {
    for route in ROUTES {
        let mut f = fixture::Fixture::new();
        f.state = with_owned_operations(f.state, 32).unwrap();
        let ownership = f.state.operation_ownership.clone().unwrap();
        let before = serde_json::to_value(f.state.state.list().await.unwrap()).unwrap();
        let (entered, seen) = oneshot::channel();
        let (release, released) = oneshot::channel();
        let body = Body::from_stream(futures::stream::once(async move {
            entered.send(()).unwrap();
            released.await.unwrap();
            Err::<Bytes, _>(std::io::Error::other("fixture body read failure"))
        }));
        let app = crate::router(f.state.clone(), "/nonexistent-fixture-web");
        let task = tokio::spawn(app.clone().oneshot(route_request(route, &f, body, true)));
        tokio::time::timeout(WAIT, seen).await.unwrap().unwrap();
        assert_pristine(&mut f, &before).await;
        // Abort and join only the HTTP waiter, never the admitted extraction task.
        task.abort();
        let error = tokio::time::timeout(WAIT, task).await.unwrap().unwrap_err();
        assert!(error.is_cancelled());
        assert!(!release.is_closed());
        assert_eq!(ownership.gate.stats().unwrap().active, 2);
        assert!(!ownership.gate.stats().unwrap().unconfirmed);
        let mut ticket = ownership.seal().unwrap();
        assert!(tokio::time::timeout(Duration::from_millis(5),
            ticket.wait_application()).await.is_err());
        release.send(()).unwrap();
        finish_application_drain(&ownership, ticket).await;
        assert_pristine(&mut f, &before).await;
        // Status/body are unobservable after waiter cancellation; no claim is made.
        subsequent_admission(&mut f, app, route).await;
    }
}

#[tokio::test]
async fn local_preflight_refusals_keep_original_response_and_release_ownership() {
    const UNAVAILABLE: &str = concat!(
        "model gpt-4o is not available on this gateway: ",
        "blocked by the gateway's upstream guardrails (404) on every route. ",
        "Working free models: claude-opus-5-5, claude-sonnet-5-5, claude-haiku-4-5, ",
        "gpt-4.1, gpt-4.1-mini, gemini-2.5-flash, gemini-3-flash-preview",
    );
    let cases = [
        (ROUTES[0], r#"{"model":"gpt-4o","messages":[]}"#, UNAVAILABLE),
        (ROUTES[1], r#"{"model":"gpt-4o","input":"fixture"}"#, UNAVAILABLE),
        (ROUTES[2], r#"{"model":"gpt-4o","messages":[]}"#, UNAVAILABLE),
        (ROUTES[0], r#"{"model":"claude-opus-5-5"}"#, "messages is required"),
        (ROUTES[2], r#"{"model":"gpt-6-astra","n":2,"messages":[{"role":"user","content":"fixture"}]}"#,
            "n != 1 is not supported on this gateway"),
        (ROUTES[2], r#"{"model":"gpt-6-astra"}"#, "messages is required"),
    ];
    for (route, payload, expected_text) in cases {
        for consume in [false, true] {
            let mut f = fixture::Fixture::new();
            let before = serde_json::to_value(f.state.state.list().await.unwrap()).unwrap();
            // Pin the original text independently of both router modes.
            let original = tokio::time::timeout(WAIT,
                crate::router(f.state.clone(), "/nonexistent-fixture-web").oneshot(
                    route_request(route, &f, Body::from(payload), true)))
                .await.unwrap().unwrap();
            let (parts, body) = original.into_parts();
            assert_eq!(parts.status, StatusCode::INTERNAL_SERVER_ERROR);
            assert_eq!(parts.headers["content-type"], "text/plain; charset=utf-8");
            let expected = tokio::time::timeout(WAIT, to_bytes(body, 65536))
                .await.unwrap().unwrap();
            assert!(!expected.is_empty());
            assert_eq!(expected.as_ref(), expected_text.as_bytes());
            assert_pristine(&mut f, &before).await;
            f.state = with_owned_operations(f.state, 32).unwrap();
            let ownership = f.state.operation_ownership.clone().unwrap();
            let app = crate::router(f.state.clone(), "/nonexistent-fixture-web");
            let response = tokio::time::timeout(WAIT, app.clone().oneshot(
                route_request(route, &f, Body::from(payload), true)))
                .await.unwrap().unwrap();
            assert_eq!(response.status(), parts.status, "{route:?}");
            assert_eq!(response.headers().get("content-type"), parts.headers.get("content-type"));
            assert!(!response.body().is_end_stream());
            assert_eq!(response.body().size_hint().exact(), Some(expected.len() as u64));
            assert_eq!(ownership.gate.stats().unwrap().active, 2);
            assert_pristine(&mut f, &before).await;
            if consume {
                let bytes = tokio::time::timeout(WAIT, to_bytes(response.into_body(), 65536))
                    .await.unwrap().unwrap();
                assert_eq!(bytes, expected);
            } else { drop(response); }
            finish_application_drain(&ownership, ownership.seal().unwrap()).await;
            assert_pristine(&mut f, &before).await;
            subsequent_admission(&mut f, app, route).await;
        }
    }
}

#[tokio::test]
async fn paid_paths_and_normal_paid_overrides_still_dispatch_to_proxy() {
    for route in ROUTES {
        for prefixed in [false, true] {
            let mut f = fixture::Fixture::new();
            f.state = with_owned_operations(f.state, 32).unwrap();
            let ownership = f.state.operation_ownership.clone().unwrap();
            let mut request = route_request(route, &f, Body::from(route.payload), true);
            if prefixed { *request.uri_mut() = format!("/paid{}", route.path).parse().unwrap(); }
            else { request.headers_mut().insert("x-framerize-mode", "paid".parse().unwrap()); }
            let app = crate::router(f.state.clone(), "/nonexistent-fixture-web");
            let task = tokio::spawn(app.oneshot(request));
            let call = f.next().await;
            assert_eq!(call.kind, "proxy");
            assert_eq!(call.account, f.accounts[0].id);
            assert_eq!(f.loads.try_recv().unwrap(), f.accounts[0].framer_user_id);
            call.reply.send(Ok(fixture::Reply::Proxy(framer_core::port::ProxyResponse {
                status: 201, content_type: "text/plain".into(), queue_wait_ms: 0,
                stream: Box::pin(futures::stream::iter([
                    Ok::<Bytes, String>(Bytes::from_static(b"fixture-paid")),
                ])),
            }))).ok().unwrap();
            let response = tokio::time::timeout(WAIT, task).await.unwrap().unwrap().unwrap();
            assert_eq!(response.status(), StatusCode::CREATED);
            assert_eq!(response.headers()["content-type"], "text/plain");
            let bytes = tokio::time::timeout(WAIT, to_bytes(response.into_body(), 65536))
                .await.unwrap().unwrap();
            assert_eq!(bytes.as_ref(), b"fixture-paid");
            tokio::time::timeout(WAIT, call.dropped).await.unwrap().unwrap();
            f.no_call();
            finish_application_drain(&ownership, ownership.seal().unwrap()).await;
            assert!(matches!(f.loads.try_recv(),
                Err(tokio::sync::mpsc::error::TryRecvError::Empty)));
            f.no_call();
        }
    }
}

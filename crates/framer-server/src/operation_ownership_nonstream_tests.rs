use super::*;

#[derive(Clone, Copy)]
struct NonstreamRoute {
    path: &'static str,
    payload: &'static str,
    gate_model: &'static str,
    kind: &'static str,
}

const ROUTES: [NonstreamRoute; 2] = [
    NonstreamRoute {
        path: "/openai/v1/responses", gate_model: "gpt-6-astra", kind: "responses",
        payload: r#"{"model":"gpt-6-astra","input":"fixture","stream":false}"#,
    },
    NonstreamRoute {
        path: "/anthropic/v1/messages", gate_model: "claude-opus-5-5", kind: "messages",
        payload: r#"{"model":"claude-opus-5-5","messages":[{"role":"user","content":"fixture"}],"stream":false}"#,
    },
];

impl NonstreamRoute {
    fn request(self, f: &fixture::Fixture) -> HttpRequest<Body> {
        HttpRequest::builder().method("POST").uri(self.path)
            .header("content-type", "application/json")
            .header("x-framerize-account", f.accounts[0].id.0.to_string())
            .header("x-framerize-session", "fixture-conversation")
            .body(Body::from(self.payload)).unwrap()
    }
}

#[tokio::test]
async fn nonstream_unknown_cleanup_stays_fenced_after_body_drop_or_consumption() {
    for route in ROUTES { for consume in [false, true] {
        let mut f = fixture::Fixture::new();
        f.state = with_owned_operations(f.state, 32).unwrap();
        let ownership = f.state.operation_ownership.clone().unwrap();
        let app = crate::router(f.state.clone(), "/nonexistent-fixture-web");
        let mut request = route.request(&f);
        request.headers_mut().remove("x-framerize-account");
        let task = tokio::spawn(app.clone().oneshot(request));
        let call = f.next().await;
        assert_eq!(call.account, f.accounts[0].id);
        call.reply.send(Err("host-lifecycle-unconfirmed: fixture cleanup lost".into())).ok().unwrap();
        let response = tokio::time::timeout(WAIT, task).await.unwrap().unwrap().unwrap();
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        if consume {
            let bytes = to_bytes(response.into_body(), 65536).await.unwrap();
            assert!(String::from_utf8_lossy(&bytes).contains("fixture cleanup lost"));
        } else { drop(response); }
        tokio::time::timeout(WAIT, async {
            while ownership.gate.stats().unwrap().active > 1 { tokio::task::yield_now().await; }
        }).await.unwrap();
        assert!(ownership.gate.stats().unwrap().unconfirmed);
        let response = app.oneshot(route.request(&f)).await.unwrap();
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert!(String::from_utf8_lossy(&to_bytes(response.into_body(), 4096).await.unwrap())
            .contains("fixture cleanup lost"));
        f.no_call();
    } }
}

#[tokio::test]
async fn disconnected_nonstream_waits_for_current_call_and_never_repicks() {
    for (route, error) in ROUTES.into_iter().flat_map(|route| {
        [None, Some("risk-refused"), Some("retryable 429")].map(|error| (route, error))
    }) {
        let mut f = fixture::Fixture::new();
        f.state = with_owned_operations(f.state, 32).unwrap();
        let ownership = f.state.operation_ownership.clone().unwrap();
        let app = crate::router(f.state.clone(), "/nonexistent-fixture-web");
        let task = tokio::spawn(app.clone().oneshot(route.request(&f)));
        let call = f.next().await;
        assert_eq!(call.kind, route.kind);
        task.abort(); let _ = task.await;
        assert!(!call.reply.is_closed());
        assert_eq!(ownership.gate.stats().unwrap().active, 2);
        assert!(!ownership.gate.stats().unwrap().unconfirmed);
        let reply = match error {
            Some(error) => Err(error.into()),
            None => Ok(fixture::success(call.kind)),
        };
        call.reply.send(reply).ok().unwrap();
        tokio::time::timeout(WAIT, async {
            while ownership.gate.stats().unwrap().active > 1 {
                tokio::task::yield_now().await;
            }
        }).await.unwrap();
        f.no_call();
        assert!(!ownership.gate.stats().unwrap().unconfirmed);
        let current = f.state.state.get(f.accounts[0].id).await.unwrap().unwrap();
        let risk_refused = error == Some("risk-refused");
        assert_eq!(current.consecutive_failures, u32::from(risk_refused));
        assert_eq!(current.cooldown_step, u32::from(risk_refused));
        assert_eq!(current.cooling_until_ms.is_some(), risk_refused);
        assert!(f.state.wave.check().is_none());
        recovered_request(&mut f, app, route).await;
    }
}

async fn recovered_request(f: &mut fixture::Fixture, app: Router, route: NonstreamRoute) {
    let mut request = route.request(f);
    request.headers_mut().insert("x-framerize-account",
        f.accounts[1].id.0.to_string().parse().unwrap());
    let task = tokio::spawn(app.oneshot(request));
    let call = f.next().await;
    assert_eq!(call.kind, route.kind);
    assert_eq!(call.account, f.accounts[1].id);
    call.reply.send(Ok(fixture::success(call.kind))).ok().unwrap();
    let response = tokio::time::timeout(WAIT, task).await.unwrap().unwrap().unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), 65536).await.unwrap();
    assert!(String::from_utf8_lossy(&body).contains("fixture"));
}

#[tokio::test]
async fn dropping_completed_nonstream_response_preserves_admission() {
    for route in ROUTES {
        let mut f = fixture::Fixture::new();
        f.state = with_owned_operations(f.state, 32).unwrap();
        let ownership = f.state.operation_ownership.clone().unwrap();
        let app = crate::router(f.state.clone(), "/nonexistent-fixture-web");
        let task = tokio::spawn(app.clone().oneshot(route.request(&f)));
        let call = f.next().await;
        assert_eq!(call.kind, route.kind);
        call.reply.send(Ok(fixture::success(call.kind))).ok().unwrap();
        let response = tokio::time::timeout(WAIT, task).await.unwrap().unwrap().unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(ownership.gate.stats().unwrap().active, 2);
        drop(response);
        assert_eq!(ownership.gate.stats().unwrap().active, 1);
        assert!(!ownership.gate.stats().unwrap().unconfirmed);
        recovered_request(&mut f, app, route).await;
    }
}

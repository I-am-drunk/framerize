use super::*;

#[tokio::test]
async fn discarded_messages_wave_refusal_keeps_cooldown_without_sealing() {
    let mut f = fixture::Fixture::new();
    f.state = with_owned_operations(f.state, 32).unwrap();
    let ownership = f.state.operation_ownership.clone().unwrap();
    f.state.wave.note_refusal("fixture-wave-a");
    f.state.wave.note_refusal("fixture-wave-b");
    assert!(f.state.wave.check().is_some());
    let request = HttpRequest::builder().method("POST").uri("/anthropic/v1/messages")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::json!({"model":"claude-opus-5-5",
            "messages":[{"role":"user","content":"fixture"}],
            "max_tokens":16,"stream":true}).to_string())).unwrap();
    let response = crate::router(f.state.clone(), "/nonexistent-fixture-web")
        .oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert!(response.headers().contains_key(axum::http::header::RETRY_AFTER));
    drop(response);
    f.no_call();
    assert!(f.loads.try_recv().is_err());
    assert_eq!(f.state.serving_accounts.residents.snapshot().unwrap().occupied, 0);
    assert_eq!(ownership.gate.stats().unwrap().active, 1);
    assert!(!ownership.gate.stats().unwrap().unconfirmed);
    assert!(f.state.wave.check().is_some());
}

#[tokio::test]
async fn read_only_heads_and_discarded_gets_preserve_running_stream_and_admission() {
    let mut f = fixture::Fixture::new();
    f.state = with_owned_operations(f.state, 32).unwrap();
    let ownership = f.state.operation_ownership.clone().unwrap();
    let app = crate::router(f.state.clone(), "/nonexistent-fixture-web");
    let stream = app.clone().oneshot(fixture_request(&f, true)).await.unwrap();
    let call = f.next().await;
    for (path, content_type, length) in [
        ("/anthropic/api/hello", "text/plain; charset=utf-8", Some("2")),
        ("/openai/api/hello", "text/plain; charset=utf-8", Some("2")),
        ("/api/hello", "text/plain; charset=utf-8", Some("2")),
        ("/api/accounts", "application/json", None),
        ("/api/pool", "application/json", None),
        ("/api/pool/config", "application/json", None),
    ] {
        for method in ["HEAD", "GET"] {
            let response = app.clone().oneshot(HttpRequest::builder().method(method)
                .uri(path).body(Body::empty()).unwrap()).await.unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            assert_eq!(response.headers()[axum::http::header::CONTENT_TYPE], content_type);
            if let Some(length) = length {
                assert_eq!(response.headers()[axum::http::header::CONTENT_LENGTH], length);
            }
            if method == "HEAD" {
                assert!(to_bytes(response.into_body(), 16).await.unwrap().is_empty());
            } else { drop(response); }
            assert!(!ownership.gate.stats().unwrap().unconfirmed, "{method} {path}");
            assert_eq!(ownership.gate.stats().unwrap().active, 3);
            assert!(!call.reply.is_closed());
        }
    }
    f.no_call();
    call.reply.send(Ok(fixture::success(call.kind))).ok().unwrap();
    let body = tokio::time::timeout(WAIT, to_bytes(stream.into_body(), 65536))
        .await.unwrap().unwrap();
    assert!(String::from_utf8_lossy(&body).contains("response.completed"));
    assert_eq!(ownership.gate.stats().unwrap().active, 1);
    assert!(!ownership.gate.stats().unwrap().unconfirmed);
    let next = app.oneshot(fixture_request(&f, true)).await.unwrap();
    assert_eq!(next.status(), StatusCode::OK);
    let call = f.next().await;
    call.reply.send(Ok(fixture::success(call.kind))).ok().unwrap();
    to_bytes(next.into_body(), 65536).await.unwrap();
}

#[tokio::test]
async fn dropped_stream_preflight_refusal_does_not_seal_admission() {
    let mut f = fixture::Fixture::new();
    f.state = with_owned_operations(f.state, 32).unwrap();
    let ownership = f.state.operation_ownership.clone().unwrap();
    let mut cooling = f.accounts[0].clone();
    cooling.cooling_until_ms = Some(framer_core::rotation::now_ms() + 60_000);
    f.state.state.upsert(&cooling).await.unwrap();
    let app = crate::router(f.state.clone(), "/nonexistent-fixture-web");
    let response = app.clone().oneshot(fixture_request(&f, true)).await.unwrap();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert!(response.headers().contains_key(axum::http::header::RETRY_AFTER));
    f.no_call();
    drop(response);
    assert!(!ownership.gate.stats().unwrap().unconfirmed);
    assert_eq!(ownership.gate.stats().unwrap().active, 1);

    let mut request = fixture_request(&f, true);
    request.headers_mut().insert("x-framerize-account",
        f.accounts[1].id.0.to_string().parse().unwrap());
    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let call = f.next().await;
    call.reply.send(Ok(fixture::success(call.kind))).ok().unwrap();
    let body = tokio::time::timeout(WAIT, to_bytes(response.into_body(), 65536))
        .await.unwrap().unwrap();
    assert!(String::from_utf8_lossy(&body).contains("response.completed"));
    assert!(!ownership.gate.stats().unwrap().unconfirmed);
}

use super::*;
use axum::{body::Body, http::{Request, StatusCode}, routing::post, Router};
use tower::ServiceExt;

#[tokio::test]
async fn routing_http_provider_keeps_captured_context_after_handler_returns() {
    let (state, ports, spec, sessions) = fixture();
    let source_ports = ports.clone();
    let provider = Provider::with_sources(state.clone(), sessions.clone(),
        Arc::new(crate::offline_request_context::Current), port::Policy {
            timeout: Duration::from_secs(3), max_body_bytes: 65536,
            missing_content_type: "application/json".into(), chunk_tokens: 32,
            max_chunks: 1, continuation: None, allow_paid: false, credit_floor: 1.0,
        }, None, Arc::new(move |_| Arc::new(Builder(source_ports.clone())))).unwrap();
    let admitted = native_control::admit(&state, spec.identity.clone(), "gpt-4.1", spec.deadline)
        .await.unwrap();
    admitted.completion.complete().unwrap();
    ports.events.lock().unwrap().clear();
    let pending = Arc::new(Mutex::new(Some((provider, request(&spec), admitted.capacity, admitted.pool))));
    let output: Arc<Mutex<Option<Box<dyn OwnedInvocation>>>> = Arc::new(Mutex::new(None));
    let saved = output.clone();
    let app = Router::new().route("/openai/v1/responses", post(move || {
        let (provider, request, capacity, pool) = pending.lock().unwrap().take().unwrap();
        let output = output.clone();
        async move {
            let call = provider.prepare_in_pool(request, capacity, pool).await.unwrap();
            *output.lock().unwrap() = Some(call);
            StatusCode::OK
        }
    })).layer(axum::middleware::from_fn(crate::offline_request_context::middleware));
    let metadata = serde_json::json!({"page_url":"https://framer.com/projects/fixture-project",
        "proxy":null,"attempts":[{"feature_category":"actual-category","feature_name":"actual-feature",
        "step":"actual-step","turn_id":"actual-turn","user_prompt":"actual display"}]});
    let response = app.oneshot(Request::post("/openai/v1/responses")
        .header("x-framerize-context", metadata.to_string()).body(Body::empty()).unwrap()).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert!(ports.events.lock().unwrap().is_empty());
    ports.load_failure.store(2, Ordering::SeqCst);
    let call = saved.lock().unwrap().take().unwrap();
    let run = call.responses(spec.payload);
    assert_eq!(run.result.await.err().unwrap().kind, FailureKind::Authorization);
    assert_eq!(run.cleanup.await, Ok(()));
    assert_eq!(*ports.events.lock().unwrap(), vec!["load"]);
    assert_eq!(users(&state, spec.identity.account).await, 0);
}

#[tokio::test]
async fn routing_http_configuration_requires_worker_before_io() {
    let (state, ports, _, _) = fixture();
    let result = crate::offline_http::configure(state, None, Some("fixture-missing-file"));
    assert!(matches!(result, Err("offline routing requires its worker")));
    assert!(ports.events.lock().unwrap().is_empty());
}

#[tokio::test]
async fn routing_http_disabled_configuration_preserves_existing_state() {
    let (state, ports, _, _) = fixture();
    let (configured, runtime) = crate::offline_http::configure(state.clone(), None, None).unwrap();
    assert!(runtime.is_none() && configured.native_dispatch.is_none());
    assert!(Arc::ptr_eq(&state.sessions, &configured.sessions));
    assert!(Arc::ptr_eq(&state.state, &configured.state));
    assert!(Arc::ptr_eq(&state.serving_accounts, &configured.serving_accounts));
    assert!(ports.events.lock().unwrap().is_empty());
}

#[tokio::test]
async fn routing_http_missing_context_leaves_the_real_owner_drained() {
    let (state, ports, _, _) = fixture();
    let app = framer_server::router_with_request_layer(state.clone(), "fixture-no-web-root", |router| {
        router.layer(axum::middleware::from_fn(crate::offline_request_context::middleware))
    });
    let response = app.oneshot(Request::post("/openai/v1/responses")
        .body(Body::empty()).unwrap()).await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let _ = axum::body::to_bytes(response.into_body(), 16384).await.unwrap();
    assert!(ports.events.lock().unwrap().is_empty());
    let mut seal = state.operation_ownership.as_ref().unwrap().seal().unwrap();
    tokio::time::timeout(Duration::from_secs(1), seal.wait_application()).await.unwrap().unwrap();
}

#[tokio::test]
async fn routing_http_context_crosses_the_real_owned_server_router() {
    let (mut state, ports, spec, sessions) = fixture();
    let source_ports = ports.clone();
    let provider = Provider::with_sources(state.clone(), sessions,
        Arc::new(crate::offline_request_context::Current), port::Policy {
            timeout: Duration::from_secs(3), max_body_bytes: 65536,
            missing_content_type: "application/json".into(), chunk_tokens: 32,
            max_chunks: 1, continuation: None, allow_paid: false, credit_floor: 1.0,
        }, None, Arc::new(move |_| Arc::new(Builder(source_ports.clone())))).unwrap();
    state.native_dispatch = Some(Arc::new(provider));
    ports.load_failure.store(2, Ordering::SeqCst);
    let app = framer_server::router_with_request_layer(state.clone(), "fixture-no-web-root", |router| {
        router.layer(axum::middleware::from_fn(crate::offline_request_context::middleware))
    });
    let metadata = serde_json::json!({"page_url":"https://framer.com/projects/fixture-project",
        "proxy":null,"attempts":[{"feature_category":"actual-category","feature_name":"actual-feature",
        "step":"actual-step","turn_id":"actual-turn","user_prompt":"actual display"}]});
    let response = app.oneshot(Request::post("/openai/v1/responses")
        .header("content-type", "application/json")
        .header("x-framerize-session", spec.identity.chat_session.as_str())
        .header("x-framerize-context", metadata.to_string())
        .body(Body::from(r#"{"model":"gpt-4.1","input":"hello"}"#)).unwrap()).await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    let bytes = axum::body::to_bytes(response.into_body(), 16384).await.unwrap();
    let error: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(error["error"]["code"], "Authorization");
    assert_eq!(*ports.events.lock().unwrap(), vec!["status", "load"]);
    assert_eq!(users(&state, spec.identity.account).await, 0);
    assert!(state.operation_ownership.as_ref().unwrap().seal().is_ok());
}

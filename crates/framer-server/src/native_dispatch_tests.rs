use super::*;
use serde_json::json;
use tower::ServiceExt;
use futures::FutureExt;
use std::time::Duration;

#[path = "native_dispatch_fixture.rs"]
mod fixture;
use fixture::{fixture, account};

#[tokio::test]
async fn server_native_honors_explicit_account_without_repick() {
    for known in [false, true] {
        let (state, spy, provider) = fixture(true, None);
        let mut headers = HeaderMap::new();
        headers.insert("x-framerize-session", HeaderValue::from_static("fixture-chat"));
        let id = if known { account().id.0.to_string() } else { uuid::Uuid::from_u128(2).to_string() };
        headers.insert("x-framerize-account", HeaderValue::from_str(&id).unwrap());
        let response = chain(&state, &headers, ClientDialect::OpenAiResponses, response_payload()).await;
        assert_eq!(response.status(), if known { StatusCode::OK } else { StatusCode::SERVICE_UNAVAILABLE });
        assert_eq!(*provider.preparations.lock().unwrap(), usize::from(known));
        assert_eq!(provider.records.lock().unwrap().len(), usize::from(known));
        assert!(spy.calls.lock().unwrap().is_empty());
    }
}

#[tokio::test]
async fn server_native_closed_scope_never_falls_back_to_legacy() {
    let (mut state, spy, mut provider) = fixture(true, None);
    state.native_dispatch = None;
    Arc::get_mut(&mut provider).unwrap().prepare_failure = Some(FailureKind::ScopeClosed);
    state.native_dispatch = Some(provider.clone());
    assert_eq!(request(state, "/openai/v1/responses", response_payload(), false, Some("fixture-chat")).await.status(), StatusCode::CONFLICT);
    assert_eq!(*provider.preparations.lock().unwrap(), 1);
    assert!(provider.records.lock().unwrap().is_empty());
    assert!(spy.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn server_native_rejects_stateful_options_and_conflicting_budgets() {
    for delta in [
        json!({"store":false}), json!({"background":true}),
        json!({"previous_response_id":"fixture-prior"}), json!({"instructions":["fixture"]}),
        json!({"max_tokens":64}), json!({"max_tokens":"32"}),
        json!({"max_output_tokens":0}), json!({"max_output_tokens":4294967296_u64}),
    ] {
        let (state, spy, provider) = fixture(true, None);
        let mut payload = response_payload();
        payload.as_object_mut().unwrap().extend(delta.as_object().unwrap().clone());
        assert_eq!(request(state, "/openai/v1/responses", payload, false, Some("fixture-chat")).await.status(), StatusCode::BAD_REQUEST);
        assert_eq!(*provider.preparations.lock().unwrap(), 0);
        assert!(spy.calls.lock().unwrap().is_empty());
    }
}

#[tokio::test]
async fn server_native_deadline_does_not_wait_for_independent_cleanup() {
    struct Cancel(std::sync::atomic::AtomicUsize);
    impl framer_core::native_dispatch::Cancellation for Cancel {
        fn cancel(&self) { self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst); }
    }
    let (release, pending) = tokio::sync::oneshot::channel();
    let cleanup = async move { pending.await.map_err(|_| FailureKind::Cleanup) }.boxed().shared();
    let cancellation = Arc::new(Cancel(std::sync::atomic::AtomicUsize::new(0)));
    let run = InvocationRun { result: async { Ok(42) }.boxed(), cleanup: cleanup.clone(), cancellation: cancellation.clone(),
        completed_attempts: Some(Arc::new(|| vec![AttemptMetadata { status: 201, step_index: 0,
            usage_header: Some("fixture-prior".into()), original_content_type: None,
            elapsed: Duration::from_millis(1) }])) };
    let error = await_run(run, Instant::now() + Duration::from_millis(50)).await.err().unwrap();
    assert_eq!(error.kind, FailureKind::Deadline);
    assert_eq!(error.attempts.len(), 1);
    assert_eq!(error.attempts[0].status, 201);
    assert_eq!(cancellation.0.load(std::sync::atomic::Ordering::SeqCst), 1);
    assert!(cleanup.clone().now_or_never().is_none());
    release.send(()).unwrap();
    assert_eq!(cleanup.await, Ok(()));
}

#[tokio::test]
async fn server_native_chat_rejects_fields_that_conversion_would_discard() {
    for delta in [
        json!({"tool_choice":{"type":"function","function":{"name":"read","extension":1}}}),
        json!({"tool_choice":{"type":"function","function":{"wrong":"read"}}}),
        json!({"response_format":{"type":"json_schema","json_schema":{"name":"fixture"},"extension":1}}),
        json!({"response_format":{"type":"json_object"},"text":{"format":{"type":"text"}}}),
        json!({"messages":[{"role":"assistant","content":null,"tool_calls":[{"type":"function","id":"fixture","function":{"name":"read","arguments":"{}"},"extension":1}]}]}),
        json!({"messages":[{"role":"assistant","content":"fixture","tool_calls":{}}]}),
    ] {
        let (state, spy, provider) = fixture(true, None);
        let mut payload = json!({"model":"gpt-4.1","messages":[{"role":"user","content":"fixture"}]});
        payload.as_object_mut().unwrap().extend(delta.as_object().unwrap().clone());
        assert_eq!(request(state, "/openai/v1/chat/completions", payload, false, Some("fixture-chat")).await.status(), StatusCode::BAD_REQUEST);
        assert_eq!(*provider.preparations.lock().unwrap(), 0);
        assert!(spy.calls.lock().unwrap().is_empty());
    }
}

async fn request(state: AppState, path: &str, body: Value, paid: bool,
    session: Option<&str>) -> Response {
    let mut builder = axum::http::Request::builder().method("POST").uri(path)
        .header("content-type", "application/json");
    if paid { builder = builder.header("x-framerize-mode", "paid"); }
    if let Some(session) = session { builder = builder.header("x-framerize-session", session); }
    tokio::time::timeout(Duration::from_secs(3), crate::router(state, "/fixture-no-web").oneshot(builder
        .body(axum::body::Body::from(body.to_string())).unwrap())).await.unwrap().unwrap()
}

#[tokio::test]
async fn server_native_chat_preserves_nested_format_and_explicit_choice() {
    let (state, spy, provider) = fixture(true, None);
    let schema = json!({"name":"fixture","strict":true,"schema":{"type":"object"},"extension":42});
    let payload = json!({"model":"gpt-4.1","messages":[{"role":"system","content":"fixture"}],
        "tool_choice":{"type":"function","function":{"name":"read"}},
        "response_format":{"type":"json_schema","json_schema":schema},
        "text":{"verbosity":"low"}});
    assert_eq!(request(state, "/openai/v1/chat/completions", payload, false, Some("fixture-chat")).await.status(), StatusCode::OK);
    let records = provider.records.lock().unwrap();
    let result = &records[0].payload;
    assert_eq!(result["input"][0]["role"], "system");
    assert_eq!(result["tool_choice"], json!({"type":"function","name":"read"}));
    assert_eq!(result["text"]["verbosity"], "low");
    let mut expected = schema; expected["type"] = json!("json_schema");
    assert_eq!(result["text"]["format"], expected);
    assert!(spy.calls.lock().unwrap().is_empty());
}

fn response_payload() -> Value {
    json!({"model":"gpt-4.1","stream":false,"max_output_tokens":32,
        "input":[{"type":"reasoning","id":"fixture-r","encrypted_content":"fixture-opaque"},
            {"type":"function_call","call_id":"fixture-call","name":"read",
             "namespace":"fixture","arguments":"{}"},
            {"type":"function_call_output","call_id":"fixture-call","output":"fixture result"}],
        "tools":[{"type":"function","name":"read","namespace":"fixture","strict":true}],
        "reasoning":{"effort":"max"},"service_tier":"priority",
        "fixture_extension":{"keep":[null,true,3]}})
}

#[tokio::test]
async fn server_native_responses_preserves_typed_history_and_fields() {
    let (state, spy, provider) = fixture(true, None);
    let payload = response_payload();
    let response = request(state, "/openai/v1/responses", payload.clone(), false, Some("fixture-chat")).await;
    assert_eq!(response.status(), StatusCode::OK);
    let records = provider.records.lock().unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].method, "responses");
    for key in ["input", "tools", "reasoning", "service_tier", "fixture_extension"] {
        assert_eq!(records[0].payload[key], payload[key], "field {key}");
    }
    assert!(spy.calls.lock().unwrap().is_empty());
    let metadata = response.extensions().get::<Arc<Vec<AttemptMetadata>>>().unwrap();
    assert_eq!(metadata[0].usage_header.as_deref(), Some("fixture-usage"));
}

#[tokio::test]
async fn server_native_messages_preserves_thinking_tools_and_extensions() {
    let (state, spy, provider) = fixture(true, None);
    let payload = json!({"model":"claude-sonnet-5-5","stream":false,"max_tokens":32,
        "messages":[{"role":"assistant","content":[
            {"type":"thinking","thinking":"fixture","signature":"fixture-signed"},
            {"type":"tool_use","id":"fixture-tool","name":"read","input":{"x":1}}]},
            {"role":"user","content":[{"type":"tool_result","tool_use_id":"fixture-tool","content":"fixture"}]}],
        "tools":[{"name":"read","input_schema":{"type":"object"}}],
        "thinking":{"type":"enabled","budget_tokens":4096},
        "fixture_extension":{"opaque":true}});
    let response = request(state, "/anthropic/v1/messages", payload.clone(), false, Some("fixture-chat")).await;
    assert_eq!(response.status(), StatusCode::OK);
    let records = provider.records.lock().unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].method, "messages");
    for key in ["messages", "tools", "thinking", "fixture_extension"] {
        assert_eq!(records[0].payload[key], payload[key], "field {key}");
    }
    assert!(spy.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn server_native_chat_preserves_strict_tools_reasoning_and_image_detail() {
    let (state, spy, provider) = fixture(true, None);
    let payload = json!({"model":"gpt-4.1","stream":false,"max_tokens":32,
        "messages":[{"role":"developer","content":[{"type":"text","text":"one"},{"type":"text","text":"two"}]},
            {"role":"user","content":[{"type":"text","text":"fixture"},
                {"type":"image_url","image_url":{"url":"https://fixture.invalid/image","detail":"high"}}]},
            {"role":"assistant","content":[{"type":"text","text":""}],
                "tool_calls":[{"type":"function","id":"fixture-call","function":{"name":"read","arguments":"{}"}}]},
            {"role":"tool","tool_call_id":"fixture-call","content":[{"type":"text","text":"fixture-result"}]}],
        "tools":[{"type":"function","function":{"name":"read","strict":true,"parameters":{"type":"object"}}}],
        "reasoning_effort":"max","service_tier":"priority","fixture_extension":{"opaque":true}});
    let response = request(state, "/openai/v1/chat/completions", payload, false, Some("fixture-chat")).await;
    assert_eq!(response.status(), StatusCode::OK);
    let records = provider.records.lock().unwrap();
    let prepared = &records[0].payload;
    assert_eq!(records[0].method, "responses");
    assert_eq!(prepared["input"][0]["content"].as_array().unwrap().len(), 2);
    assert_eq!(prepared["input"][1]["content"][1]["detail"], "high");
    assert_eq!(prepared["input"][2]["content"], json!([{"type":"output_text","text":""}]));
    assert_eq!(prepared["input"][3]["type"], "function_call");
    assert_eq!(prepared["input"][4]["output"], json!([{"type":"input_text","text":"fixture-result"}]));
    assert_eq!(prepared["tools"][0]["strict"], true);
    assert_eq!(prepared["reasoning"]["effort"], "max");
    assert_eq!(prepared["fixture_extension"]["opaque"], true);
    assert!(spy.calls.lock().unwrap().is_empty());
}

const RAW_ROUTES: [(&str, ClientDialect, bool); 6] = [
    ("/paid/anthropic/v1/messages", ClientDialect::AnthropicMessages, false),
    ("/paid/openai/v1/responses", ClientDialect::OpenAiResponses, false),
    ("/paid/openai/v1/chat/completions", ClientDialect::OpenAiChat, false),
    ("/anthropic/v1/messages", ClientDialect::AnthropicMessages, true),
    ("/openai/v1/responses", ClientDialect::OpenAiResponses, true),
    ("/openai/v1/chat/completions", ClientDialect::OpenAiChat, true),
];

#[tokio::test]
async fn server_native_raw_aliases_keep_exact_payload_and_dialect() {
    for (path, dialect, paid) in RAW_ROUTES {
        let (state, spy, provider) = fixture(true, None);
        let payload = json!({"model":"gpt-4.1","stream":false,"input":"fixture",
            "messages":[{"role":"user","content":"fixture"}],"fixture_extension":42});
        let response = request(state, path, payload.clone(), paid, Some("fixture-chat")).await;
        assert_eq!(response.status(), StatusCode::OK, "route {path}");
        let records = provider.records.lock().unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].route, Route::Raw(dialect));
        assert_eq!(records[0].method, "raw");
        assert_eq!(records[0].payload, payload);
        assert!(spy.calls.lock().unwrap().is_empty());
    }
}

#[tokio::test]
async fn server_native_streaming_rejects_before_provider_or_legacy_io() {
    for (path, _, paid) in RAW_ROUTES.into_iter().chain([
        ("/anthropic/v1/messages", ClientDialect::AnthropicMessages, false),
        ("/openai/v1/responses", ClientDialect::OpenAiResponses, false),
        ("/openai/v1/chat/completions", ClientDialect::OpenAiChat, false),
    ]) {
        let (state, spy, provider) = fixture(true, None);
        let response = request(state, path, json!({"model":"gpt-4.1","stream":true}), paid, Some("fixture-chat")).await;
        assert_eq!(response.status(), StatusCode::NOT_IMPLEMENTED);
        assert_eq!(*provider.preparations.lock().unwrap(), 0);
        assert!(provider.records.lock().unwrap().is_empty());
        assert!(spy.calls.lock().unwrap().is_empty());
    }
}

#[tokio::test]
async fn server_native_refusals_preserve_status_bytes_and_never_retry() {
    for status in [403, 429] {
        for path in ["/openai/v1/responses", "/paid/openai/v1/responses"] {
            let (state, spy, provider) = fixture(true, Some(status));
            let response = request(state, path, response_payload(), false, Some("fixture-chat")).await;
            assert_eq!(response.status().as_u16(), status);
            assert_eq!(axum::body::to_bytes(response.into_body(), 4096).await.unwrap().as_ref(),
                b"fixture upstream rejection\nexact bytes");
            assert_eq!(*provider.preparations.lock().unwrap(), 1);
            assert_eq!(provider.records.lock().unwrap().len(), 1);
            assert!(spy.calls.lock().unwrap().is_empty());
        }
    }
}

#[tokio::test]
async fn server_native_missing_context_and_unsupported_chat_fail_before_prepare() {
    let (state, spy, provider) = fixture(true, None);
    let response = request(state, "/openai/v1/responses", response_payload(), false, None).await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(*provider.preparations.lock().unwrap(), 0);
    assert!(spy.calls.lock().unwrap().is_empty());
    for payload in [
        json!({"model":"gpt-4.1","messages":[{"role":"user","content":[{"type":"audio","data":"fixture"}]}]}),
        json!({"model":"gpt-4.1","messages":[{"role":"user","content":"fixture"}],"tools":[{"type":"custom","name":"fixture"}]}),
    ] {
        let (state, spy, provider) = fixture(true, None);
        let response = request(state, "/openai/v1/chat/completions", payload, false, Some("fixture-chat")).await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert_eq!(*provider.preparations.lock().unwrap(), 0);
        assert!(spy.calls.lock().unwrap().is_empty());
    }
}

#[tokio::test]
async fn server_native_rejects_tier_substitution_and_retains_supported_tier() {
    for model in ["gpt-6-sol-ultrafast", "gpt-4.1-ultrafast"] {
        let (state, spy, provider) = fixture(true, None);
        let mut payload = response_payload();
        payload["model"] = json!(model);
        let response = request(state, "/openai/v1/responses", payload, false, Some("fixture-chat")).await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert_eq!(*provider.preparations.lock().unwrap(), 0);
        assert!(spy.calls.lock().unwrap().is_empty());
    }
    let (state, _, provider) = fixture(true, None);
    let mut payload = response_payload();
    payload["model"] = json!("gpt-4.1-ultrafast"); payload["service_tier"] = json!("ultrafast");
    assert_eq!(request(state, "/openai/v1/responses", payload, false, Some("fixture-chat")).await.status(), StatusCode::OK);
    assert_eq!(provider.records.lock().unwrap()[0].payload["service_tier"], "ultrafast");
}

#[tokio::test]
async fn server_native_binds_one_identity_and_absolute_deadline() {
    let (state, spy, provider) = fixture(true, None);
    let before = Instant::now();
    let response = request(state, "/openai/v1/responses", response_payload(), false, Some("fixture-exact-session")).await;
    assert_eq!(response.status(), StatusCode::OK);
    let after = Instant::now();
    let records = provider.records.lock().unwrap();
    assert_eq!(records.len(), 1);
    let expected = account();
    assert_eq!(records[0].identity.account, expected.id);
    assert_eq!(records[0].identity.user, expected.framer_user_id);
    assert_eq!(records[0].identity.project, expected.current_project.unwrap());
    assert_eq!(records[0].identity.chat_session, "fixture-exact-session");
    assert!(records[0].deadline >= before + provider.policy.timeout);
    assert!(records[0].deadline <= after + provider.policy.timeout);
    assert!(spy.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn server_absent_provider_keeps_all_three_legacy_entries() {
    for (path, method, payload) in [
        ("/paid/openai/v1/responses", "proxy", response_payload()),
        ("/openai/v1/responses", "responses", response_payload()),
        ("/anthropic/v1/messages", "messages", json!({"model":"claude-sonnet-5-5",
            "stream":false,"messages":[{"role":"user","content":"fixture"}]})),
    ] {
        let (state, spy, provider) = fixture(false, None);
        let response = request(state, path, payload, false, Some("fixture-chat")).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert!(spy.calls.lock().unwrap().contains(&method));
        assert_eq!(*provider.preparations.lock().unwrap(), 0);
        assert!(provider.records.lock().unwrap().is_empty());
    }
}

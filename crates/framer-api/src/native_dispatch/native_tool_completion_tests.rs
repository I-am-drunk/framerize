include!("composed_fixture.rs");

#[test]
fn native_completed_tool_at_actual_budget_is_withheld_without_retry() {
    for kind in ["function_call", "custom_tool_call"] {
        rejected_collected(tool_body(kind, 32));
    }
}

#[test]
fn native_explicitly_incomplete_tool_below_budget_is_withheld() {
    let mut body = tool_body("custom_tool_call", 7);
    body["status"] = json!("incomplete");
    body["incomplete_details"] = json!({"reason":"max_output_tokens"});
    rejected_collected(body);
}

#[test]
fn native_tool_followed_by_text_cannot_hide_budget_exhaustion() {
    let mut body = tool_body("function_call", 33);
    body["output"].as_array_mut().unwrap().push(minimal_body()["output"][0].clone());
    rejected_collected(body);
}

#[test]
fn native_valid_tool_below_budget_preserves_fields_and_usage() {
    for kind in ["function_call", "custom_tool_call"] {
        let body = tool_body(kind, 31);
        accepted_collected(body);
    }
}

#[test]
fn native_completed_text_at_budget_stays_complete() {
    let mut body = minimal_body();
    body["usage"]["output_tokens"] = json!(32);
    accepted_collected(body);
}

#[test]
fn native_tool_guard_uses_remaining_budget_and_keeps_prior_evidence() {
    let (first, next) = responses_continuation();
    let body = tool_body("custom_tool_call", 25);
    let server = Server::new(vec![
        expected("/ai/v3/responses/", expected_responses(), 0, Reply::json(first)),
        expected("/ai/v3/responses/", next.clone(), 1, Reply::json(body.clone())),
    ]);
    run(async {
        let mut configured = policy(); configured.continuation = Some(CALLER_CONTINUE.into());
        let f = Fixture::new(&server, Supplier::default(), configured);
        let until = deadline();
        let started = f.admit(Route::Responses, until).await.unwrap().responses(responses_payload());
        let progress = started.completed_attempts.clone().unwrap();
        let error = completed(started).await.err().unwrap();
        assert_eq!(error.kind, FailureKind::UnsupportedResponse);
        assert_eq!(error.attempts.len(), 1);
        metadata(&error.attempts[0], 0);
        let failure = error.upstream.unwrap();
        metadata(&failure.metadata, 1);
        assert_eq!(serde_json::from_slice::<Value>(&failure.body).unwrap(), body);
        assert_eq!(progress().len(), 2);
        seen(&f.supplier, &f.scope, until, "OpenRouterResponses", &[expected_responses(), next]);
        f.finish().await;
    });
    checked(&server.finish(), 2, 2);
}

#[test]
fn composed_capped_tool_emits_no_tool_frame_or_success_in_either_dialect() {
    for dialect in [ClientDialect::OpenAiResponses, ClientDialect::OpenAiChat] {
        let mut body = tool_body("function_call", 32);
        if dialect == ClientDialect::OpenAiChat {
            body["output"][0].as_object_mut().unwrap().remove("id");
        }
        let server = Server::new(vec![expected("/ai/v3/responses/", expected_responses(), 0,
            Reply::json(body.clone()))]);
        run(async {
            let f = Fixture::new(&server, Supplier::default(), policy());
            let started = compose(&f, Route::Responses, responses_payload(), options(dialect), deadline()).await;
            let head = started.response.result.await.unwrap();
            let (bytes, error) = composed_outcome(head.body).await;
            let frames = if dialect == ClientDialect::OpenAiChat {
                assert_eq!(error, Some(FailureKind::UnsupportedResponse));
                assert!(bytes.is_empty());
                Vec::new()
            } else {
                responses_failure_frames(&bytes, error)
            };
            assert!(frames.iter().all(|frame| frame["type"] == "error"));
            let done = summary(started.finished).await;
            assert_eq!(done.result, Err(FailureKind::UnsupportedResponse));
            assert_eq!(done.attempts.len(), 1);
            assert_eq!(done.attempts[0].usage.as_deref(), Some(&body["usage"]));
            cleanup(started.response.cleanup).await;
            assert_eq!(f.supplier.calls.load(Ordering::Acquire), 1);
            f.finish().await;
        });
        checked(&server.finish(), 1, 1);
    }
}

#[test]
fn composed_valid_tool_below_budget_keeps_tool_delivery() {
    for dialect in [ClientDialect::OpenAiResponses, ClientDialect::OpenAiChat] {
        let mut body = tool_body("function_call", 31);
        if dialect == ClientDialect::OpenAiChat {
            body["output"][0].as_object_mut().unwrap().remove("id");
        }
        let server = Server::new(vec![expected("/ai/v3/responses/", expected_responses(), 0,
            Reply::json(body.clone()))]);
        run(async {
            let f = Fixture::new(&server, Supplier::default(), policy());
            let started = compose(&f, Route::Responses, responses_payload(), options(dialect), deadline()).await;
            let (bytes, done) = composed_finished(started).await;
            assert_eq!(done.result, Ok(()));
            assert_eq!(done.attempts.len(), 1);
            let frames = events(&bytes);
            if dialect == ClientDialect::OpenAiResponses {
                assert_eq!(completed_output(&frames), &body["output"]);
            } else {
                assert!(frames.iter().any(|f| f["choices"][0]["delta"]["tool_calls"].is_array()));
                assert!(bytes.ends_with(b"data: [DONE]\n\n"));
            }
            assert_eq!(f.supplier.calls.load(Ordering::Acquire), 1);
            f.finish().await;
        });
        checked(&server.finish(), 1, 1);
    }
}

#[test]
fn composed_later_capped_tool_keeps_prefix_evidence_but_no_tool_or_success() {
    let (first, next) = responses_continuation();
    let server = Server::new(vec![
        expected("/ai/v3/responses/", expected_responses(), 0, Reply::json(first)),
        expected("/ai/v3/responses/", next, 1, Reply::json(tool_body("custom_tool_call", 25))),
    ]);
    run(async {
        let mut configured = policy(); configured.continuation = Some(CALLER_CONTINUE.into());
        let f = Fixture::new(&server, Supplier::default(), configured);
        let started = compose(&f, Route::Responses, responses_payload(),
            options(ClientDialect::OpenAiResponses), deadline()).await;
        let head = started.response.result.await.unwrap();
        let (bytes, error) = composed_outcome(head.body).await;
        let frames = responses_failure_frames(&bytes, error);
        assert!(frames.iter().any(|f| f["type"] == "response.output_text.delta"));
        assert!(!frames.iter().any(|f| f["item"]["type"] == "custom_tool_call"));
        assert!(!frames.iter().any(|f| f["type"].as_str()
            .is_some_and(|name| name.contains("custom_tool_call_input"))));
        let done = summary(started.finished).await;
        assert_eq!(done.result, Err(FailureKind::UnsupportedResponse));
        assert_eq!(done.attempts.len(), 2);
        assert_eq!(done.attempts[0].step_index, 0);
        assert_eq!(done.attempts[1].step_index, 1);
        cleanup(started.response.cleanup).await;
        assert_eq!(f.supplier.calls.load(Ordering::Acquire), 2);
        f.finish().await;
    });
    checked(&server.finish(), 2, 2);
}

#[test]
fn composed_explicit_incomplete_tool_is_withheld_below_budget() {
    let mut body = tool_body("custom_tool_call", 7);
    body["status"] = json!("incomplete");
    body["incomplete_details"] = json!({"reason":"max_output_tokens"});
    let server = Server::new(vec![expected("/ai/v3/responses/", expected_responses(), 0,
        Reply::json(body))]);
    run(async {
        let f = Fixture::new(&server, Supplier::default(), policy());
        let started = compose(&f, Route::Responses, responses_payload(),
            options(ClientDialect::OpenAiResponses), deadline()).await;
        let head = started.response.result.await.unwrap();
        let (bytes, error) = composed_outcome(head.body).await;
        let frames = responses_failure_frames(&bytes, error);
        assert!(frames.iter().all(|frame| frame["type"] == "error"));
        assert_eq!(summary(started.finished).await.result, Err(FailureKind::UnsupportedResponse));
        cleanup(started.response.cleanup).await;
        assert_eq!(f.supplier.calls.load(Ordering::Acquire), 1);
        f.finish().await;
    });
    checked(&server.finish(), 1, 1);
}

#[test]
fn native_capped_tool_never_invokes_the_event_callback() {
    let server = Server::new(vec![expected("/ai/v3/responses/", expected_responses(), 0,
        Reply::json(tool_body("custom_tool_call", 32)))]);
    run(async {
        let f = Fixture::new(&server, Supplier::default(), policy());
        let calls = AtomicUsize::new(0);
        let event = |_: framer_core::convert::ChainEvent| { calls.fetch_add(1, Ordering::Relaxed); };
        let native = crate::experimental_native::application::NativeInvocation {
            client: &f.client, scope: &f.scope, authorization: Some(f.supplier.as_ref()),
            deadline: deadline(), missing_content_type: MISSING_TYPE, max_body_bytes: 16 * 1024,
        };
        let cfg = framer_core::port::ChainConfig { chunk_tokens: 32,
            max_total_tokens: Some(32), max_chunks: 3, metered_fallback: false,
            ..Default::default() };
        let result = crate::nativechain::complete_long_native(&native, &project(),
            responses_payload(), &cfg, CHAT, Some(&event)).await;
        assert!(matches!(result,
            Err(crate::experimental_native::application::ApplicationError::InvalidResponse { .. })));
        assert_eq!(calls.load(Ordering::Relaxed), 0);
        assert_eq!(f.supplier.calls.load(Ordering::Acquire), 1);
        f.finish().await;
    });
    checked(&server.finish(), 1, 1);
}

fn accepted_collected(body: Value) {
    let server = Server::new(vec![expected("/ai/v3/responses/", expected_responses(), 0,
        Reply::json(body.clone()))]);
    run(async {
        let f = Fixture::new(&server, Supplier::default(), policy());
        let call = f.admit(Route::Responses, deadline()).await.unwrap();
        let completed = completed(call.responses(responses_payload())).await.unwrap();
        assert_eq!(completed.value.output, *body["output"].as_array().unwrap());
        assert_eq!(completed.value.usage, body["usage"]);
        assert_eq!(completed.value.status, "completed");
        assert_eq!(completed.attempts.len(), 1);
        assert_eq!(f.supplier.calls.load(Ordering::Acquire), 1);
        f.finish().await;
    });
    checked(&server.finish(), 1, 1);
}

fn responses_failure_frames(bytes: &[u8], error: Option<FailureKind>) -> Vec<Value> {
    // Responses delivers the failure in its terminal event, followed by EOF.
    // The separate completion still retains the typed failure.
    assert_eq!(error, None);
    assert!(!bytes.windows(6).any(|part| part == b"[DONE]"));
    let frames = events(bytes);
    no_success(&frames);
    assert_eq!(frames.iter().filter(|frame| frame["type"] == "error").count(), 1);
    let terminal = frames.last().expect("missing Responses error event");
    assert_eq!(terminal["type"], "error");
    assert_eq!(terminal["code"], "server_error");
    assert_eq!(terminal["message"], "unsupported upstream response");
    assert!(terminal["param"].is_null());
    frames
}

fn tool_body(kind: &str, tokens: u64) -> Value {
    let mut body = minimal_body();
    let mut item = json!({"type":kind, "id":"tool-item", "call_id":"tool-call",
        "name":"fixture_tool", "status":"completed"});
    if kind == "custom_tool_call" { item["input"] = json!("text(1);"); }
    else { item["arguments"] = json!("{\"value\":1}"); }
    body["output"] = json!([item]);
    body["usage"]["output_tokens"] = json!(tokens);
    body["usage"]["total_tokens"] = json!(tokens + 2);
    body
}

fn rejected_collected(body: Value) {
    let server = Server::new(vec![expected("/ai/v3/responses/", expected_responses(), 0,
        Reply::json(body.clone()))]);
    run(async {
        let f = Fixture::new(&server, Supplier::default(), policy());
        let call = f.admit(Route::Responses, deadline()).await.unwrap();
        let started = call.responses(responses_payload());
        let progress = started.completed_attempts.clone().unwrap();
        let error = completed(started).await.err().unwrap();
        assert_eq!(error.kind, FailureKind::UnsupportedResponse);
        assert!(error.attempts.is_empty());
        let failure = error.upstream.unwrap();
        metadata(&failure.metadata, 0);
        assert_eq!(serde_json::from_slice::<Value>(&failure.body).unwrap(), body);
        assert_eq!(progress().len(), 1);
        assert_eq!(f.supplier.calls.load(Ordering::Acquire), 1);
        f.finish().await;
    });
    checked(&server.finish(), 1, 1);
}

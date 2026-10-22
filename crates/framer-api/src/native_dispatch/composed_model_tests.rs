// Real composed-provider fixtures. Every upstream request is checked by the
// existing bounded synthetic peer, including model, effort and stream=false.
include!("composed_fixture.rs");

const CLIENT_ALIAS: &str = "gpt-4.1-ultrafast";

#[test]
fn client_model_default_method_never_silently_uses_wire_model_provider() {
    struct Existing(tokio::sync::OwnedSemaphorePermit);
    impl OwnedInvocation for Existing {
        fn composed_stream(self: Box<Self>, _: Value, _: ComposedStreamOptions)
            -> ComposedStreamRun { panic!("alias context must not be discarded") }
        fn raw(self: Box<Self>, _: Value) -> InvocationRun<port::RawResult> { panic!("not called") }
        fn messages(self: Box<Self>, _: Value)
            -> InvocationRun<port::Completed<framer_core::convert::AssembledResponse>> { panic!("not called") }
        fn responses(self: Box<Self>, _: Value)
            -> InvocationRun<port::Completed<framer_core::convert::AssembledResponses>> { panic!("not called") }
    }
    run(async {
        let capacity = Arc::new(Semaphore::new(1));
        let call: Box<dyn OwnedInvocation> = Box::new(Existing(capacity.clone().try_acquire_owned().unwrap()));
        let started = call.composed_stream_for_client(responses_payload(),
            options(ClientDialect::OpenAiResponses), CLIENT_ALIAS.into());
        assert_eq!(started.response.result.await.err().unwrap().kind, FailureKind::UnsupportedStreaming);
        assert_eq!(summary(started.finished).await.result, Err(FailureKind::UnsupportedStreaming));
        cleanup(started.response.cleanup).await;
        assert_eq!(capacity.available_permits(), 1);
    });
}

#[test]
fn client_model_refusal_keeps_original_body_and_its_allocation_lease() {
    let mut reply = Reply::json(json!({"error":{"model":"provider/refusal-model","detail":"unchanged"}}));
    reply.status = 403;
    let wire = reply.body.clone();
    let server = Server::new(vec![expected("/ai/v3/responses/", expected_responses(), 0, reply)]);
    run(async {
        let f = Fixture::new(&server, Supplier::default(), policy());
        let started = compose_client(&f, Route::Responses, responses_payload(),
            options(ClientDialect::OpenAiResponses), CLIENT_ALIAS.into()).await;
        let error = started.response.result.await.err().unwrap();
        assert_eq!(error.kind, FailureKind::Upstream);
        let upstream = error.upstream.unwrap();
        assert_eq!(upstream.body.as_ref(), wire.as_slice());
        assert_eq!(upstream.metadata.status, 403);
        let held = upstream.body.clone();
        drop(upstream);
        assert_eq!(summary(started.finished).await.result, Err(FailureKind::Upstream));
        assert!(started.response.cleanup.clone().now_or_never().is_none());
        assert_eq!(f.capacity.available_permits(), 0);
        assert_eq!(held.as_ref(), wire.as_slice());
        drop(held);
        cleanup(started.response.cleanup).await;
        assert_eq!(f.capacity.available_permits(), 1);
        f.finish().await;
    });
    checked(&server.finish(), 1, 1);
}

#[test]
fn client_model_retained_capacity_is_admitted_before_authorization() {
    let small_payload = json!({"model":"gpt-4.1", "stream":false,
        "max_output_tokens":32, "input":[]});
    let mut expected_payload = small_payload.clone();
    expected_payload["model"] = json!("openai/gpt-4.1");
    let small_body = json!({"id":"resp_capacity", "status":"completed", "output":[],
        "usage":{"input_tokens":0,"output_tokens":0}});
    let server = Server::new(vec![expected("/ai/v3/responses/",
        expected_payload, 0, Reply::json(small_body))]);
    run(async {
        let f = Fixture::new(&server, Supplier::default(), policy());
        let mut requested = options(ClientDialect::OpenAiResponses);
        requested.limits.state_bytes = 1024 * 1024;
        let (_, successful) = composed_finished(compose_client(&f, Route::Responses,
            small_payload.clone(), requested, CLIENT_ALIAS.into()).await).await;
        assert_eq!(successful.result, Ok(()));
        assert_eq!(f.supplier.calls.load(Ordering::Acquire), 1);
        let mut retained = String::with_capacity(2 * 1024 * 1024);
        retained.push_str(CLIENT_ALIAS);
        assert!(retained.len() < requested.limits.state_bytes);
        assert!(retained.capacity() > requested.limits.state_bytes);
        let rejected = compose_client(&f, Route::Responses,
            small_payload.clone(), requested, retained).await;
        assert_eq!(rejected.response.result.await.err().unwrap().kind, FailureKind::BodyTooLarge);
        assert_eq!(summary(rejected.finished).await.result, Err(FailureKind::BodyTooLarge));
        cleanup(rejected.response.cleanup).await;
        assert_eq!(f.supplier.calls.load(Ordering::Acquire), 1);
        assert_eq!(f.capacity.available_permits(), 1);
        f.finish().await;
    });
    checked(&server.finish(), 1, 1);
}

#[test]
fn client_model_empty_alias_is_rejected_before_authorization() {
    let server = Server::new(vec![]);
    run(async {
        let f = Fixture::new(&server, Supplier::default(), policy());
        let rejected = compose_client(&f, Route::Responses, responses_payload(),
            options(ClientDialect::OpenAiResponses), String::new()).await;
        assert_eq!(rejected.response.result.await.err().unwrap().kind, FailureKind::InvalidRequest);
        assert_eq!(summary(rejected.finished).await.result, Err(FailureKind::InvalidRequest));
        cleanup(rejected.response.cleanup).await;
        assert_eq!(f.supplier.calls.load(Ordering::Acquire), 0);
        assert_eq!(f.capacity.available_permits(), 1);
        f.finish().await;
    });
    checked(&server.finish(), 0, 0);
}

#[test]
fn client_model_chat_keeps_alias_wire_ids_tools_and_usage() {
    for include_usage in [false, true] {
        let mut body = minimal_body();
        body["model"] = json!("provider/chat-wire-model");
        body["service_tier"] = json!("priority");
        body["output"].as_array_mut().unwrap().push(json!({"type":"function_call",
            "call_id":"call_exact", "name":"write", "arguments":"{\"text\":\"λ🙂\"}"}));
        let server = Server::new(vec![expected("/ai/v3/responses/",
            expected_responses(), 0, Reply::json(body))]);
        run(async {
            let f = Fixture::new(&server, Supplier::default(), policy());
            let mut requested = options(ClientDialect::OpenAiChat);
            requested.include_usage = include_usage;
            let (bytes, done) = composed_finished(compose_client(&f, Route::Responses,
                responses_payload(), requested, CLIENT_ALIAS.into()).await).await;
            assert_eq!(done.result, Ok(()));
            let emitted = events(&bytes);
            assert!(bytes.ends_with(b"data: [DONE]\n\n"));
            for event in &emitted {
                assert_eq!(event["model"], CLIENT_ALIAS);
                assert_eq!(event["id"], "resp_exact-wire-id");
                assert_eq!(event["service_tier"], "priority");
                assert_eq!(event.get("usage").is_some(), include_usage);
            }
            let tool = &emitted.iter().find(|v| v["choices"][0]["delta"]["tool_calls"].is_array())
                .unwrap()["choices"][0]["delta"]["tool_calls"][0];
            assert_eq!(tool["id"], "call_exact");
            assert_eq!(tool["function"]["arguments"], "{\"text\":\"λ🙂\"}");
            if include_usage {
                assert_eq!(emitted.last().unwrap()["usage"]["total_tokens"], 5);
            }
            assert_eq!(done.attempts[0].response_id.as_deref(), Some("resp_exact-wire-id"));
            f.finish().await;
        });
        checked(&server.finish(), 1, 1);
    }
}

#[test]
fn client_model_is_used_when_upstream_omits_its_model_in_every_dialect() {
    for dialect in [ClientDialect::AnthropicMessages, ClientDialect::OpenAiResponses,
        ClientDialect::OpenAiChat] {
        let messages = dialect == ClientDialect::AnthropicMessages;
        let (route, path, payload, expected_payload, body) = if messages {
            (Route::Messages, "/ai/v3/messages/", messages_payload(), expected_messages(), message_body())
        } else {
            (Route::Responses, "/ai/v3/responses/", responses_payload(), expected_responses(),
                if dialect == ClientDialect::OpenAiChat { minimal_body() } else { response_body() })
        };
        assert!(body.get("model").is_none());
        let server = Server::new(vec![expected(path, expected_payload, 0, Reply::json(body))]);
        run(async {
            let f = Fixture::new(&server, Supplier::default(), policy());
            let (bytes, done) = composed_finished(compose_client(&f, route, payload,
                options(dialect), CLIENT_ALIAS.into()).await).await;
            assert_eq!(done.result, Ok(()));
            let emitted = events(&bytes);
            let models: Vec<_> = emitted.iter().filter_map(|event| match dialect {
                ClientDialect::AnthropicMessages => event["message"]["model"].as_str(),
                ClientDialect::OpenAiResponses => event["response"]["model"].as_str(),
                ClientDialect::OpenAiChat => event["model"].as_str(),
            }).collect();
            assert!(!models.is_empty());
            assert!(models.iter().all(|model| *model == CLIENT_ALIAS));
            f.finish().await;
        });
        checked(&server.finish(), 1, 1);
    }
}

#[test]
fn client_model_messages_keeps_alias_with_exact_blocks_and_attempt_ids() {
    let (mut first, next) = messages_continuation();
    first["id"] = json!("msg_first_exact");
    first["model"] = json!("provider/first-message-model");
    let mut last = message_body();
    last["id"] = json!("msg_last_exact");
    last["model"] = json!("provider/last-message-model");
    last["stop_reason"] = json!("tool_use");
    last["content"] = json!([{"type":"tool_use", "id":"tool_exact", "name":"write",
        "input":{"text":"kept λ🙂"}, "fixture_detail":{"keep":true}}]);
    let server = Server::new(vec![
        expected("/ai/v3/messages/", expected_messages(), 0, Reply::json(first)),
        expected("/ai/v3/messages/", next, 1, Reply::json(last)),
    ]);
    run(async {
        let mut configured = policy();
        configured.continuation = Some(CALLER_CONTINUE.into());
        let f = Fixture::new(&server, Supplier::default(), configured);
        let alias = "claude-fixture-ultrafast";
        let (bytes, done) = composed_finished(compose_client(&f, Route::Messages,
            messages_payload(), options(ClientDialect::AnthropicMessages), alias.into()).await).await;
        assert_eq!(done.result, Ok(()));
        let emitted = events(&bytes);
        let message = &emitted.iter().find(|v| v["type"] == "message_start").unwrap()["message"];
        assert_eq!(message["model"], alias);
        assert_eq!(message["id"], "msg_first_exact");
        assert_eq!(emitted.iter().filter(|v| v["type"] == "message_start").count(), 1);
        let tool = &emitted.iter().find(|v| v["content_block"]["type"] == "tool_use").unwrap()["content_block"];
        assert_eq!(tool["id"], "tool_exact");
        assert_eq!(tool["name"], "write");
        assert_eq!(tool["fixture_detail"], json!({"keep":true}));
        let partial = emitted.iter().find_map(|v| v["delta"]["partial_json"].as_str()).unwrap();
        assert_eq!(serde_json::from_str::<Value>(partial).unwrap(), json!({"text":"kept λ🙂"}));
        assert_eq!(emitted.last().unwrap()["type"], "message_stop");
        assert_eq!(done.attempts[0].response_id.as_deref(), Some("msg_first_exact"));
        assert_eq!(done.attempts[1].response_id.as_deref(), Some("msg_last_exact"));
        f.finish().await;
    });
    checked(&server.finish(), 2, 2);
}

#[test]
fn client_model_responses_keeps_alias_across_continuation_and_exact_items() {
    let (mut first, mut next) = responses_continuation();
    first["id"] = json!("resp_first_exact");
    first["model"] = json!("provider/first-wire-model");
    let mut last = response_body();
    last["id"] = json!("resp_last_exact");
    last["model"] = json!("provider/last-wire-model");
    let mut payload = responses_payload();
    payload["service_tier"] = json!("ultrafast");
    let mut initial = expected_responses();
    initial["service_tier"] = json!("ultrafast");
    next["service_tier"] = json!("ultrafast");
    let mut exact_items = first["output"].as_array().unwrap().clone();
    exact_items.extend(last["output"].as_array().unwrap().iter().cloned());
    let server = Server::new(vec![
        expected("/ai/v3/responses/", initial, 0, Reply::json(first)),
        expected("/ai/v3/responses/", next, 1, Reply::json(last)),
    ]);
    run(async {
        let mut configured = policy();
        configured.continuation = Some(CALLER_CONTINUE.into());
        let f = Fixture::new(&server, Supplier::default(), configured);
        let (bytes, done) = composed_finished(compose_client(&f, Route::Responses,
            payload, options(ClientDialect::OpenAiResponses), CLIENT_ALIAS.into()).await).await;
        assert_eq!(done.result, Ok(()));
        let emitted = events(&bytes);
        let envelopes: Vec<_> = emitted.iter().filter_map(|event| event.get("response")).collect();
        assert_eq!(envelopes.len(), 3);
        for response in envelopes {
            assert_eq!(response["model"], CLIENT_ALIAS);
            assert_eq!(response["id"], "resp_first_exact");
        }
        assert_eq!(completed_output(&emitted), &Value::Array(exact_items));
        assert_eq!(done.attempts.len(), 2);
        assert_eq!(done.attempts[0].response_id.as_deref(), Some("resp_first_exact"));
        assert_eq!(done.attempts[1].response_id.as_deref(), Some("resp_last_exact"));
        assert_eq!(f.supplier.calls.load(Ordering::Acquire), 2);
        f.finish().await;
    });
    checked(&server.finish(), 2, 2);
}

async fn compose_client(f: &Fixture, route: Route, payload: Value,
    options: ComposedStreamOptions, client_model: String) -> ComposedStreamRun {
    f.admit(route, deadline()).await.unwrap()
        .composed_stream_for_client(payload, options, client_model)
}

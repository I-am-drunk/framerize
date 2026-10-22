include!("composed_fixture.rs");

#[test]
fn composed_completed_attempt_observation_keeps_original_usage_header() {
    struct Recording { inner: Arc<Supplier>, statuses: Mutex<Vec<u16>> }
    impl AttemptAuthorization for Recording {
        fn authorize<'a>(&'a self, attempt: PreparedAttempt<'a>) -> BoxFuture<'a, Result<AuthorizedAttempt, String>> {
            self.inner.authorize(attempt)
        }
        fn observe_finish(&self, metadata: &crate::experimental_native::application::AttemptMetadata) {
            assert_eq!(metadata.usage_header.as_deref(), Some("remaining=7.5&fixture=application"));
            assert_eq!(metadata.original_content_type.as_deref(), Some("application/json"));
            assert_eq!(metadata.step_index, 0);
            self.statuses.lock().unwrap().push(metadata.status);
        }
    }
    for status in [200, 402] {
        let mut reply = Reply::json(response_body()); reply.status = status;
        let server = Server::new(vec![expected("/ai/v3/responses/", expected_responses(), 0, reply)]);
        run(async {
            let mut f = Fixture::new(&server, Supplier::default(), policy());
            let observer = Arc::new(Recording { inner: f.supplier.clone(), statuses: Mutex::new(Vec::new()) });
            f.provider.bindings.get_mut(&f.scope.account()).unwrap().authorization = Some(observer.clone());
            let started = compose(&f, Route::Responses, responses_payload(), options(ClientDialect::OpenAiResponses), deadline()).await;
            if status == 200 {
                assert_eq!(composed_finished(started).await.1.result, Ok(()));
            } else {
                assert_eq!(started.response.result.await.err().unwrap().kind, FailureKind::Upstream);
                assert_eq!(summary(started.finished).await.result, Err(FailureKind::Upstream));
                cleanup(started.response.cleanup).await;
            }
            assert_eq!(*observer.statuses.lock().unwrap(), vec![status]); f.finish().await;
        });
        checked(&server.finish(), 1, 1);
    }
}

#[test]
fn composed_chat_refuses_a_second_wire_response_identity() {
    let mut body = minimal_body(); body["status"] = json!("incomplete");
    body["incomplete_details"] = json!({"reason":"max_output_tokens"});
    let server = Server::new(vec![expected("/ai/v3/responses/", expected_responses(), 0, Reply::json(body))]);
    run(async {
        let mut configured = policy(); configured.continuation = Some(CALLER_CONTINUE.into());
        let f = Fixture::new(&server, Supplier::default(), configured);
        let started = compose(&f, Route::Responses, responses_payload(), options(ClientDialect::OpenAiChat), deadline()).await;
        let head = started.response.result.await.unwrap();
        let (bytes, error) = composed_outcome(head.body).await;
        assert_eq!(error, Some(FailureKind::UnsupportedResponse));
        assert!(!bytes.windows(6).any(|window| window == b"[DONE]"));
        assert!(events(&bytes).iter().all(|event| event["choices"][0]["finish_reason"].is_null()));
        assert_eq!(summary(started.finished).await.result, Err(FailureKind::UnsupportedResponse));
        cleanup(started.response.cleanup).await; f.finish().await;
    });
    checked(&server.finish(), 1, 1);
}

#[test]
fn composed_default_provider_is_unsupported_and_releases_unused_capacity() {
    struct Existing(tokio::sync::OwnedSemaphorePermit);
    impl OwnedInvocation for Existing {
        fn raw(self: Box<Self>, _: Value) -> InvocationRun<port::RawResult> { panic!("not called") }
        fn messages(self: Box<Self>, _: Value)
            -> InvocationRun<port::Completed<framer_core::convert::AssembledResponse>> { panic!("not called") }
        fn responses(self: Box<Self>, _: Value)
            -> InvocationRun<port::Completed<framer_core::convert::AssembledResponses>> { panic!("not called") }
    }
    run(async {
        let capacity = Arc::new(Semaphore::new(1));
        let call: Box<dyn OwnedInvocation> = Box::new(Existing(capacity.clone().try_acquire_owned().unwrap()));
        let started = call.composed_stream(responses_payload(), options(ClientDialect::OpenAiResponses));
        assert_eq!(started.response.result.await.err().unwrap().kind, FailureKind::UnsupportedStreaming);
        assert_eq!(summary(started.finished).await.result, Err(FailureKind::UnsupportedStreaming));
        cleanup(started.response.cleanup).await; assert_eq!(capacity.available_permits(), 1);
    });
}

#[test]
fn composed_chat_rejects_unmappable_output_without_done_or_field_loss() {
    let mut cases = vec![response_body()];
    for (path, value) in [
        ("/output/0/id", json!("must-not-disappear")),
        ("/output/0/content/0/annotations", json!([{"type":"url_citation"}])),
        ("/output/0/content/0/logprobs", json!([])),
        ("/usage/unknown", json!(7)),
        ("/metadata", json!({"opaque":true})),
    ] {
        let mut body = minimal_body();
        let (parent, key) = path.rsplit_once('/').unwrap();
        body.pointer_mut(parent).unwrap()[key] = value;
        cases.push(body);
    }
    let mut tool = minimal_body();
    tool["output"] = json!([{"type":"function_call","call_id":"call","name":"run",
        "arguments":"{}","namespace":"must-not-disappear"}]); cases.push(tool);
    for body in cases {
        let server = Server::new(vec![expected("/ai/v3/responses/", expected_responses(), 0, Reply::json(body))]);
        run(async {
            let f = Fixture::new(&server, Supplier::default(), policy());
            let started = compose(&f, Route::Responses, responses_payload(),
                options(ClientDialect::OpenAiChat), deadline()).await;
            let head = started.response.result.await.unwrap();
            let (bytes, error) = composed_outcome(head.body).await;
            assert_eq!(error, Some(FailureKind::UnsupportedResponse));
            assert!(!bytes.windows(6).any(|window| window == b"[DONE]"));
            assert_eq!(summary(started.finished).await.result, Err(FailureKind::UnsupportedResponse));
            cleanup(started.response.cleanup).await; f.finish().await;
        });
        checked(&server.finish(), 1, 1);
    }
}

#[test]
fn composed_chat_usage_is_omitted_when_not_requested() {
    let server = Server::new(vec![expected("/ai/v3/responses/", expected_responses(), 0, Reply::json(minimal_body()))]);
    run(async {
        let f = Fixture::new(&server, Supplier::default(), policy());
        let mut requested = options(ClientDialect::OpenAiChat); requested.include_usage = false;
        let (bytes, done) = composed_finished(compose(&f, Route::Responses,
            responses_payload(), requested, deadline()).await).await;
        assert!(events(&bytes).iter().all(|event| event.get("usage").is_none()));
        assert!(bytes.ends_with(b"data: [DONE]\n\n")); assert_eq!(done.result, Ok(()));
        f.finish().await;
    });
    checked(&server.finish(), 1, 1);
}

#[test]
fn composed_chat_minimal_mapping_keeps_wire_id_tool_call_and_usage() {
    let mut body = minimal_body();
    body["model"] = json!("fixture-wire-model"); body["service_tier"] = json!("priority");
    body["system_fingerprint"] = json!("fp_exact");
    body["output"].as_array_mut().unwrap().push(json!({"type":"function_call",
        "call_id":"call_exact","name":"write","arguments":"{\"text\":\"λ🙂\"}"}));
    body["usage"]["output_tokens_details"] = json!({"reasoning_tokens":1});
    let server = Server::new(vec![expected("/ai/v3/responses/", expected_responses(), 0, Reply::json(body.clone()))]);
    run(async {
        let f = Fixture::new(&server, Supplier::default(), policy());
        let (bytes, done) = composed_finished(compose(&f, Route::Responses, responses_payload(),
            options(ClientDialect::OpenAiChat), deadline()).await).await;
        let emitted = events(&bytes);
        assert!(bytes.ends_with(b"data: [DONE]\n\n"));
        for event in &emitted {
            assert_eq!(event["id"], "resp_exact-wire-id");
            assert_eq!(event["model"], "fixture-wire-model");
            assert_eq!(event["service_tier"], "priority");
            assert_eq!(event["system_fingerprint"], "fp_exact");
        }
        let tool = emitted.iter().find(|v| v["choices"][0]["delta"]["tool_calls"].is_array()).unwrap();
        assert_eq!(tool["choices"][0]["delta"]["tool_calls"][0]["id"], "call_exact");
        assert_eq!(tool["choices"][0]["delta"]["tool_calls"][0]["function"]["arguments"], body["output"][1]["arguments"]);
        let usage = emitted.last().unwrap();
        assert_eq!(usage["choices"], json!([]));
        assert_eq!(usage["usage"]["completion_tokens_details"], json!({"reasoning_tokens":1}));
        assert_eq!(usage["usage"]["total_tokens"], 5);
        assert_eq!(done.result, Ok(())); f.finish().await;
    });
    checked(&server.finish(), 1, 1);
}

#[test]
fn composed_invalid_and_failed_json_never_succeed() {
    for (wire, kind) in [
        (b"not json".to_vec(), FailureKind::UnsupportedResponse),
        (serde_json::to_vec(&json!({"id":"failed","status":"failed","error":{"code":"fixture"}})).unwrap(), FailureKind::TerminalFailure),
        (serde_json::to_vec(&json!({"id":"invalid","status":"completed","output":false})).unwrap(), FailureKind::UnsupportedResponse),
    ] {
        let mut reply = Reply::json(Value::Null); reply.body = wire;
        let server = Server::new(vec![expected("/ai/v3/responses/", expected_responses(), 0, reply)]);
        run(async {
            let f = Fixture::new(&server, Supplier::default(), policy());
            let (bytes, done) = composed_finished(compose(&f, Route::Responses, responses_payload(),
                options(ClientDialect::OpenAiResponses), deadline()).await).await;
            let emitted = events(&bytes); no_success(&emitted);
            assert_eq!(emitted.last().unwrap()["type"], "error");
            assert_eq!(done.result, Err(kind)); f.finish().await;
        });
        checked(&server.finish(), 1, 1);
    }
}

#[test]
fn composed_incomplete_tool_preserves_attempt_without_retry() {
    let mut body = response_body();
    body["status"] = json!("incomplete");
    body["incomplete_details"] = json!({"reason":"max_output_tokens"});
    body["output"][1]["status"] = json!("incomplete");
    body["output"][1]["arguments"] = json!("{\"value\":");
    let server = Server::new(vec![expected("/ai/v3/responses/", expected_responses(), 0, Reply::json(body.clone()))]);
    run(async {
        let mut configured = policy(); configured.continuation = Some(CALLER_CONTINUE.into());
        let f = Fixture::new(&server, Supplier::default(), configured);
        let (bytes, done) = composed_finished(compose(&f, Route::Responses, responses_payload(),
            options(ClientDialect::OpenAiResponses), deadline()).await).await;
        let emitted = events(&bytes);
        assert_eq!(emitted.len(), 1);
        assert_eq!(emitted[0]["type"], "error");
        no_success(&emitted);
        assert_eq!(done.result, Err(FailureKind::UnsupportedResponse));
        assert_eq!(done.attempts.len(), 1);
        assert_eq!(done.attempts[0].headers.status, 200);
        assert_eq!(done.attempts[0].step_index, 0);
        assert_eq!(done.attempts[0].usage.as_deref(), Some(&body["usage"]));
        assert_eq!(f.supplier.calls.load(Ordering::Acquire), 1);
        f.finish().await;
    });
    checked(&server.finish(), 1, 1);
}

#[test]
fn composed_first_refusal_keeps_original_body_status_and_headers() {
    for status in [402, 403, 429, 500] {
        let wire = vec![0, 255, b'{', b'x', b'}'];
        let mut reply = Reply::json(Value::Null); reply.status = status; reply.body = wire.clone();
        reply.content_type = Some("application/x-original"); reply.usage = Some("original=exact");
        let server = Server::new(vec![expected("/ai/v3/responses/", expected_responses(), 0, reply)]);
        run(async {
            let f = Fixture::new(&server, Supplier::default(), policy());
            let started = compose(&f, Route::Responses, responses_payload(),
                options(ClientDialect::OpenAiResponses), deadline()).await;
            let error = started.response.result.await.err().unwrap();
            assert_eq!(error.kind, FailureKind::Upstream);
            let upstream = error.upstream.unwrap();
            assert_eq!(upstream.body, wire); assert_eq!(upstream.metadata.status, status);
            assert_eq!(upstream.metadata.original_content_type.as_deref(), Some("application/x-original"));
            assert_eq!(upstream.metadata.usage_header.as_deref(), Some("original=exact"));
            // R2 keeps the invocation charged while the refusal body is held.
            drop(upstream);
            let done = summary(started.finished).await;
            assert_eq!(done.result, Err(FailureKind::Upstream)); assert_eq!(done.emitted_bytes, 0);
            cleanup(started.response.cleanup).await; f.finish().await;
        });
        checked(&server.finish(), 1, 1);
    }
}

#[test]
fn composed_continuation_refusal_emits_error_without_success_terminal() {
    let (first, next) = responses_continuation();
    let mut refusal = Reply::json(json!({"error":"fixture refusal"})); refusal.status = 429;
    let server = Server::new(vec![
        expected("/ai/v3/responses/", expected_responses(), 0, Reply::json(first)),
        expected("/ai/v3/responses/", next, 1, refusal)]);
    run(async {
        let mut configured = policy(); configured.continuation = Some(CALLER_CONTINUE.into());
        let f = Fixture::new(&server, Supplier::default(), configured);
        let (bytes, done) = composed_finished(compose(&f, Route::Responses, responses_payload(),
            options(ClientDialect::OpenAiResponses), deadline()).await).await;
        let emitted = events(&bytes); no_success(&emitted);
        assert_eq!(emitted.last().unwrap()["type"], "error");
        assert_eq!(emitted.last().unwrap()["code"], "server_error");
        assert_eq!(done.result, Err(FailureKind::Upstream));
        assert_eq!(done.attempts[1].headers.status, 429); f.finish().await;
    });
    checked(&server.finish(), 2, 2);
}

#[test]
fn composed_messages_preserve_chunk_boundaries_and_current_text() {
    let (first, next) = messages_continuation();
    let mut last = message_body(); last["id"] = json!("message_last");
    last["content"][0]["text"] = json!(" plus last");
    last["content"][0]["fixture_detail"] = json!({"second":true});
    let pair = [expected("/ai/v3/messages/", expected_messages(), 0, Reply::json(first.clone())),
        expected("/ai/v3/messages/", next.clone(), 1, Reply::json(last.clone()))];
    let server = Server::new(pair.iter().cloned().chain(pair.iter().cloned()).collect());
    run(async {
        let mut configured = policy(); configured.continuation = Some(CALLER_CONTINUE.into());
        let old = Fixture::new(&server, Supplier::default(), configured.clone());
        let previous = completed(old.admit(Route::Messages, deadline()).await.unwrap()
            .messages(messages_payload())).await.unwrap().value;
        assert_eq!(previous.content.len(), 1); old.finish().await;
        let f = Fixture::new(&server, Supplier::default(), configured);
        let until = deadline();
        let (bytes, done) = composed_finished(compose(&f, Route::Messages, messages_payload(),
            options(ClientDialect::AnthropicMessages), until).await).await;
        let emitted = events(&bytes);
        let text: String = emitted.iter().filter_map(|v| v["delta"]["text"].as_str()).collect();
        assert_eq!(previous.content[0]["text"], text);
        let starts: Vec<_> = emitted.iter().filter(|v| v["type"] == "content_block_start").collect();
        assert_eq!(starts.len(), 2); assert_eq!(starts[1]["index"], 1);
        assert_eq!(starts[1]["content_block"]["fixture_detail"], json!({"second":true}));
        assert_eq!(done.attempts[1].response_id.as_deref(), Some("message_last"));
        assert_eq!(done.result, Ok(()));
        seen(&f.supplier, &f.scope, until, "OpenRouterMessages", &[expected_messages(), next]);
        f.finish().await;
    });
    checked(&server.finish(), 4, 4);
}

#[test]
fn composed_responses_preserve_boundaries_that_current_chain_merges() {
    let (first, next) = responses_continuation();
    let mut last = first.clone();
    last["id"] = json!("resp_last"); last["created_at"] = json!(999);
    last["status"] = json!("completed"); last["incomplete_details"] = Value::Null;
    last["output"] = json!([{"id":"msg_last","type":"message","role":"assistant",
        "status":"completed","content":[{"type":"output_text","text":" plus last","annotations":[]}]}]);
    let pair = [expected("/ai/v3/responses/", expected_responses(), 0, Reply::json(first.clone())),
        expected("/ai/v3/responses/", next.clone(), 1, Reply::json(last.clone()))];
    let server = Server::new(pair.iter().cloned().chain(pair.iter().cloned()).collect());
    run(async {
        let mut configured = policy(); configured.continuation = Some(CALLER_CONTINUE.into());
        let old = Fixture::new(&server, Supplier::default(), configured.clone());
        let previous = completed(old.admit(Route::Responses, deadline()).await.unwrap()
            .responses(responses_payload())).await.unwrap().value;
        assert_eq!(previous.output.len(), 2);
        assert_eq!(previous.output[1]["id"], "fixture-text-item");
        old.finish().await;
        let f = Fixture::new(&server, Supplier::default(), configured);
        let until = deadline();
        let (bytes, done) = composed_finished(compose(&f, Route::Responses, responses_payload(),
            options(ClientDialect::OpenAiResponses), until).await).await;
        let emitted = events(&bytes); let output = completed_output(&emitted);
        assert_eq!(output.as_array().unwrap().len(), 3);
        assert_eq!(output[0], first["output"][0]); assert_eq!(output[1], first["output"][1]);
        assert_eq!(output[2], last["output"][0]);
        assert_eq!(previous.output[1]["content"][0]["text"], "fixture first text plus last");
        assert_eq!(done.attempts[1].response_id.as_deref(), Some("resp_last"));
        assert_eq!(done.attempts[1].step_index, 1);
        assert_eq!(done.result, Ok(()));
        seen(&f.supplier, &f.scope, until, "OpenRouterResponses", &[expected_responses(), next]);
        f.finish().await;
    });
    checked(&server.finish(), 4, 4);
}

#[test]
fn composed_messages_keep_thinking_tools_and_json_escaping() {
    let mut body = message_body();
    body["content"].as_array_mut().unwrap().extend([
        json!({"type":"thinking","thinking":"fixture thought","signature":"exact-signature"}),
        json!({"type":"redacted_thinking","data":"exact-redacted"}),
        json!({"type":"tool_use","id":"tool_exact","name":"write",
            "input":{"text":"quote \" slash \\ newline\nλ🙂"},"fixture_field":[1,2]})
    ]);
    let server = Server::new(vec![expected("/ai/v3/messages/", expected_messages(), 0, Reply::json(body.clone()))]);
    run(async {
        let f = Fixture::new(&server, Supplier::default(), policy());
        let (bytes, done) = composed_finished(compose(&f, Route::Messages, messages_payload(),
            options(ClientDialect::AnthropicMessages), deadline()).await).await;
        let emitted = events(&bytes);
        let starts: Vec<_> = emitted.iter().filter(|v| v["type"] == "content_block_start").collect();
        assert_eq!(starts[1]["content_block"], body["content"][1]);
        assert_eq!(starts[2]["content_block"], body["content"][2]);
        assert_eq!(starts[3]["content_block"]["id"], "tool_exact");
        assert_eq!(starts[3]["content_block"]["fixture_field"], json!([1,2]));
        let tool = emitted.iter().find(|v| v["delta"]["type"] == "input_json_delta").unwrap();
        let input: Value = serde_json::from_str(tool["delta"]["partial_json"].as_str().unwrap()).unwrap();
        assert_eq!(input, body["content"][3]["input"]);
        assert_eq!(emitted.last().unwrap()["type"], "message_stop");
        assert_eq!(done.result, Ok(()));
        assert_eq!(done.attempts[0].usage.as_deref(), Some(&body["usage"]));
        f.finish().await;
    });
    checked(&server.finish(), 1, 1);
}

#[test]
fn composed_responses_preserve_rich_items_ids_usage_and_sequence() {
    let mut body = response_body();
    body["output"].as_array_mut().unwrap().extend([
        json!({"type":"custom_tool_call","id":"ctc_exact","call_id":"call_custom",
            "name":"write","input":"raw\\text\nλ🙂","namespace":"fixture_ns"}),
        json!({"type":"message","id":"msg_exact","role":"assistant","status":"completed",
            "content":[{"type":"output_text","text":"line 1\nλ🙂","annotations":[{"type":"fixture"}],
                "logprobs":[{"token":"λ","logprob":-0.5,"top_logprobs":[]}]}]})
    ]);
    let server = Server::new(vec![expected("/ai/v3/responses/", expected_responses(), 0, Reply::json(body.clone()))]);
    run(async {
        let f = Fixture::new(&server, Supplier::default(), policy());
        let until = deadline();
        let (bytes, done) = composed_finished(compose(&f, Route::Responses, responses_payload(),
            options(ClientDialect::OpenAiResponses), until).await).await;
        let emitted = events(&bytes);
        assert_eq!(completed_output(&emitted), &body["output"]);
        let items: Vec<_> = emitted.iter().filter(|v| v["type"] == "response.output_item.done")
            .map(|v| v["item"].clone()).collect();
        assert_eq!(items, *body["output"].as_array().unwrap());
        for (sequence, event) in emitted.iter().enumerate() {
            assert_eq!(event["sequence_number"], sequence as u64);
        }
        assert_eq!(done.result, Ok(())); assert_eq!(done.emitted_bytes, bytes.len());
        assert_eq!(done.attempts[0].response_id.as_deref(), Some("fixture-response"));
        assert_eq!(done.attempts[0].usage.as_deref(), Some(&body["usage"]));
        seen(&f.supplier, &f.scope, until, "OpenRouterResponses", &[expected_responses()]);
        f.finish().await;
    });
    checked(&server.finish(), 1, 1);
}

#[test]
fn composed_stream_waits_for_real_headers_and_first_body() {
    let mut reply = Reply::json(response_body()); reply.gate_body = true;
    let server = Server::with_held_headers(vec![expected("/ai/v3/responses/", expected_responses(), 0, reply)]);
    run(async {
        let f = Fixture::new(&server, Supplier::default(), policy());
        let mut started = compose(&f, Route::Responses, responses_payload(),
            options(ClientDialect::OpenAiResponses), deadline()).await;
        wait_count(&server.shared.requests, 1).await;
        assert!(tokio::time::timeout(PAUSE, &mut started.response.result).await.is_err());
        server.shared.release_headers.store(true, Ordering::Release);
        let mut head = started.response.result.await.unwrap();
        assert_eq!(head.first_attempt.status, 200);
        assert_eq!(head.content_type, "text/event-stream");
        assert_eq!(head.first_attempt.original_content_type.as_deref(), Some("application/json"));
        assert!(tokio::time::timeout(PAUSE, head.body.next()).await.is_err());
        server.shared.release_body.store(true, Ordering::Release);
        let bytes = composed_bytes(head.body).await;
        assert_eq!(events(&bytes)[0]["type"], "response.created");
        assert_eq!(started.finished.await.result, Ok(()));
        cleanup(started.response.cleanup).await;
        f.finish().await;
    });
    checked(&server.finish(), 1, 1);
}

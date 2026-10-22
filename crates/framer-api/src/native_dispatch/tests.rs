include!("fixture.rs");

#[test]
fn dispatch_raw_snapshot_survives_deadline_before_public_delivery() {
    let mut reply = Reply::json(json!({"fixture":"raw deadline"}));
    reply.status = 429;
    reply.content_type = Some("application/problem+json");
    let server = Server::new(vec![expected("/ai/v3/openai/responses/", responses_payload(), 0, reply)]);
    run(async {
        let f = Fixture::new(&server, Supplier::default(), policy());
        let until = deadline();
        let call = f.admit(Route::Raw(ClientDialect::OpenAiResponses), until).await.unwrap();
        let started = call.raw(responses_payload());
        let progress = started.completed_attempts.clone().unwrap();
        let cleanup = started.cleanup.clone();
        let cancellation = started.cancellation.clone();
        let (complete, observed) = oneshot::channel();
        // Delay only delivery. Collection, authorization and snapshot are real.
        let delivery = async move {
            let value = started.result.await?;
            let _ = complete.send(value.metadata.clone());
            futures::future::pending::<()>().await;
            Ok::<_, DispatchError>(value)
        };
        let mut bounded = Box::pin(tokio::time::timeout_at(until, delivery));
        let original = tokio::select! {
            value = observed => value.expect("raw completion before delivery deadline"),
            _ = &mut bounded => panic!("deadline arrived before real raw completion"),
        };
        assert!(bounded.await.is_err());
        cancellation.cancel();
        assert_eq!(cleanup.await, Ok(()));
        let snapshot = progress();
        assert_eq!(snapshot.len(), 1);
        assert_eq!(snapshot[0].status, 429);
        assert_eq!(snapshot[0].usage_header.as_deref(), Some("remaining=7.5&fixture=application"));
        assert_eq!(snapshot[0].original_content_type.as_deref(), Some("application/problem+json"));
        assert_eq!(snapshot[0].step_index, 0);
        assert_eq!(snapshot[0].elapsed, original.elapsed);
        assert_eq!(snapshot[0].status, original.status);
        assert_eq!(snapshot[0].usage_header, original.usage_header);
        assert_eq!(snapshot[0].original_content_type, original.original_content_type);
        assert_eq!(f.supplier.calls.load(Ordering::Acquire), 1);
        f.finish().await;
    });
    checked(&server.finish(), 1, 1);
}

#[test]
fn dispatch_raw_completion_populates_shared_attempt_snapshot_once() {
    let mut reply = Reply::json(json!({"fixture":"raw snapshot"}));
    reply.status = 207;
    reply.content_type = Some("application/x-fixture-raw");
    reply.gate_body = true;
    let bytes = reply.body.clone();
    let server = Server::new(vec![expected("/ai/v3/openai/responses/", responses_payload(), 0, reply)]);
    run(async {
        let f = Fixture::new(&server, Supplier::default(), policy());
        let call = f.admit(Route::Raw(ClientDialect::OpenAiResponses), deadline()).await.unwrap();
        let started = call.raw(responses_payload());
        let progress = started.completed_attempts.clone().unwrap();
        wait_count(&server.shared.headers, 1).await;
        assert!(progress().is_empty());
        server.shared.release_body.store(true, Ordering::Release);
        let value = completed(started).await.unwrap();
        assert_eq!(value.body, bytes);
        let snapshot = progress();
        assert_eq!(snapshot.len(), 1);
        assert_eq!(snapshot[0].status, 207);
        assert_eq!(snapshot[0].usage_header.as_deref(), Some("remaining=7.5&fixture=application"));
        assert_eq!(snapshot[0].original_content_type.as_deref(), Some("application/x-fixture-raw"));
        assert_eq!(snapshot[0].step_index, 0);
        assert_eq!(snapshot[0].elapsed, value.metadata.elapsed);
        assert_eq!(value.metadata.status, snapshot[0].status);
        assert_eq!(value.metadata.usage_header, snapshot[0].usage_header);
        assert_eq!(value.metadata.original_content_type, snapshot[0].original_content_type);
        assert_eq!(f.supplier.calls.load(Ordering::Acquire), 1);
        f.finish().await;
        assert_eq!(progress().len(), 1);
    });
    checked(&server.finish(), 1, 1);
}

#[test]
fn dispatch_failed_cleanup_quarantines_capacity_until_runtime_shutdown() {
    let server = Server::new(vec![]);
    let (capacity, supervisor) = run(async {
        let f = Fixture::new(&server, Supplier::default(), policy());
        let client = f.client.clone(); let scope = f.scope.clone();
        let permit = f.capacity.clone().try_acquire_owned().unwrap();
        let until = deadline();
        let started = super::lifetime::launch(f.client.clone(), f.scope.clone(), permit, until, 1,
            move |registry| async move {
                let context = RequestContext {
                    dialect: Dialect::OpenAiResponses, project: project(), payload: responses_payload(),
                    feature_category: "fixture-application-category".into(),
                    feature_name: "fixture-application-feature".into(), step: "fixture-application-step".into(),
                    chat_session: CHAT.into(), step_index: 0, turn_id: "fixture-application-turn-0".into(),
                    user_prompt: "fixture outer annotation\nkept exactly".into(),
                };
                let pending = client.start_for_test(&scope, context, auth(&scope), until,
                    MISSING_TYPE.into(), |pool, inputs, deadline, content_type| {
                        let owner = pool.start(inputs, Mode::Raw, deadline)
                            .map_err(bridge::StartError::Admission)?;
                        // This current-thread runtime cannot poll the driver
                        // before this synchronous abort. Reap its real failure.
                        assert!(owner.abort_task_for_test());
                        bridge::from_owner(owner, content_type).map_err(bridge::StartError::Attach)
                    }).unwrap();
                let actual: bridge::Cleanup = pending.cleanup.clone();
                registry.started(actual.clone(), pending.cancellation.clone());
                pending.cancellation.cancel();
                drop(pending);
                assert_eq!(actual.await,
                    Err(crate::experimental_native::native_owner::CompletionError::TaskLost));
                Ok(())
            });
        let capacity = f.capacity.clone();
        let supervisor = f.client.supervisor_for_test();
        drop(f);
        let error = tokio::time::timeout(STEP, started.result).await.unwrap().err().unwrap();
        assert_eq!(error.kind, FailureKind::Cleanup);
        assert_eq!(started.cleanup.await, Err(FailureKind::Cleanup));
        assert_eq!(capacity.available_permits(), 0);
        assert!(!supervisor.is_finished());
        (capacity, supervisor)
    });
    assert_eq!(capacity.available_permits(), 1);
    assert!(supervisor.is_finished());
    checked(&server.finish(), 0, 0);
}

#[test]
fn dispatch_later_authorization_timeout_retains_prior_attempt_metadata() {
    let (first, _) = responses_continuation();
    let server = Server::new(vec![expected("/ai/v3/responses/", expected_responses(), 0, Reply::json(first))]);
    run(async {
        let mut policy = policy(); policy.continuation = Some(CALLER_CONTINUE.into());
        let f = Fixture::new(&server, Supplier { never: true, gate_index: 1, ..Supplier::default() }, policy);
        let until = Instant::now() + Duration::from_millis(500);
        let call = f.admit(Route::Responses, until).await.unwrap();
        let run = call.responses(responses_payload());
        let progress = run.completed_attempts.clone().unwrap();
        let error = completed(run).await.err().unwrap();
        assert_eq!(error.kind, FailureKind::Deadline);
        assert_eq!(error.attempts.len(), 1);
        metadata(&error.attempts[0], 0);
        assert_eq!(progress().len(), 1);
        assert_eq!(f.supplier.calls.load(Ordering::Acquire), 2);
        f.finish().await;
    });
    checked(&server.finish(), 1, 1);
}

#[test]
fn dispatch_deadline_returns_while_guardian_retains_client_and_capacity() {
    let server = Server::new(vec![expected("/ai/v3/openai/responses/", responses_payload(), 0,
        Reply::json(json!({"fixture":true})))]);
    run(async {
        let f = Fixture::new(&server, Supplier::default(), policy());
        let (release, tail) = oneshot::channel();
        let until = Instant::now() + Duration::from_millis(500);
        let client = f.client.clone(); let scope = f.scope.clone();
        let permit = f.capacity.clone().try_acquire_owned().unwrap();
        let started = super::lifetime::launch(f.client.clone(), f.scope.clone(), permit, until, 1,
            move |registry| async move {
                let pending = hooked(&client, &scope, until, tail);
                registry.started(pending.cleanup.clone(), pending.cancellation.clone());
                let response = pending.response.await.map_err(|_| DispatchError::new(FailureKind::Transport))?;
                let mut body = response.proxy.stream;
                while let Some(chunk) = body.next().await {
                    chunk.map_err(|_| DispatchError::new(FailureKind::Body))?;
                }
                Ok(())
            });
        let capacity = f.capacity.clone();
        let supervisor = f.client.supervisor_for_test();
        drop(f);
        wait_count(&server.shared.headers, 1).await;
        assert_eq!(capacity.available_permits(), 0);
        let error = tokio::time::timeout(STEP, started.result).await.unwrap().err().unwrap();
        assert_eq!(error.kind, FailureKind::Deadline);
        assert!(started.cleanup.clone().now_or_never().is_none());
        assert_eq!(capacity.available_permits(), 0);
        assert!(!supervisor.is_finished());
        release.send(()).unwrap();
        assert_eq!(tokio::time::timeout(STEP, started.cleanup).await.unwrap(), Ok(()));
        assert_eq!(capacity.available_permits(), 1);
        tokio::time::timeout(STEP, async {
            while !supervisor.is_finished() { tokio::time::sleep(TICK).await; }
        }).await.unwrap();
    });
    checked(&server.finish(), 1, 1);
}

#[test]
fn dispatch_later_failures_keep_prior_attempts_and_original_terminal_object() {
    for status in [200, 403] {
        let (first, next) = responses_continuation();
        let body = json!({"status":"failed","error":{"code":"fixture","detail":[1,2]},"opaque":true});
        let mut refused = Reply::json(body); refused.status = status;
        let bytes = refused.body.clone();
        let server = Server::new(vec![
            expected("/ai/v3/responses/", expected_responses(), 0, Reply::json(first)),
            expected("/ai/v3/responses/", next, 1, refused),
        ]);
        run(async {
            let mut policy = policy(); policy.continuation = Some(CALLER_CONTINUE.into());
            let f = Fixture::new(&server, Supplier::default(), policy);
            let call = f.admit(Route::Responses, deadline()).await.unwrap();
            let error = completed(call.responses(responses_payload())).await.err().unwrap();
            assert_eq!(error.kind, if status == 200 { FailureKind::TerminalFailure } else { FailureKind::Upstream });
            assert_eq!(error.attempts.len(), 1);
            metadata(&error.attempts[0], 0);
            let upstream = error.upstream.unwrap();
            assert_eq!(upstream.body, bytes);
            assert_eq!(upstream.metadata.status, status);
            assert_eq!(upstream.metadata.step_index, 1);
            assert_eq!(f.supplier.calls.load(Ordering::Acquire), 2);
            f.finish().await;
        });
        checked(&server.finish(), 2, 2);
    }
}

#[test]
fn dispatch_body_limits_are_explicit_and_cleanup_remains_observable() {
    for kind in 0..3 {
        let (route, path, input, prepared, body) = match kind {
            0 => (Route::Raw(ClientDialect::OpenAiResponses), "/ai/v3/openai/responses/",
                responses_payload(), responses_payload(), response_body()),
            1 => (Route::Messages, "/ai/v3/messages/", messages_payload(), expected_messages(), message_body()),
            _ => (Route::Responses, "/ai/v3/responses/", responses_payload(), expected_responses(), response_body()),
        };
        let server = Server::new(vec![expected(path, prepared, 0, Reply::json(body))]);
        run(async {
            let mut limits = policy(); limits.max_body_bytes = 4;
            let f = Fixture::new(&server, Supplier::default(), limits);
            let call = f.admit(route, deadline()).await.unwrap();
            let error = match kind {
                0 => completed(call.raw(input)).await.err().unwrap(),
                1 => completed(call.messages(input)).await.err().unwrap(),
                _ => completed(call.responses(input)).await.err().unwrap(),
            };
            assert_eq!(error.kind, FailureKind::BodyTooLarge);
            assert_eq!(error.attempts.len(), 1);
            assert_eq!(error.attempts[0].status, 200);
            assert_eq!(f.supplier.calls.load(Ordering::Acquire), 1);
            f.finish().await;
        });
        let snapshot = server.finish();
        assert_eq!(snapshot.fault, None);
        assert_eq!(snapshot.requests, 1);
    }
}

#[test]
fn dispatch_cancel_or_drop_after_headers_keeps_capacity_through_cleanup() {
    for drop_result in [false, true] {
        let mut reply = Reply::json(json!({"fixture":true}));
        reply.gate_body = true;
        let server = Server::new(vec![expected("/ai/v3/openai/responses/", responses_payload(), 0, reply)]);
        run(async {
            let f = Fixture::new(&server, Supplier::default(), policy());
            let call = f.admit(Route::Raw(ClientDialect::OpenAiResponses), deadline()).await.unwrap();
            let started = call.raw(responses_payload());
            wait_count(&server.shared.headers, 1).await;
            assert_eq!(f.capacity.available_permits(), 0);
            assert!(started.cleanup.clone().now_or_never().is_none());
            if drop_result { drop(started.result); }
            else {
                started.cancellation.cancel();
                let error = tokio::time::timeout(STEP, started.result).await.unwrap().err().unwrap();
                assert_eq!(error.kind, FailureKind::Cancelled);
            }
            assert_eq!(tokio::time::timeout(STEP, started.cleanup).await.unwrap(), Ok(()));
            wait_count(&server.shared.peer_eof, 1).await;
            f.finish().await;
        });
        checked(&server.finish(), 1, 0);
    }
}

#[test]
fn dispatch_dropping_unpolled_result_cancels_before_authorization() {
    let server = Server::new(vec![]);
    run(async {
        let f = Fixture::new(&server, Supplier::default(), policy());
        let call = f.admit(Route::Responses, deadline()).await.unwrap();
        let run = call.responses(responses_payload());
        assert_eq!(f.capacity.available_permits(), 0);
        drop(run.result);
        assert_eq!(tokio::time::timeout(STEP, run.cleanup).await.unwrap(), Ok(()));
        assert_eq!(f.supplier.calls.load(Ordering::Acquire), 0);
        f.finish().await;
    });
    checked(&server.finish(), 0, 0);
}

#[test]
fn dispatch_original_deadline_cancels_pending_authorization() {
    let server = Server::new(vec![]);
    run(async {
        let f = Fixture::new(&server, Supplier { never: true, ..Supplier::default() }, policy());
        let until = Instant::now() + Duration::from_millis(150);
        let call = f.admit(Route::Responses, until).await.unwrap();
        assert_eq!(completed(call.responses(responses_payload())).await.err().unwrap().kind, FailureKind::Deadline);
        assert_eq!(f.supplier.calls.load(Ordering::Acquire), 1);
        assert_eq!(f.supplier.cancelled.load(Ordering::Acquire), 1);
        f.finish().await;
    });
    checked(&server.finish(), 0, 0);
}

#[test]
fn dispatch_rejects_streaming_lossy_history_and_stateful_options_before_auth() {
    for case in 0..5 {
        let server = Server::new(vec![]);
        run(async {
            let f = Fixture::new(&server, Supplier::default(), policy());
            let call = f.admit(Route::Responses, deadline()).await.unwrap();
            let mut payload = responses_payload();
            match case {
                0 => payload["stream"] = json!(true),
                1 => { payload["input"][1].as_object_mut().unwrap().remove("namespace"); }
                2 => payload["store"] = json!(false),
                3 => payload["previous_response_id"] = json!("fixture-prior"),
                _ => payload["background"] = json!(true),
            }
            let error = completed(call.responses(payload)).await.err().unwrap();
            assert_eq!(error.kind, if case == 0 { FailureKind::UnsupportedStreaming } else { FailureKind::InvalidRequest });
            assert_eq!(f.supplier.calls.load(Ordering::Acquire), 0);
            f.finish().await;
        });
        checked(&server.finish(), 0, 0);
    }
}

#[test]
fn dispatch_closed_scope_never_authorizes_or_reopens() {
    let server = Server::new(vec![]);
    run(async {
        let f = Fixture::new(&server, Supplier::default(), policy());
        close(&f.client, &f.scope).await;
        let error = f.admit(Route::Responses, deadline()).await.err().unwrap();
        assert_eq!(error.kind, FailureKind::ScopeClosed);
        assert_eq!(f.supplier.calls.load(Ordering::Acquire), 0);
        f.finish().await;
    });
    checked(&server.finish(), 0, 0);
}

#[test]
fn dispatch_refusals_keep_status_body_and_single_attempt() {
    for status in [403, 429] {
        for messages in [true, false] {
            let (route, path, input, prepared) = if messages {
                (Route::Messages, "/ai/v3/messages/", messages_payload(), expected_messages())
            } else { (Route::Responses, "/ai/v3/responses/", responses_payload(), expected_responses()) };
            let bytes = b"fixture rejected\nkeep exact bytes".to_vec();
            let reply = Reply { status, body: bytes.clone(), content_type: Some("text/plain"),
                usage: Some("fixture-rejection-usage"), gate_body: false };
            let server = Server::new(vec![expected(path, prepared, 0, reply)]);
            run(async {
                let f = Fixture::new(&server, Supplier::default(), policy());
                let call = f.admit(route, deadline()).await.unwrap();
                let error = if messages {
                    completed(call.messages(input)).await.err().unwrap()
                } else { completed(call.responses(input)).await.err().unwrap() };
                assert_eq!(error.kind, FailureKind::Upstream);
                let original = error.upstream.unwrap();
                assert_eq!(original.body, bytes);
                assert_eq!(original.metadata.status, status);
                assert_eq!(original.metadata.usage_header.as_deref(), Some("fixture-rejection-usage"));
                assert_eq!(original.metadata.original_content_type.as_deref(), Some("text/plain"));
                assert_eq!(f.supplier.calls.load(Ordering::Acquire), 1);
                f.finish().await;
            });
            checked(&server.finish(), 1, 1);
        }
    }
}

#[test]
fn dispatch_paid_requests_require_current_scoped_credit_evidence() {
    for case in 0..5 {
        let server = Server::new(vec![]);
        run(async {
            let mut f = Fixture::new(&server, Supplier::default(), policy());
            let binding = f.provider.bindings.get_mut(&f.scope.account()).unwrap();
            match case {
                0 => f.provider.policy.allow_paid = false,
                1 => binding.paid_credit = None,
                2 => binding.paid_credit.as_mut().unwrap().available = 24.0,
                3 => binding.paid_credit.as_mut().unwrap().available = f64::NAN,
                _ => binding.paid_credit.as_mut().unwrap().valid_until = Instant::now(),
            }
            let error = f.admit(Route::Raw(ClientDialect::OpenAiResponses), deadline()).await.err().unwrap();
            assert_eq!(error.kind, if case == 0 { FailureKind::PaidDisabled } else { FailureKind::CreditFloor });
            assert_eq!(f.supplier.calls.load(Ordering::Acquire), 0);
            f.finish().await;
        });
        checked(&server.finish(), 0, 0);
    }
}

#[test]
fn dispatch_missing_supplied_context_fails_before_transport() {
    let server = Server::new(vec![]);
    run(async {
        let f = Fixture::new(&server, Supplier { missing_context: true, ..Supplier::default() }, policy());
        let call = f.admit(Route::Responses, deadline()).await.unwrap();
        assert_eq!(completed(call.responses(responses_payload())).await.err().unwrap().kind, FailureKind::MissingContext);
        assert_eq!(f.supplier.calls.load(Ordering::Acquire), 1);
        f.finish().await;
    });
    checked(&server.finish(), 0, 0);
}

#[test]
fn dispatch_prepare_rejects_missing_or_mismatched_identity_before_auth() {
    for case in 0..7 {
        let server = Server::new(vec![]);
        run(async {
            let mut f = Fixture::new(&server, Supplier::default(), policy());
            let mut request = f.request(Route::Responses, deadline());
            let expected = match case {
                0 => { request.identity.account = AccountId::new(); FailureKind::InvalidScope }
                1 => { request.identity.user = FramerUserId("other".into()); FailureKind::InvalidScope }
                2 => { request.identity.project = ProjectId("other".into()); FailureKind::InvalidScope }
                3 => { request.identity.team = TeamId("other".into()); FailureKind::InvalidScope }
                4 => { request.identity.chat_session.clear(); FailureKind::MissingContext }
                5 => { request.deadline = Instant::now(); FailureKind::Deadline }
                _ => {
                    f.provider.bindings.get_mut(&f.scope.account()).unwrap().authorization = None;
                    FailureKind::MissingAuthorization
                }
            };
            let permit = f.capacity.clone().try_acquire_owned().unwrap();
            let error = f.provider.prepare(request, permit).await.err().unwrap();
            assert_eq!(error.kind, expected);
            assert_eq!(f.supplier.calls.load(Ordering::Acquire), 0);
            f.finish().await;
        });
        checked(&server.finish(), 0, 0);
    }
}

#[test]
fn dispatch_continuation_is_only_the_explicit_caller_policy() {
    for messages in [true, false] {
        for allow in [false, true] {
            let (first, next) = if messages { messages_continuation() } else { responses_continuation() };
            let (route, path, dialect, input, prepared, last) = if messages {
                (Route::Messages, "/ai/v3/messages/", "OpenRouterMessages", messages_payload(), expected_messages(), message_body())
            } else {
                (Route::Responses, "/ai/v3/responses/", "OpenRouterResponses", responses_payload(), expected_responses(), response_body())
            };
            let mut expected_payloads = vec![prepared.clone()];
            let mut replies = vec![expected(path, prepared, 0, Reply::json(first))];
            if allow {
                expected_payloads.push(next.clone());
                replies.push(expected(path, next, 1, Reply::json(last)));
            }
            let server = Server::new(replies);
            run(async {
                let mut policy = policy();
                policy.continuation = allow.then(|| CALLER_CONTINUE.into());
                let f = Fixture::new(&server, Supplier::default(), policy);
                let until = deadline();
                let call = f.admit(route, until).await.unwrap();
                let (chunks, stop) = if messages {
                    let value = completed(call.messages(input)).await.unwrap().value;
                    (value.chunks, value.stop_reason)
                } else {
                    let value = completed(call.responses(input)).await.unwrap().value;
                    (value.chunks, value.status)
                };
                assert_eq!(chunks, if allow { 2 } else { 1 });
                assert_eq!(stop, match (messages, allow) {
                    (true, true) => "end_turn", (true, false) => "max_tokens",
                    (false, true) => "completed", (false, false) => "incomplete",
                });
                seen(&f.supplier, &f.scope, until, dialect, &expected_payloads);
                f.finish().await;
            });
            checked(&server.finish(), if allow { 2 } else { 1 }, if allow { 2 } else { 1 });
        }
    }
}

#[test]
fn dispatch_chains_authorize_exact_prepared_history_and_keep_metadata() {
    for messages in [true, false] {
        let (route, path, dialect, input, prepared, body) = if messages {
            (Route::Messages, "/ai/v3/messages/", "OpenRouterMessages",
                messages_payload(), expected_messages(), message_body())
        } else {
            (Route::Responses, "/ai/v3/responses/", "OpenRouterResponses",
                responses_payload(), expected_responses(), response_body())
        };
        let server = Server::new(vec![expected(path, prepared.clone(), 0, Reply::json(body.clone()))]);
        run(async {
            let f = Fixture::new(&server, Supplier::default(), policy());
            let until = deadline();
            let call = f.admit(route, until).await.unwrap();
            if messages {
                let result = completed(call.messages(input)).await.unwrap();
                assert_eq!(result.value.content, body["content"].as_array().unwrap().clone());
                metadata(&result.attempts[0], 0);
            } else {
                let result = completed(call.responses(input)).await.unwrap();
                assert_eq!(result.value.output, body["output"].as_array().unwrap().clone());
                metadata(&result.attempts[0], 0);
            }
            seen(&f.supplier, &f.scope, until, dialect, &[prepared]);
            f.finish().await;
        });
        checked(&server.finish(), 1, 1);
    }
}

#[test]
fn dispatch_raw_aliases_keep_original_payload_status_and_headers() {
    for (dialect, path, name) in [
        (ClientDialect::AnthropicMessages, "/ai/v3/anthropic/messages/", "AnthropicMessages"),
        (ClientDialect::OpenAiChat, "/ai/v3/chat/", "OpenRouterChat"),
        (ClientDialect::OpenAiResponses, "/ai/v3/openai/responses/", "OpenAiResponses"),
    ] {
        let payload = responses_payload();
        let mut reply = Reply::json(json!({"fixture":"exact bytes"}));
        reply.status = 207; reply.content_type = None;
        let bytes = reply.body.clone();
        let server = Server::new(vec![expected(path, payload.clone(), 0, reply)]);
        run(async {
            let f = Fixture::new(&server, Supplier::default(), policy());
            let until = deadline();
            let call = f.admit(Route::Raw(dialect), until).await.unwrap();
            let value = completed(call.raw(payload.clone())).await.unwrap();
            assert_eq!(value.body, bytes);
            assert_eq!(value.metadata.status, 207);
            assert_eq!(value.metadata.original_content_type, None);
            assert_eq!(value.metadata.usage_header.as_deref(), Some("remaining=7.5&fixture=application"));
            seen(&f.supplier, &f.scope, until, name, &[payload.clone()]);
            f.finish().await;
        });
        checked(&server.finish(), 1, 1);
    }
}

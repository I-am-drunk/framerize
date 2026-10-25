// New, unrun cases. Shared helpers are copied unchanged from the frozen union.
include!("fixture.rs");

#[path = "exact_context_integration_tests.rs"]
mod exact_context;

#[path = "streaming_tests.rs"]
mod streaming;

#[path = "continuation_tests.rs"]
mod continuation;

struct NoopCancellation;
impl port::Cancellation for NoopCancellation { fn cancel(&self) {} }

struct PendingResult(Arc<AtomicUsize>);
impl Future for PendingResult {
    type Output = Result<super::Facts, DispatchError>;
    fn poll(self: std::pin::Pin<&mut Self>, _: &mut std::task::Context<'_>)
        -> std::task::Poll<Self::Output> { std::task::Poll::Pending }
}
impl Drop for PendingResult {
    fn drop(&mut self) { self.0.fetch_add(1, Ordering::Release); }
}

#[test]
fn json_probe_unpolled_abandonment_drops_original_result_owner() {
    let server = Server::new(vec![]);
    run(async {
        let f = Fixture::new(&server, Supplier::default());
        let plan = super::Plan::new(f.scope.clone(), specification(&f,
            super::FreeRoute::Responses, responses_payload())).unwrap();
        let drops = Arc::new(AtomicUsize::new(0));
        let original = InvocationRun { result: Box::pin(PendingResult(drops.clone())),
            cleanup: futures::future::ready(Ok(())).boxed().shared(),
            cancellation: Arc::new(NoopCancellation) };
        let probe = super::finish_run(original, plan.contract.clone(), |v| v);
        drop(probe.result);
        assert_eq!(drops.load(Ordering::Acquire), 1);
        assert!(probe.completed.await.is_none());
        assert_eq!(probe.cleanup.await, Ok(()));
        drop(plan);
        f.finish().await;
    });
    checked(&server.finish(), 0, 0);
}

fn successful_facts() -> super::Facts {
    super::Facts { complete: true,
        attempts: vec![AttemptMetadata { status: 200, usage_header: None,
            original_content_type: Some("application/json".into()), step_index: 0,
            elapsed: Duration::from_millis(1) }],
        input_tokens: Some(2), output_tokens: Some(1), output_items: 1, failure: None,
        refusal: None, completed: Some(super::CompletedJson::Messages(framer_core::convert::AssembledResponse {
            id: "fixture-completed-message".into(),
            content: vec![json!({"type":"text", "text":"fixture-completed-text"})],
            stop_reason: "end_turn".into(), stop_sequence: None,
            usage: json!({"input_tokens":2, "output_tokens":1}),
            input_tokens: 2, output_tokens: 1, chunks: 1, metered_chunks: 0,
        })) }
}

#[test]
fn json_probe_acceptance_requires_original_json_media_type() {
    let server = Server::new(vec![]);
    run(async {
        let f = Fixture::new(&server, Supplier::default());
        for (header, accepted) in [
            (Some("application/json"), true),
            (Some("Application/JSON; charset=utf-8"), true),
            (Some("application/vendor+json"), true),
            (None, false), (Some("text/html"), false),
            (Some("text/event-stream; charset=utf-8"), false),
        ] {
            let plan = super::Plan::new(f.scope.clone(), specification(&f,
                super::FreeRoute::Responses, responses_payload())).unwrap();
            plan.contract.calls.store(1, Ordering::Release);
            plan.contract.starts.store(1, Ordering::Release);
            plan.contract.finishes.store(1, Ordering::Release);
            let mut facts = successful_facts();
            facts.attempts[0].original_content_type = header.map(str::to_owned);
            let original = InvocationRun {
                result: futures::future::ready(Ok(facts)).boxed(),
                cleanup: futures::future::ready(Ok(())).boxed().shared(),
                cancellation: Arc::new(NoopCancellation),
            };
            let report = probe_result(super::finish_run(original, plan.contract.clone(), |v| v)).await;
            assert_eq!(report.accepted, accepted);
        }
        f.finish().await;
    });
    checked(&server.finish(), 0, 0);
}

#[test]
fn json_probe_waits_for_original_cleanup_and_refuses_failed_cleanup() {
    let server = Server::new(vec![]);
    run(async {
        let f = Fixture::new(&server, Supplier::default());
        for failed in [false, true] {
            let plan = super::Plan::new(f.scope.clone(), specification(&f,
                super::FreeRoute::Responses, responses_payload())).unwrap();
            plan.contract.calls.store(1, Ordering::Release);
            plan.contract.starts.store(1, Ordering::Release);
            plan.contract.finishes.store(1, Ordering::Release);
            let (send, receive) = oneshot::channel();
            let cleanup = async move { receive.await.unwrap() }.boxed().shared();
            let original = InvocationRun {
                result: futures::future::ready(Ok(successful_facts())).boxed(),
                cleanup: cleanup.clone(), cancellation: Arc::new(NoopCancellation),
            };
            let mut probe = super::finish_run(original, plan.contract.clone(), |v| v);
            assert!((&mut probe.result).now_or_never().is_none());
            assert!((&mut probe.completed).now_or_never().is_none());
            send.send(if failed { Err(FailureKind::Cleanup) } else { Ok(()) }).unwrap();
            let report = probe.result.await;
            assert_eq!(report.accepted, !failed);
            assert_eq!(report.cleanup_confirmed, !failed);
            assert_eq!(probe.completed.await.is_some(), !failed);
            assert_eq!(probe.cleanup.await.is_err(), failed);
        }
        f.finish().await;
    });
    checked(&server.finish(), 0, 0);
}

fn specification(f: &Fixture, route: super::FreeRoute, payload: Value) -> super::Specification {
    super::Specification {
        identity: f.identity(),
        route, payload, context: AttemptContext {
            feature_category: "fixture-application-category".into(),
            feature_name: "fixture-application-feature".into(),
            step: "fixture-application-step".into(),
            turn_id: "fixture-application-turn-0".into(),
            user_prompt: "fixture outer annotation\nkept exactly".into(),
        }, deadline: deadline(), token_limit: 32,
    }
}

async fn start_probe(f: &Fixture, route: super::FreeRoute, payload: Value)
    -> Result<super::ProbeRun, DispatchError> {
    start_specification(f, specification(f, route, payload)).await
}

async fn start_specification(f: &Fixture, spec: super::Specification)
    -> Result<super::ProbeRun, DispatchError> {
    let plan = super::Plan::new(f.scope.clone(), spec)?;
    let permit = f.capacity.clone().try_acquire_owned().unwrap();
    plan.launch_in_pool(f.client.clone(), f.supplier.clone(), permit, f.admission()).await
}

async fn probe_result(probe: super::ProbeRun) -> super::Evidence {
    let report = tokio::time::timeout(STEP, probe.result).await.expect("probe result");
    assert_eq!(tokio::time::timeout(STEP, probe.cleanup).await.unwrap(), Ok(()));
    report
}

fn prepared<'a>(c: &'a super::Contract) -> PreparedAttempt<'a> {
    PreparedAttempt { scope: &c.scope, project: &c.identity.project,
        dialect: Dialect::OpenRouterResponses, payload: &c.expected,
        chat_session: &c.identity.chat_session, step_index: 0, deadline: c.deadline }
}

struct ObservedSupplier { inner: Arc<Supplier>, observed: Arc<AtomicUsize> }
impl AttemptAuthorization for ObservedSupplier {
    fn authorize<'a>(&'a self, _: PreparedAttempt<'a>)
        -> BoxFuture<'a, Result<AuthorizedAttempt, String>> {
        futures::future::ready(Err("unexpected unobserved hook".into())).boxed()
    }
    fn authorize_observed<'a>(&'a self, attempt: PreparedAttempt<'a>,
        _: &'a dyn super::PreparationObserver)
        -> BoxFuture<'a, Result<AuthorizedAttempt, String>> {
        self.observed.fetch_add(1, Ordering::Release);
        self.inner.authorize(attempt)
    }
}

#[test]
fn json_probe_forwards_observed_hook_through_actual_dispatch() {
    let server = Server::new(vec![expected("/ai/v3/responses/", expected_responses(), 0,
        Reply::json(response_body()))]);
    run(async {
        let f = Fixture::new(&server, Supplier::default());
        let observed = Arc::new(AtomicUsize::new(0));
        let supplier = Arc::new(ObservedSupplier { inner: f.supplier.clone(), observed: observed.clone() });
        let plan = super::Plan::new(f.scope.clone(), specification(&f,
            super::FreeRoute::Responses, responses_payload())).unwrap();
        let permit = f.capacity.clone().try_acquire_owned().unwrap();
        let started = plan.launch_in_pool(f.client.clone(), supplier, permit, f.admission()).await.unwrap();
        assert!(probe_result(started).await.accepted);
        assert_eq!(observed.load(Ordering::Acquire), 1);
        f.finish().await;
    });
    checked(&server.finish(), 1, 1);
}

#[test]
fn json_probe_rejects_changed_contract_before_calling_supplier() {
    let server = Server::new(vec![]);
    run(async {
        let f = Fixture::new(&server, Supplier::default());
        for case in 0..6 {
            let plan = super::Plan::new(f.scope.clone(), specification(&f,
                super::FreeRoute::Responses, responses_payload())).unwrap();
            let gate = super::CheckedAuthorization { inner: f.supplier.clone(), contract: plan.contract.clone() };
            let mut changed = plan.contract.expected.clone();
            changed["reasoning"]["effort"] = json!("different");
            let other_project = ProjectId("different-project".into());
            let mut attempt = prepared(&plan.contract);
            match case {
                0 => attempt.dialect = Dialect::OpenAiResponses,
                1 => attempt.chat_session = "different-conversation",
                2 => attempt.project = &other_project,
                3 => attempt.payload = &changed,
                4 => attempt.step_index = 1,
                _ => attempt.deadline += Duration::from_millis(1),
            }
            let observer = |_, _| {};
            assert!(gate.authorize_observed(attempt, &observer).await.is_err());
        }
        assert_eq!(f.supplier.calls.load(Ordering::Acquire), 0);
        f.finish().await;
    });
    checked(&server.finish(), 0, 0);
}

#[test]
fn json_probe_rejects_a_different_generation_before_credentials() {
    let server = Server::new(vec![]);
    run(async {
        let f = Fixture::new(&server, Supplier::default());
        let plan = super::Plan::new(f.scope.clone(), specification(&f,
            super::FreeRoute::Responses, responses_payload())).unwrap();
        close(&f.client, &f.scope).await;
        let mut identity = Identity::new();
        identity.account = f.scope.account();
        let other = identity.open(&f.client, &server);
        assert_eq!(other.account(), f.scope.account());
        assert_eq!(other.active_user(), f.scope.active_user());
        assert!(!other.same_generation(&f.scope));
        let gate = super::CheckedAuthorization { inner: f.supplier.clone(), contract: plan.contract.clone() };
        let mut attempt = prepared(&plan.contract);
        attempt.scope = &other;
        assert!(gate.authorize_observed(attempt, &|_, _| {}).await.is_err());
        assert_eq!(f.supplier.calls.load(Ordering::Acquire), 0);
        close(&f.client, &other).await;
        drop((gate, plan, other));
        f.finish().await;
    });
    checked(&server.finish(), 0, 0);
}

#[test]
fn json_probe_missing_actual_turn_context_never_sends_request() {
    let server = Server::new(vec![]);
    run(async {
        let f = Fixture::new(&server, Supplier { missing_context: true, ..Supplier::default() });
        let report = probe_result(start_probe(&f, super::FreeRoute::Responses, responses_payload()).await.unwrap()).await;
        assert!(!report.accepted && report.cleanup_confirmed);
        assert_eq!(report.failure, Some(FailureKind::Authorization));
        assert_eq!(report.authorization_calls, 1);
        assert_eq!(report.transport_starts, 0);
        f.finish().await;
    });
    checked(&server.finish(), 0, 0);
}

#[test]
fn json_probe_mismatched_supplied_context_never_sends_request() {
    let cases: [(&str, fn(&mut AttemptContext)); 5] = [
        ("feature category", |context| context.feature_category.push_str("-changed")),
        ("feature name", |context| context.feature_name.push_str("-changed")),
        ("step", |context| context.step.push_str("-changed")),
        ("turn ID", |context| context.turn_id.push_str("-changed")),
        ("user prompt", |context| context.user_prompt.push_str("-changed")),
    ];
    let server = Server::new(vec![]);
    run(async {
        for (name, change) in cases {
            let f = Fixture::new(&server, Supplier::default());
            let mut spec = specification(&f, super::FreeRoute::Responses, responses_payload());
            change(&mut spec.context);
            let report = probe_result(start_specification(&f, spec).await.unwrap()).await;
            assert!(!report.accepted && report.cleanup_confirmed, "{name}");
            assert_eq!(report.failure, Some(FailureKind::Authorization), "{name}");
            assert_eq!(report.transport_starts, 0, "{name}");
            assert_eq!(f.supplier.calls.load(Ordering::Acquire), 1, "{name}");
            f.finish().await;
        }
    });
    checked(&server.finish(), 0, 0);
}

#[test]
fn json_probe_forbids_unobserved_authorization_and_repeated_attempts() {
    let server = Server::new(vec![]);
    run(async {
        let f = Fixture::new(&server, Supplier::default());
        let plan = super::Plan::new(f.scope.clone(), specification(&f,
            super::FreeRoute::Responses, responses_payload())).unwrap();
        let gate = super::CheckedAuthorization { inner: f.supplier.clone(), contract: plan.contract.clone() };
        assert!(gate.authorize(prepared(&plan.contract)).await.is_err());
        assert_eq!(f.supplier.calls.load(Ordering::Acquire), 0);
        let observer = |_, _| {};
        assert!(gate.authorize_observed(prepared(&plan.contract), &observer).await.is_err());
        drop((gate, plan));
        let plan = super::Plan::new(f.scope.clone(), specification(&f,
            super::FreeRoute::Responses, responses_payload())).unwrap();
        let gate = super::CheckedAuthorization { inner: f.supplier.clone(), contract: plan.contract.clone() };
        assert!(gate.authorize_observed(prepared(&plan.contract), &observer).await.is_ok());
        assert!(gate.authorize_observed(prepared(&plan.contract), &observer).await.is_err());
        assert_eq!(f.supplier.calls.load(Ordering::Acquire), 1);
        drop((gate, plan));
        f.finish().await;
    });
    checked(&server.finish(), 0, 0);
}

#[test]
fn json_probe_invalid_specification_never_reaches_credentials() {
    let cases: [(&str, fn(&mut super::Specification)); 10] = [
        ("streaming", |spec| spec.payload["stream"] = json!(true)),
        ("missing chat session", |spec| spec.identity.chat_session.clear()),
        ("missing model", |spec| spec.payload["model"] = Value::Null),
        ("zero token limit", |spec| spec.token_limit = 0),
        ("long deadline", |spec| spec.deadline = Instant::now() + Duration::from_secs(61)),
        ("previous response", |spec| spec.payload["previous_response_id"] = json!("must-not-be-dropped")),
        ("oversized payload", |spec| spec.payload["extra"] = json!("x".repeat(super::BYTE_LIMIT))),
        ("missing feature name", |spec| spec.context.feature_name.clear()),
        ("missing turn ID", |spec| spec.context.turn_id.clear()),
        ("oversized user prompt", |spec| spec.context.user_prompt = "x".repeat(super::BYTE_LIMIT + 1)),
    ];
    let server = Server::new(vec![]);
    run(async {
        let f = Fixture::new(&server, Supplier::default());
        for (name, change) in cases {
            let mut spec = specification(&f, super::FreeRoute::Responses, responses_payload());
            change(&mut spec);
            assert!(super::Plan::validate(&spec).is_err(), "{name}");
            assert!(super::Plan::new(f.scope.clone(), spec).is_err(), "{name}");
        }
        assert_eq!(f.supplier.calls.load(Ordering::Acquire), 0);
        f.finish().await;
    });
    checked(&server.finish(), 0, 0);
}

#[test]
fn json_probe_responses_uses_free_builder_with_exact_history_and_alias() {
    let server = Server::new(vec![expected("/ai/v3/responses/", expected_responses(), 0,
        Reply::json(response_body()))]);
    run(async {
        let f = Fixture::new(&server, Supplier::default());
        let probe = start_probe(&f, super::FreeRoute::Responses, responses_payload()).await.unwrap();
        let r = tokio::time::timeout(STEP, probe.result).await.unwrap();
        let Some(super::CompletedJson::Responses(value)) = probe.completed.await else {
            panic!("accepted response missing: {r:?}");
        };
        assert_eq!(value.output, response_body()["output"].as_array().unwrap().clone());
        assert_eq!(probe.cleanup.await, Ok(()));
        assert!(r.accepted);
        assert_eq!(r.client_model, "gpt-4.1");
        assert_eq!(r.wire_model, "openai/gpt-4.1");
        assert_eq!((r.attempts, r.authorization_calls, r.transport_starts, r.transport_finishes), (1, 1, 1, 1));
        assert_eq!((r.input_tokens, r.output_tokens), (Some(11), Some(7)));
        assert_eq!(f.capacity.available_permits(), 1);
        f.finish().await;
    });
    checked(&server.finish(), 1, 1);
}

#[test]
fn json_probe_messages_preserves_model_tools_and_thinking() {
    let server = Server::new(vec![expected("/ai/v3/messages/", expected_messages(), 0,
        Reply::json(message_body()))]);
    run(async {
        let f = Fixture::new(&server, Supplier::default());
        let probe = start_probe(&f, super::FreeRoute::Messages, messages_payload()).await.unwrap();
        let r = tokio::time::timeout(STEP, probe.result).await.unwrap();
        let Some(super::CompletedJson::Messages(value)) = probe.completed.await else {
            panic!("accepted message missing");
        };
        assert_eq!(value.content, message_body()["content"].as_array().unwrap().clone());
        assert_eq!(probe.cleanup.await, Ok(()));
        assert!(r.accepted);
        assert_eq!(r.wire_model, "anthropic/fixture-model");
        assert_eq!(r.client_model, r.wire_model);
        assert_eq!(r.path, "/ai/v3/messages/");
        f.finish().await;
    });
    checked(&server.finish(), 1, 1);
}

#[test]
fn json_probe_incomplete_response_is_not_retried_or_accepted() {
    let (first, _) = responses_continuation();
    let server = Server::new(vec![expected("/ai/v3/responses/", expected_responses(), 0, Reply::json(first))]);
    run(async {
        let f = Fixture::new(&server, Supplier::default());
        let r = probe_result(start_probe(&f, super::FreeRoute::Responses, responses_payload()).await.unwrap()).await;
        assert!(!r.accepted && !r.complete_json && r.cleanup_confirmed);
        assert_eq!(r.authorization_calls, 1);
        f.finish().await;
    });
    checked(&server.finish(), 1, 1);
}

#[test]
fn json_probe_refusal_is_one_attempt_and_discards_private_body() {
    let mut reply = Reply::json(json!({"private_marker":"must not appear in evidence"}));
    reply.status = 403;
    let server = Server::new(vec![expected("/ai/v3/responses/", expected_responses(), 0, reply)]);
    run(async {
        let f = Fixture::new(&server, Supplier::default());
        let r = probe_result(start_probe(&f, super::FreeRoute::Responses, responses_payload()).await.unwrap()).await;
        assert!(!r.accepted && r.cleanup_confirmed);
        assert_eq!(r.status, Some(403));
        assert_eq!(r.failure, Some(FailureKind::Upstream));
        assert_eq!(r.refusal.unwrap().basis, super::RefusalBasis::ElapsedFallback);
        assert!(!format!("{r:?}").contains("private_marker"));
        assert_eq!(f.supplier.calls.load(Ordering::Acquire), 1);
        f.finish().await;
    });
    checked(&server.finish(), 1, 1);
}

#[test]
fn json_probe_both_free_routes_retain_private_refusal_classification() {
    for route in [super::FreeRoute::Responses, super::FreeRoute::Messages] {
        for provider in [false, true] {
            let mut body = json!({"error":{"status":403,
                "message":"This request was blocked by our security checks",
                "data":{"reason":"request_blocked"}, "ref":"private-reference",
                "timestamp":"private-time"}});
            if provider { body["request_id"] = json!("gen-private-provider-reference"); }
            let (path, expected_body, payload) = match route {
                super::FreeRoute::Responses => ("/ai/v3/responses/", expected_responses(), responses_payload()),
                super::FreeRoute::Messages => ("/ai/v3/messages/", expected_messages(), messages_payload()),
            };
            let mut reply = Reply::json(body);
            reply.status = 403;
            let server = Server::new(vec![expected(path, expected_body, 0, reply)]);
            run(async {
                let f = Fixture::new(&server, Supplier::default());
                let r = probe_result(start_probe(&f, route, payload).await.unwrap()).await;
                let wanted = if provider { super::RefusalObservation {
                    policy: super::Refusal::Upstream, basis: super::RefusalBasis::ProviderMarker }
                } else { super::RefusalObservation { policy: super::Refusal::Castle,
                    basis: super::RefusalBasis::FramerSecurityEnvelope } };
                assert_eq!(r.refusal, Some(wanted));
                assert_eq!(r.status, Some(403));
                assert!(!r.accepted && r.cleanup_confirmed);
                assert_eq!((r.attempts, r.authorization_calls, r.transport_starts), (1, 1, 1));
                assert!(!format!("{r:?}").contains("private-reference"));
                assert!(!format!("{r:?}").contains("gen-private"));
                f.finish().await;
            });
            checked(&server.finish(), 1, 1);
        }
    }
}

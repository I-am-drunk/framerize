use super::*;
use crate::request::complete_json::{CompletedJson, Evidence, FreeRoute, Plan, ProbeRun};
use crate::request::continuation::Continuation;

fn options(spec: &super::super::Specification) -> Continuation {
    let mut next = spec.context.clone();
    next.turn_id = "fixture-application-turn-1".into();
    Continuation { max_output_tokens: 32, prompt: CALLER_CONTINUE.into(),
        contexts: vec![next] }
}

struct Case {
    path: &'static str, payload: Value, initial: Value,
    first: Value, next: Value, last: Value,
}

async fn completed(probe: ProbeRun) -> (Evidence, CompletedJson) {
    let report = tokio::time::timeout(STEP, probe.result).await.unwrap();
    assert_eq!(tokio::time::timeout(STEP, probe.cleanup).await.unwrap(), Ok(()));
    let response = tokio::time::timeout(STEP, probe.completed).await.unwrap().unwrap();
    (report, response)
}

#[test]
fn continuation_keeps_a_later_refusal_terminal_and_private() {
    let c = case(FreeRoute::Responses);
    let mut denied = Reply::json(json!({"error":{"status":403,
        "message":"This request was blocked by our security checks",
        "data":{"reason":"request_blocked"}, "ref":"private-marker"}}));
    denied.status = 403;
    let server = Server::new(vec![expected(c.path, c.initial, 0, Reply::json(c.first)),
        expected(c.path, c.next, 1, denied)]);
    run(async {
        let f = Fixture::new(&server, Supplier::default());
        let spec = specification(&f, FreeRoute::Responses, c.payload);
        let mut continuation = options(&spec);
        continuation.contexts.push(continuation.contexts[0].clone());
        let plan = Plan::new_with_continuation(f.scope.clone(), spec, Some(continuation)).unwrap();
        let probe = plan.launch_in_pool(f.client.clone(), f.supplier.clone(),
            f.capacity.clone().try_acquire_owned().unwrap(), f.admission()).await.unwrap();
        let report = probe_result(probe).await;
        assert!(!report.accepted && report.cleanup_confirmed);
        assert_eq!(report.status, Some(403));
        assert_eq!(report.attempts, 2);
        assert_eq!(report.failure, Some(FailureKind::Upstream));
        assert_eq!(report.refusal.unwrap().policy, super::super::Refusal::Castle);
        assert!(!format!("{report:?}").contains("private-marker"));
        assert_eq!(f.supplier.calls.load(Ordering::Acquire), 2);
        f.finish().await;
    });
    checked(&server.finish(), 2, 2);
}

#[test]
fn continuation_rejects_a_changed_authorization_context_before_transport() {
    let c = case(FreeRoute::Responses);
    let server = Server::new(vec![expected(c.path, c.initial, 0, Reply::json(c.first))]);
    run(async {
        let f = Fixture::new(&server, Supplier::default());
        let spec = specification(&f, FreeRoute::Responses, c.payload);
        let mut continuation = options(&spec);
        continuation.contexts[0].turn_id = "different-caller-context".into();
        let plan = Plan::new_with_continuation(f.scope.clone(), spec, Some(continuation)).unwrap();
        let probe = plan.launch_in_pool(f.client.clone(), f.supplier.clone(),
            f.capacity.clone().try_acquire_owned().unwrap(), f.admission()).await.unwrap();
        let report = probe_result(probe).await;
        assert!(!report.accepted && report.cleanup_confirmed);
        assert_eq!(report.failure, Some(FailureKind::Authorization));
        assert_eq!(report.transport_starts, 1);
        assert_eq!(report.authorization_calls, 2);
        f.finish().await;
    });
    checked(&server.finish(), 1, 1);
}

#[test]
fn continuation_cancellation_joins_pending_second_authorization() {
    let c = case(FreeRoute::Responses);
    let server = Server::new(vec![expected(c.path, c.initial, 0, Reply::json(c.first))]);
    run(async {
        let (mut supplier, release) = Supplier::held();
        supplier.gate_index = 1;
        let f = Fixture::new(&server, supplier);
        let spec = specification(&f, FreeRoute::Responses, c.payload);
        let continuation = options(&spec);
        let plan = Plan::new_with_continuation(f.scope.clone(), spec, Some(continuation)).unwrap();
        let probe = plan.launch_in_pool(f.client.clone(), f.supplier.clone(),
            f.capacity.clone().try_acquire_owned().unwrap(), f.admission()).await.unwrap();
        wait_count(&f.supplier.calls, 2).await;
        probe.cancellation.cancel();
        let report = probe_result(probe).await;
        assert!(!report.accepted && report.cleanup_confirmed);
        assert_eq!(report.failure, Some(FailureKind::Cancelled));
        assert_eq!(report.transport_starts, 1);
        assert_eq!(f.supplier.cancelled.load(Ordering::Acquire), 1);
        drop(release);
        f.finish().await;
    });
    checked(&server.finish(), 1, 1);
}

#[test]
fn continuation_exhaustion_never_reports_a_complete_reply() {
    let c = case(FreeRoute::Responses);
    let server = Server::new(vec![expected(c.path, c.initial, 0, Reply::json(c.first.clone())),
        expected(c.path, c.next, 1, Reply::json(c.first))]);
    run(async {
        let f = Fixture::new(&server, Supplier::default());
        let spec = specification(&f, FreeRoute::Responses, c.payload);
        let continuation = options(&spec);
        let plan = Plan::new_with_continuation(f.scope.clone(), spec, Some(continuation)).unwrap();
        let probe = plan.launch_in_pool(f.client.clone(), f.supplier.clone(),
            f.capacity.clone().try_acquire_owned().unwrap(), f.admission()).await.unwrap();
        let report = probe_result(probe).await;
        assert!(!report.accepted && !report.complete_json && report.cleanup_confirmed);
        assert_eq!(report.attempts, 2);
        assert_eq!(f.supplier.calls.load(Ordering::Acquire), 2);
        f.finish().await;
    });
    checked(&server.finish(), 2, 2);
}

#[test]
fn invalid_continuation_bounds_refuse_before_authorization() {
    let server = Server::new(vec![]);
    run(async {
        let f = Fixture::new(&server, Supplier::default());
        for fault in 0..7 {
            let spec = specification(&f, FreeRoute::Responses, responses_payload());
            let mut continuation = options(&spec);
            match fault {
                0 => continuation.contexts.clear(),
                1 => continuation.contexts = vec![spec.context.clone(); 16],
                2 => continuation.max_output_tokens = 31,
                3 => continuation.max_output_tokens = 32_769,
                4 => continuation.prompt.clear(),
                5 => continuation.prompt = "x".repeat(4097),
                6 => continuation.contexts[0].turn_id = "bad\ncontext".into(),
                _ => unreachable!(),
            }
            assert!(Plan::new_with_continuation(f.scope.clone(), spec, Some(continuation)).is_err());
        }
        assert_eq!(f.supplier.calls.load(Ordering::Acquire), 0);
        f.finish().await;
    });
    checked(&server.finish(), 0, 0);
}

#[test]
fn continuation_does_not_publish_a_later_unaccepted_response() {
    for (status, content_type) in [(202, Some("application/json")), (200, None)] {
        let c = case(FreeRoute::Responses);
        let mut last = Reply::json(c.last);
        last.status = status;
        last.content_type = content_type;
        let server = Server::new(vec![expected(c.path, c.initial, 0, Reply::json(c.first)),
            expected(c.path, c.next, 1, last)]);
        run(async {
            let f = Fixture::new(&server, Supplier::default());
            let spec = specification(&f, FreeRoute::Responses, c.payload);
            let continuation = options(&spec);
            let (sender, mut receiver) = tokio::sync::mpsc::channel(2);
            let plan = Plan::new_with_continuation(f.scope.clone(), spec,
                Some(continuation)).unwrap().with_events(Some(sender));
            let probe = plan.launch_in_pool(f.client.clone(), f.supplier.clone(),
                f.capacity.clone().try_acquire_owned().unwrap(), f.admission()).await.unwrap();
            let report = probe_result(probe).await;
            assert!(!report.accepted && report.cleanup_confirmed);
            assert!(receiver.try_recv().is_ok());
            assert!(receiver.try_recv().is_err());
            f.finish().await;
        });
        checked(&server.finish(), 2, 2);
    }
}

#[test]
fn continuation_uses_fresh_generation_authorization_for_each_chunk() {
    for route in [FreeRoute::Responses, FreeRoute::Messages] {
        let c = case(route);
        let server = Server::new(vec![
            expected(c.path, c.initial, 0, Reply::json(c.first)),
            expected(c.path, c.next, 1, Reply::json(c.last)),
        ]);
        run(async {
            let f = Fixture::new(&server, Supplier::default());
            let spec = specification(&f, route, c.payload);
            let continuation = options(&spec);
            let plan = Plan::new_with_continuation(f.scope.clone(), spec,
                Some(continuation)).unwrap();
            let source = Arc::new(super::exact_context::MaterialSource(AtomicUsize::new(0)));
            let probe = plan.launch_with_source_in_pool(f.client.clone(), source.clone(),
                f.capacity.clone().try_acquire_owned().unwrap(), f.admission()).await.unwrap();
            let (report, _) = completed(probe).await;
            assert!(report.accepted, "{report:?}");
            assert_eq!(source.0.load(Ordering::Acquire), 2);
            assert_eq!(report.authorization_calls, 2);
            assert_eq!(report.transport_starts, 2);
            assert_eq!(report.transport_finishes, 2);
            f.finish().await;
        });
        checked(&server.finish(), 2, 2);
    }
}

fn case(route: FreeRoute) -> Case {
    match route {
        FreeRoute::Responses => {
            let (first, next) = responses_continuation();
            Case { path: "/ai/v3/responses/", payload: responses_payload(),
                initial: expected_responses(), first, next, last: response_body() }
        },
        FreeRoute::Messages => {
            let (first, next) = messages_continuation();
            Case { path: "/ai/v3/messages/", payload: messages_payload(),
                initial: expected_messages(), first, next, last: message_body() }
        },
    }
}

#[test]
fn continuation_preserves_history_context_events_and_total_usage() {
    for route in [FreeRoute::Responses, FreeRoute::Messages] {
        let c = case(route);
        let server = Server::new(vec![
            expected(c.path, c.initial, 0, Reply::json(c.first)),
            expected(c.path, c.next, 1, Reply::json(c.last)),
        ]);
        run(async {
            let f = Fixture::new(&server, Supplier::default());
            let spec = specification(&f, route, c.payload);
            let continuation = options(&spec);
            let (sender, mut receiver) = tokio::sync::mpsc::channel(2);
            let plan = Plan::new_with_continuation(f.scope.clone(), spec,
                Some(continuation)).unwrap().with_events(Some(sender));
            let probe = plan.launch_in_pool(f.client.clone(), f.supplier.clone(),
                f.capacity.clone().try_acquire_owned().unwrap(), f.admission()).await.unwrap();
            let (report, response) = completed(probe).await;
            assert!(report.accepted, "{report:?}");
            assert_eq!(report.attempts, 2);
            assert_eq!(report.authorization_calls, 2);
            assert_eq!(report.output_tokens, Some(14));
            assert_eq!(report.input_tokens, Some(11));
            if let CompletedJson::Responses(value) = response {
                assert_eq!(value.usage["total_tokens"], 25);
                assert_eq!(value.usage["output_tokens_details"]["reasoning_tokens"], 4);
            }
            assert!(receiver.try_recv().is_ok());
            assert!(receiver.try_recv().is_ok());
            assert!(receiver.try_recv().is_err());
            f.finish().await;
        });
        checked(&server.finish(), 2, 2);
    }
}

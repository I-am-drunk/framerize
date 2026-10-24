use super::*;

fn report() -> Value {
    json!({"accepted":true, "status":200, "content_type":"json",
        "response_complete":true, "complete_json":true, "nonempty_text":true,
        "body_bytes":128, "transport_starts":1, "transport_finishes":1,
        "fetch_settled":true, "refusal":null, "failure":null})
}

#[test]
fn comparison_refusal_keeps_status_without_retaining_private_fields() {
    let mut value = report();
    value["accepted"] = json!(false);
    value["status"] = json!(403);
    value["failure"] = json!("fixture-private-marker");
    value["token"] = json!("fixture-private-marker");
    value["refusal"] = json!({"policy":"castle", "basis":"framer_security_envelope"});
    let parsed = BaselineReport::parse(value).unwrap();
    assert_eq!(parsed.status, Some(403));
    assert_eq!(parsed.refusal.unwrap().policy, RefusalPolicy::Castle);
    assert!(!format!("{parsed:?}").contains("fixture-private-marker"));
}

#[test]
fn comparison_acceptance_requires_complete_single_settled_response() {
    assert!(BaselineReport::parse(report()).unwrap().accepted);
    for field in ["response_complete", "complete_json", "nonempty_text", "fetch_settled"] {
        let mut changed = report();
        changed[field] = json!(false);
        assert!(BaselineReport::parse(changed).is_err());
    }
    for (field, value) in [("transport_starts", json!(2)), ("transport_finishes", json!(null)),
        ("status", json!(403)), ("content_type", json!("event_stream")),
        ("body_bytes", json!(65_537)), ("failure", json!("incomplete"))] {
        let mut changed = report();
        changed[field] = value;
        assert!(BaselineReport::parse(changed).is_err());
    }
}

fn body(turn: &str) -> Value {
    json!({"projectId":"fixture", "turnId":turn, "sessionId":"conversation",
        "request":{"model":"fixture-model", "stream":false}})
}

#[test]
fn comparison_bodies_allow_only_a_fresh_turn_identifier() {
    let make = |other| ComparisonPlan::new(ProjectId("fixture".into()),
        "/ai/v3/responses/", body("first"), other);
    assert!(make(body("second")).is_ok());
    assert!(make(body("first")).is_err());
    for field in ["projectId", "sessionId", "request", "castleToken"] {
        let mut changed = body("second");
        changed[field] = json!("different");
        assert!(make(changed).is_err());
    }
    let mut oversized = body("second");
    oversized["turnId"] = json!("x".repeat(65_536));
    assert!(make(oversized).is_err());
}

#[test]
fn comparison_body_binding_preserves_json_types_and_nested_values() {
    let mut first = body("first");
    first["request"]["limit"] = json!(1);
    for changed in [json!(true), json!(1.0), json!("1"), json!(null), json!(2)] {
        let mut second = first.clone();
        second["turnId"] = json!("second");
        second["request"]["limit"] = changed;
        assert!(ComparisonPlan::new(ProjectId("fixture".into()),
            "/ai/v3/responses/", first.clone(), second).is_err());
    }
}

#[test]
fn comparison_turns_reject_empty_control_and_oversized_identifiers() {
    for invalid in [json!(""), json!("line\nbreak"), json!("x".repeat(257)),
        json!(true), json!(1), Value::Null] {
        for position in 0..2 {
            let mut pair = [body("first"), body("second")];
            pair[position]["turnId"] = invalid.clone();
            let [first, second] = pair;
            assert!(ComparisonPlan::new(ProjectId("fixture".into()),
                "/ai/v3/responses/", first, second).is_err());
        }
    }
}

fn context() -> (ComparisonContext, Arc<state::ScopeState>) {
    let (state, registry) = super::super::tests::local_state();
    let client = ComparisonClient { account: state.account,
        bridge: Arc::new(bridge::ScopedBridge { state: state.clone(), registry }) };
    (ComparisonContext { client, plan: ComparisonPlan::new(ProjectId("fixture".into()),
        "/ai/v3/responses/", body("first"), body("second")).unwrap(),
        handle: "fixture-handle".into(), page_url: "https://framer.com/projects/fixture".into(),
        cookie_header: "session=fixture-only".into(), baseline_started: false,
        baseline_accepted: false, native_issued: false }, state)
}

#[tokio::test]
async fn comparison_reserves_space_for_predecessor_retirement_before_opening() {
    let (state, _) = super::super::tests::local_state();
    let factory = state._owner.observer_factory(ObserverLimits {
        scopes: 1, commands: 1, cleanup_timeout: Duration::from_secs(1),
    }).unwrap();
    assert!(matches!(factory.comparison_scope(state.account, state.application),
        Err(ObserverError::InvalidLimits)));
    assert!(factory.registry.accounts.lock().unwrap().is_empty());
    assert!(state._owner.inner.lock().await.is_none());
}

#[tokio::test]
async fn comparison_rejection_and_uncertain_baseline_never_admit_native_token() {
    for started in [false, true] {
        let (mut context, state) = context();
        context.baseline_started = started;
        assert!(context.native_token().await.is_err());
        assert!(!state.lock_phase().started);
        if started {
            assert!(context.baseline("fixture-bearer", "session=fixture-only").await.is_err());
            assert!(!state.lock_phase().started);
        }
    }
}

#[tokio::test]
async fn comparison_rejects_changed_bearer_jar_and_second_native_issuance_before_io() {
    let (mut context, state) = context();
    assert!(context.baseline("fixture-bearer", "session=different").await.is_err());
    assert!(!context.baseline_started);
    context.baseline_accepted = true;
    context.native_issued = true;
    assert!(context.native_token().await.is_err());
    assert!(!state.lock_phase().started);
}

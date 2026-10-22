//! Admission tests use synthetic records, in-process HTTP and denied legacy ports.
use super::*;
use serde_json::json;
use std::time::Duration;
use tower::ServiceExt;

#[path = "native_serving_policy_fixture.rs"]
mod fixture;
use fixture::{account, fixture, ProbeProvider, Spy};

#[derive(Clone, Copy)]
struct Case { path: &'static str, dialect: ClientDialect, paid: bool, raw: bool }

const CASES: [Case; 9] = [
    Case { path: "/paid/anthropic/v1/messages", dialect: ClientDialect::AnthropicMessages, paid: false, raw: true },
    Case { path: "/paid/openai/v1/responses", dialect: ClientDialect::OpenAiResponses, paid: false, raw: true },
    Case { path: "/paid/openai/v1/chat/completions", dialect: ClientDialect::OpenAiChat, paid: false, raw: true },
    Case { path: "/anthropic/v1/messages", dialect: ClientDialect::AnthropicMessages, paid: true, raw: true },
    Case { path: "/openai/v1/responses", dialect: ClientDialect::OpenAiResponses, paid: true, raw: true },
    Case { path: "/openai/v1/chat/completions", dialect: ClientDialect::OpenAiChat, paid: true, raw: true },
    Case { path: "/anthropic/v1/messages", dialect: ClientDialect::AnthropicMessages, paid: false, raw: false },
    Case { path: "/openai/v1/responses", dialect: ClientDialect::OpenAiResponses, paid: false, raw: false },
    Case { path: "/openai/v1/chat/completions", dialect: ClientDialect::OpenAiChat, paid: false, raw: false },
];

fn payload(case: Case) -> Value {
    let mut value = json!({"model":"gpt-4.1", "stream":false,
        "fixture_extension":{"opaque":[null,true,3]}});
    if case.dialect == ClientDialect::OpenAiResponses {
        value["input"] = json!("policy-fixture");
        value["max_output_tokens"] = json!(32);
    } else {
        value["messages"] = json!([{"role":"user","content":"policy-fixture"}]);
        value["max_tokens"] = json!(32);
    }
    value
}

fn expected_payload(case: Case, original: &Value) -> Value {
    if case.raw || case.dialect != ClientDialect::OpenAiChat { return original.clone(); }
    json!({"model":"gpt-4.1", "stream":false, "max_output_tokens":32,
        "input":[{"type":"message", "role":"user",
            "content":[{"type":"input_text", "text":"policy-fixture"}]}],
        "fixture_extension":{"opaque":[null,true,3]}})
}

async fn request(state: AppState, case: Case, selected: Option<u8>, payload: Value) -> Response {
    let mut builder = axum::http::Request::builder().method("POST").uri(case.path)
        .header("content-type", "application/json")
        .header("x-framerize-session", "policy-fixture-session");
    if case.paid { builder = builder.header("x-framerize-mode", "paid"); }
    if let Some(index) = selected {
        builder = builder.header("x-framerize-account", account(index).id.0.to_string());
    }
    tokio::time::timeout(Duration::from_secs(3), crate::router(state, "/fixture-no-web")
        .oneshot(builder.body(axum::body::Body::from(payload.to_string())).unwrap()))
        .await.expect("bounded in-process request").unwrap()
}

fn assert_refused(response: &Response, spy: &Spy, provider: &ProbeProvider) {
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(response.extensions().get::<FailureKind>(), Some(&FailureKind::Ineligible));
    assert!(provider.preparations.lock().unwrap().is_empty());
    assert!(provider.records.lock().unwrap().is_empty());
    spy.assert_untouched();
}

#[tokio::test]
async fn server_native_policy_explicit_parked_never_repicks_across_routes() {
    for case in CASES {
        let (state, spy, provider) = fixture(vec![account(1), account(2)], &[2], &[1, 2], true);
        let response = request(state, case, Some(1), payload(case)).await;
        assert_refused(&response, &spy, &provider);
        assert_eq!(*provider.membership.lock().unwrap(), vec![account(2).id]);
    }
}

#[tokio::test]
async fn server_native_policy_parked_rejection_does_not_wait_for_capacity() {
    for case in CASES {
        let (state, spy, provider) = fixture(vec![account(1), account(2)], &[2], &[1, 2], true);
        let key = format!("{}:openai/gpt-4.1", account(1).current_team.unwrap().0);
        let held = tokio::time::timeout(Duration::from_millis(500), state.gate.acquire(&key))
            .await.expect("fixture gate starts with one permit");
        let response = tokio::time::timeout(Duration::from_millis(500),
            request(state.clone(), case, Some(1), payload(case)))
            .await.expect("parked account rejection must not wait on its held gate");
        assert_refused(&response, &spy, &provider);
        assert_eq!(*provider.membership.lock().unwrap(), vec![account(2).id]);
        drop(held);
    }
}

#[tokio::test]
async fn server_native_policy_auto_selects_only_full_intersection() {
    for case in CASES {
        let mut cooling = account(2);
        cooling.cooling_until_ms = Some(u64::MAX);
        let (state, spy, provider) = fixture(
            vec![account(1), cooling, account(3), account(4)], &[2, 3, 4], &[1, 2, 4], true);
        let response = request(state, case, None, payload(case)).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(*provider.membership.lock().unwrap(),
            vec![account(3).id, account(4).id, account(4).id]);
        let preparations = provider.preparations.lock().unwrap();
        assert_eq!(preparations.len(), 1);
        assert_eq!(preparations[0].identity.account, account(4).id);
        assert_eq!(provider.records.lock().unwrap().len(), 1);
        spy.assert_read_only();
    }
}

#[tokio::test]
async fn server_native_policy_empty_intersection_never_falls_back() {
    for case in CASES {
        let mut cooling = account(2);
        cooling.cooling_until_ms = Some(u64::MAX);
        for (pool, allowed, members, queried) in [
            (vec![account(1)], vec![2], vec![1, 2], vec![]),
            (vec![cooling], vec![2], vec![2], vec![]),
            (vec![account(2)], vec![2], vec![1], vec![account(2).id]),
            (vec![account(1), account(2)], vec![9], vec![1, 2, 9], vec![]),
        ] {
            let (state, spy, provider) = fixture(pool, &allowed, &members, true);
            let response = request(state, case, None, payload(case)).await;
            assert_refused(&response, &spy, &provider);
            assert_eq!(*provider.membership.lock().unwrap(), queried);
        }
    }
}

#[tokio::test]
async fn server_native_policy_explicit_unavailable_never_substitutes() {
    for case in CASES {
        for cooling in [false, true] {
            let mut selected = account(1);
            if cooling { selected.cooling_until_ms = Some(u64::MAX); }
            let members = if cooling { vec![1, 2] } else { vec![2] };
            let (state, spy, provider) = fixture(vec![selected, account(2)], &[1, 2], &members, true);
            let response = request(state, case, Some(1), payload(case)).await;
            assert_refused(&response, &spy, &provider);
            let queried = if cooling { vec![account(2).id] } else { vec![account(1).id, account(2).id] };
            assert_eq!(*provider.membership.lock().unwrap(), queried);
        }
    }
}

#[tokio::test]
async fn server_native_policy_provider_absence_keeps_legacy_admission() {
    for case in CASES {
        let (state, spy, provider) = fixture(vec![account(1), account(2)], &[2], &[1, 2], false);
        let response = request(state, case, Some(1), payload(case)).await;
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert!(response.extensions().get::<FailureKind>().is_none());
        assert!(provider.membership.lock().unwrap().is_empty());
        assert!(provider.preparations.lock().unwrap().is_empty());
        assert!(provider.records.lock().unwrap().is_empty());
        spy.assert_untouched();
    }
}

#[tokio::test]
async fn server_native_policy_explicit_allowed_preserves_identity_and_request() {
    for case in CASES {
        let (state, spy, provider) = fixture(
            vec![account(1), account(2), account(3)], &[1, 2, 3], &[1, 2, 3], true);
        let body = payload(case);
        let started = Instant::now();
        let response = request(state, case, Some(2), body.clone()).await;
        let finished = Instant::now();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()["x-framerize-session"], "policy-fixture-session");
        let selected = account(2);
        let identity = Identity { account: selected.id, user: selected.framer_user_id,
            team: selected.current_team.unwrap(), project: selected.current_project.unwrap(),
            chat_session: "policy-fixture-session".into() };
        let (route, method) = if case.raw { (Route::Raw(case.dialect), "raw") }
            else if case.dialect == ClientDialect::AnthropicMessages { (Route::Messages, "messages") }
            else { (Route::Responses, "responses") };
        let preparations = provider.preparations.lock().unwrap();
        assert_eq!(preparations.len(), 1);
        assert_eq!(preparations[0].identity, identity);
        assert_eq!(preparations[0].route, route);
        assert_eq!(preparations[0].max_total_tokens, 32);
        assert!(preparations[0].deadline >= started + provider.policy.timeout);
        assert!(preparations[0].deadline <= finished + provider.policy.timeout);
        let records = provider.records.lock().unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].identity, identity);
        assert_eq!(records[0].route, route);
        assert_eq!(records[0].method, method);
        assert_eq!(records[0].max_total_tokens, 32);
        assert_eq!(records[0].deadline, preparations[0].deadline);
        assert_eq!(records[0].payload, expected_payload(case, &body));
        assert_eq!(*provider.membership.lock().unwrap(),
            vec![account(1).id, account(2).id, account(3).id, account(2).id]);
        spy.assert_read_only();
    }
}

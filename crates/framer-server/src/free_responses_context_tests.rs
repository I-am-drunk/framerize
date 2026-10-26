use crate::{operation_ownership, AppState};
use axum::{body::{to_bytes, Body}, http::{Request, StatusCode}, response::Response};
use framer_core::{account::ManagedAccount, ids::AccountId, port::StateStore};
use serde_json::{json, Value};
use std::sync::{Arc, atomic::{AtomicUsize, Ordering}};
use tower::ServiceExt;

#[path = "permit_lifetime_fixture.rs"]
mod fixture;

const ERROR: &str = "previous_response_id is unsupported on this route; send complete input history";
const LOOKUP: &str = "fixture account lookup";
const RESPONSES: &str = "/openai/v1/responses";

fn payload(reference: Option<Value>, stream: bool) -> Value {
    let mut value = json!({"model":"gpt-6-astra", "input":"fixture follow-up",
        "messages":[{"role":"user","content":"fixture"}], "stream":stream});
    if let Some(reference) = reference { value["previous_response_id"] = reference; }
    value
}

async fn request(state: AppState, path: &str, paid: bool, body: Value) -> Response {
    let mut request = Request::builder().method("POST").uri(path)
        .header("content-type", "application/json");
    if paid { request = request.header("x-framerize-mode", "paid"); }
    crate::router(state, "/nonexistent-fixture-web").oneshot(
        request.body(Body::from(body.to_string())).unwrap()).await.unwrap()
}

async fn rejects_before_ports(stream: bool) {
    for owned in [false, true] {
        let mut f = fixture::Fixture::new();
        let state = Arc::new(BoundaryState::default());
        f.state.runtime_mut().unwrap().state = state.clone();
        if owned { f.state = operation_ownership::with_owned_operations(f.state, 32).unwrap(); }
        for reference in [json!("private-reference"), json!(""), json!(7),
            json!(false), json!([]), json!({"id":"private-reference"})] {
            let response = request(f.state.clone(), RESPONSES, false,
                payload(Some(reference), stream)).await;
            assert_eq!(response.status(), StatusCode::BAD_REQUEST);
            let bytes = to_bytes(response.into_body(), 4096).await.unwrap();
            assert_eq!(serde_json::from_slice::<Value>(&bytes).unwrap(),
                json!({"error":{"code":"invalid_request","message":ERROR}}));
            assert_eq!(state.0.load(Ordering::SeqCst), 0, "account selection started");
            assert!(f.loads.try_recv().is_err(), "session loaded");
            f.no_call();
            assert_eq!(f.state.serving_accounts.residents.snapshot().unwrap().occupied, 0);
            if let Some(owner) = &f.state.operation_ownership {
                assert!(owner.accepting_requests(), "preflight poisoned ownership");
            }
        }
    }
}

#[tokio::test]
async fn nonstream_previous_response_is_refused_before_account_work() {
    rejects_before_ports(false).await;
}

#[tokio::test]
async fn streaming_previous_response_is_refused_before_account_work() {
    rejects_before_ports(true).await;
}

#[tokio::test]
async fn absent_and_null_previous_response_keep_existing_dispatch() {
    for stream in [false, true] {
        for reference in [None, Some(Value::Null)] {
            reaches_account_lookup(RESPONSES, false, payload(reference, stream)).await;
        }
    }
}

#[tokio::test]
async fn paid_previous_response_keeps_existing_dispatch() {
    for stream in [false, true] {
        for (path, paid) in [("/paid/openai/v1/responses", false), (RESPONSES, true)] {
            reaches_account_lookup(path, paid,
                payload(Some(json!("private-reference")), stream)).await;
        }
    }
}

#[tokio::test]
async fn previous_response_field_does_not_change_other_dialects() {
    for path in ["/anthropic/v1/messages", "/openai/v1/chat/completions"] {
        for stream in [false, true] {
            reaches_account_lookup(path, false,
                payload(Some(json!("private-reference")), stream)).await;
        }
    }
}

#[tokio::test]
async fn complete_input_history_keeps_existing_dispatch() {
    for stream in [false, true] {
        for reference in [None, Some(Value::Null)] {
            let mut body = payload(reference, stream);
            body["input"] = json!([
                {"role":"user", "content":"fixture question"},
                {"role":"assistant", "content":"fixture answer"},
                {"role":"user", "content":"fixture follow-up"}]);
            reaches_account_lookup(RESPONSES, false, body).await;
        }
    }
}

async fn reaches_account_lookup(path: &str, paid: bool, body: Value) {
    let mut f = fixture::Fixture::new();
    let state = Arc::new(BoundaryState::default());
    f.state.runtime_mut().unwrap().state = state.clone();
    let response = request(f.state.clone(), path, paid, body).await;
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR, "{path}");
    assert_eq!(to_bytes(response.into_body(), 4096).await.unwrap().as_ref(), LOOKUP.as_bytes());
    assert_eq!(state.0.load(Ordering::SeqCst), 1, "{path}");
    assert!(f.loads.try_recv().is_err());
    f.no_call();
}

#[derive(Default)]
struct BoundaryState(AtomicUsize);

#[async_trait::async_trait]
impl StateStore for BoundaryState {
    async fn get(&self, _: AccountId) -> Result<Option<ManagedAccount>, String> {
        panic!("unexpected account read");
    }
    async fn list(&self) -> Result<Vec<ManagedAccount>, String> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Err(LOOKUP.into())
    }
    async fn upsert(&self, _: &ManagedAccount) -> Result<(), String> {
        panic!("unexpected account write");
    }
    async fn remove(&self, _: AccountId) -> Result<(), String> {
        panic!("unexpected account removal");
    }
}

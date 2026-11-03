use super::validate;
use crate::{operation_ownership, AppState};
use axum::{body::{to_bytes, Body}, http::{Request, StatusCode}, response::Response};
use framer_core::{account::ManagedAccount, ids::AccountId, port::StateStore};
use serde_json::{json, Value};
use std::sync::{Arc, atomic::{AtomicUsize, Ordering}};
use tower::ServiceExt;

#[path = "permit_lifetime_fixture.rs"]
mod fixture;

const ERROR: &str = "request body must be a JSON object";
const LOOKUP: &str = "fixture account lookup";
const FREE: [&str; 3] = [
    "/anthropic/v1/messages", "/openai/v1/responses", "/openai/v1/chat/completions",
];
const PAID: [&str; 3] = [
    "/paid/anthropic/v1/messages", "/paid/openai/v1/responses",
    "/paid/openai/v1/chat/completions",
];

#[tokio::test]
async fn free_messages_reject_non_objects_before_ports() {
    rejects_before_ports(FREE[0], false).await;
}
#[tokio::test]
async fn free_responses_reject_non_objects_before_ports() {
    rejects_before_ports(FREE[1], false).await;
}
#[tokio::test]
async fn free_chat_reject_non_objects_before_ports() {
    rejects_before_ports(FREE[2], false).await;
}
#[tokio::test]
async fn paid_messages_reject_non_objects_before_ports() {
    rejects_before_ports(PAID[0], false).await;
}
#[tokio::test]
async fn paid_responses_reject_non_objects_before_ports() {
    rejects_before_ports(PAID[1], false).await;
}
#[tokio::test]
async fn paid_chat_reject_non_objects_before_ports() {
    rejects_before_ports(PAID[2], false).await;
}
#[tokio::test]
async fn paid_header_rejects_non_objects_on_all_free_routes() {
    for path in FREE { rejects_before_ports(path, true).await; }
}

#[tokio::test]
async fn valid_objects_reach_existing_account_lookup() {
    for (path, paid) in FREE.into_iter().map(|p| (p, false))
        .chain(PAID.into_iter().map(|p| (p, false)))
        .chain(FREE.into_iter().map(|p| (p, true))) {
        let mut f = fixture::Fixture::new();
        let state = Arc::new(BoundaryState::default());
        f.state.runtime_mut().unwrap().state = state.clone();
        let model = if path.contains("anthropic") { "claude-opus-5-5" } else { "gpt-6-astra" };
        let response = request(f.state.clone(), path, paid, json!({
            "model":model, "messages":[{"role":"user","content":"fixture"}],
            "input":"fixture", "stream":false,
        })).await;
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR, "{path}");
        assert_eq!(to_bytes(response.into_body(), 4096).await.unwrap().as_ref(), LOOKUP.as_bytes());
        assert_eq!(state.0.load(Ordering::SeqCst), 1, "{path}");
        assert!(f.loads.try_recv().is_err());
        f.no_call();
    }
}

#[tokio::test]
async fn object_validation_preserves_checked_budget_rejection() {
    for path in FREE {
        let mut f = fixture::Fixture::new();
        let state = Arc::new(BoundaryState::default());
        f.state.runtime_mut().unwrap().state = state.clone();
        f.state = operation_ownership::with_owned_operations(f.state, 32).unwrap();
        let response = request(f.state.clone(), path, false, json!({
            "model":"gpt-6-astra", "max_tokens":u64::from(u32::MAX) + 1,
        })).await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert_eq!(response.headers()["x-should-retry"], "false");
        let body = to_bytes(response.into_body(), 4096).await.unwrap();
        assert_eq!(serde_json::from_slice::<Value>(&body).unwrap(), json!({
            "type":"error", "error":{"type":"invalid_request_error",
                "code":"invalid_request","message":"output token limit exceeds 4294967295"},
        }));
        assert_eq!(state.0.load(Ordering::SeqCst), 0);
        assert!(f.loads.try_recv().is_err());
        f.no_call();
        assert!(f.state.operation_ownership.as_ref().unwrap().accepting_requests());
    }
}

async fn request(state: AppState, path: &str, paid: bool, body: Value) -> Response {
    let mut request = Request::builder().method("POST").uri(path)
        .header("content-type", "application/json");
    if paid { request = request.header("x-framerize-mode", "paid"); }
    crate::router(state, "/nonexistent-fixture-web").oneshot(
        request.body(Body::from(body.to_string())).unwrap()).await.unwrap()
}

async fn rejects_before_ports(path: &str, paid: bool) {
    for owned in [false, true] {
        let mut f = fixture::Fixture::new();
        let state = Arc::new(BoundaryState::default());
        f.state.runtime_mut().unwrap().state = state.clone();
        if owned { f.state = operation_ownership::with_owned_operations(f.state, 32).unwrap(); }
        for body in non_objects() {
            let response = request(f.state.clone(), path, paid, body).await;
            assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{path}");
            assert_eq!(response.headers()["x-should-retry"], "false");
            let bytes = to_bytes(response.into_body(), 4096).await.unwrap();
            assert_eq!(serde_json::from_slice::<Value>(&bytes).unwrap(),
                json!({"type":"error", "error":{"type":"invalid_request_error",
                    "code":"invalid_request","message":ERROR}}));
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

fn non_objects() -> Vec<Value> {
    vec![Value::Null, json!([]), json!([{"private":"payload-sentinel"}]),
        json!("payload-sentinel"), json!(0), json!(-1), json!(1.5),
        json!(true), json!(false)]
}

#[test]
fn validator_preserves_empty_and_nested_objects() {
    for value in [json!({}), json!({"input":[null, 7], "metadata":{"private":"sentinel"}})] {
        let before = value.clone();
        assert!(validate(&value).is_ok());
        assert_eq!(value, before);
    }
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

#[test]
fn validator_rejects_every_non_object_kind() {
    for value in non_objects() {
        assert!(matches!(validate(&value), Err(crate::ApiError::InvalidRequest(message))
            if message == ERROR));
    }
}

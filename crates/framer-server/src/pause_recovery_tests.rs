use crate::{operation_ownership::with_owned_operations, ServingAccountPolicy};
use axum::{body::{Body, BodyDataStream}, http::{Request, StatusCode}};
use framer_core::{account::ManagedAccount, ids::AccountId, port::StateStore};
use futures::StreamExt;
use serde_json::{json, Value};
use std::{sync::{Arc, Mutex, atomic::{AtomicUsize, Ordering}}, time::Duration};
use tower::ServiceExt;

#[path = "permit_lifetime_fixture.rs"]
mod fixture;

const WAIT: Duration = Duration::from_secs(3);
const MODEL: &str = "gpt-6-astra";

fn request() -> Request<Body> {
    Request::builder().method("POST").uri("/openai/v1/responses")
        .header("content-type", "application/json")
        .body(Body::from(json!({"model":MODEL,"input":"fixture","stream":true})
            .to_string())).unwrap()
}

async fn open(f: &fixture::Fixture) -> axum::response::Response {
    let response = tokio::time::timeout(WAIT,
        crate::router(f.state.clone(), "/nonexistent-fixture-web").oneshot(request()))
        .await.expect("paused request did not return headers promptly").unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["content-type"], "text/event-stream");
    response
}

fn events(wire: &str) -> Vec<Value> {
    wire.lines().filter_map(|line| line.strip_prefix("data: "))
        .map(|data| serde_json::from_str(data).unwrap()).collect()
}

async fn heartbeat(body: &mut BodyDataStream, wire: &mut String) {
    tokio::time::timeout(Duration::from_secs(12), async {
        loop {
            let bytes = body.next().await.expect("stream ended while paused").unwrap();
            wire.push_str(std::str::from_utf8(&bytes).unwrap());
            if events(wire).iter().filter(|event| event["type"] == "response.in_progress")
                .count() >= 2 { return; }
        }
    }).await.expect("paused stream did not send a heartbeat");
}

async fn idle(f: &mut fixture::Fixture) {
    f.no_call();
    assert!(matches!(f.loads.try_recv(), Err(tokio::sync::mpsc::error::TryRecvError::Empty)));
    let snapshot = f.state.serving_accounts.residents.snapshot().unwrap();
    assert!(snapshot.residents.iter().all(|slot| slot.users == 0 && !slot.uncertain));
    for index in 0..f.accounts.len() {
        drop(f.state.gate.acquire_now(&f.key(index, MODEL)).await
            .expect("paused or completed attempt retained a model permit"));
    }
}

async fn drained(f: &mut fixture::Fixture) {
    let owner = f.state.operation_ownership.clone().unwrap();
    let mut ticket = owner.seal().unwrap();
    let proof = tokio::time::timeout(WAIT, ticket.wait_application()).await
        .expect("request cleanup leaked").unwrap();
    owner.check_application_drained(&proof).unwrap();
    drop(proof);
    ticket.cancel_before_control().unwrap();
    assert!(owner.accepting_requests());
    assert!(owner.unmanaged_host_close_required());
    assert_eq!(f.state.response_output.counts(), (0, 0, 0));
    idle(f).await;
}

async fn finish(body: &mut BodyDataStream, wire: &mut String) {
    while let Some(frame) = tokio::time::timeout(WAIT, body.next()).await.unwrap() {
        wire.push_str(std::str::from_utf8(&frame.unwrap()).unwrap());
    }
    let values = events(wire);
    assert_eq!(values.iter().filter(|event| event["type"] == "response.completed").count(), 1);
    assert!(values.iter().all(|event| event["type"] != "response.failed"));
    assert_eq!(values.last().unwrap()["response"]["output"][0]["content"][0]["text"], "fixture");
    for (index, event) in values.iter().enumerate() {
        assert_eq!(event["sequence_number"], index);
        if matches!(event["type"].as_str(), Some("response.created" | "response.in_progress")) {
            assert_eq!(event["response"]["id"], values[0]["response"]["id"]);
        }
    }
    assert!(!wire.contains("risk-refused"));
}

fn pause(f: &fixture::Fixture) {
    for account in &f.accounts[..2] {
        f.state.wave.note_refusal(&account.id.0.to_string());
    }
    assert!(f.state.wave.check().is_some());
}

#[tokio::test(start_paused = true)]
async fn refusal_wave_does_not_block_account_selection() {
    let mut f = fixture::Fixture::new();
    pause(&f);
    let before = serde_json::to_value(f.state.state.list().await.unwrap()).unwrap();
    let mut budget = Duration::from_secs(2);
    let outcome = tokio::time::timeout(Duration::from_secs(3), crate::account_wait::pick(
        &f.state, &[], crate::account_wait::WaitFor::Capacity, &mut budget,
    )).await.expect("admission wait exceeded its total budget");
    assert!(outcome.result.is_ok(), "healthy account was blocked by a refusal wave");
    assert!(outcome.cleanup_confirmed);
    assert_eq!(budget, Duration::from_secs(2));
    assert!(f.state.wave.check().is_some());
    let (_, lease) = outcome.result.unwrap();
    drop(lease);
    idle(&mut f).await;
    assert_eq!(serde_json::to_value(f.state.state.list().await.unwrap()).unwrap(), before);
}

#[tokio::test(start_paused = true)]
async fn initial_account_cooldown_waits_without_work_or_history_changes() {
    let mut f = fixture::Fixture::new();
    for account in &mut f.accounts {
        account.cooling_until_ms = Some(framer_core::rotation::now_ms() + 600_000);
        f.state.state.upsert(account).await.unwrap();
    }
    f.state = with_owned_operations(f.state, 32).unwrap();
    let before = serde_json::to_value(f.state.state.list().await.unwrap()).unwrap();
    let mut body = open(&f).await.into_body().into_data_stream();
    heartbeat(&mut body, &mut String::new()).await;
    idle(&mut f).await;
    assert_eq!(f.state.serving_accounts.residents.snapshot().unwrap().occupied, 0);
    drop(body);
    drained(&mut f).await;
    assert_eq!(serde_json::to_value(f.state.state.list().await.unwrap()).unwrap(), before);
    assert!(f.state.wave.check().is_none());
}

#[tokio::test(start_paused = true)]
async fn initial_refusal_wave_serves_a_healthy_account_immediately() {
    let mut f = fixture::Fixture::new();
    f.state = with_owned_operations(f.state, 32).unwrap();
    pause(&f);
    let mut body = open(&f).await.into_body().into_data_stream();
    let mut wire = String::new();
    assert!(f.state.wave.check().is_some());
    let call = f.next().await;
    assert_eq!(call.kind, "responses");
    let loaded = f.loads.recv().await.unwrap();
    assert_eq!(loaded, f.accounts.iter().find(|a| a.id == call.account).unwrap().framer_user_id);
    call.reply.send(Ok(fixture::success("responses"))).ok().unwrap();
    finish(&mut body, &mut wire).await;
    tokio::time::timeout(WAIT, call.dropped).await.unwrap().unwrap();
    drop(body);
    drained(&mut f).await;
    assert!(f.state.wave.check().is_none());
}

#[tokio::test(start_paused = true)]
async fn disconnect_after_refusal_wave_cancels_admitted_work() {
    let mut f = fixture::Fixture::new();
    f.state = with_owned_operations(f.state, 32).unwrap();
    pause(&f);
    let body = open(&f).await.into_body().into_data_stream();
    let call = f.next().await;
    assert_eq!(call.kind, "responses");
    f.loads.recv().await.unwrap();
    drop(body);
    drop(call);
    drained(&mut f).await;
    idle(&mut f).await;
    f.no_call();
}

struct Accounts {
    records: Mutex<Vec<ManagedAccount>>,
    writes: AtomicUsize,
}

#[async_trait::async_trait]
impl StateStore for Accounts {
    async fn get(&self, id: AccountId) -> Result<Option<ManagedAccount>, String> {
        Ok(self.records.lock().unwrap().iter().find(|a| a.id == id).cloned())
    }
    async fn list(&self) -> Result<Vec<ManagedAccount>, String> {
        Ok(self.records.lock().unwrap().clone())
    }
    async fn upsert(&self, account: &ManagedAccount) -> Result<(), String> {
        self.writes.fetch_add(1, Ordering::SeqCst);
        *self.records.lock().unwrap().iter_mut().find(|a| a.id == account.id).unwrap()
            = account.clone();
        Ok(())
    }
    async fn remove(&self, _: AccountId) -> Result<(), String> {
        panic!("unexpected account removal")
    }
}

fn replace_store(f: &mut fixture::Fixture) -> Arc<Accounts> {
    let store = Arc::new(Accounts {
        records: Mutex::new(f.accounts.clone()), writes: AtomicUsize::new(0),
    });
    let ids = f.accounts.iter().map(|a| a.id.0.to_string()).collect::<Vec<_>>().join(",");
    let runtime = f.state.runtime_mut().unwrap();
    runtime.state = store.clone();
    runtime.serving_accounts = Arc::new(ServingAccountPolicy::parse(Some(&ids)).unwrap());
    store
}

fn three_accounts() -> fixture::Fixture {
    let mut f = fixture::Fixture::new();
    let mut third = f.accounts[0].clone();
    third.id = AccountId(uuid::Uuid::new_v4());
    third.alias = "fixture-third".into();
    third.framer_user_id.0.push_str("-third");
    third.castle_device_id.push_str("-third");
    third.current_team.as_mut().unwrap().0.push_str("-third");
    third.current_project.as_mut().unwrap().0.push_str("-third");
    third.consecutive_failures = 2;
    f.accounts[1].consecutive_failures = 1;
    f.accounts.push(third);
    replace_store(&mut f);
    f.state = with_owned_operations(f.state, 32).unwrap();
    f
}

#[tokio::test(start_paused = true)]
async fn refusal_rotates_to_a_healthy_account_without_global_pause() {
    let mut f = three_accounts();
    let response = open(&f).await;
    let call = f.next().await;
    assert_eq!(call.account, f.accounts[0].id);
    f.loaded(0).await;
    call.reply.send(Err("risk-refused".into())).ok().unwrap();
    tokio::time::timeout(WAIT, call.dropped).await.unwrap().unwrap();
    let peer = f.next().await;
    assert_ne!(peer.account, call.account);
    let peer_index = f.accounts.iter().position(|account| account.id == peer.account).unwrap();
    f.loaded(peer_index).await;
    peer.reply.send(Ok(fixture::success("responses"))).ok().unwrap();
    tokio::time::timeout(WAIT, peer.dropped).await.unwrap().unwrap();
    let body = crate::response_output::consume_body(response.into_body(), 65536).await.unwrap();
    let wire = String::from_utf8(body.to_vec()).unwrap();
    assert!(wire.contains("response.completed"));
    assert!(wire.contains("fixture"));
    assert!(f.state.wave.check().is_none());
    let refused = f.state.state.get(call.account).await.unwrap().unwrap();
    assert!(refused.cooling_until_ms.is_some());
    for account in &f.accounts[1..] {
        let current = f.state.state.get(account.id).await.unwrap().unwrap();
        assert!(current.cooling_until_ms.is_none());
    }
    drained(&mut f).await;
}

#[tokio::test]
async fn server_error_ending_in_refusal_sentinel_never_changes_health_or_wave() {
    let mut f = fixture::Fixture::new();
    f.accounts[0].consecutive_failures = 3;
    f.accounts[0].cooldown_step = 2;
    f.accounts[0].cooling_until_ms = Some(u64::MAX);
    let store = replace_store(&mut f);
    let before = serde_json::to_value(f.state.state.list().await.unwrap()).unwrap();
    for paused in [false, true] {
        if paused { pause(&f); }
        for error in ["retryable 500: risk-refused",
            "chunk 0 retry died (fallback off): retryable 500: risk-refused"] {
            for account in &f.accounts {
                crate::record_account_health(&f.state, account,
                    &Err::<(), _>(error.to_string())).await;
                assert_eq!(f.state.wave.check().is_some(), paused);
            }
        }
    }
    assert_eq!(store.writes.load(Ordering::SeqCst), 0);
    assert_eq!(serde_json::to_value(f.state.state.list().await.unwrap()).unwrap(), before);
    idle(&mut f).await;
}

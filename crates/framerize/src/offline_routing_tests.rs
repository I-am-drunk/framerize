use super::*;
use crate::offline_routing::{Contexts, Provider, RequestContext};
use port::{InvocationProvider, OwnedInvocation, Preparation, Route};

#[path = "offline_routing_http_tests.rs"]
mod http;

#[path = "offline_routing_credit_tests.rs"]
mod credit;
#[path = "offline_routing_startup_tests.rs"]
mod startup;

struct Metadata { identity: port::Identity, proxy: Option<String>, missing: bool }
impl Contexts for Metadata {
    fn contains_account(&self, account: AccountId) -> bool { self.identity.account == account }
    fn resolve(&self, request: &Preparation, _: &serde_json::Value)
        -> Result<RequestContext, port::DispatchError> {
        let count = if self.missing { 0 } else if matches!(request.route, Route::Raw(_)) { 1 } else { 2 };
        Ok(RequestContext {
            identity: self.identity.clone(), proxy: self.proxy.clone(),
            page_url: "https://framer.com/projects/fixture-project".into(), paid_credit: None,
            attempts: (0..count).map(|i| AttemptContext {
                feature_category: "fixture".into(), feature_name: "fixture".into(),
                step: format!("step-{i}"), turn_id: format!("turn-{i}"),
                user_prompt: format!("supplied prompt {i}"),
            }).collect(),
        })
    }
}

fn provider(state: &AppState, ports: &Arc<Ports>, spec: &Specification,
    sessions: &Sessions, missing: bool) -> Provider {
    let metadata = Metadata { identity: spec.identity.clone(),
        proxy: ports.account.lock().unwrap().proxy.clone(), missing };
    let ports = ports.clone();
    Provider::with_sources(state.clone(), sessions.clone(), Arc::new(metadata), port::Policy {
        timeout: Duration::from_secs(3), max_body_bytes: 64 * 1024,
        missing_content_type: "application/json".into(), chunk_tokens: 32,
        max_chunks: 2, continuation: Some("fixture continuation".into()),
        allow_paid: false, credit_floor: 1.0,
    }, None, Arc::new(move |_| Arc::new(Builder(ports.clone())))).unwrap()
}

fn request(spec: &Specification) -> Preparation {
    Preparation { identity: spec.identity.clone(), route: Route::Responses,
        deadline: spec.deadline, max_total_tokens: spec.token_limit }
}

async fn prepare(state: &AppState, ports: &Arc<Ports>, spec: &Specification,
    sessions: &Sessions, missing: bool) -> Box<dyn OwnedInvocation> {
    let provider = provider(state, ports, spec, sessions, missing);
    let admission = native_control::admit(state, spec.identity.clone(), "gpt-4.1", spec.deadline)
        .await.unwrap();
    admission.completion.complete().unwrap();
    ports.events.lock().unwrap().clear();
    let call = provider.prepare_in_pool(request(spec), admission.capacity, admission.pool).await.unwrap();
    assert!(ports.events.lock().unwrap().is_empty());
    call
}

async fn snapshot(state: &AppState) -> serde_json::Value {
    use tower::ServiceExt;
    let mut inspection = state.clone();
    inspection.operation_ownership = None;
    let request = axum::http::Request::builder().uri("/api/pool")
        .body(axum::body::Body::empty()).unwrap();
    let response = framer_server::router(inspection, "fixture-no-web-root")
        .oneshot(request).await.unwrap();
    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let bytes = axum::body::to_bytes(response.into_body(), 16 * 1024).await.unwrap();
    serde_json::from_slice::<serde_json::Value>(&bytes).unwrap()["residency"].clone()
}

async fn users(state: &AppState, id: AccountId) -> u64 {
    snapshot(state).await["residents"].as_array().unwrap().iter()
        .find(|s| s["account"] == serde_json::json!(id))
        .map_or(0, |s| s["users"].as_u64().unwrap())
}

#[tokio::test]
async fn routing_preparation_keeps_original_capacity_without_io() {
    let (state, ports, spec, sessions) = fixture();
    let call = prepare(&state, &ports, &spec, &sessions, false).await;
    let key = format!("{}:{}", spec.identity.team.0, convert::to_openrouter_id("gpt-4.1"));
    assert!(state.gate.acquire(&key).now_or_never().is_none());
    assert_eq!(users(&state, spec.identity.account).await, 1);
    drop(call);
    assert!(ports.events.lock().unwrap().is_empty());
    assert_eq!(users(&state, spec.identity.account).await, 0);
    drop(tokio::time::timeout(Duration::from_secs(1), state.gate.acquire(&key)).await.unwrap());
}

#[tokio::test]
async fn routing_missing_step_context_rejects_before_private_io() {
    let (state, ports, spec, sessions) = fixture();
    let call = prepare(&state, &ports, &spec, &sessions, true).await;
    let run = call.responses(spec.payload);
    assert_eq!(run.result.await.err().unwrap().kind, FailureKind::MissingContext);
    assert_eq!(run.cleanup.await, Ok(()));
    assert!(ports.events.lock().unwrap().is_empty());
}

#[tokio::test]
async fn routing_cancel_before_inner_creation_starts_no_private_work() {
    let (state, ports, spec, sessions) = fixture();
    let call = prepare(&state, &ports, &spec, &sessions, false).await;
    let run = call.responses(spec.payload);
    run.cancellation.cancel();
    assert_eq!(run.result.await.err().unwrap().kind, FailureKind::Cancelled);
    assert_eq!(run.cleanup.await, Ok(()));
    assert!(ports.events.lock().unwrap().is_empty());
    assert_eq!(ports.builds.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn routing_cancelled_or_dropped_result_retains_private_load() {
    for drop_result in [false, true] {
        let (state, ports, spec, sessions) = fixture();
        let call = prepare(&state, &ports, &spec, &sessions, false).await;
        ports.read.try_acquire().unwrap().forget();
        let run = call.responses(spec.payload);
        wait(&ports.loading).await;
        let mut result = Some(run.result);
        if drop_result { drop(result.take()); } else { run.cancellation.cancel(); }
        assert!(run.cleanup.clone().now_or_never().is_none());
        assert!(users(&state, spec.identity.account).await > 0);
        assert_eq!(ports.builds.load(Ordering::SeqCst), 0);
        ports.read.add_permits(1);
        if let Some(result) = result {
            assert_eq!(result.await.err().unwrap().kind, FailureKind::Cancelled);
        }
        assert_eq!(run.cleanup.await, Ok(()));
        assert_eq!(users(&state, spec.identity.account).await, 0);
        assert_eq!(ports.builds.load(Ordering::SeqCst), 0);
        assert_eq!(*ports.events.lock().unwrap(), vec!["load", "loaded"]);
    }
}

#[tokio::test]
async fn routing_scope_change_after_load_prevents_issuance() {
    let (state, ports, spec, sessions) = fixture();
    let call = prepare(&state, &ports, &spec, &sessions, false).await;
    ports.read.try_acquire().unwrap().forget();
    let run = call.responses(spec.payload);
    wait(&ports.loading).await;
    ports.account.lock().unwrap().current_project = Some(ProjectId("changed".into()));
    ports.read.add_permits(1);
    assert_eq!(run.result.await.err().unwrap().kind, FailureKind::InvalidScope);
    assert_eq!(run.cleanup.await, Err(FailureKind::InvalidScope));
    assert_eq!(ports.builds.load(Ordering::SeqCst), 0);
    assert_eq!(users(&state, spec.identity.account).await, 0);
}

#[tokio::test]
async fn routing_store_rejection_and_worker_loss_remain_distinct() {
    for failure in [1, 2, 3] {
        let (state, ports, spec, sessions) = fixture();
        let call = prepare(&state, &ports, &spec, &sessions, false).await;
        ports.load_failure.store(failure, Ordering::SeqCst);
        let run = call.responses(spec.payload);
        let kind = if failure == 1 { FailureKind::Cleanup } else { FailureKind::Authorization };
        assert_eq!(run.result.await.err().unwrap().kind, kind);
        assert_eq!(run.cleanup.await, if failure == 1 { Err(FailureKind::Cleanup) } else { Ok(()) });
        assert_eq!(users(&state, spec.identity.account).await > 0, failure == 1);
        assert_eq!(ports.builds.load(Ordering::SeqCst), 0);
    }
}

#[tokio::test]
async fn routing_cancellation_retains_issuance_until_observed_cleanup() {
    let (state, ports, spec, sessions) = fixture();
    let call = prepare(&state, &ports, &spec, &sessions, false).await;
    ports.issue.try_acquire().unwrap().forget();
    let run = call.responses(spec.payload);
    wait(&ports.issuing).await;
    run.cancellation.cancel();
    tokio::time::timeout(Duration::from_secs(2), async {
        while !ports.cancelled.load(Ordering::SeqCst) { tokio::task::yield_now().await; }
    }).await.unwrap();
    assert!(run.cleanup.clone().now_or_never().is_none());
    assert!(users(&state, spec.identity.account).await > 0);
    ports.issue.add_permits(1);
    assert_eq!(run.result.await.err().unwrap().kind, FailureKind::Cancelled);
    assert_eq!(run.cleanup.await, Ok(()));
    let scope = ports.scope.lock().unwrap().clone().unwrap();
    assert_eq!(scope.closed().await, Ok(()));
}

#[tokio::test]
async fn routing_sealed_gate_rejects_before_private_work() {
    let (state, ports, spec, sessions) = fixture();
    let call = prepare(&state, &ports, &spec, &sessions, false).await;
    let seal = state.operation_ownership.as_ref().unwrap().seal().unwrap();
    let run = call.responses(spec.payload);
    assert_eq!(run.result.await.err().unwrap().kind, FailureKind::Ineligible);
    assert_eq!(run.cleanup.await, Ok(()));
    assert!(ports.events.lock().unwrap().is_empty());
    drop(seal);
}

#[tokio::test]
async fn routing_unconfirmed_source_or_builder_loss_retains_the_account() {
    for panic in [false, true] {
        let (state, ports, spec, sessions) = fixture();
        let call = prepare(&state, &ports, &spec, &sessions, false).await;
        ports.uncertain.store(!panic, Ordering::SeqCst);
        ports.panic_build.store(panic, Ordering::SeqCst);
        let run = call.responses(spec.payload);
        assert_eq!(run.result.await.err().unwrap().kind, FailureKind::Cleanup);
        assert_eq!(run.cleanup.await, Err(FailureKind::Cleanup));
        assert!(users(&state, spec.identity.account).await > 0);
        let snapshot = snapshot(&state).await;
        assert!(snapshot["residents"].as_array().unwrap().iter().any(|s|
            s["account"] == serde_json::json!(spec.identity.account) && s["uncertain"] == true));
    }
}

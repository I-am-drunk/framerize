//! Inert observation contracts. All execution belongs to the runtime owner.
//! Requires the observation sink/ObservedJoin contract from 3b9b128 on 95f29867.
//! Captures lifecycle fields, not the private ai.chain Event or log-file output.
//! Router disposal proves application drain, not unmanaged-host retirement.
use super::*;
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::sync::atomic::{AtomicUsize, Ordering};

struct Events {
    values: Arc<Mutex<Vec<Value>>>,
    changed: tokio::sync::mpsc::UnboundedReceiver<()>,
}

fn operation_events<'a>(events: &'a [Value], id: &str) -> Vec<&'a Value> {
    events.iter().filter(|event| event["operation_id"] == id).collect()
}
fn assert_schema(events: &[Value]) {
    let keys: BTreeSet<_> = ["event", "request_id", "operation_id", "parent_operation_id",
        "kind", "outcome", "cleanup_confirmed", "cause"].into_iter().collect();
    for event in events {
        assert_eq!(event.as_object().unwrap().keys().map(String::as_str).collect::<BTreeSet<_>>(), keys);
        for field in ["request_id", "operation_id"] {
            let text = event[field].as_str().unwrap();
            let id = uuid::Uuid::parse_str(text).unwrap();
            assert!(!id.is_nil());
            assert_eq!(id.to_string(), text);
        }
        if !event["parent_operation_id"].is_null() {
            uuid::Uuid::parse_str(event["parent_operation_id"].as_str().unwrap()).unwrap();
        }
        assert!(matches!(event["kind"].as_str(), Some("Inference" | "ExternalCleanup")));
        assert!(event["cause"].is_null() || event["cause"].is_string());
        if event["cleanup_confirmed"] == false {
            assert!(event["cause"].as_str().is_some_and(|cause| !cause.is_empty()));
        }
        match event["event"].as_str().unwrap() {
            "operation.registered" => {
                assert_eq!(event["outcome"], "active");
                assert!(event["cleanup_confirmed"].is_null());
            }
            "operation.joined" => {
                assert!(matches!(event["outcome"].as_str(),
                    Some("ok" | "error" | "panicked" | "cancelled")));
                assert!(event["cleanup_confirmed"].is_boolean());
            }
            "operation.released" => {
                assert_eq!(event["outcome"], "confirmed");
                assert_eq!(event["cleanup_confirmed"], true);
            }
            "operation.unresolved" => {
                assert!(matches!(event["outcome"].as_str(),
                    Some("dropped" | "completion_error")));
                assert_eq!(event["cleanup_confirmed"], false);
            }
            _ => panic!("unexpected lifecycle event"),
        }
    }
}
fn root_id(response: &Response) -> String {
    let id = response.headers()["x-framerize-operation-id"].to_str().unwrap();
    assert!(!uuid::Uuid::parse_str(id).unwrap().is_nil());
    id.to_owned()
}
fn child_id(events: &[Value], root: &str) -> String {
    let root_events = operation_events(events, root);
    let registration = root_events.iter().find(|event| event["event"] == "operation.registered").unwrap();
    assert_eq!(registration["request_id"], root);
    assert!(registration["parent_operation_id"].is_null());
    assert_eq!(registration["kind"], "Inference");
    let children: Vec<_> = events.iter().filter(|event|
        event["event"] == "operation.registered" && event["parent_operation_id"] == root).collect();
    assert_eq!(children.len(), 1);
    assert_eq!(children[0]["request_id"], root);
    assert_eq!(children[0]["kind"], "ExternalCleanup");
    children[0]["operation_id"].as_str().unwrap().to_owned()
}

fn assert_join(events: &[Value], id: &str, outcome: &str, clean: bool) {
    assert_schema(events);
    let rows = operation_events(events, id);
    assert_eq!(rows.iter().map(|e| e["event"].as_str().unwrap()).collect::<Vec<_>>(),
        vec!["operation.registered", "operation.joined",
            if clean { "operation.released" } else { "operation.unresolved" }]);
    assert_eq!(rows[0]["outcome"], "active");
    assert!(rows[0]["cleanup_confirmed"].is_null());
    assert_eq!(rows[1]["outcome"], outcome);
    assert_eq!(rows[1]["cleanup_confirmed"], clean);
    assert_eq!(rows[2]["cleanup_confirmed"], clean);
    assert_eq!(rows[2]["outcome"], if clean { "confirmed" } else { "dropped" });
    for row in &rows[1..] {
        for field in ["request_id", "parent_operation_id", "kind"] {
            assert_eq!(row[field], rows[0][field]);
        }
    }
}
fn stream_request(f: &fixture::Fixture, index: usize, messages: bool) -> HttpRequest<Body> {
    let mut request = if messages {
        HttpRequest::builder().method("POST").uri("/anthropic/v1/messages")
            .header("content-type", "application/json")
            .body(Body::from(json!({"model":"claude-opus-5-5", "max_tokens":16,
                "messages":[{"role":"user","content":"fixture"}],"stream":true}).to_string())).unwrap()
    } else { fixture_request(f, true) };
    request.headers_mut().insert("x-framerize-account", f.accounts[index].id.0.to_string().parse().unwrap());
    request
}
fn assert_output(body: &[u8], messages: bool) {
    let text = std::str::from_utf8(body).unwrap();
    let events: Vec<Value> = text.lines().filter_map(|line| line.strip_prefix("data: "))
        .map(|data| serde_json::from_str(data).unwrap()).collect();
    assert!(!events.iter().any(|e| e["type"] == "error" || e["type"] == "response.failed"));
    let end = if messages { "message_stop" } else { "response.completed" };
    assert_eq!(events.iter().filter(|e| e["type"] == end).count(), 1);
    if messages {
        assert_eq!(events.iter().filter(|e| e["type"] == "content_block_delta")
            .map(|e| e["delta"]["text"].as_str().unwrap()).collect::<String>(), "fixture");
        let endings: Vec<_> = events.iter().filter(|e| e["type"] == "message_delta").collect();
        assert_eq!(endings.len(), 1);
        assert_eq!(endings[0]["delta"]["stop_reason"], "end_turn");
        assert_eq!(endings[0]["usage"]["output_tokens"], 1);
    } else {
        let result = events.iter().find(|e| e["type"] == end).unwrap();
        assert_eq!(result["response"]["status"], "completed");
        assert_eq!(result["response"]["output"][0]["content"][0]["text"], "fixture");
        assert_eq!(result["response"]["usage"]["output_tokens"], 1);
    }
}

async fn routed_completion(messages: bool, disconnect: bool) {
    let mut f = fixture::Fixture::new();
    f.state = with_owned_operations(f.state, 32).unwrap();
    let ownership = f.state.operation_ownership.clone().unwrap();
    let mut events = Events::install(&ownership);
    let app = crate::router(f.state.clone(), "/nonexistent-fixture-web");
    let response = tokio::time::timeout(WAIT, app.oneshot(stream_request(&f, 0, messages)))
        .await.unwrap().unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let root = root_id(&response);
    let call = f.next().await;
    assert_eq!(call.account, f.accounts[0].id);
    assert_eq!(call.kind, if messages { "messages" } else { "responses" });
    let child = child_id(&events.snapshot(), &root);
    let completion = response.extensions().get::<ResponseCompletion>().unwrap().clone();
    let mut response = Some(response);
    if disconnect { drop(response.take()); }
    if disconnect && !messages { assert!(call.config.as_ref().unwrap().is_cancelled()); }
    assert!(!call.reply.is_closed());
    assert_eq!(ownership.gate.stats().unwrap().active, 3);
    assert_eq!(operation_events(&events.snapshot(), &child).len(), 1);
    assert_eq!(operation_events(&events.snapshot(), &root).len(), 1);
    let mut ticket = ownership.seal().unwrap();
    assert!(ticket.wait_application().now_or_never().is_none());
    call.reply.send(Ok(fixture::success(call.kind))).ok().unwrap();
    tokio::time::timeout(WAIT, call.dropped).await.unwrap().unwrap();
    if let Some(response) = response {
        assert_eq!(operation_events(&events.snapshot(), &root).len(), 1);
        let body = tokio::time::timeout(WAIT, to_bytes(response.into_body(), 65536)).await.unwrap().unwrap();
        assert_output(&body, messages);
    }
    assert!(tokio::time::timeout(WAIT, completion.0).await.unwrap());
    events.event(&root, "operation.released").await;
    let proof = tokio::time::timeout(WAIT, ticket.wait_application()).await.unwrap().unwrap();
    ownership.check_application_drained(&proof).unwrap();
    // The original attempt joined cleanly, but disconnected Responses output failed.
    assert_join(&events.snapshot(), &child,
        if disconnect && !messages { "error" } else { "ok" }, true);
    let root_rows = events.snapshot();
    assert_eq!(operation_events(&root_rows, &root).len(), 2);
    assert_eq!(ownership.gate.stats().unwrap().active, 1);
    assert!(!ownership.gate.stats().unwrap().unconfirmed);
    assert_eq!(f.state.response_output.counts(), (0, 0, 0));
    f.no_call();
    drop(proof);
    assert!(ownership.unmanaged_host_close_required());
    assert!(ticket.wait().now_or_never().is_none());
    ticket.cancel_before_control().unwrap();
}

#[tokio::test]
async fn disconnected_routes_keep_original_producer_until_join_and_release() {
    for messages in [true, false] { routed_completion(messages, true).await; }
}

#[tokio::test]
async fn clean_route_eof_preserves_output_and_releases_each_registration_once() {
    for messages in [true, false] { routed_completion(messages, false).await; }
}

#[derive(Debug)]
struct UniqueOutput { value: &'static str, drops: Arc<AtomicUsize> }
impl Drop for UniqueOutput {
    fn drop(&mut self) { self.drops.fetch_add(1, Ordering::SeqCst); }
}

#[tokio::test]
async fn shared_join_preserves_nonclone_result_and_joins_once() {
    let ownership = scope();
    let events = Events::install(&ownership);
    let drops = Arc::new(AtomicUsize::new(0));
    let output = Box::new(UniqueOutput { value: "synthetic-output", drops: drops.clone() });
    let address = (&*output as *const UniqueOutput) as usize;
    let (entered, started) = oneshot::channel();
    let (release, held) = oneshot::channel();
    let (handle, completion) = spawn_observed(Some(&ownership), OperationKind::Inference, async move {
        entered.send(tokio::task::id()).unwrap();
        held.await.unwrap();
        (Ok(output), true)
    }).unwrap();
    let original = handle.abort_handle();
    assert_eq!(tokio::time::timeout(WAIT, started).await.unwrap().unwrap(), original.id());
    assert!(completion.0.clone().now_or_never().is_none());
    release.send(()).unwrap();
    let (first, second) = (completion.0.clone(), completion.0.clone());
    let (result, a, b, c) = tokio::time::timeout(WAIT, async {
        tokio::join!(handle, first, second, completion.0)
    }).await.unwrap();
    assert!(a && b && c);
    assert!(original.is_finished());
    let output = result.unwrap().unwrap();
    assert_eq!((&*output as *const UniqueOutput) as usize, address);
    assert_eq!(output.value, "synthetic-output");
    assert_eq!(drops.load(Ordering::SeqCst), 0);
    let values = events.snapshot();
    assert_join(&values, values[0]["operation_id"].as_str().unwrap(), "ok", true);
    assert_eq!(ownership.gate.stats().unwrap().active, 1);
    drop(output);
    assert_eq!(drops.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn shared_join_preserves_original_panic_and_cancel_error() {
    for cancel in [false, true] {
        let ownership = scope();
        let events = Events::install(&ownership);
        let drops = Arc::new(AtomicUsize::new(0));
        let payload = Box::new(UniqueOutput { value: "synthetic-panic", drops: drops.clone() });
        let address = (&*payload as *const UniqueOutput) as usize;
        let (entered, started) = oneshot::channel();
        let (release, held) = oneshot::channel();
        let (handle, completion) = spawn_observed::<(), _>(Some(&ownership),
            OperationKind::Inference, async move {
                entered.send(()).unwrap();
                held.await.unwrap();
                std::panic::panic_any(payload);
            }).unwrap();
        let abort = handle.abort_handle();
        tokio::time::timeout(WAIT, started).await.unwrap().unwrap();
        if cancel { abort.abort(); drop(release); } else { release.send(()).unwrap(); }
        let (result, confirmed) = tokio::time::timeout(WAIT, async {
            tokio::join!(handle, completion.0)
        }).await.unwrap();
        assert!(!confirmed);
        let error = result.unwrap_err();
        assert_eq!(error.id(), abort.id());
        if cancel {
            assert!(error.is_cancelled());
        } else {
            assert!(error.is_panic());
            let payload = *error.into_panic().downcast::<Box<UniqueOutput>>()
                .unwrap_or_else(|_| panic!("original panic payload replaced"));
            assert_eq!((&*payload as *const UniqueOutput) as usize, address);
            assert_eq!(payload.value, "synthetic-panic");
            assert_eq!(drops.load(Ordering::SeqCst), 0);
            drop(payload);
        }
        assert_eq!(drops.load(Ordering::SeqCst), 1);
        let values = events.snapshot();
        assert_join(&values, values[0]["operation_id"].as_str().unwrap(),
            if cancel { "cancelled" } else { "panicked" }, false);
        assert_eq!(ownership.gate.stats().unwrap().active, 1);
        assert!(matches!(ownership.seal(), Err(GateError::Unconfirmed)));
    }
}

#[tokio::test]
async fn unknown_cleanup_keeps_error_and_disconnected_output_fenced() {
    for messages in [true, false] {
        for disconnect in [false, true] {
            let mut f = fixture::Fixture::new();
            f.state = with_owned_operations(f.state, 32).unwrap();
            let ownership = f.state.operation_ownership.clone().unwrap();
            let mut events = Events::install(&ownership);
            let app = crate::router(f.state.clone(), "/nonexistent-fixture-web");
            let mut request = stream_request(&f, 0, messages);
            request.headers_mut().remove("x-framerize-account");
            let response = tokio::time::timeout(WAIT, app.oneshot(request))
                .await.unwrap().unwrap();
            let root = root_id(&response);
            let first = f.next().await;
            assert_eq!(first.account, f.accounts[0].id);
            first.reply.send(Err("host-lifecycle-unconfirmed: synthetic first call".into())).ok().unwrap();
            tokio::time::timeout(WAIT, first.dropped).await.unwrap().unwrap();
            let child = child_id(&events.snapshot(), &root);
            let completion = response.extensions().get::<ResponseCompletion>().unwrap().clone();
            let mut response = Some(response);
            if disconnect { drop(response.take()); }
            if let Some(response) = response {
                let body = tokio::time::timeout(WAIT, to_bytes(response.into_body(), 65536)).await.unwrap().unwrap();
                assert!(String::from_utf8_lossy(&body).contains("synthetic first call"));
            }
            assert!(!tokio::time::timeout(WAIT, completion.0).await.unwrap());
            events.event(&root, "operation.unresolved").await;
            let values = events.snapshot();
            assert_join(&values, &child, "error", false);
            assert_eq!(operation_events(&values, &root).len(), 2);
            assert!(!values.iter().any(|event| event["event"] == "operation.released"));
            assert!(matches!(ownership.seal(), Err(GateError::Unconfirmed)));
            assert_eq!(ownership.gate.stats().unwrap().active, 1);
            let snapshot = f.state.serving_accounts.residents.snapshot().unwrap();
            assert!(snapshot.residents.iter().any(|slot|
                slot.account == first.account && slot.uncertain && slot.users == 0));
            assert_eq!(snapshot.residents.len(), 1);
            assert_eq!(f.state.response_output.counts(), (0, 0, 0));
            f.no_call();
        }
    }
}

#[tokio::test]
async fn failed_completion_lock_observes_join_without_reporting_release() {
    let ownership = scope();
    let events = Events::install(&ownership);
    let (send_owner, receive_owner) = oneshot::channel();
    let (release, held) = oneshot::channel();
    let (handle, completion) = spawn_observed(Some(&ownership), OperationKind::Inference, async move {
        send_owner.send(CURRENT.with(Arc::clone)).ok().unwrap();
        held.await.unwrap();
        (Ok("unchanged successful result"), true)
    }).unwrap();
    let owner = tokio::time::timeout(WAIT, receive_owner).await.unwrap().unwrap();
    let poison = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _guard = owner.owner.lock().unwrap();
        panic!("synthetic completion lock failure");
    }));
    assert!(poison.is_err());
    release.send(()).unwrap();
    let (result, confirmed) = tokio::time::timeout(WAIT, async {
        tokio::join!(handle, completion.0)
    }).await.unwrap();
    assert_eq!(result.unwrap().unwrap(), "unchanged successful result");
    assert!(!confirmed);
    let before_drop = events.snapshot();
    assert_eq!(before_drop.len(), 2);
    assert_eq!(before_drop[1]["event"], "operation.joined");
    assert_eq!(before_drop[1]["cleanup_confirmed"], true);
    assert_eq!(ownership.gate.stats().unwrap().active, 2);
    drop(owner);
    let values = events.snapshot();
    assert_schema(&values);
    assert_eq!(values.len(), 3);
    assert_eq!(values[2]["event"], "operation.unresolved");
    assert_eq!(values[2]["outcome"], "dropped");
    assert_eq!(values[2]["cleanup_confirmed"], false);
    assert!(!values.iter().any(|event| event["event"] == "operation.released"));
    assert_eq!(ownership.gate.stats().unwrap().active, 1);
    assert!(matches!(ownership.seal(), Err(GateError::Unconfirmed)));
}

#[tokio::test]
async fn concurrent_roots_keep_original_children_and_release_evidence_separate() {
    let mut f = fixture::Fixture::new();
    f.state = with_owned_operations(f.state, 32).unwrap();
    let ownership = f.state.operation_ownership.clone().unwrap();
    let mut events = Events::install(&ownership);
    let app = crate::router(f.state.clone(), "/nonexistent-fixture-web");
    let (a, b) = tokio::time::timeout(WAIT, async {
        tokio::join!(app.clone().oneshot(stream_request(&f, 0, false)),
            app.oneshot(stream_request(&f, 1, false)))
    }).await.unwrap();
    let (a, b) = (a.unwrap(), b.unwrap());
    let (root_a, root_b) = (root_id(&a), root_id(&b));
    assert_ne!(root_a, root_b);
    let (first, second) = (f.next().await, f.next().await);
    let (call_a, call_b) = if first.account == f.accounts[0].id {
        (first, second)
    } else { (second, first) };
    assert_eq!(call_a.account, f.accounts[0].id);
    assert_eq!(call_b.account, f.accounts[1].id);
    let child_a = child_id(&events.snapshot(), &root_a);
    let child_b = child_id(&events.snapshot(), &root_b);
    assert_eq!([&root_a, &root_b, &child_a, &child_b].into_iter().collect::<BTreeSet<_>>().len(), 4);
    assert_eq!(ownership.gate.stats().unwrap().active, 5);
    let completion_a = a.extensions().get::<ResponseCompletion>().unwrap().clone();
    drop(a);
    assert!(call_a.config.as_ref().unwrap().is_cancelled());
    call_b.reply.send(Ok(fixture::success(call_b.kind))).ok().unwrap();
    tokio::time::timeout(WAIT, call_b.dropped).await.unwrap().unwrap();
    let body = tokio::time::timeout(WAIT, to_bytes(b.into_body(), 65536)).await.unwrap().unwrap();
    assert_output(&body, false);
    events.event(&root_b, "operation.released").await;
    assert_join(&events.snapshot(), &child_b, "ok", true);
    assert_eq!(operation_events(&events.snapshot(), &child_a).len(), 1);
    assert_eq!(operation_events(&events.snapshot(), &root_a).len(), 1);
    assert!(!call_a.reply.is_closed());
    assert_eq!(ownership.gate.stats().unwrap().active, 3);
    call_a.reply.send(Ok(fixture::success(call_a.kind))).ok().unwrap();
    tokio::time::timeout(WAIT, call_a.dropped).await.unwrap().unwrap();
    assert!(tokio::time::timeout(WAIT, completion_a.0).await.unwrap());
    events.event(&root_a, "operation.released").await;
    assert_join(&events.snapshot(), &child_a, "error", true);
    assert_eq!(ownership.gate.stats().unwrap().active, 1);
    assert!(!ownership.gate.stats().unwrap().unconfirmed);
    assert_eq!(f.state.response_output.counts(), (0, 0, 0));
    for event in events.snapshot() {
        let id = event["operation_id"].as_str().unwrap();
        assert!([&root_a, &root_b, &child_a, &child_b].iter().any(|known| known.as_str() == id));
        let expected = if id == root_a || id == child_a { &root_a } else { &root_b };
        assert_eq!(event["request_id"], expected.as_str());
        if id == root_a || id == root_b {
            assert!(event["parent_operation_id"].is_null());
        } else {
            assert_eq!(event["parent_operation_id"], expected.as_str());
        }
    }
    f.no_call();
}

#[tokio::test]
async fn panicking_observer_runs_outside_locks_and_preserves_completion() {
    let ownership = scope();
    let values = Arc::new(Mutex::new(Vec::<Value>::new()));
    let checks = Arc::new(Mutex::new(Vec::new()));
    let observed_owner = Arc::new(Mutex::new(None::<std::sync::Weak<OwnerContext>>));
    let (v, c, slot) = (values.clone(), checks.clone(), observed_owner.clone());
    let weak = Arc::downgrade(&ownership);
    *ownership.observation_sink.lock().unwrap() = Some(Arc::new(move |event| {
        let released = event["event"] == "operation.released";
        v.lock().unwrap().push(event);
        let scope = weak.upgrade().unwrap();
        let sink_unlocked = scope.observation_sink.try_lock().is_ok();
        let owner_unlocked = slot.lock().unwrap().as_ref().and_then(|owner| owner.upgrade())
            .map(|owner| owner.owner.try_lock().is_ok()).unwrap_or(true);
        let registration_removed = !released || scope.gate.stats().unwrap().active == 1;
        c.lock().unwrap().push((sink_unlocked, owner_unlocked, registration_removed));
        panic!("synthetic observation sink failure");
    }));
    let (send_owner, receive_owner) = oneshot::channel();
    let (release, held) = oneshot::channel();
    let (handle, completion) = spawn_observed(Some(&ownership), OperationKind::Inference, async move {
        send_owner.send(CURRENT.with(Arc::downgrade)).ok().unwrap();
        held.await.unwrap();
        (Ok("original output"), true)
    }).unwrap();
    let weak_owner = tokio::time::timeout(WAIT, receive_owner).await.unwrap().unwrap();
    *observed_owner.lock().unwrap() = Some(weak_owner);
    release.send(()).unwrap();
    let (result, confirmed) = tokio::time::timeout(WAIT, async {
        tokio::join!(handle, completion.0)
    }).await.unwrap();
    assert_eq!(result.unwrap().unwrap(), "original output");
    assert!(confirmed);
    assert_eq!(*checks.lock().unwrap(), vec![(true, true, true); 3]);
    let events = values.lock().unwrap().clone();
    assert_join(&events, events[0]["operation_id"].as_str().unwrap(), "ok", true);
    assert!(!ownership.observation_sink.is_poisoned());
    assert_eq!(ownership.gate.stats().unwrap().active, 1);
    assert!(!ownership.gate.stats().unwrap().unconfirmed);
}

#[tokio::test]
async fn poisoned_observation_sink_does_not_change_join_or_release() {
    let ownership = scope();
    let poisoned = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _guard = ownership.observation_sink.lock().unwrap();
        panic!("synthetic sink lock failure");
    }));
    assert!(poisoned.is_err());
    let (handle, completion) = spawn_observed(Some(&ownership), OperationKind::Inference,
        async { (Ok("unchanged output"), true) }).unwrap();
    let (result, confirmed) = tokio::time::timeout(WAIT, async {
        tokio::join!(handle, completion.0)
    }).await.unwrap();
    assert_eq!(result.unwrap().unwrap(), "unchanged output");
    assert!(confirmed);
    assert_eq!(ownership.gate.stats().unwrap().active, 1);
    assert!(!ownership.gate.stats().unwrap().unconfirmed);
}

#[tokio::test]
async fn generated_ids_do_not_reflect_inputs_and_unowned_routes_stay_unchanged() {
    const CLIENT_ID: &str = "00000000-0000-4000-8000-000000000099";
    for owned in [false, true] {
        let ownership = scope();
        let events = Events::install(&ownership);
        let state = owned.then(|| ownership.clone());
        let app = Router::new().route("/probe", post(|| async { "static result" }))
            .route("/api/health", axum::routing::get(|| async { "healthy" }))
            .layer(axum::middleware::from_fn_with_state(state, middleware));
        let request = HttpRequest::builder().method("POST").uri("/probe")
            .header("authorization", "Bearer synthetic-auth-secret")
            .header("cookie", "session=synthetic-cookie-secret")
            .header("x-request-id", CLIENT_ID).header("x-framerize-operation-id", CLIENT_ID)
            .body(Body::from("synthetic-body-secret")).unwrap();
        let response = app.clone().oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let root = if owned {
            let id = root_id(&response);
            assert_ne!(id, CLIENT_ID);
            Some(id)
        } else {
            assert!(!response.headers().contains_key("x-framerize-operation-id"));
            None
        };
        assert_eq!(to_bytes(response.into_body(), 1024).await.unwrap().as_ref(), b"static result");
        let values = events.snapshot();
        assert_schema(&values);
        if let Some(root) = root {
            assert_eq!(values.len(), 2);
            assert!(values.iter().all(|event| event["request_id"] == root));
            assert_eq!(values[1]["event"], "operation.released");
        } else { assert!(values.is_empty()); }
        let serialized = serde_json::to_string(&values).unwrap();
        for forbidden in [CLIENT_ID, "synthetic-auth-secret", "synthetic-cookie-secret", "synthetic-body-secret"] {
            assert!(!serialized.contains(forbidden));
        }
        let health = app.oneshot(HttpRequest::builder().uri("/api/health").body(Body::empty()).unwrap())
            .await.unwrap();
        assert!(!health.headers().contains_key("x-framerize-operation-id"));
        assert_eq!(to_bytes(health.into_body(), 1024).await.unwrap().as_ref(), b"healthy");
        assert_eq!(events.snapshot(), values);
        assert_eq!(ownership.gate.stats().unwrap().active, 1);
        assert!(!ownership.gate.stats().unwrap().unconfirmed);
    }
}

#[tokio::test]
async fn final_error_classification_preserves_original_error_and_cleanup() {
    for (text, accumulated, clean) in [("synthetic plain error", true, true),
        ("host-lifecycle-unconfirmed: synthetic error", true, false),
        ("host-lifecycle-unconfirmed: synthetic error; Authorization: Bearer private-fixture", true, false),
        ("synthetic plain error", false, false)] {
        let ownership = scope();
        let events = Events::install(&ownership);
        let error = text.to_owned();
        let address = error.as_ptr() as usize;
        let (handle, completion) = spawn_observed::<(), _>(Some(&ownership),
            OperationKind::Inference, async move { (Err(error), accumulated) }).unwrap();
        let (result, confirmed) = tokio::time::timeout(WAIT, async {
            tokio::join!(handle, completion.0)
        }).await.unwrap();
        let error = result.unwrap().unwrap_err();
        assert_eq!(error.as_ptr() as usize, address);
        assert_eq!(error, text);
        assert_eq!(confirmed, clean);
        let values = events.snapshot();
        assert_join(&values, values[0]["operation_id"].as_str().unwrap(), "error", clean);
        let joined = values.iter().find(|event| event["event"] == "operation.joined").unwrap();
        if clean {
            assert!(joined["cause"].is_null());
        } else if text.starts_with("host-lifecycle-unconfirmed:") {
            assert!(joined["cause"].as_str().unwrap().contains("synthetic error"));
        } else {
            assert!(joined["cause"].as_str().unwrap().contains("incomplete cleanup"));
        }
        assert!(!serde_json::to_string(&values).unwrap().contains("private-fixture"));
        assert_eq!(ownership.gate.stats().unwrap().active, 1);
        assert_eq!(ownership.gate.stats().unwrap().unconfirmed, !clean);
    }
}

#[tokio::test]
async fn unowned_join_preserves_default_result_without_registration() {
    let ownership = scope();
    let events = Events::install(&ownership);
    let (handle, completion) = spawn_observed(None, OperationKind::Inference, async {
        assert!(CURRENT.try_with(|_| ()).is_err());
        (Ok("unowned original result"), true)
    }).unwrap();
    let (result, confirmed) = tokio::time::timeout(WAIT, async {
        tokio::join!(handle, completion.0)
    }).await.unwrap();
    assert_eq!(result.unwrap().unwrap(), "unowned original result");
    assert!(confirmed);
    assert!(events.snapshot().is_empty());
    assert_eq!(ownership.gate.stats().unwrap().active, 1);
}

impl Events {
    fn install(ownership: &Arc<ServingOwnership>) -> Self {
        let values = Arc::new(Mutex::new(Vec::new()));
        let (send, changed) = tokio::sync::mpsc::unbounded_channel();
        let recorded = values.clone();
        *ownership.observation_sink.lock().unwrap() = Some(Arc::new(move |event| {
            recorded.lock().unwrap().push(event);
            let _ = send.send(());
        }));
        Self { values, changed }
    }
    fn snapshot(&self) -> Vec<Value> { self.values.lock().unwrap().clone() }
    async fn event(&mut self, operation: &str, name: &str) -> Value {
        tokio::time::timeout(WAIT, async {
            loop {
                if let Some(event) = self.snapshot().into_iter().find(|event|
                    event["operation_id"] == operation && event["event"] == name) {
                    return event;
                }
                self.changed.recv().await.expect("observation sink closed");
            }
        }).await.expect("expected lifecycle event missing")
    }
}

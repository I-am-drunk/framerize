use super::*;
use axum::{body::Body, http::{Request, StatusCode}};
use crate::operation_ownership::ServingOwnership;
use framer_core::operation_gate::GateError;
use http_body::Body as _;
use tower::ServiceExt;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RepickClose { Error, Unconfirmed, BeforeCloseBusy }

#[tokio::test]
async fn cooldown_refusal_skips_an_unconfirmed_peer_and_uses_the_next_account() {
    for stream in [false, true] {
        let (mut state, ports) = fixture(3, 3);
        activated(&state, 1).await;
        // The host knows B, but the pool must still validate it before use.
        ports.live.lock().unwrap().insert(id(2), 7);
        *ports.status_mode.lock().unwrap() = StatusMode::UnconfirmedFor(id(2));
        *ports.inference_error.lock().unwrap() = Some("account-cooling:600".into());
        ports.fail_first_account.store(true, Ordering::SeqCst);
        ports.request_gate.add_permits(2);
        let (scope, gate) = ServingOwnership::new_with_host_factory(
            state.wave.clone(), 32, |gate| gate).unwrap();
        state.runtime_mut().expect("unique fixture runtime").operation_ownership = Some(scope);
        let app = crate::router(state.clone(), "/fixture-no-web");
        let response = app.clone().oneshot(repick_request(Route::Native, stream)).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), 65536).await.unwrap();
        assert!(String::from_utf8_lossy(&body).contains("fixture result"));
        until(|| gate.stats().unwrap().active == 1).await;
        assert!(!gate.stats().unwrap().unconfirmed, "account B must not poison global admission");
        assert!(state.serving_accounts.residents.snapshot().unwrap().residents.iter()
            .any(|resident| resident.account == id(2) && resident.uncertain));
        assert_eq!(ports.request_started.load(Ordering::SeqCst), 2);
        assert!(!ports.events.lock().unwrap().iter().any(|e| e.0 == "close_started"));
        let mut unavailable = repick_request(Route::Native, false);
        unavailable.headers_mut().insert("x-framerize-account", id(2).0.to_string().parse().unwrap());
        let response = app.oneshot(unavailable).await.unwrap();
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        to_bytes(response.into_body(), 65536).await.unwrap();
        assert!(slot(&state, id(2)).uncertain);
        assert_eq!(ports.request_started.load(Ordering::SeqCst), 2);
        assert!(!ports.mint_routes.lock().unwrap().iter().any(|(account, _)| *account == id(2)));
        until(|| gate.stats().unwrap().active == 1).await;
        assert!(!gate.stats().unwrap().unconfirmed);
    }
}

#[tokio::test]
async fn refusal_does_not_start_an_eviction_that_would_fail() {
    check_repick(RepickClose::Error).await;
}

#[tokio::test]
async fn repick_exhausts_clean_refusals_but_stops_on_unconfirmed_cleanup() {
    for (mode, confirmed, probes) in [(CloseMode::BusyResident, true, 2),
        (CloseMode::Unconfirmed, false, 1)] {
        let (state, ports) = fixture(1, 3);
        activated(&state, 1).await;
        *ports.close_mode.lock().unwrap() = mode;
        let outcome = tokio::time::timeout(Duration::from_secs(2),
            crate::pick_fresh_account(&state, &[id(1).0.to_string()]))
            .await.expect("replacement selection must terminate");
        assert!(outcome.result.is_err());
        assert_eq!(outcome.cleanup_confirmed, confirmed);
        assert_eq!(ports.events.lock().unwrap().iter()
            .filter(|event| **event == ("close_started", id(1))).count(), probes);
        assert_eq!(ports.request_started.load(Ordering::SeqCst), 0);
        assert_eq!(slot(&state, id(1)).uncertain, !confirmed);
        assert_eq!(ports.live.lock().unwrap().len(), 1);
    }
}

#[tokio::test]
async fn refusal_stops_when_peer_eviction_is_unconfirmed() {
    check_repick(RepickClose::Unconfirmed).await;
}

#[tokio::test]
async fn refusal_keeps_the_original_busy_browser_without_fencing() {
    check_repick(RepickClose::BeforeCloseBusy).await;
}

async fn check_repick(mode: RepickClose) {
    for route in [Route::Free, Route::Native] {
        for stream in [false, true] {
            for consume in [false, true] {
                no_replay_case(mode, route, stream, consume).await;
            }
        }
    }
}

async fn no_replay_case(mode: RepickClose, route: Route, stream: bool, consume: bool) {
    let (mut state, ports) = fixture(1, 2);
    activated(&state, 1).await;
    let (ownership, gate) = ServingOwnership::new_with_host_factory(state.wave.clone(), 32, |gate| gate).unwrap();
    state.runtime_mut().unwrap().operation_ownership = Some(ownership.clone());
    *ports.close_mode.lock().unwrap() = match mode {
        RepickClose::Error => CloseMode::Error,
        RepickClose::Unconfirmed => CloseMode::Unconfirmed,
        RepickClose::BeforeCloseBusy => CloseMode::BusyResident,
    };
    ports.fail_first_account.store(true, Ordering::SeqCst);
    ports.gate_close.store(true, Ordering::SeqCst);
    let peer_before = serde_json::to_value(&ports.records.lock().unwrap()[1]).unwrap();
    let app = crate::router(state.clone(), "/fixture-no-web");
    let task = tokio::spawn(app.oneshot(repick_request(route, stream)));
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if ports.events.lock().unwrap().iter().any(|event| event.0 == "close_started") { break; }
            tokio::task::yield_now().await;
        }
    }).await.expect("peer retry did not start its required bounded eviction");
    ports.close_gate.add_permits(1);
    let response = tokio::time::timeout(Duration::from_secs(2), task).await.unwrap().unwrap().unwrap();
    assert_eq!(response.status().as_u16(), if stream { 200 } else { 503 });
    if consume {
        let body = to_bytes(response.into_body(), 65536).await.unwrap();
        assert_original_error(&body, route, stream);
    } else { drop(response); }
    until(|| gate.stats().unwrap().active == 1).await;
    assert_eq!(gate.stats().unwrap().unconfirmed, mode != RepickClose::BeforeCloseBusy);
    assert_eq!(slot(&state, id(1)).users, 0);
    assert_eq!(serde_json::to_value(&ports.records.lock().unwrap()[1]).unwrap(), peer_before);
    assert_single_attempt(&ports);
    assert_eq!(ports.events.lock().unwrap().iter()
        .filter(|event| event.0 == "close_started").count(), 1);
}

fn assert_single_attempt(ports: &Ports) {
    assert_eq!(ports.request_started.load(Ordering::SeqCst), 1);
    assert_eq!(ports.request_drops.load(Ordering::SeqCst), 1);
    assert_eq!(ports.request_finished.load(Ordering::SeqCst), 0);
    assert_eq!(ports.request_steps.load(Ordering::SeqCst), 0);
    assert_eq!(*ports.observed_chats.lock().unwrap(),
        vec![(id(1), "repick-cleanup-fixture".to_owned())]);
    let minted: Vec<_> = ports.mint_routes.lock().unwrap().iter()
        .map(|(account, _)| *account).collect();
    assert_eq!(minted, vec![id(1), id(1)]); // Activation and the one inference.
    assert_eq!(ports.max_live.load(Ordering::SeqCst), 1);
    let live = ports.live.lock().unwrap();
    assert_eq!(live.len(), 1);
    assert!(live.contains_key(&id(1)));
}

fn repick_request(route: Route, stream: bool) -> Request<Body> {
    let (path, payload) = match route {
        Route::Free => ("/anthropic/v1/messages", json!({"model":"claude-opus-5-5",
            "stream":stream,"messages":[{"role":"user","content":"fixture"}]})),
        Route::Native => ("/openai/v1/responses", json!({"model":"gpt-6-astra",
            "stream":stream,"input":"fixture"})),
        Route::Compat => panic!("paid routes are outside this fixture"),
    };
    Request::builder().method("POST").uri(path)
        .header("content-type", "application/json")
        .header("x-framerize-session", "repick-cleanup-fixture")
        .body(Body::from(payload.to_string())).unwrap()
}

fn assert_original_error(body: &[u8], route: Route, stream: bool) {
    let error = if !stream {
        let value: Value = serde_json::from_slice(body).unwrap();
        value["error"].clone()
    } else {
        let text = std::str::from_utf8(body).unwrap();
        let events: Vec<Value> = text.lines().filter_map(|line| line.strip_prefix("data: "))
            .map(|data| serde_json::from_str(data).unwrap()).collect();
        let kind = if matches!(route, Route::Free) { "error" } else { "response.failed" };
        let errors: Vec<_> = events.iter().filter(|event| event["type"] == kind).collect();
        assert_eq!(errors.len(), 1);
        assert!(!events.iter().any(|event| matches!(event["type"].as_str(),
            Some("message_stop" | "response.completed" | "content_block_delta"
                | "response.output_text.delta"))));
        if matches!(route, Route::Free) { errors[0]["error"].clone() }
        else {
            assert_eq!(errors[0]["response"]["status"], "failed");
            errors[0]["response"]["error"].clone()
        }
    };
    assert_eq!(error["message"], "Framer refused the request with HTTP 403 during its security checks. Its response did not disclose a more specific cause.");
    assert_eq!(error["type"], "overloaded_error");
}

#[tokio::test]
async fn initial_failed_close_preserves_refusal_and_fences_admission() {
    check_initial_admission(RepickClose::Error).await;
}

#[tokio::test]
async fn initial_unconfirmed_close_preserves_refusal_and_fences_admission() {
    check_initial_admission(RepickClose::Unconfirmed).await;
}

#[tokio::test]
async fn initial_busy_before_close_preserves_refusal_without_fencing() {
    check_initial_admission(RepickClose::BeforeCloseBusy).await;
}

async fn check_initial_admission(mode: RepickClose) {
    for route in [Route::Free, Route::Native] {
        for stream in [false, true] {
            for consume in [false, true] {
                initial_admission_case(mode, route, stream, consume).await;
            }
        }
    }
}

fn assert_no_initial_inference(ports: &Ports) {
    assert_eq!(ports.request_started.load(Ordering::SeqCst), 0);
    assert_eq!(ports.request_drops.load(Ordering::SeqCst), 0);
    assert_eq!(ports.request_finished.load(Ordering::SeqCst), 0);
    assert_eq!(ports.request_steps.load(Ordering::SeqCst), 0);
    assert!(ports.observed_chats.lock().unwrap().is_empty());
    assert_eq!(ports.account_writes.load(Ordering::SeqCst), 0);
    assert_eq!(ports.setting_writes.load(Ordering::SeqCst), 0);
    assert_eq!(ports.max_live.load(Ordering::SeqCst), 1);
    let live = ports.live.lock().unwrap();
    assert_eq!(live.len(), 1);
    assert!(live.contains_key(&id(1)));
}

fn assert_initial_refusal(body: &[u8], mode: RepickClose) {
    let message = match mode {
        RepickClose::Error => "fixture close failure",
        RepickClose::Unconfirmed => "the browser reported state Absent with 0 active commands; browser close returned Unconfirmed and its occupied slot is retained",
        RepickClose::BeforeCloseBusy => "account browser is still busy or its identity changed",
    };
    let value: Value = serde_json::from_slice(body).unwrap();
    assert_eq!(value, json!({"error":{"code":"framerize_pool_busy",
        "message":message,"retry_after_seconds":1}}));
}

fn assert_initial_stream_refusal(body: &[u8], mode: RepickClose) {
    let wire = std::str::from_utf8(body).unwrap();
    let errors: Vec<Value> = wire.lines().filter_map(|line| line.strip_prefix("data: "))
        .map(|json| serde_json::from_str::<Value>(json).unwrap())
        .filter(|value| value["type"] == "error").collect();
    assert_eq!(errors.len(), 1);
    assert!(!wire.contains("event: message_stop"));
    let cause = match mode {
        RepickClose::Error => "fixture close failure",
        RepickClose::Unconfirmed => "browser close returned Unconfirmed",
        RepickClose::BeforeCloseBusy => "account browser is still busy or its identity changed",
    };
    assert!(errors[0]["error"]["message"].as_str().unwrap().contains(cause));
}

async fn initial_admission_case(mode: RepickClose, route: Route, stream: bool, consume: bool) {
    let deferred = stream && matches!(route, Route::Free);
    let (mut state, ports) = fixture(1, 2);
    activated(&state, 1).await;
    let before = records(&ports);
    let initial_mints = ports.mint_routes.lock().unwrap().clone();
    assert_eq!(initial_mints.len(), 1);
    assert_eq!(initial_mints[0].0, id(1));
    let (ownership, gate) = ServingOwnership::new_with_host_factory(
        state.wave.clone(), 32, |gate| gate).unwrap();
    state.runtime_mut().expect("unique fixture runtime").operation_ownership = Some(ownership.clone());
    let uncertain = !matches!(mode, RepickClose::BeforeCloseBusy);
    *ports.close_mode.lock().unwrap() = match mode {
        RepickClose::Error => CloseMode::Error,
        RepickClose::Unconfirmed => CloseMode::Unconfirmed,
        RepickClose::BeforeCloseBusy => CloseMode::Confirm,
    };
    if !uncertain { *ports.status_mode.lock().unwrap() = StatusMode::Busy; }
    ports.gate_close.store(uncertain, Ordering::SeqCst);
    let app = crate::router(state.clone(), "/fixture-no-web");
    let mut request = repick_request(route, stream);
    request.headers_mut().insert("x-framerize-account", id(2).0.to_string().parse().unwrap());
    let task = tokio::spawn(app.clone().oneshot(request));
    if uncertain {
        until(|| ports.events.lock().unwrap().contains(&("close_started", id(1)))).await;
        let resident = slot(&state, id(1));
        assert!(resident.close_started);
        assert_eq!(resident.replacement, Some(id(2)));
        if !deferred { assert!(!task.is_finished()); }
        assert!(gate.stats().unwrap().active > 1);
        assert_no_initial_inference(&ports);
        assert_eq!(*ports.mint_routes.lock().unwrap(), initial_mints);
        ports.close_gate.add_permits(1);
    }
    let response = tokio::time::timeout(Duration::from_secs(2), task)
        .await.unwrap().unwrap().unwrap();
    if deferred {
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()["content-type"], "text/event-stream");
        until(|| ports.events.lock().unwrap().len() >= 3).await;
    } else {
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE,
            "{mode:?} {route:?} stream={stream} consume={consume}");
        assert_eq!(response.headers()["retry-after"], "1");
        assert_eq!(response.headers()["content-type"], "application/json");
    }
    assert!(!response.body().is_end_stream(), "exercise an unread refusal body");
    if consume {
        let body = tokio::time::timeout(Duration::from_secs(2),
            to_bytes(response.into_body(), 4096)).await.unwrap().unwrap();
        if deferred { assert_initial_stream_refusal(&body, mode); }
        else { assert_initial_refusal(&body, mode); }
    } else { drop(response); }
    until(|| gate.stats().unwrap().active == 1).await;
    assert_eq!(gate.stats().unwrap().unconfirmed, uncertain,
        "{mode:?} {route:?} stream={stream} consume={consume}");
    let snapshot = state.serving_accounts.residents.snapshot().unwrap();
    assert_eq!(snapshot.occupied, 1);
    assert_eq!(snapshot.residents.len(), 1);
    let resident = &snapshot.residents[0];
    assert_eq!(resident.account, id(1));
    assert_eq!(resident.users, 0);
    assert_eq!(resident.uncertain, uncertain);
    assert_eq!(resident.replacement, None);
    assert_eq!(records(&ports), before);
    assert!(state.wave.check().is_none());
    assert_no_initial_inference(&ports);
    assert_eq!(*ports.mint_routes.lock().unwrap(), initial_mints);
    let mut expected_events = vec![("status", id(1)), ("mint", id(1)), ("status", id(1))];
    if uncertain { expected_events.push(("close_started", id(1))); }
    assert_eq!(*ports.events.lock().unwrap(), expected_events);
    if uncertain { assert!(matches!(ownership.seal(), Err(GateError::Unconfirmed))); }
    else {
        let mut ticket = ownership.seal().unwrap();
        let proof = tokio::time::timeout(Duration::from_secs(2), ticket.wait_application())
            .await.unwrap().unwrap();
        ownership.check_application_drained(&proof).unwrap();
        drop(proof);
        ticket.cancel_before_control().unwrap();
    }
    let probe = Request::builder().uri("/api/hello").body(Body::empty()).unwrap();
    let response = tokio::time::timeout(Duration::from_secs(2), app.oneshot(probe))
        .await.unwrap().unwrap();
    assert_eq!(response.status(), if uncertain { StatusCode::SERVICE_UNAVAILABLE }
        else { StatusCode::OK });
    let body = to_bytes(response.into_body(), 1024).await.unwrap();
    let expected = if uncertain { format!("Cannot start this request: {}.", gate.error_explanation(GateError::Unconfirmed)) } else { "ok".into() };
    assert_eq!(body.as_ref(), expected.as_bytes());
    assert_eq!(gate.stats().unwrap().active, 1); // The unmanaged host stays retained.
    assert_eq!(gate.stats().unwrap().unconfirmed, uncertain);
    assert_eq!(records(&ports), before);
    assert_eq!(*ports.events.lock().unwrap(), expected_events);
    assert_eq!(*ports.mint_routes.lock().unwrap(), initial_mints);
    assert_no_initial_inference(&ports);
}

#[tokio::test]
async fn api_run_failed_acquisition_preserves_refusal_and_fences_admission() {
    for consume in [false, true] {
        api_run_acquisition_case(RepickClose::Error, consume).await;
    }
}

#[tokio::test]
async fn api_run_unconfirmed_acquisition_preserves_refusal_and_fences_admission() {
    for consume in [false, true] {
        api_run_acquisition_case(RepickClose::Unconfirmed, consume).await;
    }
}

#[tokio::test]
async fn api_run_busy_before_close_preserves_refusal_without_fencing() {
    for consume in [false, true] {
        api_run_acquisition_case(RepickClose::BeforeCloseBusy, consume).await;
    }
}

async fn api_run_acquisition_case(mode: RepickClose, consume: bool) {
    let (mut state, ports) = fixture(1, 2);
    activated(&state, 1).await;
    let before = records(&ports);
    let initial_mints = ports.mint_routes.lock().unwrap().clone();
    assert_eq!(initial_mints, vec![(id(1), None)]);
    let (ownership, gate) = ServingOwnership::new_with_host_factory(
        state.wave.clone(), 32, |gate| gate).unwrap();
    state.runtime_mut().expect("unique fixture runtime").operation_ownership = Some(ownership.clone());
    let uncertain = !matches!(mode, RepickClose::BeforeCloseBusy);
    *ports.close_mode.lock().unwrap() = match mode {
        RepickClose::Error => CloseMode::Error,
        RepickClose::Unconfirmed => CloseMode::Unconfirmed,
        RepickClose::BeforeCloseBusy => CloseMode::Confirm,
    };
    if !uncertain { *ports.status_mode.lock().unwrap() = StatusMode::Busy; }
    ports.gate_close.store(uncertain, Ordering::SeqCst);
    let app = crate::router(state.clone(), "/fixture-no-web");
    let request = Request::builder().method("POST").uri("/api/run")
        .header("content-type", "application/json")
        .body(Body::from(json!({"account_id":id(2), "prompt":"inert acquisition",
            "model":"gpt-6-astra", "chat_session":"inert-api-run"}).to_string())).unwrap();
    let task = tokio::spawn(app.clone().oneshot(request));
    if uncertain {
        until(|| ports.events.lock().unwrap().contains(&("close_started", id(1)))).await;
        let resident = slot(&state, id(1));
        assert!(resident.close_started);
        assert_eq!(resident.replacement, Some(id(2)));
        assert!(!task.is_finished());
        assert!(gate.stats().unwrap().active > 1);
        assert_no_initial_inference(&ports);
        assert_eq!(*ports.mint_routes.lock().unwrap(), initial_mints);
        ports.close_gate.add_permits(1);
    }
    let response = tokio::time::timeout(Duration::from_secs(2), task)
        .await.unwrap().unwrap().unwrap();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE, "{mode:?} {consume}");
    assert_eq!(response.headers()["retry-after"], "1");
    assert_eq!(response.headers()["content-type"], "application/json");
    assert!(!response.body().is_end_stream());
    if consume {
        let body = to_bytes(response.into_body(), 4096).await.unwrap();
        assert_initial_refusal(&body, mode);
    } else { drop(response); }
    until(|| gate.stats().unwrap().active == 1).await;
    assert_eq!(gate.stats().unwrap().unconfirmed, uncertain, "{mode:?} {consume}");
    let snapshot = state.serving_accounts.residents.snapshot().unwrap();
    assert_eq!(snapshot.occupied, 1);
    assert_eq!(snapshot.residents.len(), 1);
    let resident = &snapshot.residents[0];
    assert_eq!(resident.account, id(1));
    assert_eq!(resident.users, 0);
    assert_eq!(resident.uncertain, uncertain);
    assert_eq!(resident.replacement, None);
    assert_eq!(records(&ports), before); // Includes both requested B and retained A.
    assert_eq!(resident.status, if uncertain {
        framer_core::resident_pool::ResidentStatus::Uncertain
    } else { framer_core::resident_pool::ResidentStatus::Idle });
    assert!(state.wave.check().is_none());
    assert_no_initial_inference(&ports);
    assert_eq!(*ports.mint_routes.lock().unwrap(), initial_mints);
    let mut expected_events = vec![("status", id(1)), ("mint", id(1)), ("status", id(1))];
    if uncertain { expected_events.push(("close_started", id(1))); }
    assert_eq!(*ports.events.lock().unwrap(), expected_events);
    assert!(ownership.unmanaged_host_close_required());
    if uncertain { assert!(matches!(ownership.seal(), Err(GateError::Unconfirmed))); }
    else {
        let mut ticket = ownership.seal().unwrap();
        let proof = tokio::time::timeout(Duration::from_secs(2), ticket.wait_application())
            .await.unwrap().unwrap();
        ownership.check_application_drained(&proof).unwrap();
        drop(proof);
        {
            let waiting = ticket.wait();
            tokio::pin!(waiting);
            assert!(futures::poll!(waiting.as_mut()).is_pending());
        }
        ticket.cancel_before_control().unwrap();
    }
    let probe = Request::builder().uri("/api/hello").body(Body::empty()).unwrap();
    let response = tokio::time::timeout(Duration::from_secs(2), app.oneshot(probe))
        .await.unwrap().unwrap();
    assert_eq!(response.status(), if uncertain { StatusCode::SERVICE_UNAVAILABLE }
        else { StatusCode::OK });
    let body = to_bytes(response.into_body(), 1024).await.unwrap();
    let expected = if uncertain { format!("Cannot start this request: {}.", gate.error_explanation(GateError::Unconfirmed)) } else { "ok".into() };
    assert_eq!(body.as_ref(), expected.as_bytes());
    assert_eq!(gate.stats().unwrap().active, 1);
    assert_eq!(gate.stats().unwrap().unconfirmed, uncertain);
    assert_eq!(records(&ports), before);
    assert_eq!(*ports.events.lock().unwrap(), expected_events);
    assert_eq!(*ports.mint_routes.lock().unwrap(), initial_mints);
    assert!(ports.saved_sessions.lock().unwrap().is_empty());
    assert!(ports.registered_proxies.lock().unwrap().is_empty());
    assert_eq!(serde_json::to_value(state.serving_accounts.residents.snapshot().unwrap()).unwrap(),
        serde_json::to_value(snapshot).unwrap());
    assert_no_initial_inference(&ports);
}

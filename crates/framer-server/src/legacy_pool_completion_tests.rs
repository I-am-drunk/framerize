//! Inert admin and import-child ownership through the actual router.
//! Prepared against b1f47cbb; parent wiring/seams: Server 6a6073c7.
//! All compilation and execution remain UNRUN in this source lane.
use super::*;
use axum::{body::Body, http::{Request, StatusCode}, response::Response};
use crate::operation_ownership::ServingOwnership;
use framer_core::operation_gate::{GateError, OperationGate};
use tower::ServiceExt;

const WAIT: Duration = Duration::from_secs(2);
const UNKNOWN: &str = "account browser cleanup is unconfirmed; retry after cleanup";

fn owned(mut state: AppState) -> (AppState, Arc<ServingOwnership>, OperationGate) {
    let (scope, gate) = ServingOwnership::new_with_host_factory(
        state.wave.clone(), 32, |gate| gate).unwrap();
    state.runtime_mut().expect("unique fixture runtime").operation_ownership = Some(scope.clone());
    (state, scope, gate)
}

async fn post(state: &AppState, path: &str, payload: Value) -> Response {
    let request = Request::builder().method("POST").uri(path)
        .header("content-type", "application/json")
        .body(Body::from(payload.to_string())).unwrap();
    tokio::time::timeout(WAIT, crate::router(state.clone(), "/fixture-no-web").oneshot(request))
        .await.unwrap().unwrap()
}

async fn observe(response: Response, status: StatusCode, expected: Value, consume: bool) {
    assert_eq!(response.status(), status);
    if consume {
        let bytes = to_bytes(response.into_body(), 65536).await.unwrap();
        assert_eq!(serde_json::from_slice::<Value>(&bytes).unwrap(), expected);
    } else { drop(response); }
}

async fn finished(state: &AppState, scope: &ServingOwnership, gate: &OperationGate, unknown: bool) {
    until(|| gate.stats().unwrap().active == 1).await;
    assert_eq!(gate.stats().unwrap().unconfirmed, unknown);
    assert!(scope.unmanaged_host_close_required());
    assert!(state.serving_accounts.mutations.lock().unwrap().is_empty());
    assert!(state.serving_accounts.residents.snapshot().unwrap().residents.iter().all(|slot| slot.users == 0));
    if unknown { assert!(matches!(scope.seal(), Err(GateError::Unconfirmed))); }
    else {
        let mut ticket = scope.seal().unwrap();
        let proof = tokio::time::timeout(WAIT, ticket.wait_application()).await.unwrap().unwrap();
        scope.check_application_drained(&proof).unwrap();
        drop(proof);
        {
            let wait = ticket.wait();
            tokio::pin!(wait);
            assert!(futures::poll!(wait.as_mut()).is_pending());
        }
        ticket.cancel_before_control().unwrap();
    }
    let request = Request::builder().uri("/api/hello").body(Body::empty()).unwrap();
    let response = tokio::time::timeout(WAIT,
        crate::router(state.clone(), "/fixture-no-web").oneshot(request)).await.unwrap().unwrap();
    assert_eq!(response.status(), if unknown { StatusCode::SERVICE_UNAVAILABLE } else { StatusCode::OK });
    assert_eq!(to_bytes(response.into_body(), 1024).await.unwrap().as_ref(),
        if unknown { b"Cannot start this request: earlier work ended without confirmed cleanup; its resources cannot be reused.".as_slice() } else { b"ok".as_slice() });
    assert_eq!(gate.stats().unwrap().active, 1);
    assert_eq!(gate.stats().unwrap().unconfirmed, unknown);
}

fn busy_error(message: &str) -> Value {
    json!({"error":{"code":"framerize_pool_busy","message":message,"retry_after_seconds":1}})
}

#[tokio::test]
async fn admin_park_and_configure_preserve_close_evidence_on_disposal() {
    for configure in [false, true] {
        for mode in [CloseMode::Confirm, CloseMode::Error, CloseMode::Unconfirmed, CloseMode::Busy] {
            for consume in [false, true] {
                let (state, ports) = fixture(1, 2);
                activated(&state, 1).await;
                let before = records(&ports);
                let (state, scope, gate) = owned(state);
                let busy = matches!(mode, CloseMode::Busy);
                let ok = matches!(mode, CloseMode::Confirm);
                if busy { *ports.status_mode.lock().unwrap() = StatusMode::Busy; }
                else { *ports.close_mode.lock().unwrap() = mode; }
                let (path, payload) = if configure {
                    ("/api/pool/config", json!({"eligible_accounts":[id(2)]}))
                } else { ("/api/pool/park", json!({"account_id":id(1)})) };
                let response = post(&state, path, payload).await;
                let expected = if ok && configure {
                    json!({"active_capacity":1,"eligible_accounts":[id(2)]})
                } else if ok { json!({"account_id":id(1),"status":"parked","eligible":true}) }
                else { busy_error(match mode {
                    CloseMode::Error => "fixture close failure",
                    CloseMode::Unconfirmed => "account browser closure is unconfirmed; its slot is retained",
                    _ => "account browser is busy or its identity changed",
                }) };
                observe(response, if ok { StatusCode::OK } else { StatusCode::SERVICE_UNAVAILABLE },
                    expected, consume).await;
                finished(&state, &scope, &gate, !ok && !busy).await;
                assert_eq!(records(&ports), before);
                assert_eq!(ports.account_writes.load(Ordering::SeqCst), 0);
                assert_eq!(ports.setting_writes.load(Ordering::SeqCst), usize::from(ok && configure));
                assert_eq!(state.serving_accounts.allows(&id(1)), !(ok && configure));
                assert!(state.serving_accounts.allows(&id(2)));
                assert_eq!(state.serving_accounts.residents.snapshot().unwrap().occupied, usize::from(!ok));
                if !ok { assert_eq!(slot(&state, id(1)).uncertain, !busy); }
                assert_eq!(ports.events.lock().unwrap().iter()
                    .filter(|event| event.0 == "close_started").count(), usize::from(!busy));
                assert_eq!(ports.mint_routes.lock().unwrap().len(), 1);
                assert_eq!(ports.request_started.load(Ordering::SeqCst), 0);
                assert!(ports.saved_sessions.lock().unwrap().is_empty());
            }
        }
    }
}

#[tokio::test]
async fn admin_activation_mint_preserves_clean_and_unknown_results() {
    for error in [None, Some("fixture mint refused"), Some(UNKNOWN)] {
        for consume in [false, true] {
            let (state, ports) = fixture(1, 2);
            let before = records(&ports);
            let (state, scope, gate) = owned(state);
            let (release, control) = tokio::sync::oneshot::channel();
            *ports.mint_control.lock().unwrap() = Some((id(1), control));
            let held = state.clone();
            let task = tokio::spawn(async move {
                post(&held, "/api/pool/activate", json!({"account_id":id(1)})).await
            });
            until(|| ports.events.lock().unwrap().contains(&("mint_wait", id(1)))).await;
            assert_eq!(slot(&state, id(1)).users, 1);
            assert!(gate.stats().unwrap().active > 1);
            assert!(!task.is_finished());
            release.send(error.map_or(Ok(()), |error| Err(error.into()))).unwrap();
            let response = tokio::time::timeout(WAIT, task).await.unwrap().unwrap();
            if let Some(error) = error {
                assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
                if consume {
                    assert_eq!(to_bytes(response.into_body(), 65536).await.unwrap().as_ref(), error.as_bytes());
                } else { drop(response); }
            } else {
                observe(response, StatusCode::OK,
                    json!({"account_id":id(1),"status":"active"}), consume).await;
            }
            let unknown = error == Some(UNKNOWN);
            finished(&state, &scope, &gate, unknown).await;
            assert_eq!(slot(&state, id(1)).uncertain, unknown);
            assert_eq!(records(&ports), before);
            assert_eq!(ports.mint_routes.lock().unwrap().len(), usize::from(error.is_none()));
            assert_eq!(ports.account_writes.load(Ordering::SeqCst), 0);
            assert_eq!(ports.request_started.load(Ordering::SeqCst), 0);
        }
    }
}

#[tokio::test]
async fn admin_authentication_results_keep_cleanup_evidence() {
    for complete in [false, true] { for unknown in [false, true] { for consume in [false, true] {
        let (state, ports) = fixture(1, 2);
        let before = records(&ports);
        let (state, scope, gate) = owned(state);
        let error = if unknown { UNKNOWN } else { "fixture authentication refused" };
        let path = if complete {
            *ports.auth_complete_reply.lock().unwrap() = Some(Err(error.into()));
            "/api/accounts/complete"
        } else {
            *ports.auth_begin_reply.lock().unwrap() = Some(Err(error.into()));
            "/api/accounts/begin"
        };
        let response = post(&state, path, json!({"account_id":id(1),
            "email":"fixture-1@example.invalid","email_token":"inert"})).await;
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        if consume {
            assert_eq!(to_bytes(response.into_body(), 65536).await.unwrap().as_ref(), error.as_bytes());
        } else { drop(response); }
        finished(&state, &scope, &gate, unknown).await;
        assert_eq!(slot(&state, id(1)).uncertain, unknown);
        assert_eq!(records(&ports), before);
        assert_eq!(ports.account_writes.load(Ordering::SeqCst), 0);
        assert_eq!(ports.request_started.load(Ordering::SeqCst), 0);
        assert!(ports.mint_routes.lock().unwrap().is_empty());
        assert!(ports.saved_sessions.lock().unwrap().is_empty());
        assert_eq!(*ports.events.lock().unwrap(), vec![("status", id(1)),
            (if complete { "auth_complete" } else { "auth_begin" }, id(1))]);
    } } }
}

#[tokio::test]
async fn admin_authentication_success_completes_consumed_and_discarded_output() {
    for complete in [false, true] { for consume in [false, true] {
        let (state, ports) = fixture(1, 2);
        let before = records(&ports);
        let (state, scope, gate) = owned(state);
        ports.allow_import.store(complete, Ordering::SeqCst);
        let path = if complete {
            *ports.auth_complete_reply.lock().unwrap() = Some(Ok(("session=fixture-only".into(),
                json!({"id":"user-2","spaces":[{"scope":"organization","ownerId":"team-2"}]}))));
            "/api/accounts/complete"
        } else {
            *ports.auth_begin_reply.lock().unwrap() = Some(Ok(()));
            "/api/accounts/begin"
        };
        let response = post(&state, path, json!({"account_id":id(2),
            "email_token":"inert","email":"fixture-2@example.invalid"})).await;
        assert_eq!(response.status(), StatusCode::OK);
        if consume {
            let bytes = to_bytes(response.into_body(), 65536).await.unwrap();
            if complete {
                assert_eq!(serde_json::from_slice::<Value>(&bytes).unwrap(), json!({"status":"signed-in",
                    "credits":1000.0,"account_id":id(2),"framer_user_id":"user-2",
                    "team":"team-2","project":"project-2"}));
            } else { assert_eq!(bytes.as_ref(), b"sent"); }
        } else { drop(response); }
        finished(&state, &scope, &gate, false).await;
        let mut expected = before;
        if complete {
            expected[1]["alias"] = json!("fixture-2@example.invalid");
            expected[1]["credit"] = json!({"available":1000.0,"funds":1,"grants":1,"allocated":1000.0});
        }
        assert_eq!(records(&ports), expected);
        assert_eq!(ports.account_writes.load(Ordering::SeqCst), if complete { 3 } else { 0 });
        assert_eq!(ports.saved_sessions.lock().unwrap().len(), usize::from(complete));
        assert_eq!(*ports.events.lock().unwrap(), vec![("status", id(2)),
            (if complete { "auth_complete" } else { "auth_begin" }, id(2))]);
        assert!(!slot(&state, id(2)).uncertain);
        assert!(ports.mint_routes.lock().unwrap().is_empty());
        assert_eq!(ports.request_started.load(Ordering::SeqCst), 0);
    } }
}

#[tokio::test]
async fn admin_remove_and_dedupe_keep_failed_close_fenced() {
    for dedupe in [false, true] {
        for mode in [CloseMode::Confirm, CloseMode::Error, CloseMode::Unconfirmed, CloseMode::Busy] {
            for consume in [false, true] {
                let (state, ports) = fixture(1, 2);
                if dedupe { ports.records.lock().unwrap()[1].castle_device_id = "device-1".into(); }
                activated(&state, 2).await;
                let before = records(&ports);
                let (state, scope, gate) = owned(state);
                let busy = matches!(mode, CloseMode::Busy);
                let ok = matches!(mode, CloseMode::Confirm);
                if busy { *ports.status_mode.lock().unwrap() = StatusMode::Busy; }
                else { *ports.close_mode.lock().unwrap() = mode; }
                *ports.remove_reply.lock().unwrap() = Some(Ok(()));
                let (path, payload) = if dedupe {
                    ("/api/accounts/dedupe", json!({"apply":true}))
                } else { ("/api/accounts/remove", json!({"account_id":id(2)})) };
                let response = post(&state, path, payload).await;
                assert_eq!(response.status(), if ok { StatusCode::OK } else { StatusCode::SERVICE_UNAVAILABLE });
                if consume {
                    let bytes = to_bytes(response.into_body(), 65536).await.unwrap();
                    let body: Value = serde_json::from_slice(&bytes).unwrap();
                    if ok && dedupe {
                        assert_eq!(body["applied"], true);
                        assert_eq!(body["removing"], json!([{"name":"fixture-2@example.invalid",
                            "reason":"shares a Castle device id with another record"}]));
                        assert_eq!(body["keeping"], json!([{"name":"fixture-1@example.invalid",
                            "team":"team-1","device":"device-1"}]));
                        assert_eq!(body["needs_project"], json!([]));
                    } else if ok {
                        assert_eq!(body, json!({"status":"removed","account":"fixture-2@example.invalid"}));
                    } else {
                        assert_eq!(body, busy_error(match mode {
                            CloseMode::Error => "fixture close failure",
                            CloseMode::Unconfirmed => "account browser closure is unconfirmed; its slot is retained",
                            _ => "account browser is busy or its identity changed",
                        }));
                    }
                } else { drop(response); }
                finished(&state, &scope, &gate, !ok && !busy).await;
                assert_eq!(records(&ports), if ok { json!([before[0]]) } else { before });
                assert_eq!(ports.account_writes.load(Ordering::SeqCst), usize::from(ok));
                assert_eq!(ports.events.lock().unwrap().iter()
                    .filter(|event| **event == ("remove", id(2))).count(), usize::from(ok));
                assert_eq!(state.serving_accounts.residents.snapshot().unwrap().occupied, usize::from(!ok));
                if !ok { assert_eq!(slot(&state, id(2)).uncertain, !busy); }
                assert_eq!(ports.mint_routes.lock().unwrap().len(), 1);
                assert_eq!(ports.request_started.load(Ordering::SeqCst), 0);
            }
        }
    }
}

#[tokio::test]
async fn admin_remove_keeps_best_effort_rotation_result_and_uncertainty() {
    for unknown in [false, true] { for consume in [false, true] {
        let (state, ports) = fixture(1, 2);
        activated(&state, 1).await;
        let before = records(&ports);
        let (state, scope, gate) = owned(state);
        *ports.rotate_reply.lock().unwrap() = Some(Err(
            if unknown { UNKNOWN } else { "fixture profile already absent" }.into()));
        *ports.remove_reply.lock().unwrap() = Some(Ok(()));
        let response = post(&state, "/api/accounts/remove",
            json!({"account_id":id(1),"wipe_profile":true})).await;
        observe(response, StatusCode::OK,
            json!({"status":"removed","account":"fixture-1@example.invalid"}), consume).await;
        finished(&state, &scope, &gate, unknown).await;
        assert_eq!(records(&ports), json!([before[1]]));
        assert_eq!(state.serving_accounts.residents.snapshot().unwrap().occupied, 0);
        assert_eq!(ports.account_writes.load(Ordering::SeqCst), 1);
        let events = ports.events.lock().unwrap().clone();
        assert_eq!(events.iter().filter(|event| **event == ("rotate_identity", id(1))).count(), 1);
        assert_eq!(events.iter().filter(|event| **event == ("remove", id(1))).count(), 1);
        assert_eq!(ports.mint_routes.lock().unwrap().len(), 1);
        assert_eq!(ports.request_started.load(Ordering::SeqCst), 0);
    } }
}

fn import_body() -> Value {
    json!({"account_id":id(2),"session_cookie":"fixture-only", "expected_framer_user_id":"user-2",
        "team_id":"team-2","project_id":"project-2","alias":"import fixture"})
}

fn imported() -> Value {
    json!({"status":"imported","serving_allowed":true,"account_id":id(2),
        "framer_user_id":"user-2","team":"team-2","project":"project-2"})
}

fn assert_import(ports: &Ports, before: &Value) -> Value {
    let mut expected = before.clone();
    expected[1]["alias"] = json!("import fixture");
    expected[1]["credit"] = json!({"available":1000.0,"funds":1,"grants":1,"allocated":1000.0});
    assert_eq!(records(ports), expected);
    assert_eq!(ports.account_writes.load(Ordering::SeqCst), 3);
    let saved = ports.saved_sessions.lock().unwrap();
    assert_eq!(saved.len(), 1);
    assert_eq!(saved[0].active_user, FramerUserId("user-2".into()));
    assert_eq!(saved[0].cookie_header, "session=fixture-only");
    assert_eq!(ports.registered_proxies.lock().unwrap().get(&id(2)),
        Some(&Some("http://proxy-2.invalid:8080".into())));
    assert_eq!(ports.request_started.load(Ordering::SeqCst), 0);
    assert_eq!(ports.setting_writes.load(Ordering::SeqCst), 0);
    expected
}

#[tokio::test]
async fn import_response_disposal_cannot_complete_its_pending_warmup_child() {
    for error in [None, Some("fixture warmup refused"), Some(UNKNOWN)] {
        for consume in [false, true] {
            let (state, ports) = fixture(1, 2);
            activated(&state, 1).await;
            ports.allow_import.store(true, Ordering::SeqCst);
            let before = records(&ports);
            let (state, scope, gate) = owned(state);
            let (release, control) = tokio::sync::oneshot::channel();
            *ports.mint_control.lock().unwrap() = Some((id(2), control));
            let response = post(&state, "/api/accounts/import", import_body()).await;
            until(|| ports.events.lock().unwrap().contains(&("mint_wait", id(2)))).await;
            observe(response, StatusCode::OK, imported(), consume).await;
            until(|| gate.stats().unwrap().active == 2).await;
            assert!(!gate.stats().unwrap().unconfirmed);
            assert_eq!(slot(&state, id(2)).users, 1);
            assert!(!slot(&state, id(2)).uncertain);
            assert!(ports.live.lock().unwrap().is_empty()); // A closed; B mint is still held.
            let expected = assert_import(&ports, &before);
            let mut ticket = scope.seal().unwrap();
            {
                let waiting = ticket.wait_application();
                tokio::pin!(waiting);
                assert!(futures::poll!(waiting.as_mut()).is_pending());
            }
            ticket.cancel_before_control().unwrap();
            release.send(error.map_or(Ok(()), |error| Err(error.into()))).unwrap();
            finished(&state, &scope, &gate, error == Some(UNKNOWN)).await;
            assert_eq!(slot(&state, id(2)).uncertain, error == Some(UNKNOWN));
            assert_eq!(records(&ports), expected);
            assert_import(&ports, &before);
            let mut minted = vec![id(1)];
            if error.is_none() { minted.push(id(2)); }
            assert_eq!(ports.mint_routes.lock().unwrap().iter().map(|entry| entry.0)
                .collect::<Vec<_>>(), minted);
            assert_eq!(ports.max_live.load(Ordering::SeqCst), 1);
            assert_eq!(ports.events.lock().unwrap().iter()
                .filter(|event| **event == ("close_confirmed", id(1))).count(), 1);
        }
    }
}

#[tokio::test]
async fn import_child_acquisition_failure_retains_independent_cleanup_verdict() {
    for mode in [CloseMode::Error, CloseMode::Unconfirmed, CloseMode::Busy] {
        for consume in [false, true] {
            let (state, ports) = fixture(1, 2);
            activated(&state, 1).await;
            ports.allow_import.store(true, Ordering::SeqCst);
            let before = records(&ports);
            let (state, scope, gate) = owned(state);
            let busy = matches!(mode, CloseMode::Busy);
            if busy { *ports.status_mode.lock().unwrap() = StatusMode::Busy; }
            else { *ports.close_mode.lock().unwrap() = mode; }
            let response = post(&state, "/api/accounts/import", import_body()).await;
            observe(response, StatusCode::OK, imported(), consume).await;
            finished(&state, &scope, &gate, !busy).await;
            assert_eq!(slot(&state, id(1)).uncertain, !busy);
            assert_eq!(state.serving_accounts.residents.snapshot().unwrap().occupied, 1);
            assert_eq!(slot(&state, id(1)).replacement, None);
            assert_import(&ports, &before);
            assert_eq!(ports.mint_routes.lock().unwrap().iter().map(|entry| entry.0)
                .collect::<Vec<_>>(), vec![id(1)]);
            assert_eq!(ports.events.lock().unwrap().iter()
                .filter(|event| event.0 == "close_started").count(), usize::from(!busy));
            assert_eq!(ports.max_live.load(Ordering::SeqCst), 1);
        }
    }
}

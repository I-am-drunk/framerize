use super::*;
use futures::StreamExt;

const RISK: &str = "risk-refused";
const COOLING: &str = "account-cooling:60";
const UNKNOWN: &str = "host-lifecycle-unconfirmed: inert cleanup lost";

#[tokio::test]
async fn explicit_risk_refusal_never_starts_peer() {
    for consume in [true, false] { exercise(RISK, true, consume).await; }
}

#[tokio::test]
async fn explicit_local_cooldown_never_starts_peer() {
    for consume in [true, false] { exercise(COOLING, true, consume).await; }
}

#[tokio::test]
async fn explicit_unknown_cleanup_never_starts_peer_or_clears_fence() {
    for consume in [true, false] { exercise(UNKNOWN, true, consume).await; }
}

#[tokio::test]
async fn unbound_risk_refusal_still_repicks() { exercise(RISK, false, true).await; }

#[tokio::test]
async fn unbound_local_cooldown_still_repicks() { exercise(COOLING, false, true).await; }

#[tokio::test]
async fn unbound_unknown_cleanup_still_repicks_and_keeps_fence() {
    exercise(UNKNOWN, false, true).await;
}

fn retry_request(f: &fixture::Fixture, messages: bool, stream: bool, explicit: bool)
    -> HttpRequest<Body> {
    let (path, payload) = if messages {
        ("/anthropic/v1/messages", serde_json::json!({"model":"claude-opus-5-5",
            "messages":[{"role":"user","content":"fixture"}],"stream":stream}))
    } else {
        ("/openai/v1/responses", serde_json::json!({"model":"gpt-6-astra",
            "input":"fixture","stream":stream}))
    };
    let mut request = HttpRequest::builder().method("POST").uri(path)
        .header("content-type", "application/json")
        .header("x-framerize-session", "fixture-conversation")
        .body(Body::from(payload.to_string())).unwrap();
    if explicit {
        request.headers_mut().insert("x-framerize-account",
            f.accounts[0].id.0.to_string().parse().unwrap());
    }
    request
}

async fn exercise(error: &str, explicit: bool, consume: bool) {
    for (messages, stream) in [(true, true), (true, false), (false, true), (false, false)] {
        let mut f = fixture::Fixture::new();
        f.accounts[1].consecutive_failures = 3;
        f.state.state.upsert(&f.accounts[1]).await.unwrap();
        assert!(f.accounts.iter().all(|account| account.is_eligible()));
        f.state = with_owned_operations(f.state, 32).unwrap();
        let ownership = f.state.operation_ownership.clone().unwrap();
        let model = if messages { "claude-opus-5-5" } else { "gpt-6-astra" };
        let app = crate::router(f.state.clone(), "/nonexistent-fixture-web");
        let task = tokio::spawn(app.oneshot(retry_request(&f, messages, stream, explicit)));
        let first = f.next().await;
        assert_eq!(first.account, f.accounts[0].id);
        assert_eq!(first.kind, if messages { "messages" } else { "responses" });
        let loaded = tokio::time::timeout(WAIT, f.loads.recv()).await.unwrap().unwrap();
        assert_eq!(loaded, f.accounts[0].framer_user_id);
        assert!(f.state.gate.acquire_now(&f.key(0, model)).await.is_none());
        let attempted_at = framer_core::rotation::now_ms();
        first.reply.send(Err(error.to_string())).ok().unwrap();
        tokio::time::timeout(WAIT, first.dropped).await.unwrap().unwrap();
        if !explicit {
            let second = f.next().await;
            assert_eq!(second.account, f.accounts[1].id);
            assert_eq!(second.kind, first.kind);
            let loaded = tokio::time::timeout(WAIT, f.loads.recv()).await.unwrap().unwrap();
            assert_eq!(loaded, f.accounts[1].framer_user_id);
            assert!(f.state.gate.acquire_now(&f.key(1, model)).await.is_none());
            second.reply.send(Ok(fixture::success(second.kind))).ok().unwrap();
            tokio::time::timeout(WAIT, second.dropped).await.unwrap().unwrap();
        }
        let response = tokio::time::timeout(WAIT, task).await.unwrap().unwrap().unwrap();
        let status = if stream || !explicit { StatusCode::OK }
            else { StatusCode::SERVICE_UNAVAILABLE };
        assert_eq!(response.status(), status);
        // Bounded Responses output must be read before its producer can join.
        // Stop at the terminal frame, retaining the response owner until EOF
        // or disposal. This cannot cancel and hide a forbidden peer attempt.
        let completion = response.extensions().get::<ResponseCompletion>()
            .expect("response lost its cleanup observer").clone();
        let mut body = response.into_body().into_data_stream();
        let mut output = if !messages && stream { read_response_terminal(&mut body).await }
            else { Vec::new() };
        let confirmed = tokio::time::timeout(WAIT, completion.0).await
            .expect("producer did not finish while its response stayed connected");
        assert_eq!(confirmed, error != UNKNOWN);
        assert_eq!(ownership.gate.stats().unwrap().active, 2); // Host and response.
        if consume {
            let tail = tokio::time::timeout(WAIT, to_bytes(Body::from_stream(body), 65536 - output.len()))
                .await.unwrap().unwrap();
            output.extend_from_slice(&tail);
        } else { drop(body); }
        if consume || (!messages && stream) {
            if explicit { assert_error_output(&output, messages, stream, error); }
            else { assert_success_output(&output, messages, stream); }
        }
        tokio::time::timeout(WAIT, async {
            while ownership.gate.stats().unwrap().active > 1 { tokio::task::yield_now().await; }
        }).await.unwrap();
        f.no_call();
        assert!(matches!(f.loads.try_recv(), Err(tokio::sync::mpsc::error::TryRecvError::Empty)));
        assert_eq!(ownership.gate.stats().unwrap().active, 1); // Unmanaged host remains.
        assert_eq!(ownership.gate.stats().unwrap().unconfirmed, error == UNKNOWN);
        let snapshot = f.state.serving_accounts.residents.snapshot().unwrap();
        assert_eq!(snapshot.residents.len(), if explicit { 1 } else { 2 });
        assert!(snapshot.residents.iter().all(|slot| slot.users == 0));
        let first_slot = snapshot.residents.iter().find(|slot| slot.account == first.account).unwrap();
        assert_eq!(first_slot.uncertain, error == UNKNOWN);
        if !explicit {
            let peer = snapshot.residents.iter().find(|slot| slot.account == f.accounts[1].id).unwrap();
            assert!(!peer.uncertain);
        }
        for index in [0, 1] {
            drop(f.state.gate.acquire_now(&f.key(index, model)).await.expect("attempt permit leaked"));
        }
        assert_records(&f, error, explicit, attempted_at).await;
        assert!(f.state.wave.check().is_none());
        if error == UNKNOWN {
            assert!(matches!(ownership.seal(), Err(GateError::Unconfirmed)));
            assert!(matches!(ownership.gate.admit(OperationKind::Inference), Err(GateError::Unconfirmed)));
        } else {
            let mut ticket = ownership.seal().unwrap();
            let proof = tokio::time::timeout(WAIT, ticket.wait_application()).await.unwrap().unwrap();
            ownership.check_application_drained(&proof).unwrap();
            drop(proof);
            ticket.cancel_before_control().unwrap();
            ownership.gate.admit(OperationKind::Inference).unwrap().complete().unwrap();
        }
        assert!(ownership.unmanaged_host_close_required());
    }
}

async fn read_response_terminal(body: &mut axum::body::BodyDataStream) -> Vec<u8> {
    let mut wire = Vec::new();
    tokio::time::timeout(WAIT, async {
        loop {
            let bytes = body.next().await.expect("response ended without a terminal event").unwrap();
            assert!(wire.len() + bytes.len() <= 65536, "fixture output exceeded its bound");
            wire.extend_from_slice(&bytes);
            drop(bytes);
            if wire.ends_with(b"\n\n") && output_values(&wire, true).last().is_some_and(|event|
                matches!(event["type"].as_str(), Some("response.failed" | "response.completed"))) {
                return;
            }
        }
    }).await.expect("response did not publish its terminal event");
    wire
}

async fn assert_records(f: &fixture::Fixture, error: &str, explicit: bool, attempted_at: u64) {
    let first = f.state.state.get(f.accounts[0].id).await.unwrap().unwrap();
    let mut expected = f.accounts[0].clone();
    if error != UNKNOWN {
        let delay = if error == RISK { 2_000 } else { 60_000 };
        let until = first.cooling_until_ms.unwrap();
        assert!((attempted_at + delay..=framer_core::rotation::now_ms() + delay).contains(&until));
        expected.cooling_until_ms = first.cooling_until_ms;
    }
    if error == RISK {
        expected.consecutive_failures = 1;
        expected.cooldown_step = 1;
    }
    assert_eq!(serde_json::to_value(first).unwrap(), serde_json::to_value(expected).unwrap());
    let peer = f.state.state.get(f.accounts[1].id).await.unwrap().unwrap();
    let mut expected = f.accounts[1].clone();
    if !explicit { expected.consecutive_failures = 0; }
    assert_eq!(serde_json::to_value(peer).unwrap(), serde_json::to_value(expected).unwrap());
}

fn output_values(body: &[u8], stream: bool) -> Vec<serde_json::Value> {
    if !stream { return vec![serde_json::from_slice(body).unwrap()]; }
    std::str::from_utf8(body).unwrap().lines()
        .filter_map(|line| line.strip_prefix("data: "))
        .map(|data| serde_json::from_str(data).unwrap()).collect()
}

fn assert_error_output(body: &[u8], messages: bool, stream: bool, original: &str) {
    let values = output_values(body, stream);
    let error = if stream {
        let kind = if messages { "error" } else { "response.failed" };
        assert_eq!(values.iter().filter(|value| value["type"] == kind).count(), 1);
        let failure = values.iter().find(|value| value["type"] == kind).unwrap();
        assert!(!values.iter().any(|value| value["type"] == "message_stop"
            || value["type"] == "response.completed" || value["type"] == "content_block_delta"));
        if messages { &failure["error"] } else { &failure["response"]["error"] }
    } else { &values[0]["error"] };
    let (message, kind) = if original == COOLING {
        ("This account is temporarily cooling down. Retry in 60 seconds.", "overloaded_error")
    } else if original == RISK {
        ("The upstream service is temporarily unavailable. Please retry shortly.", "overloaded_error")
    } else {
        ("Framerize cannot safely start another request yet. Please retry later.", "overloaded_error")
    };
    assert_eq!(error["message"], message);
    assert_eq!(error["type"], kind);
    if !messages { assert_eq!(error["code"], kind); }
}

fn assert_success_output(body: &[u8], messages: bool, stream: bool) {
    let values = output_values(body, stream);
    if stream {
        assert!(!values.iter().any(|value| value["type"] == "error"
            || value["type"] == "response.failed"));
        let kind = if messages { "message_stop" } else { "response.completed" };
        assert_eq!(values.iter().filter(|value| value["type"] == kind).count(), 1);
        if messages {
            let text: String = values.iter().filter(|value| value["type"] == "content_block_delta")
                .map(|value| value["delta"]["text"].as_str().unwrap()).collect();
            assert_eq!(text, "fixture");
        } else {
            let done = values.iter().find(|value| value["type"] == kind).unwrap();
            assert_eq!(done["response"]["output"][0]["content"][0]["text"], "fixture");
            assert_eq!(done["response"]["status"], "completed");
        }
    } else if messages {
        assert_eq!(values[0]["content"][0]["text"], "fixture");
        assert_eq!(values[0]["stop_reason"], "end_turn");
    } else {
        assert_eq!(values[0]["output"][0]["content"][0]["text"], "fixture");
        assert_eq!(values[0]["status"], "completed");
    }
}

#[tokio::test]
async fn undecodable_explicit_account_is_rejected_before_work() {
    exercise_undecodable_account(false).await;
}

#[tokio::test]
async fn wave_refusal_precedes_undecodable_explicit_account() {
    exercise_undecodable_account(true).await;
}

async fn exercise_undecodable_account(wave: bool) {
    for (messages, stream) in [(true, true), (true, false), (false, true), (false, false)] {
        for consume in [true, false] {
            let mut f = fixture::Fixture::new();
            assert!(f.accounts.iter().all(|account| account.is_eligible()));
            f.state = with_owned_operations(f.state, 32).unwrap();
            let ownership = f.state.operation_ownership.clone().unwrap();
            if wave {
                for account in &f.accounts {
                    f.state.wave.note_refusal(&account.id.0.to_string());
                }
            }
            assert_eq!(f.state.wave.check().is_some(), wave);
            let mut request = retry_request(&f, messages, stream, false);
            request.headers_mut().insert("x-framerize-account",
                axum::http::HeaderValue::from_bytes(&[0x80]).unwrap());
            let response = tokio::time::timeout(WAIT,
                crate::router(f.state.clone(), "/nonexistent-fixture-web").oneshot(request))
                .await.unwrap().unwrap();
            assert_eq!(response.status(), if wave { StatusCode::SERVICE_UNAVAILABLE }
                else { StatusCode::BAD_REQUEST });
            assert_eq!(response.headers()[axum::http::header::CONTENT_TYPE], "application/json");
            assert!(response.headers().contains_key("x-framerize-operation-id"));
            if wave {
                assert_eq!(response.headers()[axum::http::header::RETRY_AFTER], "30");
            } else {
                assert!(!response.headers().contains_key(axum::http::header::RETRY_AFTER));
            }
            assert_eq!(ownership.gate.stats().unwrap().active, 2); // Host and response.
            assert!(!ownership.gate.stats().unwrap().unconfirmed);
            if consume {
                let body = tokio::time::timeout(WAIT, to_bytes(response.into_body(), 4096))
                    .await.unwrap().unwrap();
                let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
                if wave {
                    assert_eq!(value["error"]["code"], "framerize_cooling");
                    assert_eq!(value["error"]["type"], "overloaded_error");
                    assert_eq!(value["error"]["retry_after_seconds"], 30);
                    assert_eq!(value["error"]["message"],
                        "Framerize is temporarily unavailable. Retry in 30 seconds.");
                } else {
                    assert_eq!(value, serde_json::json!({"error": {
                        "code": "invalid_request",
                        "message": "x-framerize-account must contain valid text"
                    }}));
                }
            } else {
                drop(response);
            }
            let mut ticket = ownership.seal().unwrap();
            let proof = tokio::time::timeout(WAIT, ticket.wait_application()).await.unwrap().unwrap();
            ownership.check_application_drained(&proof).unwrap();
            assert_eq!(ownership.gate.stats().unwrap().active, 1); // Unmanaged host remains.
            assert!(!ownership.gate.stats().unwrap().unconfirmed);
            assert!(ownership.unmanaged_host_close_required());
            drop(proof);
            ticket.cancel_before_control().unwrap();
            ownership.gate.admit(OperationKind::Inference).unwrap().complete().unwrap();
            f.no_call();
            assert!(matches!(f.loads.try_recv(), Err(tokio::sync::mpsc::error::TryRecvError::Empty)));
            let snapshot = f.state.serving_accounts.residents.snapshot().unwrap();
            assert_eq!(snapshot.occupied, 0);
            assert!(snapshot.residents.is_empty());
            assert_eq!(serde_json::to_value(f.state.state.list().await.unwrap()).unwrap(),
                serde_json::to_value(&f.accounts).unwrap());
            assert_eq!(f.state.wave.check().is_some(), wave);
        }
    }
}

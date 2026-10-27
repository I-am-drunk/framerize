use super::*;
#[path = "policy_worker_fixture.rs"]
mod router_fixture;
#[path = "policy_cold_mint_tests.rs"]
mod cold_mint;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_worker_cancel_after_early_disposal_keeps_generation_ready() {
    use router_fixture::*;
    use futures::StreamExt;
    let mut f = Fixture::new();
    let generation = f.stage().await;
    f.manager().activate(generation).unwrap();
    // Only this fixture pauses its reader. The other runtime thread drives disposal.
    let pause = f.state.response_output.pause_worker_accept(2);
    let response = f.post(Some(0), payload()).await;
    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let call = f.next().await;
    call.reply.send(Ok(reply("cancel at acceptance"))).ok().unwrap();
    tokio::time::timeout(BOUND, call.joined).await.unwrap().unwrap();
    let mut body = response.into_body().into_data_stream();
    for _ in 0..2 {
        drop(tokio::time::timeout(BOUND, body.next()).await.unwrap().unwrap().unwrap());
    }
    tokio::time::timeout(BOUND, pause.reached()).await.unwrap();
    let bytes = tokio::time::timeout(BOUND, body.next()).await.unwrap().unwrap().unwrap();
    assert!(String::from_utf8_lossy(&bytes).contains("event: response.output_item.added\n"));
    drop(bytes);
    drop(body);
    drop(pause);
    f.settle().await;
    let snapshot = f.manager().snapshot().unwrap();
    assert_eq!(snapshot.len(), 1);
    assert_eq!(snapshot[0].generation, generation);
    assert!(snapshot[0].alive && !snapshot[0].unconfirmed);
    assert!(snapshot[0].active);
    assert_eq!(snapshot[0].phase, crate::policy_generations::GenerationPhase::Ready);
    assert_eq!(snapshot[0].pins, 0);
    assert!(f.state.operation_ownership.as_ref().unwrap().accepting_requests());
    assert_eq!(f.state.response_output.output_counts(), (0, 0));
    let response = f.post(Some(1), payload()).await;
    let call = f.next().await;
    assert_eq!(f.manager().snapshot().unwrap()[0].generation, generation);
    call.reply.send(Ok(reply("same generation"))).ok().unwrap();
    tokio::time::timeout(BOUND, call.joined).await.unwrap().unwrap();
    trace(&consume(response).await, "same generation");
    assert!(f.calls.try_recv().is_err());
    f.settle().await;
    assert_eq!(f.state.response_output.output_counts(), (0, 0));
    f.finish().await;
}

async fn fault_response(mode: &str) -> (router_fixture::Fixture, u64, axum::response::Response) {
    let f = router_fixture::Fixture::new();
    let generation = f.stage_kind("FAULT").await;
    f.manager().activate(generation).unwrap();
    let mut input = router_fixture::payload();
    input["model"] = mode.into();
    let response = f.post(Some(0), input).await;
    (f, generation, response)
}

#[tokio::test]
async fn real_fault_before_head_emits_only_fixed_failure_and_fences() {
    let (mut f, generation, response) = fault_response("fixture-before-head").await;
    let text = router_fixture::consume(response).await;
    f.wait_failed(generation).await;
    assert_eq!(text.matches("event: response.failed\n").count(), 1);
    assert!(text.contains("upstream operation task failed"));
    assert!(text.contains("\"sequence_number\":0"));
    assert!(!text.contains("response.created"));
    assert!(f.calls.try_recv().is_err());
    f.finish_fenced().await;
}

#[tokio::test]
async fn real_fault_malformed_or_unheaded_frames_refuse_without_forward() {
    for mode in ["fixture-oversize", "fixture-duplicate", "fixture-unheaded", "fixture-foreign-op"] {
        let (mut f, generation, response) = fault_response(mode).await;
        let text = router_fixture::consume(response).await;
        f.wait_failed(generation).await;
        assert_eq!(text.matches("event: response.failed\n").count(), 1, "{mode}");
        assert!(text.contains("\"sequence_number\":0"), "{mode}");
        assert!(f.calls.try_recv().is_err(), "{mode}");
        f.finish_fenced().await;
    }
}

#[tokio::test]
async fn real_fault_over_credit_poisons_while_body_is_unread() {
    let (mut f, generation, response) = fault_response("fixture-over-credit").await;
    f.wait_failed(generation).await;
    assert_eq!(f.state.response_output.output_counts(), (0, 2));
    let text = router_fixture::consume(response).await;
    assert!(text.contains("\"sequence_number\":2"));
    assert_eq!(text.matches("event: response.failed\n").count(), 1);
    assert!(f.calls.try_recv().is_err());
    f.finish_fenced().await;
}

#[tokio::test]
async fn real_fault_mid_event_aborts_without_splicing_failure_bytes() {
    use futures::StreamExt;
    let (mut f, _, response) = fault_response("fixture-mid-event").await;
    let mut body = response.into_body().into_data_stream();
    let first = tokio::time::timeout(router_fixture::BOUND, body.next()).await.unwrap().unwrap().unwrap();
    assert_eq!(&first[..], b"event: response.created\ndata: {");
    drop(first);
    assert!(tokio::time::timeout(router_fixture::BOUND, body.next()).await.unwrap().unwrap().is_err());
    assert!(tokio::time::timeout(router_fixture::BOUND, body.next()).await.unwrap().is_none());
    drop(body);
    assert!(f.calls.try_recv().is_err());
    f.finish_fenced().await;
}

#[tokio::test]
async fn real_fault_event_boundary_appends_one_sequenced_fixed_failure() {
    let (mut f, _, response) = fault_response("fixture-boundary").await;
    let text = router_fixture::consume(response).await;
    assert!(text.starts_with("event: response.created\ndata: {}\n\n"));
    assert_eq!(text.matches("event: response.failed\n").count(), 1);
    assert!(text.contains("\"sequence_number\":1"));
    assert!(text.contains("upstream operation task failed"));
    assert!(f.calls.try_recv().is_err());
    f.finish_fenced().await;
}

#[tokio::test]
async fn real_fault_after_terminal_adds_no_second_terminal() {
    let (mut f, _, response) = fault_response("fixture-after-terminal").await;
    let call = f.next().await;
    call.reply.send(Ok(router_fixture::reply("joined"))).ok().unwrap();
    call.joined.await.unwrap();
    let text = router_fixture::consume(response).await;
    assert_eq!(text, concat!("event: response.created\ndata: {}\n\n",
        "event: response.in_progress\ndata: {}\n\n", "event: response.completed\ndata: {}\n\n"));
    assert!(f.calls.try_recv().is_err());
    f.finish_fenced().await;
}

#[tokio::test]
async fn real_fault_channel_loss_joins_pending_forward_without_clearing_fence() {
    for confirmed in [true, false] {
        let (mut f, generation, response) = fault_response("fixture-pending-forward").await;
        let call = f.next().await;
        drop(response);
        f.wait_failed(generation).await;
        assert_eq!(f.manager().snapshot().unwrap()[0].pins, 1);
        {
            let settle = f.settle();
            tokio::pin!(settle);
            assert!(futures::poll!(&mut settle).is_pending());
            let result = if confirmed { Ok(router_fixture::reply("joined")) }
                else { Err(framer_core::port::ForwardError::from_legacy(
                    "host-lifecycle-unconfirmed: fixture".into()).into()) };
            call.reply.send(result).ok().unwrap();
            tokio::time::timeout(router_fixture::BOUND, call.joined).await.unwrap().unwrap();
            settle.await;
        }
        assert!(f.calls.try_recv().is_err());
        f.finish_fenced().await;
    }
}

#[tokio::test]
async fn real_worker_terminal_bytes_hold_retirement_after_eof_and_body_drop() {
    use router_fixture::*;
    use futures::StreamExt;
    let mut f = Fixture::new();
    let worker = f.stage().await;
    f.manager().activate(worker).unwrap();
    let response = f.post(Some(0), payload()).await;
    let call = f.next().await;
    call.reply.send(Ok(reply("retained"))).ok().unwrap();
    call.joined.await.unwrap();
    let mut body = response.into_body().into_data_stream();
    let mut retained = None;
    while let Some(bytes) = tokio::time::timeout(BOUND, body.next()).await.unwrap() {
        let bytes = bytes.unwrap();
        if String::from_utf8_lossy(&bytes).contains("event: response.completed\n") {
            retained = Some(bytes.clone());
        }
    }
    drop(body);
    assert!(retained.is_some());
    let manager = f.manager();
    let retiring = manager.retire(worker);
    tokio::pin!(retiring);
    assert!(futures::poll!(&mut retiring).is_pending());
    assert_eq!(manager.snapshot().unwrap()[0].pins, 1);
    drop(retained);
    tokio::time::timeout(BOUND, retiring).await.unwrap().unwrap();
    f.finish().await;
}

#[tokio::test]
async fn real_worker_stalled_output_does_not_block_terminal_or_cancelled_forward() {
    use router_fixture::*;
    let mut f = Fixture::new();
    let worker = f.stage().await;
    f.manager().activate(worker).unwrap();
    let stalled = f.post(Some(0), payload()).await;
    let a = f.next().await;
    a.reply.send(Ok(reply(&"stalled".repeat(16384)))).ok().unwrap();
    a.joined.await.unwrap();
    tokio::time::timeout(BOUND, async {
        while f.state.response_output.output_counts().1 != 2 {
            tokio::task::yield_now().await;
        }
    }).await.expect("A never filled its unread output slots");
    let healthy = f.post(Some(1), payload()).await;
    let b = f.next().await;
    b.reply.send(Ok(reply("healthy"))).ok().unwrap();
    b.joined.await.unwrap();
    trace(&consume(healthy).await, "healthy");
    tokio::time::timeout(BOUND, async {
        while f.manager().snapshot().unwrap()[0].pins != 1 {
            tokio::task::yield_now().await;
        }
    }).await.expect("B did not settle while A remained unread");
    assert_eq!(f.state.response_output.output_counts().1, 2);
    let cancelled = f.post(Some(1), payload()).await;
    let c = f.next().await;
    drop(cancelled);
    c.reply.send(Ok(reply("cancelled after Forward"))).ok().unwrap();
    tokio::time::timeout(BOUND, c.joined).await.unwrap().unwrap();
    tokio::time::timeout(BOUND, async {
        while f.manager().snapshot().unwrap()[0].pins != 1 { tokio::task::yield_now().await; }
    }).await.expect("C cleanup was blocked by A");
    assert_eq!(f.manager().snapshot().unwrap()[0].pins, 1);
    assert!(f.state.response_output.counts().2 > 0);
    assert_eq!(f.state.response_output.output_counts().1, 2);
    drop(stalled);
    f.finish().await;
}

#[tokio::test]
async fn real_worker_disconnect_preserves_unconfirmed_forward_cleanup() {
    use router_fixture::*;
    let mut f = Fixture::new();
    let worker = f.stage().await;
    f.manager().activate(worker).unwrap();
    let response = f.post(Some(0), payload()).await;
    let call = f.next().await;
    drop(response);
    call.reply.send(Err(framer_core::port::ForwardError::from_legacy(
        "host-lifecycle-unconfirmed: fixture".into()).into())).ok().unwrap();
    call.joined.await.unwrap();
    f.settle().await;
    assert!(f.manager().snapshot().unwrap()[0].unconfirmed);
    assert!(!f.state.operation_ownership.as_ref().unwrap().accepting_requests());
    assert_eq!(tokio::time::timeout(BOUND, f.manager().shutdown(true)).await.unwrap(),
        Err(crate::policy_generations::GenerationError::Unconfirmed));
    let snapshot = f.manager().snapshot().unwrap();
    assert_eq!(snapshot[0].phase, crate::policy_generations::GenerationPhase::ReapedUnconfirmed);
    assert!(!snapshot[0].alive);
}

#[tokio::test]
async fn real_worker_activation_preserves_the_open_stream_and_exact_identity() {
    use router_fixture::*;
    use futures::StreamExt;
    let mut f = Fixture::new();
    let a = f.stage().await;
    f.manager().activate(a).unwrap();
    let response = f.post(Some(0), payload()).await;
    let call = f.next().await;
    let expected = "A exact café ".repeat(8192);
    call.reply.send(Ok(reply(&expected))).ok().unwrap();
    call.joined.await.unwrap();
    let mut body = response.into_body().into_data_stream();
    let mut prefix = Vec::new();
    for _ in 0..3 {
        let bytes = tokio::time::timeout(BOUND, body.next()).await.unwrap().unwrap().unwrap();
        prefix.extend_from_slice(&bytes);
    }
    let b = f.stage().await;
    f.manager().activate(b).unwrap();
    let second = f.post(Some(1), payload()).await;
    let second_call = f.next().await;
    assert!(f.manager().snapshot().unwrap().iter().any(|slot| slot.generation == b && slot.pins == 1));
    second_call.reply.send(Ok(reply("B exact"))).ok().unwrap();
    second_call.joined.await.unwrap();
    let b_id = trace(&consume(second).await, "B exact");
    assert!(f.manager().snapshot().unwrap().iter().any(|slot| slot.generation == a && slot.pins == 1));
    let remainder = axum::response::Response::new(axum::body::Body::from_stream(body));
    prefix.extend_from_slice(consume(remainder).await.as_bytes());
    let a_id = trace(std::str::from_utf8(&prefix).unwrap(), &expected);
    assert_ne!(a_id, b_id);
    assert!(f.calls.try_recv().is_err());
    f.finish().await;
}

#[tokio::test]
async fn real_worker_disconnect_joins_forward_and_keeps_success_health() {
    use router_fixture::*;
    let mut f = Fixture::new();
    f.accounts[0].consecutive_failures = 3;
    f.state.state.upsert(&f.accounts[0]).await.unwrap();
    let worker = f.stage().await;
    f.manager().activate(worker).unwrap();
    let response = f.post(Some(0), payload()).await;
    let call = f.next().await;
    drop(response);
    assert_eq!(f.manager().snapshot().unwrap()[0].pins, 1);
    {
        let settling = f.settle();
        tokio::pin!(settling);
        assert!(futures::poll!(&mut settling).is_pending());
        call.reply.send(Ok(reply("joined success"))).ok().unwrap();
        call.joined.await.unwrap();
        settling.await;
    }
    assert_eq!(f.state.state.get(f.accounts[0].id).await.unwrap().unwrap().consecutive_failures, 0);
    f.finish().await;
}

#[tokio::test]
async fn real_worker_uses_original_input_and_returns_exact_content() {
    use router_fixture::*;
    let mut f = Fixture::new();
    let worker = f.stage().await;
    f.manager().activate(worker).unwrap();
    let mut input = payload();
    input["instructions"] = "original instructions".into();
    input["tools"] = serde_json::json!([{"type":"function", "name":"f", "parameters":{"type":"object"}}]);
    let response = f.post(Some(0), input).await;
    let call = f.next().await;
    assert_eq!(call.account, f.accounts[0].id);
    assert_eq!(call.chunk, 0);
    assert_eq!(call.step, "translate");
    let instructions = call.payload["instructions"].as_str().unwrap();
    assert!(instructions.starts_with("original instructions\n\n"));
    assert_eq!(instructions.matches("at most ~40 lines").count(), 1);
    call.reply.send(Ok(reply("exact café 日本語 😀"))).ok().unwrap();
    call.joined.await.unwrap();
    let text = consume(response).await;
    assert!(text.contains("exact café 日本語 😀"));
    assert_eq!(text.matches("event: response.created\n").count(), 1);
    assert_eq!(text.matches("event: response.completed\n").count(), 1);
    f.finish().await;
}

#[tokio::test]
async fn real_worker_precontent_repick_keeps_generation_after_activation() {
    use router_fixture::*;
    let mut f = Fixture::new();
    let a = f.stage().await;
    f.manager().activate(a).unwrap();
    let response = f.post(None, payload()).await;
    let first = f.next().await;
    let b = f.stage().await;
    f.manager().activate(b).unwrap();
    first.reply.send(Err(framer_core::port::ForwardError::from_legacy(
        "risk-refused".into()).into())).ok().unwrap();
    first.joined.await.unwrap();
    let second = f.next().await;
    assert_ne!(first.account, second.account);
    let slots = f.manager().snapshot().unwrap();
    assert_eq!(slots.iter().find(|s| s.generation == a).unwrap().pins, 1);
    assert_eq!(slots.iter().find(|s| s.generation == b).unwrap().pins, 0);
    second.reply.send(Ok(reply("repicked exactly once"))).ok().unwrap();
    second.joined.await.unwrap();
    trace(&consume(response).await, "repicked exactly once");
    assert!(f.calls.try_recv().is_err());
    f.finish().await;
}

#[tokio::test]
async fn real_worker_published_content_prevents_account_replay() {
    use router_fixture::*;
    let mut f = Fixture::new();
    let worker = f.stage().await;
    f.manager().activate(worker).unwrap();
    let response = f.post(None, payload()).await;
    let first = f.next().await;
    let mut partial = reply("published once");
    let mut body: serde_json::Value = serde_json::from_slice(&partial.body).unwrap();
    body["status"] = "incomplete".into();
    body["incomplete_details"] = serde_json::json!({"reason":"max_output_tokens"});
    partial.body = serde_json::to_vec(&body).unwrap().into();
    first.reply.send(Ok(partial)).ok().unwrap();
    first.joined.await.unwrap();
    let consume = tokio::spawn(consume(response));
    let second = f.next().await;
    assert_eq!(first.account, second.account);
    second.reply.send(Err(framer_core::port::ForwardError::from_legacy(
        "risk-refused".into()).into())).ok().unwrap();
    second.joined.await.unwrap();
    let text = tokio::time::timeout(BOUND, consume).await.unwrap().unwrap();
    assert_eq!(text.matches("event: response.created\n").count(), 1);
    assert_eq!(text.matches("event: response.failed\n").count(), 1);
    assert!(text.contains("published once"));
    assert!(!text.contains("risk-refused"));
    assert!(f.calls.try_recv().is_err());
    f.finish().await;
}

#[tokio::test]
async fn real_worker_rollback_and_third_slot_refusal_preserve_accepted_work() {
    use router_fixture::*;
    let mut f = Fixture::new();
    let a = f.stage().await;
    f.manager().activate(a).unwrap();
    let original = f.post(Some(0), payload()).await;
    let first = f.next().await;
    let b = f.stage().await;
    f.manager().activate(b).unwrap();
    let (path, hash) = executable("WORKER");
    assert_eq!(f.manager().stage(path, hash).await, Err(crate::policy_generations::GenerationError::Full));
    let middle = f.post(Some(1), payload()).await;
    let second = f.next().await;
    second.reply.send(Ok(reply("B"))).ok().unwrap();
    second.joined.await.unwrap();
    trace(&consume(middle).await, "B");
    f.manager().activate(a).unwrap();
    let rolled_back = f.post(Some(1), payload()).await;
    let third = f.next().await;
    assert_eq!(f.manager().snapshot().unwrap().iter().find(|s| s.generation == a).unwrap().pins, 2);
    third.reply.send(Ok(reply("A new"))).ok().unwrap();
    third.joined.await.unwrap();
    trace(&consume(rolled_back).await, "A new");
    first.reply.send(Ok(reply("A original"))).ok().unwrap();
    first.joined.await.unwrap();
    trace(&consume(original).await, "A original");
    assert!(f.calls.try_recv().is_err());
    f.finish().await;
}

#[tokio::test]
async fn real_worker_activation_keeps_exact_tool_call_identity_and_arguments() {
    use router_fixture::*;
    use futures::StreamExt;
    let mut f = Fixture::new();
    let a = f.stage().await;
    f.manager().activate(a).unwrap();
    let original = f.post(Some(0), payload()).await;
    let first = f.next().await;
    let arguments = serde_json::json!({"content":"exact 日本語 ".repeat(8192)}).to_string();
    first.reply.send(Ok(tool_reply(&arguments))).ok().unwrap();
    first.joined.await.unwrap();
    let mut body = original.into_body().into_data_stream();
    let mut prefix = Vec::new();
    for _ in 0..3 {
        let part = tokio::time::timeout(BOUND, body.next()).await.unwrap().unwrap().unwrap();
        prefix.extend_from_slice(&part);
    }
    let b = f.stage().await;
    f.manager().activate(b).unwrap();
    let second = f.post(Some(1), payload()).await;
    let call = f.next().await;
    call.reply.send(Ok(reply("new generation"))).ok().unwrap();
    call.joined.await.unwrap();
    trace(&consume(second).await, "new generation");
    assert_eq!(f.manager().snapshot().unwrap().iter().find(|s| s.generation == a).unwrap().pins, 1);
    let tail = axum::response::Response::new(axum::body::Body::from_stream(body));
    prefix.extend_from_slice(consume(tail).await.as_bytes());
    trace_tool(std::str::from_utf8(&prefix).unwrap(), &arguments);
    assert!(f.calls.try_recv().is_err());
    f.finish().await;
}

#[tokio::test]
async fn real_worker_http_origin_and_private_failures_keep_kernel_classification() {
    use router_fixture::*;
    let raw = r#"upstream 403 on /responses: {"error":{"message":"maximum context length: private marker","type":"quota 日本語"}}"#;
    for http in [true, false] {
        let mut f = Fixture::new();
        let worker = f.stage().await;
        f.manager().activate(worker).unwrap();
        let response = f.post(Some(0), payload()).await;
        let call = f.next().await;
        let error = framer_core::port::ForwardError::from_legacy(raw.into());
        let error = if http { error.http() } else { error };
        call.reply.send(Err(error.into())).ok().unwrap();
        call.joined.await.unwrap();
        let text = consume(response).await;
        let failure: serde_json::Value = text.lines().filter_map(|l| l.strip_prefix("data: "))
            .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
            .find(|event| event["type"] == "response.failed").unwrap();
        let (_, mut expected) = framer_core::convert::error_json(
            framer_core::convert::ClientDialect::OpenAiResponses, raw);
        if !http { expected["error"]["message"] = "upstream operation failed".into(); }
        assert_eq!(failure["response"]["error"], expected["error"]);
        assert_eq!(failure["response"]["error"]["code"], "context_length_exceeded");
        assert_eq!(text.contains("private marker"), http);
        assert!(f.calls.try_recv().is_err());
        f.finish().await;
    }
}

#[tokio::test]
async fn real_worker_control_failure_stops_dispatch_and_fences() {
    use router_fixture::*;
    let mut f = Fixture::new();
    let worker = f.stage().await;
    f.manager().activate(worker).unwrap();
    let response = f.post(None, payload()).await;
    let call = f.next().await;
    call.reply.send(Err(framer_core::port::ControlError.into())).ok().unwrap();
    call.joined.await.unwrap();
    let text = consume(response).await;
    assert!(text.contains("upstream operation task failed"));
    assert!(f.calls.try_recv().is_err());
    f.finish_fenced().await;
}

#[tokio::test]
async fn real_worker_fixed_identity_content_and_terminal_bytes_match_encoder() {
    use router_fixture::*;
    use framer_worker_wire::{credit::{Window, Capacity, data_cost}, message::{self as wire, Message}};
    let mut peer = RealPeer::new().await;
    peer.begin().await;
    let reply = tool_reply(r#"{"content":"fixed 日本語","count":3}"#);
    let expected = expected_tool_stream(&reply);
    let mut actual = Vec::new();
    let mut window = Window::new(crate::response_output::WORKER_OUTPUT).unwrap();
    let (mut accepted, mut forwards, mut terminal) = (0, 0, 0);
    loop {
        let received = peer.next().await;
        match received.message {
            Message::Data { op: 7, seq, event, bytes, last } => {
                assert_eq!(seq, accepted);
                let cost = data_cost(received.wire_bytes);
                window.reserve(cost).unwrap();
                if event != wire::DataKind::Opening { actual.extend_from_slice(bytes.as_slice()); }
                if event == wire::DataKind::Terminal && last { terminal += 1; }
                if last { accepted += 1; }
                drop(bytes);
                peer.send(Message::Credit { op: 7, returned: cost, accepted_event_seq: accepted }).await;
                window.release(cost).unwrap();
            }
            message => if fixed_peer_control(&peer, message, &reply, &mut forwards, terminal).await { break; },
        }
    }
    assert_eq!(window.outstanding(), Capacity::default());
    assert_eq!(actual, expected);
    assert_eq!(forwards, 1);
    assert_eq!(terminal, 1);
    peer.close().await;
}

async fn fixed_peer_control(peer: &router_fixture::RealPeer, message: framer_worker_wire::message::Message,
    reply: &framer_core::port::ForwardReply, forwards: &mut usize, terminal: usize) -> bool {
    use framer_worker_wire::message::{self as wire, Message};
    match message {
        Message::Forward { op: 7, n: 1, fwd: 1, payload, .. } => {
            assert!(payload.last);
            *forwards += 1;
            peer.send(Message::ForwardReply { op: 7, n: 1, fwd: 1,
                status: reply.status, content_type: wire::Text::new(reply.content_type.clone()).unwrap(),
                queue_wait_ms: reply.queue_wait_ms, body: wire::Part { offset: 0, last: true,
                    bytes: wire::Segment::new(reply.body.to_vec()).unwrap() } }).await;
        }
        Message::Result { op: 7, n: 1, outcome: wire::Outcome::Success { summary }, .. } => {
            assert_eq!(summary.chunks, 1);
            peer.send(Message::Finish { op: 7, n: 1 }).await;
        }
        Message::Diag { op: 7, n: 1, .. } => {},
        Message::Done { op: 7, outcome: wire::DoneOutcome::Completed, .. } => {
            assert_eq!(terminal, 1); return true;
        }
        other => panic!("unexpected worker message: {other:?}"),
    }
    false
}

#[tokio::test]
#[ignore = "fresh process and private log directory; real worker required"]
async fn isolated_real_worker_diagnostics_keep_op_and_exclude_raw_failure() {
    use router_fixture::*;
    let directory = LogDirectory::new();
    let mut f = Fixture::new();
    let worker = f.stage().await;
    f.manager().activate(worker).unwrap();
    let response = f.post(Some(0), payload()).await;
    let first = f.next().await;
    directory.wait_diagnostic("chunk_start").await;
    first.reply.send(Err(framer_core::port::ForwardError::from_legacy(
        "retryable 429 on /responses: raw-private-fixture-cause".into()).http().into())).ok().unwrap();
    first.joined.await.unwrap();
    let second = f.next().await;
    directory.wait_diagnostic("backoff").await;
    assert!(f.state.serving_accounts.rotation().is_saturated(&f.accounts[0].current_team.as_ref().unwrap().0));
    let mut upstream = reply(&"diagnostic output ".repeat(4096));
    upstream.queue_wait_ms = 0; // Avoid a competing UpstreamQueued diagnostic.
    second.reply.send(Ok(upstream)).ok().unwrap();
    second.joined.await.unwrap();
    directory.wait_diagnostic("chunk_done").await;
    consume(response).await;
    f.finish().await; // Logger is joined before inspecting or removing its directory.
    let records = directory.records();
    let backoff = records.iter().find(|e| e["diagnostic"]["kind"] == "backoff").unwrap();
    let done = records.iter().find(|e| e["diagnostic"]["kind"] == "chunk_done").unwrap();
    for key in ["request_id", "operation_id", "generation", "op", "invocation"] {
        assert!(!backoff[key].is_null()); assert_eq!(backoff[key], done[key]);
    }
    assert_eq!(backoff["diagnostic"]["attempt"], 1);
    assert_eq!(backoff["diagnostic"]["wait_ms"], 0);
    assert!(!serde_json::to_string(&records).unwrap().contains("raw-private-fixture-cause"));
}

#[tokio::test]
#[ignore = "isolated unpaused profile; main runs five fresh processes"]
async fn isolated_legacy0_router_profile() { router_fixture::profile(false).await; }

#[tokio::test]
#[ignore = "isolated unpaused profile with explicitly bound real worker"]
async fn isolated_real_worker_router_profile() { router_fixture::profile(true).await; }

mod invocation_contract {
    use crate::policy_worker_request::InvocationLedger;
    use framer_core::port::{ForwardError, OutputError, ResponseError};
    use framer_worker_wire::message::{self as wire, Message};
    use std::sync::{Arc, atomic::{AtomicUsize, Ordering}};

    fn cause(fwd: u64) -> wire::Cause { wire::Cause { op: 7, n: 2, fwd } }
    fn fail(ledger: &mut InvocationLedger, fwd: u64, raw: &str, http: bool) {
        ledger.accept_forward(cause(fwd), 0).unwrap();
        let error = ForwardError::from_legacy(raw.into());
        ledger.forward_finished(cause(fwd), Some(if http { error.http() } else { error })).unwrap();
    }
    fn outcome(fwd: u64, tag: wire::FailureTag) -> wire::Outcome {
        wire::Outcome::Forward { failure: wire::FailureRecipe { cause: cause(fwd), tag, chunk: 0 } }
    }
    fn summary() -> wire::Outcome {
        wire::Outcome::Success { summary: wire::Summary { input_tokens: 2, output_tokens: 3,
            chunks: 1, metered_chunks: 0, finish: wire::FinishReason::Completed } }
    }
    #[test]
    fn saturation_uses_captured_effect_before_ack_and_rejects_overtaking() {
        let calls = Arc::new(AtomicUsize::new(0));
        let captured = calls.clone();
        let mut ledger = InvocationLedger::new(7, 2, 4,
            Some(Arc::new(move || { captured.fetch_add(1, Ordering::SeqCst); }))).unwrap();
        fail(&mut ledger, 1, "retryable 429 on /responses: {}", true);
        assert_eq!(ledger.saturated(cause(1)).unwrap(), Message::SaturatedAck { op: 7, n: 2, fwd: 1 });
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert!(!ledger.settled());
        assert!(ledger.saturated(cause(1)).is_err());
        assert!(ledger.accept_forward(cause(2), 0).is_err());
        ledger.acknowledged(cause(1)).unwrap();
        assert!(ledger.saturated(cause(1)).is_err());
        fail(&mut ledger, 2, "retryable 429", true);
        ledger.saturated(cause(2)).unwrap();
        drop(ledger); // Losing an Ack waiter cannot undo the already applied effect.
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        let next_team = Arc::new(AtomicUsize::new(0));
        let next = next_team.clone();
        let mut repicked = InvocationLedger::new(7, 3, 4,
            Some(Arc::new(move || { next.fetch_add(1, Ordering::SeqCst); }))).unwrap();
        assert!(repicked.saturated(cause(2)).is_err());
        assert_eq!(next_team.load(Ordering::SeqCst), 0);
    }
    #[test]
    fn result_renders_exhaustion_and_fallback_before_classifying_once() {
        let raw = r#"retryable 429 on /responses: {"error":{"message":"busy","type":"limit_error"}}"#;
        let mut ledger = InvocationLedger::new(7, 2, 4, None).unwrap();
        for fwd in 1..=9 {
            fail(&mut ledger, fwd, raw, true);
            ledger.saturated(cause(fwd)).unwrap();
            ledger.acknowledged(cause(fwd)).unwrap();
        }
        fail(&mut ledger, 10, raw, true);
        assert!(ledger.saturated(cause(10)).is_err());
        ledger.close_result(7, 2).unwrap();
        let result = ledger.resolve(outcome(10, wire::FailureTag::SaturationExhausted), false).unwrap();
        let public = result.public_failure.unwrap();
        assert_eq!((public.status, public.error_type.as_str()), (500, "api_error"));
        assert_eq!(public.message.as_str(), format!("chunk 0: workspace still saturated after 9 retries ({raw})"));
        let mut ledger = InvocationLedger::new(7, 2, 4, None).unwrap();
        fail(&mut ledger, 1, raw, true);
        ledger.close_result(7, 2).unwrap();
        let result = ledger.resolve(outcome(1, wire::FailureTag::FallbackOff), false).unwrap();
        let public = result.public_failure.unwrap();
        assert_eq!((public.status, public.error_type.as_str()), (429, "limit_error"));
        assert_eq!(public.message.as_str(), "busy");
    }
    #[test]
    fn local_display_is_fixed_after_legacy_overflow_and_risk_classification() {
        for (raw, status, code) in [
            ("private cause: maximum context length", 500, "context_length_exceeded"),
            ("private risk-refused cause", 403, "api_error"),
            ("private transport cause", 500, "api_error"),
        ] {
            let mut ledger = InvocationLedger::new(7, 2, 4, None).unwrap();
            fail(&mut ledger, 1, raw, false);
            ledger.close_result(7, 2).unwrap();
            let result = ledger.resolve(outcome(1, wire::FailureTag::Direct), false).unwrap();
            let public = result.public_failure.unwrap();
            assert_eq!(public.status, status);
            assert_eq!(public.code.as_str(), code);
            assert_eq!(public.message.as_str(), "upstream operation failed");
            assert!(matches!(result.result, Err(ResponseError::Upstream(message)) if message == raw));
        }
    }
    #[test]
    fn result_barrier_refuses_resolution_until_forward_verdict_and_closes_dispatch() {
        let mut ledger = InvocationLedger::new(7, 2, 4, None).unwrap();
        ledger.accept_forward(cause(1), 0).unwrap();
        ledger.close_result(7, 2).unwrap();
        assert!(!ledger.settled());
        assert!(ledger.accept_forward(cause(2), 0).is_err());
        assert!(ledger.close_result(7, 2).is_err());
        ledger.forward_finished(cause(1), None).unwrap();
        assert!(ledger.settled());
        assert!(ledger.resolve(summary(), false).unwrap().result.is_ok());
        let mut pending = InvocationLedger::new(7, 2, 4, None).unwrap();
        pending.accept_forward(cause(1), 0).unwrap();
        pending.close_result(7, 2).unwrap();
        assert!(pending.resolve(summary(), false).is_err());
    }
    #[test]
    fn result_recipe_rejects_foreign_unknown_and_out_of_range_causes() {
        for recipe in [
            wire::FailureRecipe { cause: wire::Cause { op: 8, ..cause(1) }, tag: wire::FailureTag::Direct, chunk: 0 },
            wire::FailureRecipe { cause: wire::Cause { n: 1, ..cause(1) }, tag: wire::FailureTag::Direct, chunk: 0 },
            wire::FailureRecipe { cause: cause(2), tag: wire::FailureTag::Direct, chunk: 0 },
            wire::FailureRecipe { cause: cause(1), tag: wire::FailureTag::Direct, chunk: 4 },
            wire::FailureRecipe { cause: cause(1), tag: wire::FailureTag::SaturationExhausted, chunk: 0 },
        ] {
            let mut ledger = InvocationLedger::new(7, 2, 4, None).unwrap();
            fail(&mut ledger, 1, "retryable 429", true);
            ledger.close_result(7, 2).unwrap();
            assert!(ledger.resolve(wire::Outcome::Forward { failure: recipe }, false).is_err());
        }
        let mut ledger = InvocationLedger::new(7, 2, 4, None).unwrap();
        ledger.accept_forward(cause(1), 0).unwrap();
        ledger.forward_finished(cause(1), None).unwrap();
        ledger.close_result(7, 2).unwrap();
        assert!(ledger.resolve(outcome(1, wire::FailureTag::Direct), false).is_err());
    }
    #[test]
    fn disconnected_completion_needs_last_joined_success_and_preserves_encoding_errors() {
        for (success, cancelled, failure, expected) in [
            (true, true, wire::OutputFailure::Closed, true),
            (true, true, wire::OutputFailure::Cancelled, true),
            (true, false, wire::OutputFailure::Closed, false),
            (true, true, wire::OutputFailure::Encoding, false),
            (false, true, wire::OutputFailure::Closed, false),
        ] {
            let mut ledger = InvocationLedger::new(7, 2, 4, None).unwrap();
            ledger.accept_forward(cause(1), 0).unwrap();
            ledger.forward_finished(cause(1), (!success).then(|| ForwardError::from_legacy("failed".into()))).unwrap();
            ledger.close_result(7, 2).unwrap();
            let decision = ledger.resolve(wire::Outcome::Output { failure, completed: true }, cancelled).unwrap();
            assert_eq!(decision.completed_on_disconnect, expected);
            if failure == wire::OutputFailure::Encoding {
                assert!(matches!(decision.result, Err(ResponseError::Output(OutputError::Encoding))));
            }
        }
    }
    #[test]
    fn control_failure_never_becomes_retryable_display() {
        let mut ledger = InvocationLedger::new(7, 2, 4, None).unwrap();
        ledger.close_result(7, 2).unwrap();
        assert!(ledger.accept_forward(cause(1), 0).is_err());
        assert!(ledger.resolve(wire::Outcome::Control, false).is_err());
    }
    #[test]
    fn joined_cleanup_uncertainty_survives_a_later_success() {
        let mut ledger = InvocationLedger::new(7, 2, 4, None).unwrap();
        fail(&mut ledger, 1, "host-lifecycle-unconfirmed: fixture", false);
        assert!(!ledger.cleanup_confirmed());
        ledger.accept_forward(cause(2), 0).unwrap();
        ledger.forward_finished(cause(2), None).unwrap();
        assert!(!ledger.cleanup_confirmed());
        ledger.close_result(7, 2).unwrap();
        assert!(ledger.resolve(summary(), false).unwrap().result.is_ok());
    }
    #[test]
    fn absent_saturation_callback_is_legal_but_a_stale_429_is_not() {
        let mut ledger = InvocationLedger::new(7, 2, 4, None).unwrap();
        fail(&mut ledger, 1, "retryable 429", true);
        ledger.saturated(cause(1)).unwrap();
        ledger.acknowledged(cause(1)).unwrap();
        fail(&mut ledger, 2, "retryable 429", true);
        ledger.accept_forward(cause(3), 0).unwrap();
        ledger.forward_finished(cause(3), None).unwrap();
        assert!(ledger.saturated(cause(2)).is_err());
    }
}

struct Scratch(std::path::PathBuf);
impl Scratch {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("framer-worker-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn executable(&self, bytes: &[u8]) -> Executable {
        let path = self.0.join("worker");
        std::fs::write(&path, bytes).unwrap();
        Executable { path, sha256: Sha256::digest(bytes).into() }
    }
}
impl Drop for Scratch {
    fn drop(&mut self) { std::fs::remove_dir_all(&self.0).unwrap(); }
}

#[cfg(target_os = "linux")]
#[test]
fn executable_verification_rejects_mismatch_non_elf_and_oversize() {
    let scratch = Scratch::new();
    let mut descriptor = scratch.executable(b"\x7fELF-synthetic");
    descriptor.sha256 = [0; 32];
    assert!(matches!(verify(&descriptor), Err(ProcessError::Executable)));
    let descriptor = scratch.executable(b"#!/bin/sh\n");
    assert!(matches!(verify(&descriptor), Err(ProcessError::Executable)));
    let descriptor = scratch.executable(b"\x7fELF-synthetic");
    std::fs::OpenOptions::new().write(true).open(&descriptor.path).unwrap()
        .set_len(MAX_EXECUTABLE_BYTES + 1).unwrap();
    assert!(matches!(verify(&descriptor), Err(ProcessError::Executable)));
    let relative = Executable { path: "worker".into(), sha256: [0; 32] };
    assert!(matches!(verify(&relative), Err(ProcessError::Executable)));
}

#[cfg(target_os = "linux")]
#[test]
fn opened_executable_keeps_the_hashed_inode_after_path_replacement() {
    use std::io::{Seek, SeekFrom};
    use std::os::fd::AsRawFd;
    let scratch = Scratch::new();
    let original = b"\x7fELF-original-synthetic-inode";
    let executable = scratch.executable(original);
    let mut file = verify(&executable).unwrap();
    assert!(file.as_raw_fd() > 2);
    std::fs::rename(&executable.path, scratch.0.join("old")).unwrap();
    std::fs::write(&executable.path, b"\x7fELF-replacement").unwrap();
    file.seek(SeekFrom::Start(0)).unwrap();
    let mut retained = Vec::new();
    file.read_to_end(&mut retained).unwrap();
    assert_eq!(retained, original);
    assert!(verify(&executable).is_err());
    let command = command(&file);
    assert_eq!(command.get_program(), std::ffi::OsStr::new(
        &format!("/proc/self/fd/{}", file.as_raw_fd())));
    assert_eq!(command.get_current_dir(), Some(std::path::Path::new("/")));
    assert_eq!(command.get_args().count(), 0);
}

#[tokio::test]
async fn stderr_is_counted_and_discarded_without_retaining_payloads() {
    let counts = Arc::new(StderrCount::default());
    let bytes = vec![0xa5; STDERR_RESERVE * 7 + 1];
    drain_stderr(bytes.as_slice(), counts.clone()).await;
    assert_eq!(counts.read.load(Ordering::Relaxed), bytes.len() as u64);
}

#[tokio::test]
async fn bounded_dispatch_refusal_stops_before_the_next_frame() {
    let first = frame::encode(&Message::Ready { generation: 7 }).unwrap();
    let second = frame::encode(&Message::Ready { generation: 8 }).unwrap();
    let bytes = [first, second.clone()].concat();
    let mut input = bytes.as_slice();
    let seen = Arc::new(AtomicU64::new(0));
    let count = seen.clone();
    let dispatch: Dispatch = Arc::new(move |received| {
        assert_eq!(received.message, Message::Ready { generation: 7 });
        count.fetch_add(1, Ordering::Relaxed);
        Err(WireError::Protocol)
    });
    assert!(matches!(read_frames(&mut input, dispatch).await, Err(WireError::Protocol)));
    assert_eq!(seen.load(Ordering::Relaxed), 1);
    assert_eq!(input, second);
}

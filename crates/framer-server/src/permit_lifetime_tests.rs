use crate::{AbortOnDrop, ApiError, AppState};
use axum::{body::{Body, Bytes}, http::HeaderMap, response::Response};
use crate::response_output::consume_body as to_bytes;
use framer_core::{convert::ClientDialect, gate::ConcurrencyGate, port::ProxyResponse};
use futures::StreamExt;
use serde_json::json;
use std::time::Duration;
use tokio::sync::mpsc;
#[path = "permit_lifetime_fixture.rs"]
mod fixture;
use fixture::{success, Fixture, Reply};
#[path = "responses_stream_lifecycle_tests.rs"]
mod responses_stream_lifecycle;

const WAIT: Duration = Duration::from_secs(3);
const MESSAGE_MODEL: &str = "claude-opus-5-5";
const RESPONSE_MODEL: &str = "gpt-6-astra";

#[tokio::test(start_paused = true)]
async fn open_stream_reports_local_capacity_and_releases_its_permit() {
    for messages in [false, true] {
        let mut f = Fixture::new();
        let response = chain(f.state.clone(), f.headers(0), messages, true).await.unwrap();
        let call = f.next().await;
        call.reply.send(Err("account-cooling:600; device allowance exhausted".into())).ok().unwrap();
        call.dropped.await.unwrap();
        let body = to_bytes(response.into_body(), 65536).await.unwrap();
        assert!(String::from_utf8_lossy(&body).contains("device allowance exhausted"));
        let model = if messages { MESSAGE_MODEL } else { RESPONSE_MODEL };
        available(&f.state.gate, &f.key(0, model)).await;
        tokio::time::advance(Duration::from_secs(601)).await;
        tokio::task::yield_now().await;
        f.no_call();
    }
}


#[tokio::test(start_paused = true)]
async fn capacity_wait_is_bounded_and_never_waits_for_upstream_refusal() {
    for local in [false, true] {
        let mut f = Fixture::new();
        for account in &mut f.accounts {
            account.cooling_until_ms = Some(framer_core::rotation::now_ms() + 600_000);
            f.state.state.upsert(account).await.unwrap();
        }
        let mut budget = crate::account_wait::MAX_WAIT;
        let started = tokio::time::Instant::now();
        let wait = crate::account_wait::WaitFor::after_attempt(local, true);
        let result = crate::account_wait::pick(&f.state, &[], wait, &mut budget).await;
        assert!(matches!(result.result, Err(ApiError::AccountCooling { .. })));
        assert!(result.cleanup_confirmed);
        assert_eq!(started.elapsed(), if local { crate::account_wait::MAX_WAIT }
            else { Duration::ZERO });
        assert_eq!(budget, if local { Duration::ZERO } else { crate::account_wait::MAX_WAIT });
        f.no_call();
    }
}

#[tokio::test(flavor = "current_thread")]
async fn unbound_stream_distinguishes_deferred_selection_from_an_admitted_account() {
    for messages in [false, true] {
        let mut f = Fixture::new();
        f.accounts[1].cooling_until_ms = Some(framer_core::rotation::now_ms() + 600_000);
        f.state.state.upsert(&f.accounts[1]).await.unwrap();
        let response = chain(f.state.clone(), HeaderMap::new(), messages, true).await.unwrap();
        f.accounts[0].cooling_until_ms = Some(framer_core::rotation::now_ms() + 600_000);
        f.accounts[1].cooling_until_ms = None;
        for account in &f.accounts { f.state.state.upsert(account).await.unwrap(); }
        let first = f.state.state.get(f.accounts[0].id).await.unwrap().unwrap();
        // Messages defers admission until streaming starts; Responses can cool
        // after initial selection while waiting for the model gate. Both must
        // pick the now-healthy peer before sending any content.
        let call = f.next().await;
        assert_eq!(call.account, f.accounts[1].id, "messages={messages}");
        call.reply.send(Ok(success(if messages { "messages" } else { "responses" })))
            .ok().unwrap();
        delivered(response).await;
        call.dropped.await.unwrap();
        assert_eq!(f.state.state.get(first.id).await.unwrap().unwrap().cooling_until_ms,
            first.cooling_until_ms);
        f.no_call();
    }
}

async fn occupied(gate: &ConcurrencyGate, key: &str) {
    assert!(gate.acquire_now(key).await.is_none(), "active attempt lost its slot");
}
async fn available(gate: &ConcurrencyGate, key: &str) {
    let permit = tokio::time::timeout(WAIT, gate.acquire(key)).await.expect("slot leaked");
    drop(permit);
}
fn proxy(body: mpsc::Receiver<Result<Bytes, String>>, content_type: &str) -> Reply {
    Reply::Proxy(ProxyResponse {
        status: 200, content_type: content_type.into(), queue_wait_ms: 0,
        stream: Box::pin(futures::stream::unfold(body, |mut rx| async {
            rx.recv().await.map(|item| (item, rx))
        })),
    })
}
async fn paid(state: AppState, id: String, stream: bool) -> Result<Response, ApiError> {
    crate::compat_forward(state, "openai-responses", Some(id), None,
        json!({"model":RESPONSE_MODEL,"input":"fixture","stream":stream})).await
}
async fn chain(state: AppState, headers: HeaderMap, messages: bool, stream: bool) -> Result<Response, ApiError> {
    if messages {
        crate::free_complete(state, &headers, ClientDialect::AnthropicMessages,
            json!({"model":MESSAGE_MODEL,"messages":[{"role":"user","content":"fixture"}],"stream":stream})).await
    } else {
        crate::native_complete(state, &headers, ClientDialect::OpenAiResponses,
            json!({"model":RESPONSE_MODEL,"input":"fixture","stream":stream}), stream, false, Some(64)).await
    }
}
async fn delivered(response: Response) {
    let body = tokio::time::timeout(WAIT, to_bytes(response.into_body(), 65536)).await.unwrap().unwrap();
    assert!(String::from_utf8_lossy(&body).contains("fixture"), "completion content lost");
}

#[tokio::test(flavor = "current_thread")]
async fn unbound_queued_request_uses_ready_peer_after_account_cools() {
    for messages in [false, true] {
        for stream in [false, true] {
            let mut f = Fixture::new();
            let model = if messages { MESSAGE_MODEL } else { RESPONSE_MODEL };
            f.accounts[1].cooling_until_ms = Some(framer_core::rotation::now_ms() + 600_000);
            f.state.state.upsert(&f.accounts[1]).await.unwrap();
            let held = f.state.gate.acquire(&f.key(0, model)).await;
            let mut task = AbortOnDrop(tokio::spawn(chain(
                f.state.clone(), HeaderMap::new(), messages, stream)));
            f.loaded(0).await;
            f.accounts[0].cooling_until_ms = Some(framer_core::rotation::now_ms() + 600_000);
            f.accounts[1].cooling_until_ms = None;
            for account in &f.accounts { f.state.state.upsert(account).await.unwrap(); }
            let before = serde_json::to_value(f.state.state.list().await.unwrap()).unwrap();
            drop(held);
            queued_peer_completion(&mut f, &mut task, stream, model).await;
            assert_eq!(serde_json::to_value(f.state.state.list().await.unwrap()).unwrap(), before);
        }
    }
}

#[tokio::test(flavor = "current_thread")]
async fn paid_unpolled_stream_keeps_slot_and_drop_cancels_only_its_producer() {
    let mut f = Fixture::new();
    let key = f.key(0, RESPONSE_MODEL);
    let other_key = f.key(1, RESPONSE_MODEL);
    let other = f.state.gate.acquire(&other_key).await;
    let response = paid(f.state.clone(), f.accounts[0].id.0.to_string(), true).await.unwrap();
    let call = f.next().await;
    assert_eq!(call.kind, "proxy");
    occupied(&f.state.gate, &key).await;
    drop(response);
    tokio::time::timeout(WAIT, call.dropped).await.unwrap().unwrap();
    assert!(call.reply.is_closed());
    available(&f.state.gate, &key).await;
    occupied(&f.state.gate, &other_key).await;
    drop(other);
    available(&f.state.gate, &other_key).await;
}

#[tokio::test(flavor = "current_thread")]
async fn paid_finished_producer_retains_slot_until_body_eof() {
    let mut f = Fixture::new();
    let key = f.key(0, RESPONSE_MODEL);
    let response = paid(f.state.clone(), f.accounts[0].id.0.to_string(), true).await.unwrap();
    let call = f.next().await;
    let (tx, rx) = mpsc::channel(2);
    assert!(call.reply.send(Ok(proxy(rx, "text/event-stream"))).is_ok());
    tokio::time::timeout(WAIT, call.dropped).await.unwrap().unwrap();
    occupied(&f.state.gate, &key).await;
    let mut body = response.into_body().into_data_stream();
    const FIRST: &[u8] = b"data: {\"fixture\":\"caf\xc3\xa9\"}\n\n";
    tx.send(Ok(Bytes::from_static(FIRST))).await.unwrap();
    assert_eq!(body.next().await.unwrap().unwrap().as_ref(), FIRST);
    occupied(&f.state.gate, &key).await;
    tx.send(Ok(Bytes::from_static(b"data: [DONE]\n\n"))).await.unwrap();
    drop(tx);
    assert_eq!(body.next().await.unwrap().unwrap().as_ref(), b"data: [DONE]\n\n");
    assert!(body.next().await.is_none());
    available(&f.state.gate, &key).await;
}

#[tokio::test(flavor = "current_thread")]
async fn paid_body_error_or_unpolled_drop_returns_the_slot() {
    for body_error in [false, true] {
        let mut f = Fixture::new();
        let key = f.key(0, RESPONSE_MODEL);
        let response = paid(f.state.clone(), f.accounts[0].id.0.to_string(), true).await.unwrap();
        let call = f.next().await;
        let (tx, rx) = mpsc::channel(1);
        assert!(call.reply.send(Ok(proxy(rx, "text/event-stream"))).is_ok());
        tokio::time::timeout(WAIT, call.dropped).await.unwrap().unwrap();
        occupied(&f.state.gate, &key).await;
        if body_error {
            tx.send(Err("fixture body failure".into())).await.unwrap();
            assert!(tokio::time::timeout(WAIT, to_bytes(response.into_body(), 1024)).await.unwrap().is_err());
        } else {
            drop(response);
        }
        tokio::time::timeout(WAIT, tx.closed()).await.unwrap();
        available(&f.state.gate, &key).await;
    }
}

#[tokio::test(flavor = "current_thread")]
async fn paid_nonstreaming_raw_body_keeps_slot_after_handler_return() {
    let mut f = Fixture::new();
    let key = f.key(0, RESPONSE_MODEL);
    let mut task = AbortOnDrop(tokio::spawn(paid(f.state.clone(), f.accounts[0].id.0.to_string(), false)));
    let call = f.next().await;
    let (tx, rx) = mpsc::channel(1);
    assert!(call.reply.send(Ok(proxy(rx, "application/json"))).is_ok());
    let response = tokio::time::timeout(WAIT, &mut task).await.unwrap().unwrap().unwrap();
    occupied(&f.state.gate, &key).await;
    tx.send(Ok(Bytes::from_static(b"{\"fixture\":true}"))).await.unwrap();
    drop(tx);
    assert_eq!(to_bytes(response.into_body(), 1024).await.unwrap().as_ref(), b"{\"fixture\":true}");
    available(&f.state.gate, &key).await;
}

#[tokio::test(flavor = "current_thread")]
async fn paid_nonstreaming_raw_body_releases_slot_at_eof() {
    let mut f = Fixture::new();
    let key = f.key(0, RESPONSE_MODEL);
    let mut task = AbortOnDrop(tokio::spawn(paid(
        f.state.clone(), f.accounts[0].id.0.to_string(), false)));
    let call = f.next().await;
    let (tx, rx) = mpsc::channel(1);
    assert!(call.reply.send(Ok(proxy(rx, "application/json"))).is_ok());
    let response = tokio::time::timeout(WAIT, &mut task).await.unwrap().unwrap().unwrap();
    let mut body = response.into_body().into_data_stream();
    occupied(&f.state.gate, &key).await;
    tx.send(Ok(Bytes::from_static(b"{\"fixture\":true}"))).await.unwrap();
    drop(tx);
    assert_eq!(body.next().await.unwrap().unwrap().as_ref(), b"{\"fixture\":true}");
    assert!(body.next().await.is_none());
    available(&f.state.gate, &key).await;
}

#[tokio::test(flavor = "current_thread")]
async fn messages_exhausted_retry_releases_its_only_slot() {
    let mut f = Fixture::new();
    let key = f.key(0, MESSAGE_MODEL);
    let mut task = AbortOnDrop(tokio::spawn(chain(f.state.clone(), f.headers(0), true, false)));
    let first = f.next().await;
    occupied(&f.state.gate, &key).await;
    first.reply.send(Err("retryable 429".into())).ok().unwrap();
    let response = tokio::time::timeout(WAIT, &mut task).await.unwrap().unwrap().unwrap();
    let body = to_bytes(response.into_body(), 65536).await.unwrap();
    assert!(String::from_utf8_lossy(&body).contains("retryable 429"));
    available(&f.state.gate, &key).await;
    assert!(f.state.wave.check().is_none());
    f.no_call();
}

async fn queued_peer_completion(f: &mut Fixture,
    task: &mut AbortOnDrop<Result<Response, ApiError>>, stream: bool, model: &str) {
    let mut response = if stream {
        Some(tokio::time::timeout(WAIT, &mut *task).await.unwrap().unwrap().unwrap())
    } else { None };
    let call = tokio::select! {
        call = tokio::time::timeout(WAIT, f.calls.recv()) =>
            call.expect("ready peer was not reached").expect("fixture call channel closed"),
        result = &mut *task, if !stream => {
            panic!("request returned before using the ready peer: {:?}",
                result.unwrap().map(|response| response.status()));
        }
    };
    assert_eq!(call.account, f.accounts[1].id);
    f.loaded(1).await;
    available(&f.state.gate, &f.key(0, model)).await;
    occupied(&f.state.gate, &f.key(1, model)).await;
    call.reply.send(Ok(success(call.kind))).ok().unwrap();
    if response.is_none() {
        response = Some(tokio::time::timeout(WAIT, &mut *task).await.unwrap().unwrap().unwrap());
    }
    delivered(response.unwrap()).await;
    available(&f.state.gate, &f.key(1, model)).await;
    // EOF precedes the producer's final output-disposal join.
    tokio::time::timeout(WAIT, async {
        while f.state.serving_accounts.residents.snapshot().unwrap().residents
            .iter().any(|slot| slot.users != 0) { tokio::task::yield_now().await; }
    }).await.expect("request residency did not settle after body disposal");
    assert!(f.state.serving_accounts.residents.snapshot().unwrap().residents
        .iter().all(|slot| slot.users == 0 && !slot.uncertain));
    f.no_call();
    assert!(f.loads.try_recv().is_err());
}

async fn failure_keeps_identity(messages: bool, stream: bool, cancel_wait: bool) {
    let mut f = Fixture::new();
    f.accounts[1].consecutive_failures = 3;
    f.state.state.upsert(&f.accounts[1]).await.unwrap();
    let model = if messages { MESSAGE_MODEL } else { RESPONSE_MODEL };
    let first_key = f.key(0, model);
    let next_key = f.key(1, model);
    let destination = f.state.gate.acquire(&next_key).await;
    let mut task = AbortOnDrop(tokio::spawn(chain(f.state.clone(), f.headers(0), messages, stream)));
    let first = f.next().await;
    assert_eq!(first.account, f.accounts[0].id);
    f.loaded(0).await;
    occupied(&f.state.gate, &first_key).await;
    first.reply.send(Err("account-cooling:1".into())).ok().unwrap();
    let response = tokio::time::timeout(WAIT, &mut task).await.unwrap().unwrap().unwrap();
    assert_eq!(response.status().as_u16(), if stream { 200 } else { 503 });
    if cancel_wait { drop(response); } else {
        let bytes = to_bytes(response.into_body(), 65536).await.unwrap();
        assert!(String::from_utf8_lossy(&bytes).contains("saved account cooldown"));
    }
    available(&f.state.gate, &first_key).await;
    occupied(&f.state.gate, &next_key).await;
    f.no_call();
    assert!(f.loads.try_recv().is_err());
    drop(destination);
    available(&f.state.gate, &next_key).await;
}

#[tokio::test(flavor = "current_thread")]
async fn messages_streaming_failure_does_not_charge_another_workspace() {
    failure_keeps_identity(true, true, false).await;
}
#[tokio::test(flavor = "current_thread")]
async fn responses_streaming_failure_does_not_charge_another_workspace() {
    failure_keeps_identity(false, true, false).await;
}
#[tokio::test(flavor = "current_thread")]
async fn messages_nonstreaming_failure_does_not_charge_another_workspace() {
    failure_keeps_identity(true, false, false).await;
}
#[tokio::test(flavor = "current_thread")]
async fn responses_nonstreaming_failure_does_not_charge_another_workspace() {
    failure_keeps_identity(false, false, false).await;
}
#[tokio::test(flavor = "current_thread")]
async fn discarding_failed_response_cannot_start_a_late_attempt() {
    for (messages, stream) in [(true, true), (true, false), (false, true), (false, false)] {
        failure_keeps_identity(messages, stream, true).await;
    }
}

#[tokio::test(flavor = "current_thread")]
async fn cancelling_active_stream_retains_slot_until_its_current_call_returns() {
    for (messages, stream) in [(true, true), (true, false), (false, true), (false, false)] {
        let mut f = Fixture::new();
        let key = f.key(0, if messages { MESSAGE_MODEL } else { RESPONSE_MODEL });
        let mut task = AbortOnDrop(tokio::spawn(chain(f.state.clone(), f.headers(0), messages, stream)));
        let response = if stream {
            Some(tokio::time::timeout(WAIT, &mut task).await.unwrap().unwrap().unwrap())
        } else { None };
        let call = f.next().await;
        occupied(&f.state.gate, &key).await;
        drop(response);
        if stream {
            occupied(&f.state.gate, &key).await;
            assert!(!call.reply.is_closed());
            call.reply.send(Ok(success(call.kind))).ok().unwrap();
        } else {
            task.0.abort();
            assert!((&mut task).await.unwrap_err().is_cancelled());
            assert!(call.reply.is_closed());
        }
        tokio::time::timeout(WAIT, call.dropped).await.unwrap().unwrap();
        available(&f.state.gate, &key).await;
        f.no_call();
    }
}

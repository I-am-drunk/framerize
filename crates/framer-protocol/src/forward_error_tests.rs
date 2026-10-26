use crate::convert::{error_json, ClientDialect, ResponseEncoder};
use crate::response_stream::collect;
use crate::forward_error::{ErrorClass, ErrorRecipe, ForwardError, PolicyError, ControlError};
use crate::forward::{AuthorizedForward, ChainControl, ChainLimits, ForwardReply, PolicyDiag};
use crate::responses_sink::{OutputError, ResponsesSink, ValidatedChunk};
use serde_json::{json, Value};
use std::{collections::VecDeque, sync::{Mutex, atomic::{AtomicUsize, Ordering}}};

struct Script {
    replies: Mutex<VecDeque<(u64, Result<Value, String>)>>,
    calls: Mutex<Vec<Value>>,
    callbacks: AtomicUsize,
    diagnostics: Mutex<Vec<PolicyDiag>>,
}
impl Script {
    fn new(replies: Vec<(u64, Result<Value, String>)>) -> Self {
        Self { replies: Mutex::new(replies.into()), calls: Mutex::new(Vec::new()),
            callbacks: AtomicUsize::new(0), diagnostics: Mutex::new(Vec::new()) }
    }
}

impl ChainControl for Script {
    fn is_cancelled(&self) -> bool { false }
    fn on_429(&self) { self.callbacks.fetch_add(1, Ordering::SeqCst); }
    fn diag(&self, event: PolicyDiag) { self.diagnostics.lock().unwrap().push(event); }
}
#[async_trait::async_trait]
impl AuthorizedForward for Script {
    fn category(&self) -> &str { "translation" }
    async fn forward(&self, step: &str, payload: Value, _: u32) -> Result<ForwardReply, String> {
        assert_eq!(step, "translate");
        self.calls.lock().unwrap().push(payload);
        let (seconds, reply) = self.replies.lock().unwrap().pop_front().expect("extra Forward");
        tokio::time::advance(std::time::Duration::from_secs(seconds)).await;
        reply.map(|body| ForwardReply { status: 200, content_type: "application/json".into(),
            body: body.to_string().into(), queue_wait_ms: 0 })
    }
}

struct Accept;
#[async_trait::async_trait]
impl ResponsesSink for Accept {
    async fn emit(&self, chunk: ValidatedChunk) -> Result<ValidatedChunk, OutputError> { Ok(chunk) }
}
fn limits() -> ChainLimits {
    ChainLimits { chunk_tokens: 600, max_chunks: 4, continue_prompt: "continue".into(),
        max_total_tokens: Some(4096), metered_fallback: true }
}
fn answer(partial: bool) -> Value {
    json!({"id":"fixed", "created_at":1, "status":if partial { "incomplete" } else { "completed" },
        "incomplete_details":if partial { json!({"reason":"max_output_tokens"}) } else { Value::Null },
        "output":[{"type":"message", "role":"assistant",
            "content":[{"type":"output_text","text":"answer"}]}],
        "usage":{"input_tokens":1,"output_tokens":10}})
}
fn busy() -> (u64, Result<Value, String>) { (0, Err("retryable 429: busy".into())) }

#[tokio::test(start_paused = true)]
async fn tenth_saturation_halves_but_direct_retry_429_stops() {
    for success in [true, false] {
        let mut replies: Vec<_> = (0..10).map(|_| busy()).collect();
        replies.push(if success { (0, Ok(answer(false))) } else { busy() });
        let forward = Script::new(replies);
        let result = crate::responses::complete_long_typed(
            &forward, json!({"input":"fixture"}), &limits(), &Accept).await;
        assert_eq!(result.is_ok(), success);
        if let Err(error) = result { assert_eq!(error.to_string(), "retryable 429: busy"); }
        assert_eq!(forward.callbacks.load(Ordering::SeqCst), 9);
        let calls = forward.calls.lock().unwrap();
        assert_eq!(calls.len(), 11);
        assert_eq!(calls[0]["max_output_tokens"], 600);
        assert_eq!(calls[10]["max_output_tokens"], 300);
        assert!(calls.iter().all(|call| call["stream"] == false));
    }
}

#[tokio::test(start_paused = true)]
async fn saturation_budget_belongs_to_whole_invocation() {
    let mut replies: Vec<_> = (0..8).map(|_| busy()).collect();
    let mut partial = answer(true);
    partial["usage"]["output_tokens"] = json!(600);
    replies.push((0, Ok(partial)));
    replies.extend([busy(), busy(), (0, Ok(answer(false)))]);
    let forward = Script::new(replies);
    let result = crate::responses::complete_long_typed(
        &forward, json!({"input":"fixture"}), &limits(), &Accept).await.unwrap();
    assert_eq!(result.chunks, 2);
    assert_eq!(forward.callbacks.load(Ordering::SeqCst), 9);
    let calls = forward.calls.lock().unwrap();
    assert_eq!(calls.len(), 12);
    assert_eq!(calls[11]["max_output_tokens"].as_u64().unwrap() * 2,
        calls[9]["max_output_tokens"].as_u64().unwrap());
    assert!(forward.replies.lock().unwrap().is_empty());
}

struct HeldAck {
    inner: Script, applied: AtomicUsize,
    started: tokio::sync::Notify, ack: tokio::sync::Notify,
}
impl ChainControl for HeldAck {
    fn is_cancelled(&self) -> bool { false }
    fn on_429(&self) { panic!("typed effect bypassed") }
}
#[async_trait::async_trait]
impl AuthorizedForward for HeldAck {
    fn category(&self) -> &str { "translation" }
    async fn saturated(&self) -> Result<(), ControlError> {
        self.applied.fetch_add(1, Ordering::SeqCst);
        self.started.notify_one();
        self.ack.notified().await;
        Ok(())
    }
    async fn forward(&self, step: &str, payload: Value, index: u32) -> Result<ForwardReply, String> {
        self.inner.forward(step, payload, index).await
    }
}

#[tokio::test(start_paused = true)]
async fn saturation_ack_precedes_the_next_forward() {
    let forward = HeldAck { inner: Script::new(vec![busy(), (0, Ok(answer(false)))]),
        applied: AtomicUsize::new(0), started: tokio::sync::Notify::new(),
        ack: tokio::sync::Notify::new() };
    let cfg = limits();
    let future = crate::responses::complete_long_typed(
        &forward, json!({"input":"fixture"}), &cfg, &Accept);
    tokio::pin!(future);
    tokio::select! {
        _ = &mut future => panic!("policy completed before effect acknowledgment"),
        _ = forward.started.notified() => {}
    }
    assert_eq!(forward.applied.load(Ordering::SeqCst), 1);
    assert_eq!(forward.inner.calls.lock().unwrap().len(), 1);
    assert!(futures::poll!(&mut future).is_pending());
    forward.ack.notify_one();
    assert_eq!(future.await.unwrap().status, "completed");
    assert_eq!(forward.inner.calls.lock().unwrap().len(), 2);
}

struct BrokenControl { calls: AtomicUsize, at: usize, ack: bool }
impl ChainControl for BrokenControl {
    fn is_cancelled(&self) -> bool { false }
    fn on_429(&self) { panic!("an acknowledged effect must not use the legacy callback") }
}
#[async_trait::async_trait]
impl AuthorizedForward for BrokenControl {
    fn category(&self) -> &str { "translation" }
    async fn saturated(&self) -> Result<(), ControlError> { Err(ControlError) }
    async fn forward(&self, _: &str, _: Value, _: u32) -> Result<ForwardReply, String> {
        panic!("typed path must not discard control failure provenance")
    }
    async fn forward_typed(&self, _: &str, _: Value, _: u32) -> Result<ForwardReply, PolicyError> {
        let call = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
        assert!(call <= self.at, "Forward after control failure");
        if call == self.at && !self.ack { return Err(ControlError.into()); }
        Err(ForwardError::from_legacy(if self.ack {
            "retryable 429: busy".into()
        } else { "fixture transport closed".into() }).into())
    }
}

#[tokio::test(start_paused = true)]
async fn control_failure_bypasses_initial_halved_and_metered_retry_branches() {
    for at in 1..=3 {
        let forward = BrokenControl { calls: AtomicUsize::new(0), at, ack: false };
        let error = crate::responses::complete_long_typed(
            &forward, json!({"input":"fixture"}), &limits(), &Accept).await.unwrap_err();
        assert!(matches!(error, PolicyError::Control(_)));
        assert_eq!(forward.calls.load(Ordering::SeqCst), at);
    }
}

#[tokio::test(start_paused = true)]
async fn failed_saturation_ack_stops_before_backoff_or_next_forward() {
    let forward = BrokenControl { calls: AtomicUsize::new(0), at: 1, ack: true };
    let started = tokio::time::Instant::now();
    let error = crate::responses::complete_long_typed(
        &forward, json!({"input":"fixture"}), &limits(), &Accept).await.unwrap_err();
    assert!(matches!(error, PolicyError::Control(_)));
    assert_eq!(forward.calls.load(Ordering::SeqCst), 1);
    assert_eq!(started.elapsed(), std::time::Duration::ZERO);
}

#[tokio::test(start_paused = true)]
async fn default_saturation_keeps_one_and_four_callback_contracts() {
    for expired in [false, true] {
        let replies = if expired { (0..4).map(|_| (80, busy().1)).collect() }
            else { vec![busy(), (0, Ok(answer(false)))] };
        let forward = Script::new(replies);
        let result = crate::responses::complete_long_typed(
            &forward, json!({"input":"fixture"}), &limits(), &Accept).await;
        assert_eq!(result.is_err(), expired);
        assert_eq!(forward.calls.lock().unwrap().len(), if expired { 4 } else { 2 });
        assert_eq!(forward.callbacks.load(Ordering::SeqCst), if expired { 4 } else { 1 });
        if let Err(error) = result {
            assert_eq!(error.to_string(),
                "chain exceeded 300s active-time budget after 0 chunks (excluding local cooldown waits), retry the request");
        }
    }
}

#[test]
fn legacy_wrapper_rendering_selects_status_and_effective_overflow() {
    let raw = r#"retryable 429 on /fixture: {"error":{"message":"busy","type":"api_error"},"detail":"maximum context length"}"#;
    for (recipe, status, code) in [
        (ErrorRecipe::Direct, 429, "api_error"),
        (ErrorRecipe::SaturationExhausted { chunk: 0, retries: 9 }, 500, "context_length_exceeded"),
        (ErrorRecipe::FallbackOff { chunk: 0 }, 429, "api_error"),
    ] {
        let error = ForwardError::from_legacy(raw.into()).with_recipe(recipe);
        let (actual_status, shaped) = error_json(ClientDialect::OpenAiResponses, &error.to_string());
        assert_eq!(actual_status, status);
        assert_eq!(shaped["error"]["code"], code);
    }
    let error = ForwardError::from_legacy("retryable 429: risk-refused".into())
        .with_recipe(ErrorRecipe::SaturationExhausted { chunk: 0, retries: 9 });
    assert_eq!(error_json(ClientDialect::OpenAiResponses, &error.to_string()).0, 403);
}

#[test]
fn typed_retry_classes_preserve_legacy_sentinel_boundaries() {
    for text in ["unauthorized", "credits-exhausted", "upstream 403: denied",
        "retryable 400", "retryable 429: busy"] {
        let error = ForwardError::from_legacy(text.into());
        assert!(!error.allows_transport_retry(0.0));
        assert!(!error.allows_transport_retry(90.0));
    }
    for text in ["fixture transport closed", "retryable 503", "client cancelled",
        "wrapped unauthorized", "account-cooling:5"] {
        assert!(ForwardError::from_legacy(text.into()).allows_transport_retry(0.0));
    }
    let risk = ForwardError::from_legacy("risk-refused".into());
    assert!(!risk.allows_transport_retry(8.0));
    assert!(risk.allows_transport_retry(8.1));
}

#[tokio::test(start_paused = true)]
async fn diagnostics_report_numeric_backoff_and_completion_without_raw_error() {
    let forward = Script::new(vec![
        (0, Err("retryable 429: private-fixture-sentinel".into())),
        (0, Ok(answer(false))),
    ]);
    crate::responses::complete_long_typed(
        &forward, json!({"input":"fixture"}), &limits(), &Accept).await.unwrap();
    let events = forward.diagnostics.lock().unwrap();
    assert!(matches!(events[0], PolicyDiag::ChunkStart { chunk: 0, max_tokens: 600 }));
    assert!(matches!(events[1], PolicyDiag::Backoff { chunk: 0, attempt: 1, wait_ms: 0 }));
    assert!(matches!(events[2], PolicyDiag::ChunkDone { chunk: 0, tokens: 10, .. }));
    assert_eq!(events.len(), 3);
    assert!(!format!("{events:?}").contains("private-fixture-sentinel"));
}

#[tokio::test(start_paused = true)]
async fn typed_cooldown_wait_uses_facts_without_reparsing_display() {
    let cfg = Script::new(Vec::new());
    let calls = AtomicUsize::new(0);
    let mut wait = crate::cooldown_wait::CooldownWait::default();
    let active = wait.timer();
    let wall = tokio::time::Instant::now();
    let result: Result<(), PolicyError> = wait.run_typed(&cfg, true, || async {
        calls.fetch_add(1, Ordering::SeqCst);
        Err(ForwardError::from_facts(ErrorClass::Cooldown, Some(10), 1).unwrap().into())
    }).await;
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    assert_eq!(result.unwrap_err().cooldown().unwrap().retry_after_secs, 10);
    assert_eq!(wall.elapsed(), std::time::Duration::from_secs(10));
    assert_eq!(wait.elapsed(active), std::time::Duration::ZERO);
}

struct ClosedSink;
#[async_trait::async_trait]
impl ResponsesSink for ClosedSink {
    async fn emit(&self, _: ValidatedChunk) -> Result<ValidatedChunk, OutputError> {
        Err(OutputError::Closed)
    }
}
#[tokio::test(start_paused = true)]
async fn legacy_adapter_preserves_output_error_variant_without_retry() {
    let forward = Script::new(vec![(0, Ok(answer(false)))]);
    let result = crate::responses::complete_long_with_sink(
        &forward, json!({"input":"fixture"}), &limits(), &ClosedSink).await;
    assert!(matches!(result,
        Err(crate::responses_sink::ResponseError::Output(OutputError::Closed))));
    assert_eq!(forward.calls.lock().unwrap().len(), 1);
}

#[test]
fn typed_retry_facts_do_not_depend_on_redacted_display() {
    let error = ForwardError::from_facts(ErrorClass::Retryable429, None, 1).unwrap();
    assert_eq!(error.to_string(), "upstream operation failed");
    assert!(error.is_saturated());
    assert!(!error.allows_transport_retry(100.0));
    let exhausted = error.with_recipe(ErrorRecipe::SaturationExhausted { chunk: 2, retries: 9 });
    assert!(exhausted.allows_transport_retry(0.0));
    assert!(!exhausted.is_saturated());
    assert!(ForwardError::from_facts(ErrorClass::Other, Some(1), 2).is_err());
    assert!(ForwardError::from_facts(ErrorClass::Other, None, 0).is_err());
    assert!(ForwardError::from_facts(ErrorClass::Cooldown, Some(0), 1).is_err());
}

#[test]
fn preclassified_failure_keeps_kernel_code_under_redacted_display() {
    let (_, mut shaped) = error_json(ClientDialect::OpenAiResponses,
        "maximum context length: private-fixture-sentinel");
    shaped["error"]["message"] = "upstream operation failed".into();
    let mut encoder = ResponseEncoder::new("fixture", "fixed-id");
    let frames = collect(|writer| encoder.fail_shaped(writer, &shaped["error"]));
    assert_eq!(frames.len(), 1);
    let data = frames[0].split_once("\ndata: ").unwrap().1.trim();
    let event: serde_json::Value = serde_json::from_str(data).unwrap();
    assert_eq!(event["response"]["error"]["message"], "upstream operation failed");
    assert_eq!(event["response"]["error"]["type"], "api_error");
    assert_eq!(event["response"]["error"]["code"], "context_length_exceeded");
    assert_eq!(event["sequence_number"], 0);
    assert!(!frames[0].contains("private-fixture-sentinel"));
    let (_, reparsed) = error_json(ClientDialect::OpenAiResponses, "upstream operation failed");
    assert_eq!(reparsed["error"]["code"], "api_error");
}

fn failure(message: &str, preclassified: bool) -> Vec<String> {
    let mut encoder = ResponseEncoder::new("fixture", "fixed-id");
    collect(|writer| {
        encoder.heartbeat(writer)?;
        if preclassified {
            let (_, shaped) = error_json(ClientDialect::OpenAiResponses, message);
            encoder.fail_shaped(writer, &shaped["error"])?;
        } else {
            encoder.fail(writer, message)?;
        }
        encoder.heartbeat(writer)
    })
}

#[test]
fn preclassified_failure_preserves_legacy_bytes_and_sequence() {
    for message in ["risk-refused", "ordinary failure", "maximum context length",
        r#"upstream 429: {"error":{"message":"busy","type":"api_error"}}"#] {
        let frames = failure(message, false);
        assert_eq!(frames, failure(message, true));
        assert!(frames[1].contains("\"sequence_number\":1"));
        assert!(frames[2].contains("\"sequence_number\":2"));
    }
}

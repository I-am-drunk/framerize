use super::*;
use crate::{forward::{AuthorizedForward, ChainControl, ChainLimits, ForwardReply}, responses};
use async_trait::async_trait;
use serde_json::json;
use std::{collections::VecDeque, sync::{Arc, Mutex, atomic::{AtomicUsize, Ordering}}};
use tokio::sync::Notify;

struct Forward { replies: Mutex<VecDeque<Value>>, inputs: Mutex<Vec<Value>>, calls: AtomicUsize }
impl ChainControl for Forward {
    fn is_cancelled(&self) -> bool { false }
    fn on_429(&self) { panic!("unexpected saturation") }
}
#[async_trait]
impl AuthorizedForward for Forward {
    fn category(&self) -> &str { "translation" }
    async fn forward(&self, _: &str, input: Value, _: u32) -> Result<ForwardReply, String> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.inputs.lock().unwrap().push(input);
        Ok(ForwardReply { status: 200, content_type: "application/json".into(),
            body: self.replies.lock().unwrap().pop_front().expect("unexpected retry").to_string().into(), queue_wait_ms: 0 })
    }
}
fn limits() -> ChainLimits {
    ChainLimits { chunk_tokens: 600, max_chunks: 2, continue_prompt: String::new(),
        max_total_tokens: Some(64), metered_fallback: false }
}
fn reply(partial: bool) -> Value {
    json!({"id":"upstream", "created_at":1, "status":if partial {"incomplete"} else {"completed"},
        "incomplete_details":{"reason":"max_output_tokens"},
        "output":[{"type":"message","role":"assistant","content":[{"type":"output_text","text":"part"}]}],
        "usage":{"input_tokens":1,"output_tokens":16}})
}
fn forward() -> Arc<Forward> {
    Arc::new(Forward { replies: Mutex::new([reply(true), reply(false)].into()),
        inputs: Mutex::new(Vec::new()), calls: AtomicUsize::new(0) })
}
struct Held { entered: Notify, release: Notify, calls: AtomicUsize }
#[async_trait]
impl ResponsesSink for Held {
    async fn emit(&self, chunk: ValidatedChunk) -> Result<ValidatedChunk, OutputError> {
        if self.calls.fetch_add(1, Ordering::SeqCst) == 0 {
            self.entered.notify_one(); self.release.notified().await;
        }
        Ok(chunk)
    }
}

#[tokio::test(start_paused = true)]
async fn continuation_waits_for_emission_and_keeps_original_history() {
    let forward = forward();
    let sink = Arc::new(Held { entered: Notify::new(), release: Notify::new(), calls: AtomicUsize::new(0) });
    let (f, s) = (forward.clone(), sink.clone());
    let task = tokio::spawn(async move { responses::complete_long_with_sink(f.as_ref(),
        json!({"model":"fixture","input":"hello"}), &limits(), s.as_ref()).await });
    sink.entered.notified().await;
    assert_eq!(forward.calls.load(Ordering::SeqCst), 1);
    sink.release.notify_one();
    let result = task.await.unwrap().unwrap();
    assert_eq!(result.chunks, 2);
    assert_eq!(forward.calls.load(Ordering::SeqCst), 2);
    let inputs = forward.inputs.lock().unwrap();
    assert_eq!(inputs[1]["input"][1], reply(true)["output"][0]);
}

struct Reject;
struct CompletionReject { observed: Mutex<Vec<bool>> }
#[async_trait]
impl ResponsesSink for CompletionReject {
    async fn emit(&self, chunk: ValidatedChunk) -> Result<ValidatedChunk, OutputError> {
        self.observed.lock().unwrap().push(chunk.completes_chain());
        Err(OutputError::Cancelled)
    }
}
#[async_trait]
impl ResponsesSink for Reject {
    async fn emit(&self, _: ValidatedChunk) -> Result<ValidatedChunk, OutputError> { Err(OutputError::Encoding) }
}
#[tokio::test(start_paused = true)]
async fn output_failure_never_dispatches_or_retries_the_next_chunk() {
    let forward = forward();
    let result = responses::complete_long_with_sink(forward.as_ref(),
        json!({"model":"fixture","input":"hello"}), &limits(), &Reject).await;
    assert!(matches!(result, Err(ResponseError::Output(OutputError::Encoding))));
    assert_eq!(forward.calls.load(Ordering::SeqCst), 1);
    assert_eq!(forward.replies.lock().unwrap().len(), 1);
}
struct Slow;
#[tokio::test(start_paused = true)]
async fn completion_evidence_matches_policy_before_cancelled_publication() {
    for (partial, chunks, total, complete) in [(false, 2, 64, true),
        (true, 2, 64, false), (true, 1, 64, true), (true, 2, 16, true),
        (true, 2, 31, true), (true, 2, 32, false)] {
        let forward = forward();
        forward.replies.lock().unwrap()[0] = reply(partial);
        let sink = CompletionReject { observed: Mutex::new(Vec::new()) };
        let mut cfg = limits();
        cfg.max_chunks = chunks; cfg.max_total_tokens = Some(total);
        let result = responses::complete_long_with_sink(forward.as_ref(),
            json!({"model":"fixture","input":"hello"}), &cfg, &sink).await;
        assert!(matches!(result, Err(ResponseError::Output(OutputError::Cancelled))));
        assert_eq!(*sink.observed.lock().unwrap(), vec![complete]);
        assert_eq!(forward.calls.load(Ordering::SeqCst), 1);
    }
}
#[async_trait]
impl ResponsesSink for Slow {
    async fn emit(&self, chunk: ValidatedChunk) -> Result<ValidatedChunk, OutputError> {
        tokio::time::sleep(std::time::Duration::from_secs(301)).await; Ok(chunk)
    }
}
#[tokio::test(start_paused = true)]
async fn emission_wait_charges_active_budget_before_next_dispatch() {
    let forward = forward();
    let result = responses::complete_long_with_sink(forward.as_ref(),
        json!({"model":"fixture","input":"hello"}), &limits(), &Slow).await;
    assert!(matches!(result, Err(ResponseError::Upstream(error)) if error.contains("active-time budget")));
    assert_eq!(forward.calls.load(Ordering::SeqCst), 1);
}
struct Identity;
#[tokio::test]
async fn assembled_completion_evidence_requires_validated_result() {
    for valid in [false, true] {
        let sink = CompletionReject { observed: Mutex::new(Vec::new()) };
        let mut value = fixture_result();
        if !valid { value.status = "failed".into(); }
        let result = emit_assembled(value, &sink).await;
        if valid {
            assert!(matches!(result, Err(ResponseError::Output(OutputError::Cancelled))));
            assert_eq!(*sink.observed.lock().unwrap(), vec![true]);
        } else {
            assert!(matches!(result, Err(ResponseError::Upstream(_))));
            assert!(sink.observed.lock().unwrap().is_empty());
        }
    }
    assert!(!ValidatedChunk::validate(Vec::new(), &[], false).unwrap().completes_chain());
}
#[async_trait]
impl ResponsesSink for Identity {
    async fn emit(&self, chunk: ValidatedChunk) -> Result<ValidatedChunk, OutputError> { Ok(chunk) }
}
#[tokio::test]
async fn assembled_adapter_returns_the_original_vector_owner() {
    let mut result = fixture_result();
    let owner = result.output.as_ptr();
    result = emit_assembled(result, &Identity).await.unwrap();
    assert_eq!(result.output.as_ptr(), owner);
}
fn fixture_result() -> AssembledResponses {
    AssembledResponses { id: "upstream".into(), created_at: 1,
        output: reply(false)["output"].as_array().unwrap().clone(), status: "completed".into(),
        incomplete_details: None, usage: json!({}), service_tier: Value::Null,
        input_tokens: 1, output_tokens: 16, chunks: 1, metered_chunks: 0 }
}
#[tokio::test(start_paused = true)]
async fn legacy_callback_and_awaited_sink_keep_the_same_result() {
    let plain = forward();
    let awaited = forward();
    let seen = Mutex::new(Vec::new());
    let callback = |event| seen.lock().unwrap().push(event);
    let payload = json!({"model":"fixture","input":"hello"});
    let legacy = responses::complete_long(plain.as_ref(), payload.clone(), &limits(), Some(&callback)).await.unwrap();
    let result = responses::complete_long_with_sink(awaited.as_ref(), payload, &limits(), &Identity).await.unwrap();
    assert_eq!(crate::convert::responses_json(&legacy, "fixture", "logical"),
        crate::convert::responses_json(&result, "fixture", "logical"));
    assert_eq!(seen.lock().unwrap().len(), 2);
}
#[tokio::test]
async fn assembled_adapter_refuses_unvalidated_tool_fields() {
    let mut result = fixture_result();
    result.output = vec![json!({"type":"function_call","name":"tool","arguments":"{}"})];
    let sink = Held { entered: Notify::new(), release: Notify::new(), calls: AtomicUsize::new(0) };
    let error = emit_assembled(result, &sink).await.unwrap_err();
    assert!(matches!(error, ResponseError::Upstream(_)));
    assert_eq!(sink.calls.load(Ordering::SeqCst), 0);
}

#[test]
fn validated_adapter_checks_prior_tool_ids_and_preserves_continuation() {
    let tool = json!({"type":"function_call","call_id":"same","name":"tool","arguments":"{}"});
    assert!(ValidatedChunk::validate(vec![tool.clone()], &[tool], false).is_err());
    let output = fixture_result().output;
    let owner = output.as_ptr();
    let chunk = ValidatedChunk::validate(output, &[], true).unwrap();
    assert_eq!(chunk.items().as_ptr(), owner);
    assert!(chunk.continues_text());
    let invalid = vec![json!({"type":"message","role":"user","content":[]})];
    assert!(ValidatedChunk::validate(invalid, &[], false).is_err());
}

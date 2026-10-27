use crate::emitter::{self, OutputControl, Sink};
use framer_protocol::{convert::AssembledResponses,
    responses_sink::{OutputError, ResponsesSink, ValidatedChunk}};
use framer_worker_wire::{credit::{data_cost, Capacity, DATA_WIRE_RESERVE},
    frame::{self, Decoded, Outbox}, message::{DataKind, Message, PublicFailure, Text}, WireError};
use serde_json::json;
use std::{sync::Arc, time::Duration};
use tokio::{io::DuplexStream, task::JoinHandle};

struct Fixture {
    outbox: Outbox, peer: DuplexStream, control: Arc<OutputControl>, sink: Sink, accepted: u64,
    emitter: JoinHandle<Result<(), OutputError>>, writer: JoinHandle<Result<(), WireError>>,
}
impl Fixture {
    async fn new() -> Self {
        let (outbox, writer) = Outbox::new(1).unwrap();
        let capacity = Capacity { wire_bytes: 2 * DATA_WIRE_RESERVE, slots: 2,
            downstream_bytes: 2 * framer_worker_wire::SEGMENT_LIMIT as u64 };
        let control = OutputControl::new(capacity).unwrap();
        let (sink, emitter) = emitter::channel(7, outbox.register(7).unwrap(), control.clone(), "model", "fixed");
        let (pipe, peer) = tokio::io::duplex(97);
        let mut f = Self { outbox, peer, control, sink, accepted: 0,
            emitter: tokio::spawn(emitter.run()), writer: tokio::spawn(writer.run(pipe)) };
        for sequence in 0..2 {
            let data = f.next().await;
            assert!(matches!(&data.message, Message::Data { event: DataKind::Opening, seq, last: true, .. } if *seq == sequence));
            f.accept(data, true, true);
        }
        f
    }
    async fn next(&mut self) -> Decoded<Message> {
        tokio::time::timeout(Duration::from_secs(2), frame::read(&mut self.peer))
            .await.unwrap().unwrap().unwrap()
    }
    fn accept(&mut self, data: Decoded<Message>, dispose: bool, acknowledge: bool) {
        let Message::Data { op, seq, last, .. } = data.message else { panic!("Data expected"); };
        assert_eq!(op, 7);
        if acknowledge && last { self.accepted = seq + 1; }
        let mut cost = data_cost(data.wire_bytes);
        if !dispose { cost.slots = 0; cost.downstream_bytes = 0; }
        self.control.credit(cost, self.accepted).unwrap();
    }
    async fn close(mut self) {
        self.control.cancel();
        drop(self.sink);
        let drain = tokio::spawn(async move { tokio::io::copy(&mut self.peer, &mut tokio::io::sink()).await });
        let result = tokio::time::timeout(Duration::from_secs(2), self.emitter).await.unwrap().unwrap();
        assert!(result.is_ok() || matches!(result, Err(OutputError::Cancelled)));
        self.outbox.close();
        tokio::time::timeout(Duration::from_secs(2), self.writer).await.unwrap().unwrap().unwrap();
        drain.await.unwrap().unwrap();
        assert_eq!(self.control.jobs(), 0);
    }
}

fn chunk(value: &str) -> ValidatedChunk {
    ValidatedChunk::validate(vec![json!({"type":"opaque", "value": value})], &[], false).unwrap()
}

fn assembled(output: Vec<serde_json::Value>) -> AssembledResponses {
    AssembledResponses {
        id: "upstream".into(), created_at: 123, output, status: "completed".into(),
        incomplete_details: None, usage: json!({}), service_tier: json!(null),
        input_tokens: 1, output_tokens: 1, chunks: 1, metered_chunks: 0,
    }
}
fn public_failure() -> PublicFailure {
    PublicFailure { status: 500, message: Text::new("upstream operation failed".into()).unwrap(),
        error_type: Text::new("api_error".into()).unwrap(),
        code: Text::new("context_length_exceeded".into()).unwrap() }
}

#[tokio::test]
async fn emission_waits_for_acceptance_without_waiting_for_disposal() {
    let mut f = Fixture::new().await;
    let sink = f.sink.clone();
    let emitted = tokio::spawn(async move { sink.emit(chunk("exact")).await });
    for _ in 0..2 { let data = f.next().await; f.accept(data, false, false); }
    assert!(!emitted.is_finished());
    assert_eq!(f.control.outstanding().slots, 2);
    f.control.credit(Capacity::default(), 4).unwrap();
    let owner = emitted.await.unwrap().unwrap();
    assert_eq!(owner.items()[0]["value"], "exact");
    assert_eq!(f.control.outstanding().slots, 2);
    assert_eq!(f.control.outstanding().downstream_bytes, 32768);
    f.close().await;
}

#[tokio::test]
async fn terminal_requires_command_and_accepted_frontier() {
    let mut f = Fixture::new().await;
    let sink = f.sink.clone();
    let terminal = tokio::spawn(async move { sink.finish(assembled(vec![])).await });
    let data = f.next().await;
    assert!(matches!(&data.message, Message::Data { event: DataKind::Terminal, seq: 2, last: true, .. }));
    f.accept(data, false, false);
    assert!(!terminal.is_finished());
    f.control.credit(Capacity::default(), 3).unwrap();
    terminal.await.unwrap().unwrap();
    assert_eq!(f.control.outstanding().slots, 1);
    f.close().await;
}

#[tokio::test]
async fn cancellation_closes_blocked_staging_and_joins_without_credit() {
    let mut f = Fixture::new().await;
    let sink = f.sink.clone();
    let emitted = tokio::spawn(async move { sink.emit(chunk(&"x".repeat(6 * 1024 * 1024))).await });
    for _ in 0..2 { let data = f.next().await; f.accept(data, false, false); }
    assert_eq!(f.control.jobs(), 1);
    f.control.cancel();
    assert!(matches!(emitted.await.unwrap(), Err(OutputError::Cancelled)));
    assert_eq!(f.control.jobs(), 0);
    let (peak, jobs) = f.control.peaks();
    assert!(peak.wire_bytes <= 2 * DATA_WIRE_RESERVE);
    assert_eq!((peak.slots, peak.downstream_bytes, jobs), (2, 32768, 1));
    f.close().await;
}

#[tokio::test]
async fn typed_failure_preserves_kernel_code_without_reparsing_display() {
    let mut f = Fixture::new().await;
    let sink = f.sink.clone();
    let terminal = tokio::spawn(async move { sink.fail(public_failure()).await });
    let data = f.next().await;
    let Message::Data { event: DataKind::Terminal, bytes, seq: 2, last: true, .. } = &data.message
        else { panic!("terminal expected"); };
    let text = std::str::from_utf8(bytes.as_slice()).unwrap();
    assert!(text.contains("context_length_exceeded"));
    assert!(text.contains("upstream operation failed"));
    assert!(!text.contains("maximum context length"));
    f.accept(data, true, true);
    terminal.await.unwrap().unwrap();
    assert_eq!(f.control.outstanding(), Capacity::default());
    f.close().await;
}

#[tokio::test]
async fn credit_covering_unsent_event_ends_output() {
    let f = Fixture::new().await;
    assert!(f.control.credit(Capacity::default(), 3).is_err());
    let result = tokio::time::timeout(Duration::from_secs(2), f.emitter)
        .await.unwrap().unwrap();
    assert!(matches!(result, Err(OutputError::Closed)));
    assert_eq!(f.control.jobs(), 0);
    f.outbox.close();
    f.writer.await.unwrap().unwrap();
}

struct RunnerPeer {
    peer: DuplexStream, task: JoinHandle<Result<(), WireError>>,
    output: Vec<u8>, accepted: u64, terminal: usize, diagnostics: usize,
}
impl RunnerPeer {
    async fn new() -> Self {
        let (pipe, peer) = tokio::io::duplex(131072);
        let (read, write) = tokio::io::split(pipe);
        let task = tokio::spawn(crate::runner::run(read, write));
        let mut f = Self { peer, task, output: vec![], accepted: 0, terminal: 0, diagnostics: 0 };
        assert!(matches!(f.raw().await.message, Message::Hello { wire: 1, .. }));
        f.send(Message::Ready { generation: 11 }).await;
        f
    }
    async fn send(&mut self, message: Message) {
        frame::write(&mut self.peer, &frame::encode(&message).unwrap()).await.unwrap();
    }
    async fn raw(&mut self) -> Decoded<Message> {
        tokio::time::timeout(Duration::from_secs(2), frame::read(&mut self.peer))
            .await.unwrap().unwrap().unwrap()
    }
    async fn next(&mut self) -> Message {
        loop {
            let frame = self.raw().await;
            match frame.message {
                Message::Data { op, event, seq, bytes, last } => {
                    assert_eq!(op, 7); assert_eq!(seq, self.accepted);
                    if last { self.accepted += 1; }
                    if event == DataKind::Terminal { self.terminal += 1; }
                    self.output.extend_from_slice(bytes.as_slice());
                    drop(bytes);
                    self.send(Message::Credit { op, returned: data_cost(frame.wire_bytes),
                        accepted_event_seq: self.accepted }).await;
                }
                Message::Diag { op: 7, .. } => self.diagnostics += 1,
                message => return message,
            }
        }
    }
    async fn start(&mut self) {
        self.send(Message::Start { op: 7, logical_id: Text::new("fixed".into()).unwrap(),
            model: Text::new("model".into()).unwrap(), mode: framer_worker_wire::message::Mode::Sse,
            output: Capacity { wire_bytes: 2 * DATA_WIRE_RESERVE, slots: 2, downstream_bytes: 32768 } }).await;
        assert!(matches!(self.next().await, Message::Head { op: 7, status: 200, .. }));
    }
    async fn next_panicked(&mut self) -> Message {
        let mut returns = Vec::with_capacity(2);
        let done = loop {
            let frame = self.raw().await;
            match frame.message {
                Message::Data { op, event, seq, bytes, last } => {
                    assert_eq!(op, 7); assert_eq!(seq, self.accepted);
                    if last { self.accepted += 1; }
                    if event == DataKind::Terminal { self.terminal += 1; }
                    self.output.extend_from_slice(bytes.as_slice());
                    drop(bytes);
                    assert!(returns.len() < 2);
                    returns.push(Message::Credit { op, returned: data_cost(frame.wire_bytes),
                        accepted_event_seq: self.accepted });
                }
                Message::Diag { op: 7, .. } => self.diagnostics += 1,
                message => break message,
            }
        };
        assert!(matches!(&done, Message::Done { op: 7,
            outcome: framer_worker_wire::message::DoneOutcome::Panicked, .. }));
        for credit in returns { self.return_after_panic(credit).await; }
        done
    }
    async fn return_after_panic(&mut self, credit: Message) {
        // Done was read first. Only confirmed runner loss excuses a late return.
        if let Err(error) = frame::write(&mut self.peer, &frame::encode(&credit).unwrap()).await {
            assert!(matches!(error, WireError::Io(error)
                if error.kind() == std::io::ErrorKind::BrokenPipe));
            tokio::time::timeout(Duration::from_secs(2), async {
                while !self.task.is_finished() { tokio::task::yield_now().await; }
            }).await.unwrap();
        }
    }
    async fn invoke(&mut self, n: u32) {
        use framer_worker_wire::message::{Limits, Part, Segment};
        self.send(Message::Invoke { op: 7, n, category: Text::new("translation".into()).unwrap(),
            limits: Limits { chunk_tokens: 600, max_chunks: 1, max_total_tokens: Some(600),
                metered_fallback: false, continue_prompt: Text::new("continue".into()).unwrap() },
            request: Part { offset: 0, last: true, bytes: Segment::new(serde_json::to_vec(
                &json!({"model":"model", "input":"hello"})).unwrap()).unwrap() } }).await;
    }
    async fn reply(&mut self, n: u32, fwd: u64) {
        use framer_worker_wire::message::{Part, Segment};
        let body = json!({"id":"upstream", "created_at":123, "status":"completed",
            "output":[{"type":"opaque","value":"exact"}],
            "usage":{"input_tokens":1,"output_tokens":1}});
        self.send(Message::ForwardReply { op: 7, n, fwd, status: 200,
            content_type: Text::new("application/json".into()).unwrap(), queue_wait_ms: 73,
            body: Part { offset: 0, last: true,
                bytes: Segment::new(serde_json::to_vec(&body).unwrap()).unwrap() } }).await;
    }
    async fn end(mut self, expect_ok: bool) {
        use tokio::io::AsyncWriteExt;
        self.peer.shutdown().await.unwrap();
        let result = tokio::time::timeout(Duration::from_secs(2), self.task).await.unwrap().unwrap();
        assert_eq!(result.is_ok(), expect_ok);
    }
}

#[tokio::test]
async fn runner_result_precedes_commanded_terminal_and_done() {
    let mut f = RunnerPeer::new().await;
    f.start().await; f.invoke(1).await;
    assert!(matches!(f.next().await, Message::Forward { op: 7, n: 1, fwd: 1, .. }));
    f.reply(1, 1).await;
    assert!(matches!(f.next().await, Message::Result { op: 7, n: 1,
        outcome: framer_worker_wire::message::Outcome::Success { .. }, .. }));
    assert_eq!(f.terminal, 0);
    let diagnostics = f.diagnostics;
    f.send(Message::Finish { op: 7, n: 1 }).await;
    assert!(matches!(f.next().await, Message::Done { op: 7,
        outcome: framer_worker_wire::message::DoneOutcome::Completed, .. }));
    assert_eq!(f.terminal, 1); assert_eq!(f.diagnostics, diagnostics);
    let output = std::str::from_utf8(&f.output).unwrap();
    assert_eq!(output.matches("event: response.created\n").count(), 1);
    assert_eq!(output.matches("event: response.completed\n").count(), 1);
    assert!(output.contains("exact"));
    f.end(true).await;
}

#[tokio::test]
async fn kernel_failure_before_first_invoke_finishes_without_policy_dispatch() {
    let mut f = RunnerPeer::new().await;
    f.start().await;
    f.send(Message::Fail { op: 7, n: 0, error: public_failure() }).await;
    assert!(matches!(f.next().await, Message::Done { op: 7,
        outcome: framer_worker_wire::message::DoneOutcome::Completed, diag_dropped: 0 }));
    assert_eq!(f.terminal, 1);
    assert_eq!(f.diagnostics, 0);
    let text = std::str::from_utf8(&f.output).unwrap();
    assert!(text.contains("context_length_exceeded"));
    assert!(text.contains("upstream operation failed"));
    f.end(true).await;
}

#[tokio::test]
async fn initial_kernel_failure_refuses_an_unstarted_invocation_number() {
    let mut f = RunnerPeer::new().await;
    f.start().await;
    f.send(Message::Fail { op: 7, n: 1, error: public_failure() }).await;
    let result = tokio::time::timeout(Duration::from_secs(2), f.task).await.unwrap().unwrap();
    assert!(result.is_err());
}

#[tokio::test]
async fn runner_rebind_keeps_one_encoder_and_closed_failure_recipe() {
    use framer_worker_wire::message::{ErrorClass, Outcome, FailureTag};
    let mut f = RunnerPeer::new().await;
    f.start().await; f.invoke(1).await;
    assert!(matches!(f.next().await, Message::Forward { n: 1, fwd: 1, .. }));
    f.send(Message::ForwardError { op: 7, n: 1, fwd: 1,
        class: ErrorClass::Unauthorized, cooldown_seconds: None }).await;
    let Message::Result { outcome: Outcome::Forward { failure }, .. } = f.next().await
        else { panic!("closed failure recipe expected"); };
    assert_eq!((failure.cause.op, failure.cause.n, failure.cause.fwd), (7, 1, 1));
    assert_eq!((failure.tag, failure.chunk), (FailureTag::Direct, 0));
    assert_eq!(f.terminal, 0);
    f.invoke(2).await;
    assert!(matches!(f.next().await, Message::Forward { n: 2, fwd: 1, .. }));
    f.reply(2, 1).await;
    assert!(matches!(f.next().await, Message::Result { n: 2, outcome: Outcome::Success { .. }, .. }));
    f.send(Message::Finish { op: 7, n: 2 }).await;
    assert!(matches!(f.next().await, Message::Done { .. }));
    assert_eq!(std::str::from_utf8(&f.output).unwrap().matches("event: response.created\n").count(), 1);
    assert_eq!(f.terminal, 1);
    f.end(true).await;
}

#[tokio::test]
async fn runner_cancel_keeps_forward_joined_and_completed_output_fact() {
    use framer_worker_wire::message::{DoneOutcome, Outcome, OutputFailure};
    let mut f = RunnerPeer::new().await;
    f.start().await; f.invoke(1).await;
    assert!(matches!(f.next().await, Message::Forward { n: 1, fwd: 1, .. }));
    f.send(Message::Cancel { op: 7 }).await;
    f.reply(1, 1).await;
    assert!(matches!(f.next().await, Message::Result { n: 1,
        outcome: Outcome::Output { completed: true,
            failure: OutputFailure::Cancelled | OutputFailure::Closed }, .. }));
    assert!(matches!(f.next().await, Message::Done { outcome: DoneOutcome::Cancelled, .. }));
    assert_eq!(f.terminal, 0);
    f.end(true).await;
}

#[tokio::test]
async fn runner_overlapping_invocation_poison_joins_pending_policy() {
    let mut f = RunnerPeer::new().await;
    f.start().await; f.invoke(1).await;
    assert!(matches!(f.next().await, Message::Forward { n: 1, fwd: 1, .. }));
    f.invoke(2).await;
    let result = tokio::time::timeout(Duration::from_secs(2), f.task).await.unwrap().unwrap();
    assert!(result.is_err());
}

#[tokio::test]
async fn runner_done_precedes_final_disposal_credit() {
    use framer_worker_wire::message::Outcome;
    let mut f = RunnerPeer::new().await;
    f.start().await; f.invoke(1).await;
    assert!(matches!(f.next().await, Message::Forward { .. }));
    f.reply(1, 1).await;
    assert!(matches!(f.next().await, Message::Result { outcome: Outcome::Success { .. }, .. }));
    f.send(Message::Finish { op: 7, n: 1 }).await;
    let terminal = f.raw().await;
    let Message::Data { event: DataKind::Terminal, seq, last: true, .. } = terminal.message
        else { panic!("terminal frame expected"); };
    f.send(Message::Credit { op: 7, returned: Capacity { wire_bytes: terminal.wire_bytes,
        ..Capacity::default() }, accepted_event_seq: seq + 1 }).await;
    assert!(matches!(f.raw().await.message, Message::Done { op: 7, .. }));
    assert!(!f.task.is_finished());
    f.send(Message::Credit { op: 7, returned: Capacity { wire_bytes: 0,
        slots: 1, downstream_bytes: 16384 }, accepted_event_seq: seq + 1 }).await;
    f.end(true).await;
}

#[tokio::test]
async fn runner_cancel_during_saturation_wait_keeps_ack_order() {
    use framer_worker_wire::message::{ErrorClass, DoneOutcome, LocalFailure, Outcome};
    let mut f = RunnerPeer::new().await;
    f.start().await; f.invoke(1).await;
    assert!(matches!(f.next().await, Message::Forward { n: 1, fwd: 1, .. }));
    f.send(Message::ForwardError { op: 7, n: 1, fwd: 1,
        class: ErrorClass::Retryable429, cooldown_seconds: None }).await;
    assert!(matches!(f.next().await, Message::Saturated { op: 7, n: 1, fwd: 1 }));
    f.send(Message::Cancel { op: 7 }).await;
    f.send(Message::SaturatedAck { op: 7, n: 1, fwd: 1 }).await;
    assert!(matches!(f.next().await, Message::Result {
        outcome: Outcome::Local { failure: LocalFailure::Cancelled }, .. }));
    assert!(matches!(f.next().await, Message::Done { outcome: DoneOutcome::Cancelled, .. }));
    f.end(true).await;
}

#[tokio::test]
async fn refused_writer_refunds_unaccepted_output_reservation() {
    use framer_worker_wire::message::Segment;
    let (outbox, writer) = Outbox::new(1).unwrap();
    let slot = outbox.register(7).unwrap();
    let control = OutputControl::new(Capacity { wire_bytes: DATA_WIRE_RESERVE,
        slots: 1, downstream_bytes: 16384 }).unwrap();
    control.reserve().await.unwrap();
    let permit = slot.reserve().await.unwrap();
    let encoded = frame::encode(&Message::Data { op: 7, event: DataKind::Opening,
        seq: 0, bytes: Segment::new(vec![42]).unwrap(), last: true });
    outbox.close();
    assert!(matches!(emitter::enqueue_reserved(&control, permit, encoded, 0, true),
        Err(OutputError::Closed)));
    assert_eq!(control.outstanding(), Capacity::default());
    control.disposed().await.unwrap();
    drop(writer);
}

#[tokio::test]
async fn runner_policy_panic_joins_emitter_before_done() {
    use framer_worker_wire::message::{DoneOutcome, Limits, Part, Segment};
    let mut f = RunnerPeer::new().await;
    f.start().await;
    // Internal policy requires an object. A scalar reaches its existing
    // indexed-write panic, exercising the sibling join without a fault hook.
    f.send(Message::Invoke { op: 7, n: 1, category: Text::new("translation".into()).unwrap(),
        limits: Limits { chunk_tokens: 600, max_chunks: 1, max_total_tokens: None,
            metered_fallback: false, continue_prompt: Text::new("continue".into()).unwrap() },
        request: Part { offset: 0, last: true, bytes: Segment::new(b"true".to_vec()).unwrap() } }).await;
    assert!(matches!(f.next_panicked().await, Message::Done { op: 7, outcome: DoneOutcome::Panicked, .. }));
    assert_eq!(f.terminal, 0);
    f.end(false).await;
}

#[test]
fn worker_preparation_preserves_fields_and_restarts_from_original_input() {
    use framer_protocol::responses_input::prepare_worker_payload;
    let input = json!({"model":"model", "instructions":"base", "opaque":{"keep":[1,true]},
        "tools":[{"type":"function", "name":"write", "parameters":{}}]});
    let first = prepare_worker_payload(input.clone());
    let repicked = prepare_worker_payload(input.clone());
    assert_eq!(first, repicked);
    assert_eq!(first["opaque"], input["opaque"]);
    assert_eq!(first["tools"], input["tools"]);
    assert!(first["instructions"].as_str().unwrap().starts_with("base\n\n"));
    assert_eq!(first["instructions"].as_str().unwrap().matches("split large writes").count(), 1);
    for instructions in [json!(""), json!(null), json!(false)] {
        let mut raw = input.clone(); raw["instructions"] = instructions;
        let result = prepare_worker_payload(raw);
        assert!(result["instructions"].as_str().unwrap().starts_with("When using tools"));
    }
    let no_tools = json!({"instructions":"unchanged", "tools":[]});
    assert_eq!(prepare_worker_payload(no_tools.clone()), no_tools);
}

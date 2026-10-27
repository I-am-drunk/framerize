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

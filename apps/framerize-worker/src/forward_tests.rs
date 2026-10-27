use super::*;
use framer_worker_wire::frame;
use serde_json::json;
use tokio::{io::DuplexStream, task::JoinHandle};

struct Fixture {
    client: Arc<ForwardClient>, peer: DuplexStream,
    writer: JoinHandle<Result<(), WireError>>,
}
impl Fixture {
    fn new() -> Self {
        let (outbox, writer) = Outbox::new(1).unwrap();
        let slot = outbox.register(7).unwrap();
        let client = ForwardClient::new(7, 3, "translation".into(), outbox, slot,
            Arc::new(Diagnostics::default()));
        let (pipe, peer) = tokio::io::duplex(97);
        Self { client, peer, writer: tokio::spawn(writer.run(pipe)) }
    }
    async fn next(&mut self) -> Message {
        tokio::time::timeout(std::time::Duration::from_secs(2), frame::read(&mut self.peer))
            .await.unwrap().unwrap().unwrap().message
    }
    async fn finish(self) {
        self.client.outbox.close();
        self.writer.await.unwrap().unwrap();
    }
}
fn error(fwd: u64, class: wire::ErrorClass) -> Message {
    Message::ForwardError { op: 7, n: 3, fwd, class, cooldown_seconds: None }
}
fn reply(fwd: u64, offset: u64, bytes: &[u8], last: bool) -> Message {
    Message::ForwardReply { op: 7, n: 3, fwd, status: 200,
        content_type: wire::Text::new("application/json".into()).unwrap(), queue_wait_ms: 731,
        body: wire::Part { offset, bytes: wire::Segment::new(bytes.to_vec()).unwrap(), last } }
}

#[tokio::test]
async fn segmented_forward_preserves_payload_and_actual_queue_time() {
    let mut f = Fixture::new();
    let payload = json!({"opaque": "雪".repeat(30_000), "stream": false});
    let expected = payload.clone();
    let client = f.client.clone();
    let call = tokio::spawn(async move { client.forward_typed("translate", payload, 2).await });
    let mut assembled = Vec::new();
    let mut parts = 0;
    loop {
        let Message::Forward { op, n, fwd, step, step_index, payload, .. } = f.next().await
            else { panic!("expected Forward"); };
        assert_eq!((op, n, fwd, step_index), (7, 3, 1, 2));
        assert_eq!(step.as_str(), "translate");
        assert_eq!(payload.offset, assembled.len() as u64);
        assert!(payload.bytes.as_slice().len() <= SEGMENT_LIMIT);
        assembled.extend_from_slice(payload.bytes.as_slice());
        parts += 1;
        if payload.last { break; }
    }
    assert!(parts > 2);
    assert_eq!(serde_json::from_slice::<Value>(&assembled).unwrap(), expected);
    assert!(!call.is_finished());
    f.client.dispatch(reply(1, 0, b"{\"ok\":", false)).unwrap();
    f.client.dispatch(reply(1, 6, b"true}", true)).unwrap();
    let result = call.await.unwrap().unwrap();
    assert_eq!(result.status, 200);
    assert_eq!(result.content_type, "application/json");
    assert_eq!(result.queue_wait_ms, 731);
    assert_eq!(&result.body[..], b"{\"ok\":true}");
    f.finish().await;
}

#[tokio::test]
async fn saturation_ack_remains_pending_through_cancellation() {
    let mut f = Fixture::new();
    let client = f.client.clone();
    let call = tokio::spawn(async move { client.forward_typed("translate", json!({}), 0).await });
    assert!(matches!(f.next().await, Message::Forward { fwd: 1, .. }));
    f.client.dispatch(error(1, wire::ErrorClass::Retryable429)).unwrap();
    let failure = call.await.unwrap().err().unwrap();
    assert!(failure.is_saturated());
    assert_eq!(f.client.last_failure(), Some((1, 0)));
    let client = f.client.clone();
    let ack = tokio::spawn(async move { client.saturated().await });
    assert!(matches!(f.next().await, Message::Saturated { op: 7, n: 3, fwd: 1 }));
    f.client.cancel();
    assert!(!ack.is_finished());
    f.client.dispatch(Message::SaturatedAck { op: 7, n: 3, fwd: 1 }).unwrap();
    ack.await.unwrap().unwrap();
    assert!(matches!(f.client.forward_typed("translate", json!({}), 1).await,
        Err(PolicyError::Local { kind: framer_protocol::forward_error::LocalFailure::Cancelled, .. })));
    assert!(f.client.lock().pending.is_none());
    f.finish().await;
}

#[tokio::test]
async fn overlapping_forward_is_a_channel_failure() {
    let mut f = Fixture::new();
    let client = f.client.clone();
    let call = tokio::spawn(async move { client.forward_typed("message", json!({}), 0).await });
    assert!(matches!(f.next().await, Message::Forward { .. }));
    assert!(f.client.forward_typed("message", json!({}), 1).await.err().unwrap().is_control());
    f.client.channel_lost();
    assert!(call.await.unwrap().err().unwrap().is_control());
    f.finish().await;
}

#[tokio::test]
async fn stale_invocation_and_noncontiguous_reply_are_refused() {
    let mut f = Fixture::new();
    let client = f.client.clone();
    let call = tokio::spawn(async move { client.forward_typed("message", json!({}), 0).await });
    f.next().await;
    let mut stale = error(1, wire::ErrorClass::Other);
    if let Message::ForwardError { n, .. } = &mut stale { *n = 2; }
    assert!(f.client.dispatch(stale).is_err());
    assert!(f.client.dispatch(reply(1, 1, b"bad", true)).is_err());
    f.client.channel_lost();
    assert!(call.await.unwrap().err().unwrap().is_control());
    f.finish().await;
}

#[test]
fn diagnostics_keep_one_slot_and_snapshot_after_started_frame() {
    let d = Diagnostics::default();
    let event = wire::Diagnostic::Backoff { chunk: 0, attempt: 1, wait_ms: 500 };
    d.push(event);
    assert_eq!(d.begin(), Some(event));
    for _ in 0..10 { d.push(event); }
    d.seal();
    assert!(d.snapshot().is_err());
    d.finish();
    assert_eq!(d.snapshot().unwrap(), 10);
    d.reopen().unwrap();
    d.push(event);
    d.seal();
    assert_eq!(d.snapshot().unwrap(), 11);
    d.reopen().unwrap();
    d.lock().dropped = u32::MAX;
    d.push(event); d.push(event); d.seal();
    assert_eq!(d.snapshot().unwrap(), u32::MAX);
}

#[tokio::test]
async fn channel_loss_during_ack_is_control_error_without_next_forward() {
    let mut f = Fixture::new();
    let client = f.client.clone();
    let call = tokio::spawn(async move { client.forward_typed("message", json!({}), 0).await });
    f.next().await;
    f.client.dispatch(error(1, wire::ErrorClass::Retryable429)).unwrap();
    assert!(call.await.unwrap().err().unwrap().is_saturated());
    let client = f.client.clone();
    let call = tokio::spawn(async move { client.saturated().await });
    assert!(matches!(f.next().await, Message::Saturated { .. }));
    f.client.channel_lost();
    assert!(call.await.unwrap().is_err());
    assert_eq!(f.client.lock().next, 1);
    f.finish().await;
}

#[tokio::test]
async fn reply_metadata_cannot_change_between_parts() {
    let mut f = Fixture::new();
    let client = f.client.clone();
    let call = tokio::spawn(async move { client.forward_typed("message", json!({}), 0).await });
    f.next().await;
    f.client.dispatch(reply(1, 0, b"a", false)).unwrap();
    let mut changed = reply(1, 1, b"b", true);
    if let Message::ForwardReply { queue_wait_ms, .. } = &mut changed { *queue_wait_ms += 1; }
    assert!(f.client.dispatch(changed).is_err());
    f.client.channel_lost();
    assert!(call.await.unwrap().err().unwrap().is_control());
    f.finish().await;
}

#[tokio::test]
async fn closed_staging_releases_and_joins_payload_serializer() {
    let (sender, mut receiver) = mpsc::channel(1);
    let job = tokio::task::spawn_blocking(move || {
        serialize_payload(json!({"large": "x".repeat(8 * SEGMENT_LIMIT)}), sender)
    });
    let first = receiver.recv().await.unwrap();
    assert_eq!(first.bytes.as_slice().len(), SEGMENT_LIMIT);
    assert!(!first.last);
    drop(first);
    drop(receiver);
    assert!(tokio::time::timeout(std::time::Duration::from_secs(2), job)
        .await.unwrap().unwrap().is_err());
}

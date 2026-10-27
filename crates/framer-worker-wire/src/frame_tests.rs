use super::*;
use crate::message::{self as dto, *};

async fn pending<F: std::future::Future>(mut future: std::pin::Pin<&mut F>) {
    std::future::poll_fn(|cx| {
        assert!(future.as_mut().poll(cx).is_pending());
        std::task::Poll::Ready(())
    }).await;
}

#[tokio::test]
async fn synchronous_enqueue_keeps_accepted_frame_after_receipt_drop() {
    let (outbox, writer) = Outbox::new(1).unwrap();
    let slot = outbox.register(1).unwrap();
    let expected = encode(&Message::Ready { generation: 42 }).unwrap();
    let receipt = slot.reserve().await.unwrap().enqueue_encoded(expected.clone()).unwrap();
    drop(receipt);
    assert!(outbox.remove(&slot).is_err());
    outbox.close();
    let (pipe, mut reader) = tokio::io::duplex(17);
    let joined = tokio::spawn(writer.run(pipe));
    let mut actual = Vec::new();
    reader.read_to_end(&mut actual).await.unwrap();
    joined.await.unwrap().unwrap();
    assert_eq!(actual, expected);
    outbox.remove(&slot).unwrap();
}

#[tokio::test]
async fn outbox_bounds_records_and_refuses_stale_slot_reuse() {
    assert!(Outbox::new(0).is_err());
    let (outbox, _writer) = Outbox::new(1).unwrap();
    let old = outbox.register(7).unwrap();
    assert!(outbox.register(7).is_err());
    assert!(outbox.register(8).is_err());
    let held = old.reserve().await.unwrap();
    assert!(outbox.remove(&old).is_err());
    drop(held);
    outbox.remove(&old).unwrap();
    let fresh = outbox.register(7).unwrap();
    assert!(old.reserve().await.is_err());
    assert!(fresh.reserve().await.is_ok());
}

fn text(value: &str) -> Text { Text::new(value.into()).unwrap() }

struct AckObserver { slot: WriteSlot, removed: std::sync::atomic::AtomicBool }
impl std::task::Wake for AckObserver {
    fn wake(self: Arc<Self>) {
        // oneshot invokes this on send, before FrameWriter can do later work.
        self.slot.outbox.remove(&self.slot).expect("ack still owned its slot");
        self.removed.store(true, std::sync::atomic::Ordering::SeqCst);
    }
}
#[tokio::test]
async fn success_ack_is_a_safe_synchronous_slot_removal_point() {
    use std::future::Future;
    let (outbox, writer) = Outbox::new(1).unwrap();
    let slot = outbox.register(0).unwrap();
    let observer = Arc::new(AckObserver { slot: slot.clone(),
        removed: std::sync::atomic::AtomicBool::new(false) });
    let message = Message::Ready { generation: 1 };
    let mut sending = Box::pin(slot.send(&message));
    let waker = std::task::Waker::from(observer.clone());
    let mut context = std::task::Context::from_waker(&waker);
    assert!(sending.as_mut().poll(&mut context).is_pending());
    outbox.close();
    writer.run(tokio::io::sink()).await.unwrap();
    sending.await.unwrap();
    assert!(observer.removed.load(std::sync::atomic::Ordering::SeqCst));
}

#[tokio::test]
async fn cancelled_waiter_keeps_its_partial_frame_and_next_frame_intact() {
    let (outbox, writer) = Outbox::new(2).unwrap();
    let a = outbox.register(1).unwrap();
    let b = outbox.register(2).unwrap();
    let first = Message::Ready { generation: 1 };
    let second = Message::Ready { generation: 2 };
    let mut send_a = Box::pin(a.send(&first));
    let mut send_b = Box::pin(b.send(&second));
    pending(send_a.as_mut()).await;
    pending(send_b.as_mut()).await;
    let (tx, mut rx) = tokio::io::duplex(1);
    let job = tokio::spawn(writer.run(tx));
    let mut bytes = vec![0];
    rx.read_exact(&mut bytes).await.unwrap();
    drop(send_a); // A byte is already on the wire; its owner remains the writer.
    outbox.close();
    rx.read_to_end(&mut bytes).await.unwrap();
    send_b.await.unwrap();
    job.await.unwrap().unwrap();
    assert_eq!(bytes, [encode(&first).unwrap(), encode(&second).unwrap()].concat());
    assert!(a.reserve().await.is_err());
}
fn part() -> Part {
    Part { offset: 0, bytes: Segment::new(vec![0, 255]).unwrap(), last: true }
}

#[tokio::test]
async fn ready_producers_get_a_complete_frame_before_a_repeat_sender() {
    let (outbox, writer) = Outbox::new(2).unwrap();
    let a = outbox.register(1).unwrap();
    let b = outbox.register(2).unwrap();
    let messages = [1, 2, 3].map(|generation| Message::Ready { generation });
    let mut first = Box::pin(a.send(&messages[0]));
    let mut peer = Box::pin(b.send(&messages[1]));
    pending(first.as_mut()).await;
    pending(peer.as_mut()).await;
    let (tx, mut rx) = tokio::io::duplex(1);
    let job = tokio::spawn(writer.run(tx));
    assert_eq!(read::<_, Message>(&mut rx).await.unwrap().unwrap().message, messages[0]);
    first.await.unwrap();
    let mut repeat = Box::pin(a.send(&messages[2]));
    pending(repeat.as_mut()).await;
    outbox.close();
    for expected in &messages[1..] {
        assert_eq!(&read::<_, Message>(&mut rx).await.unwrap().unwrap().message, expected);
    }
    peer.await.unwrap();
    repeat.await.unwrap();
    job.await.unwrap().unwrap();
}
fn public_failure() -> PublicFailure {
    PublicFailure { status: 429, message: text("busy"), error_type: text("api error / λ"),
        code: text("api error / λ") }
}

#[tokio::test]
async fn abandoned_writer_fails_pending_frames_and_frees_slot_owners() {
    let (outbox, writer) = Outbox::new(1).unwrap();
    let slot = outbox.register(1).unwrap();
    let message = Message::Ready { generation: 9 };
    let mut send = Box::pin(slot.send(&message));
    pending(send.as_mut()).await;
    assert!(outbox.remove(&slot).is_err());
    drop(writer);
    assert!(send.await.is_err());
    outbox.remove(&slot).unwrap();
}

#[tokio::test]
async fn oversized_reserved_capacity_is_not_hidden_by_a_small_frame() {
    let (outbox, _writer) = Outbox::new(1).unwrap();
    let slot = outbox.register(1).unwrap();
    let mut bytes = Vec::with_capacity(FRAME_LIMIT + 5);
    bytes.extend_from_slice(&encode(&Message::Ready { generation: 9 }).unwrap());
    assert!(matches!(slot.reserve().await.unwrap().send_encoded(bytes).await,
        Err(WireError::FrameLimit)));
    outbox.remove(&slot).unwrap();
}

#[tokio::test]
async fn writer_failure_closes_all_waiters_without_reusing_the_channel() {
    let (outbox, writer) = Outbox::new(2).unwrap();
    let a = outbox.register(1).unwrap();
    let b = outbox.register(2).unwrap();
    let message = Message::Ready { generation: 7 };
    let mut first = Box::pin(a.send(&message));
    let mut peer = Box::pin(b.send(&message));
    pending(first.as_mut()).await;
    pending(peer.as_mut()).await;
    let (tx, rx) = tokio::io::duplex(1);
    drop(rx);
    assert!(writer.run(tx).await.is_err());
    assert!(first.await.is_err());
    assert!(peer.await.is_err());
    assert!(a.reserve().await.is_err());
    assert!(outbox.register(3).is_err());
}
fn all_messages() -> Vec<Message> {
    vec![
        Message::Start { op: 1, logical_id: text("id"), model: text("model"),
            mode: Mode::Sse, output: crate::credit::Capacity {
                wire_bytes: 100, slots: 2, downstream_bytes: 10 } },
        Message::Invoke { op: 1, n: 0, category: text("translation"),
            limits: Limits { chunk_tokens: 600, max_chunks: 4,
                max_total_tokens: None, metered_fallback: false, continue_prompt: text("continue") },
            request: part() },
        Message::Head { op: 1, status: 200, content_type: text("text/event-stream") },
        Message::Forward { op: 1, n: 0, fwd: 1, dialect: Dialect::OpenRouterResponses,
            step: text("translate"), step_index: 0, payload: part() },
        Message::ForwardReply { op: 1, n: 0, fwd: 1, status: 200,
            content_type: text("application/json"), queue_wait_ms: 7, body: part() },
        Message::ForwardError { op: 1, n: 0, fwd: 1,
            class: ErrorClass::Retryable429, cooldown_seconds: None },
        Message::Saturated { op: 1, n: 0, fwd: 1 },
        Message::SaturatedAck { op: 1, n: 0, fwd: 1 },
        Message::Data { op: 1, event: DataKind::Content, seq: 0,
            bytes: Segment::new(vec![0, 255]).unwrap(), last: true },
        Message::Diag { op: 1, n: 0, event: Diagnostic::Backoff {
            chunk: 0, attempt: 1, wait_ms: 0 } },
        Message::Result { op: 1, n: 0, outcome: Outcome::Forward {
            failure: FailureRecipe { cause: Cause { op: 1, n: 0, fwd: 1 },
                tag: FailureTag::SaturationExhausted, chunk: 0 } }, diag_dropped: u32::MAX },
        Message::Finish { op: 1, n: 0 },
        Message::Fail { op: 1, n: 0, error: public_failure() },
        Message::Cancel { op: 1 },
        Message::Done { op: 1, outcome: DoneOutcome::Completed, diag_dropped: u32::MAX },
    ]
}

#[tokio::test]
async fn operation_messages_round_trip_through_bounded_frames() {
    for message in all_messages() {
        let frame = encode(&message).unwrap();
        let received = read::<_, Message>(&mut frame.as_slice()).await.unwrap().unwrap();
        assert_eq!(received.message, message);
        assert_eq!(received.wire_bytes, frame.len() as u64);
    }
}

#[tokio::test]
async fn nested_recipes_diagnostics_and_failures_reject_unknown_or_duplicate_fields() {
    for json in [
        r#"{"kind":"result","op":1,"n":0,"diag_dropped":0,"outcome":{"kind":"control","text":"private"}}"#,
        r#"{"kind":"diag","op":1,"n":0,"event":{"kind":"backoff","chunk":0,"attempt":1,"wait_ms":0,"text":"private"}}"#,
        r#"{"kind":"diag","op":1,"n":0,"event":{"kind":"backoff","chunk":0,"chunk":1,"attempt":1,"wait_ms":0}}"#,
        r#"{"kind":"result","op":1,"n":0,"diag_dropped":0,"outcome":{"kind":"forward","failure":{"cause":{"op":1,"op":2,"n":0,"fwd":1},"tag":"direct","chunk":0}}}"#,
        r#"{"kind":"result","op":1,"n":0,"diag_dropped":0,"outcome":{"kind":"forward","failure":{"cause":{"op":1,"n":0,"fwd":1,"team":"x"},"tag":"direct","chunk":0}}}"#,
        r#"{"kind":"fail","op":1,"n":0,"error":{"status":500,"message":"fixed","type":"api_error","code":"api_error","raw":"private"}}"#,
        r#"{"kind":"fail","op":1,"n":0,"error":{"status":500,"message":"fixed","type":"a","type":"b","code":"api_error"}}"#,
    ] {
        assert!(read::<_, Message>(&mut framed(json.as_bytes()).as_slice()).await.is_err(),
            "accepted invalid envelope: {json}");
    }
}

#[tokio::test]
async fn result_and_done_require_bounded_drop_counts_and_closed_tags() {
    for json in [
        r#"{"kind":"done","op":1,"outcome":"completed","diag_dropped":4294967296}"#,
        r#"{"kind":"done","op":1,"outcome":"completed","diag_dropped":-1}"#,
        r#"{"kind":"done","op":1,"outcome":"completed"}"#,
        r#"{"kind":"result","op":1,"n":0,"outcome":{"kind":"control"}}"#,
        r#"{"kind":"result","op":1,"n":0,"diag_dropped":0,"outcome":{"kind":"local","failure":"raw text"}}"#,
        r#"{"kind":"result","op":1,"n":0,"diag_dropped":0,"outcome":{"kind":"forward","failure":{"cause":{"op":1,"n":0,"fwd":1},"tag":"raw text","chunk":0}}}"#,
        r#"{"kind":"done","op":1,"outcome":"cleanup_confirmed","diag_dropped":0}"#,
    ] {
        assert!(read::<_, Message>(&mut framed(json.as_bytes()).as_slice()).await.is_err());
    }
}

#[tokio::test]
async fn opaque_segments_accept_exact_limit_without_utf8_and_reject_extra_byte() {
    for size in [0, 1, crate::SEGMENT_LIMIT - 1, crate::SEGMENT_LIMIT] {
        let bytes = vec![255; size];
        let message = Message::Data { op: 1, event: DataKind::Content, seq: 0,
            bytes: Segment::new(bytes.clone()).unwrap(), last: true };
        let frame = encode(&message).unwrap();
        let decoded = read::<_, Message>(&mut frame.as_slice()).await.unwrap().unwrap();
        assert_eq!(decoded.message, message);
    }
    let too_big = vec![0; crate::SEGMENT_LIMIT + 1];
    assert!(Segment::new(too_big.clone()).is_err());
    let json = format!(r#"{{"kind":"data","op":1,"event":"content","seq":0,"bytes":{},"last":true}}"#,
        serde_json::to_string(&too_big).unwrap());
    assert!(json.len() < FRAME_LIMIT);
    assert!(read::<_, Message>(&mut framed(json.as_bytes()).as_slice()).await.is_err());
}

#[tokio::test]
async fn public_failure_preserves_unicode_types_and_enforces_only_byte_and_status_bounds() {
    let error = public_failure();
    let encoded = encode(&Message::Fail { op: 1, n: 0, error: error.clone() }).unwrap();
    let decoded = read::<_, Message>(&mut encoded.as_slice()).await.unwrap().unwrap();
    assert_eq!(decoded.message, Message::Fail { op: 1, n: 0, error });
    assert!(Text::new("λ".repeat(dto::TEXT_LIMIT / 2)).is_ok());
    assert!(Text::new("λ".repeat(dto::TEXT_LIMIT / 2 + 1)).is_err());
    for status in [0, 200, 399, 600] {
        let mut error = public_failure();
        error.status = status;
        let frame = encode(&Message::Fail { op: 1, n: 0, error }).unwrap();
        assert!(read::<_, Message>(&mut frame.as_slice()).await.is_err());
    }
    let oversized = serde_json::to_string(&"x".repeat(dto::TEXT_LIMIT + 1)).unwrap();
    let json = format!(r#"{{"kind":"fail","op":1,"n":0,"error":{{"status":500,"message":{},"type":"x","code":"x"}}}}"#, oversized);
    assert!(read::<_, Message>(&mut framed(json.as_bytes()).as_slice()).await.is_err());
}

#[tokio::test]
async fn nested_parts_limits_and_summaries_stay_strict() {
    for json in [
        r#"{"kind":"forward_reply","op":1,"n":0,"fwd":1,"status":200,"content_type":"json","queue_wait_ms":0,"body":{"offset":0,"offset":1,"bytes":[],"last":true}}"#,
        r#"{"kind":"forward_reply","op":1,"n":0,"fwd":1,"status":200,"content_type":"json","queue_wait_ms":0,"body":{"offset":0,"bytes":[],"last":true,"extra":0}}"#,
        r#"{"kind":"data","op":1,"event":"content","seq":0,"bytes":[256],"last":true}"#,
        r#"{"kind":"data","op":1,"event":"content","seq":0,"bytes":[-1],"last":true}"#,
        r#"{"kind":"invoke","op":1,"n":0,"category":"translation","limits":{"chunk_tokens":600,"max_chunks":4,"max_total_tokens":null,"metered_fallback":false,"continue_prompt":"x","extra":0},"request":{"offset":0,"bytes":[],"last":true}}"#,
        r#"{"kind":"result","op":1,"n":0,"diag_dropped":0,"outcome":{"kind":"success","summary":{"input_tokens":1,"output_tokens":2,"chunks":1,"metered_chunks":0,"finish":"completed","cleanup":true}}}"#,
    ] {
        assert!(read::<_, Message>(&mut framed(json.as_bytes()).as_slice()).await.is_err());
    }
}

#[tokio::test]
async fn fragmented_transport_preserves_adjacent_complete_frames() {
    let (mut writer, mut reader) = tokio::io::duplex(1);
    let first = encode(&Message::Ready { generation: 7 }).unwrap();
    let second = encode(&Message::Ready { generation: 8 }).unwrap();
    assert_eq!(first.capacity(), first.len());
    assert_eq!(second.capacity(), second.len());
    let send = async {
        write(&mut writer, &first).await.unwrap();
        write(&mut writer, &second).await.unwrap();
        writer.shutdown().await.unwrap();
    };
    let receive = async {
        for (generation, bytes) in [(7, first.len()), (8, second.len())] {
            let frame = read::<_, Message>(&mut reader).await.unwrap().unwrap();
            assert_eq!(frame.message, Message::Ready { generation });
            assert_eq!(frame.wire_bytes, bytes as u64);
        }
        assert!(read::<_, Message>(&mut reader).await.unwrap().is_none());
    };
    tokio::join!(send, receive);
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Nested { #[allow(dead_code)] next: Option<Box<Nested>> }

#[tokio::test]
async fn recursion_limit_stays_enabled() {
    let json = format!("{}null{}", "{\"next\":".repeat(160), "}".repeat(160));
    let input = framed(json.as_bytes());
    assert!(matches!(read::<_, Nested>(&mut input.as_slice()).await, Err(WireError::Json(_))));
}

#[tokio::test]
async fn length_is_rejected_before_any_payload_is_read() {
    let mut bytes = ((FRAME_LIMIT + 1) as u32).to_be_bytes().to_vec();
    bytes.extend_from_slice(b"unread");
    let mut input = bytes.as_slice();
    assert!(matches!(read::<_, Message>(&mut input).await, Err(WireError::FrameLimit)));
    assert_eq!(input, b"unread");
    assert!(matches!(read::<_, Message>(&mut [0u8; 4].as_slice()).await,
        Err(WireError::FrameLimit)));
    let mut truncated = 10u32.to_be_bytes().to_vec();
    truncated.extend_from_slice(b"abc");
    assert!(matches!(read::<_, Message>(&mut truncated.as_slice()).await,
        Err(WireError::Io(_))));
}

#[tokio::test]
async fn hello_build_uses_the_same_utf8_byte_bound() {
    let message = Message::Hello { wire: 1, build: text(&"x".repeat(dto::TEXT_LIMIT)),
        routes: vec![Route::FreeResponses] };
    let frame = encode(&message).unwrap();
    assert_eq!(read::<_, Message>(&mut frame.as_slice()).await.unwrap().unwrap().message, message);
    let json = serde_json::json!({"kind":"hello", "wire":1,
        "build":"x".repeat(dto::TEXT_LIMIT + 1), "routes":["free_responses"]}).to_string();
    assert!(read::<_, Message>(&mut framed(json.as_bytes()).as_slice()).await.is_err());
}

#[test]
fn encoding_refuses_an_oversized_frame() {
    // Test the frame encoder independently of the stricter DTO constructor.
    let message = serde_json::json!({
        "kind": "hello", "wire": 1, "build": "x".repeat(FRAME_LIMIT), "routes": [],
    });
    assert!(matches!(encode(&message), Err(WireError::FrameLimit)));
}

fn framed(json: &[u8]) -> Vec<u8> {
    let mut bytes = (json.len() as u32).to_be_bytes().to_vec();
    bytes.extend_from_slice(json);
    bytes
}

#[tokio::test]
async fn strict_dtos_reject_duplicate_unknown_and_trailing_input() {
    for json in [
        r#"{"kind":"ready","generation":1,"generation":2}"#,
        r#"{"kind":"ready","kind":"ready","generation":1}"#,
        r#"{"kind":"ready","generation":1,"extra":0}"#,
        r#"{"kind":"unknown","generation":1}"#,
        r#"{"kind":"ready","generation":1} {}"#,
        r#"{"kind":"credit","op":1,"accepted_event_seq":0,"returned":{"wire_bytes":0,"slots":0,"slots":1,"downstream_bytes":0}}"#,
    ] {
        let input = framed(json.as_bytes());
        assert!(read::<_, Message>(&mut input.as_slice()).await.is_err());
    }
}

#[tokio::test]
async fn clean_eof_is_distinct_from_a_truncated_frame() {
    assert!(read::<_, Message>(&mut [].as_slice()).await.unwrap().is_none());
    for input in [vec![0], vec![0, 0, 0], framed(b"{\"kind\":")] {
        assert!(read::<_, Message>(&mut input.as_slice()).await.is_err());
    }
}

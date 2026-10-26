use super::*;
use futures::{FutureExt, StreamExt};
use serde_json::json;

const WAIT: Duration = Duration::from_secs(5);
struct PumpSource {
    frames: std::collections::VecDeque<bytes::Bytes>, pending: bool,
    polls: Arc<AtomicUsize>, drops: Arc<AtomicUsize>,
}
impl Stream for PumpSource {
    type Item = Result<bytes::Bytes, io::Error>;
    fn poll_next(mut self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.polls.fetch_add(1, Ordering::SeqCst);
        if self.pending { Poll::Pending } else { Poll::Ready(self.frames.pop_front().map(Ok)) }
    }
}
impl Drop for PumpSource {
    fn drop(&mut self) { self.drops.fetch_add(1, Ordering::SeqCst); }
}
fn pump_source(frames: Vec<bytes::Bytes>, pending: bool)
    -> (axum::response::Response, Arc<AtomicUsize>, Arc<AtomicUsize>) {
    let polls = Arc::new(AtomicUsize::new(0));
    let drops = Arc::new(AtomicUsize::new(0));
    let source = PumpSource { frames: frames.into(), pending,
        polls: polls.clone(), drops: drops.clone() };
    (axum::response::Response::new(axum::body::Body::from_stream(source)), polls, drops)
}
async fn reached(counter: &Counter, minimum: usize) {
    tokio::time::timeout(WAIT, async {
        while counter.current.load(Ordering::SeqCst) < minimum { counter.changed.notified().await; }
    }).await.unwrap();
}
fn result(size: usize) -> AssembledResponses {
    AssembledResponses { id: "upstream".into(), created_at: 1,
        output: vec![json!({"type":"function_call","call_id":"call","name":"tool",
            "arguments":"x".repeat(size)})], status: "completed".into(), incomplete_details: None,
        usage: json!({}), service_tier: Value::Null, input_tokens: 1, output_tokens: 1,
        chunks: 1, metered_chunks: 0 }
}
#[tokio::test]
async fn pump_stops_before_eof_and_retains_original_body_with_copied_prefix() {
    let disposed = Arc::new(AtomicUsize::new(0));
    let first = bytes::Bytes::from_owner(TrackedBytes { bytes: b"first".to_vec(), drops: disposed.clone() });
    let (mut response, polls, drops) = pump_source(vec![first, bytes::Bytes::from_static(b"second")], false);
    response.headers_mut().insert("x-fixture", "preserved".parse().unwrap());
    response.extensions_mut().insert(17u64);
    let response = pump_until_producer_returns(response, || polls.load(Ordering::SeqCst) == 1).await;
    assert_eq!(disposed.load(Ordering::SeqCst), 1);
    assert_eq!(polls.load(Ordering::SeqCst), 1);
    assert_eq!(drops.load(Ordering::SeqCst), 0);
    assert_eq!(response.headers()["x-fixture"], "preserved");
    assert_eq!(response.extensions().get::<u64>(), Some(&17));
    assert_eq!(consume_body(response.into_body(), 64).await.unwrap().as_ref(), b"firstsecond");
    assert_eq!(drops.load(Ordering::SeqCst), 1);
}
#[tokio::test]
async fn pump_checks_completed_counter_before_polling_any_frame() {
    let (response, polls, drops) = pump_source(vec![], false);
    let response = pump_until_producer_returns(response, || true).await;
    assert_eq!(polls.load(Ordering::SeqCst), 0);
    assert_eq!(drops.load(Ordering::SeqCst), 0);
    drop(response);
    assert_eq!(drops.load(Ordering::SeqCst), 1);
}
#[tokio::test]
async fn pump_refuses_eof_frame_overflow_and_byte_overflow() {
    for (frames, expected_polls) in [
        (Vec::new(), 1),
        (vec![bytes::Bytes::from_static(b"x"); 65], 64),
        (vec![bytes::Bytes::from(vec![0; 65_537])], 1),
    ] {
        let (response, polls, drops) = pump_source(frames, false);
        let result = std::panic::AssertUnwindSafe(
            pump_until_producer_returns(response, || false)).catch_unwind().await;
        assert!(result.is_err());
        assert_eq!(polls.load(Ordering::SeqCst), expected_polls);
        assert_eq!(drops.load(Ordering::SeqCst), 1);
    }
}
#[tokio::test]
async fn pump_accepts_exact_frame_and_byte_bounds_without_polling_eof() {
    let (response, polls, drops) = pump_source(vec![bytes::Bytes::from(vec![b'x'; 1024]); 64], false);
    let response = pump_until_producer_returns(response, || polls.load(Ordering::SeqCst) == 64).await;
    assert_eq!(polls.load(Ordering::SeqCst), 64);
    assert_eq!(drops.load(Ordering::SeqCst), 0);
    assert_eq!(consume_body(response.into_body(), 65_536).await.unwrap().len(), 65_536);
    assert_eq!(drops.load(Ordering::SeqCst), 1);
}
#[tokio::test(start_paused = true)]
async fn pump_timeout_and_cancellation_drop_the_original_pending_body() {
    for cancelled in [false, true] {
        let (response, polls, drops) = pump_source(vec![], true);
        let task = tokio::spawn(pump_until_producer_returns(response, || false));
        while polls.load(Ordering::SeqCst) == 0 { tokio::task::yield_now().await; }
        if cancelled { task.abort(); }
        else { tokio::time::advance(Duration::from_secs(2)).await; }
        let error = task.await.unwrap_err();
        assert_eq!(error.is_cancelled(), cancelled);
        assert_eq!(error.is_panic(), !cancelled);
        assert_eq!(drops.load(Ordering::SeqCst), 1);
    }
}
async fn start(budget: &OutputBudget) -> (Sink, OutputBody, tokio::task::JoinHandle<()>) {
    let (emitter, sink, mut body) = output(budget.reserve().unwrap(), "fixture", "logical");
    let task = tokio::spawn(emitter.run());
    for _ in 0..2 { drop(tokio::time::timeout(WAIT, body.next()).await.unwrap().unwrap().unwrap()); }
    (sink, body, task)
}
#[tokio::test]
async fn retained_clone_outlives_eof_and_holds_capacity_until_last_drop() {
    let budget = OutputBudget::new(1);
    let (sink, mut body, task) = start(&budget).await;
    sink.request(Input::Finish(result(0))).await.unwrap();
    drop(sink);
    let bytes = body.next().await.unwrap().unwrap();
    let retained = bytes.clone(); drop(bytes);
    assert!(body.next().await.is_none(), "EOF cannot depend on observed completion");
    assert!(!task.is_finished());
    assert!(budget.reserve().is_err());
    assert_eq!(budget.metrics.output_slots.current.load(Ordering::SeqCst), 1);
    drop(body);
    assert_eq!(budget.metrics.output_slots.current.load(Ordering::SeqCst), 1);
    drop(retained);
    tokio::time::timeout(WAIT, task).await.unwrap().unwrap();
    assert_eq!(budget.metrics.bytes.current.load(Ordering::SeqCst), 0);
    assert_eq!(budget.metrics.leases.current.load(Ordering::SeqCst), 0);
}
#[tokio::test]
async fn blocked_staging_cancellation_joins_job_without_output_credit() {
    let budget = OutputBudget::new(1);
    let (sink, body, task) = start(&budget).await;
    let caller = tokio::spawn(async move { framer_core::port::emit_assembled(result(SEGMENT * 20), &sink).await });
    reached(&budget.metrics.buffers, 5).await;
    assert_eq!(budget.metrics.output_slots.current.load(Ordering::SeqCst), 2);
    assert!(budget.metrics.bytes.peak.load(Ordering::SeqCst) <= 5 * SEGMENT);
    drop(body);
    assert!(matches!(caller.await.unwrap(), Err(ResponseError::Output(_))));
    tokio::time::timeout(WAIT, task).await.unwrap().unwrap();
    assert_eq!(budget.metrics.jobs.current.load(Ordering::SeqCst), 0);
    assert_eq!(budget.metrics.bytes.current.load(Ordering::SeqCst), 0);
}
async fn complete(sink: Sink, mut body: OutputBody, emitter: tokio::task::JoinHandle<()>, size: usize) -> Vec<u8> {
    let producer = tokio::spawn(async move {
        let value = framer_core::port::emit_assembled(result(size), &sink).await.unwrap();
        sink.request(Input::Finish(value)).await.unwrap();
    });
    let mut wire = Vec::new();
    while let Some(bytes) = body.next().await { wire.extend_from_slice(&bytes.unwrap()); }
    producer.await.unwrap();
    emitter.await.unwrap();
    wire
}
async fn profile(stalled: usize) {
    let before = process_snapshot();
    let budget = OutputBudget::new(stalled + 1);
    let mut bodies = Vec::new();
    let mut jobs = Vec::new();
    let mut callers = Vec::new();
    for _ in 0..stalled {
        let (sink, body, job) = start(&budget).await;
        bodies.push(body); jobs.push(job);
        callers.push(tokio::spawn(async move { framer_core::port::emit_assembled(result(SEGMENT * 20), &sink).await }));
    }
    reached(&budget.metrics.buffers, stalled * 5).await;
    let stalled_memory = process_snapshot();
    assert_eq!(budget.metrics.jobs.current.load(Ordering::SeqCst), stalled);
    let (sink, body, job) = start(&budget).await;
    let milestones = sink.lease.timings.clone();
    assert!(budget.reserve().is_err());
    let started = Instant::now();
    let wire = tokio::time::timeout(WAIT, complete(sink, body, job, SEGMENT + 1)).await.unwrap();
    assert!(std::str::from_utf8(&wire).unwrap().contains("response.completed"));
    assert_eq!(budget.metrics.jobs.current.load(Ordering::SeqCst), stalled);
    assert!(budget.metrics.jobs.peak.load(Ordering::SeqCst) <= stalled + 1);
    assert!(budget.metrics.bytes.peak.load(Ordering::SeqCst) <= (stalled + 1) * 5 * SEGMENT);
    eprintln!("output profile peers={stalled} progress_us={} peak_bytes={} peak_jobs={}",
        started.elapsed().as_micros(), budget.metrics.bytes.peak.load(Ordering::SeqCst),
        budget.metrics.jobs.peak.load(Ordering::SeqCst));
    eprintln!("output profile runtime=multi_thread workers=2 admitted_us=0 segment={SEGMENT} slots_per_stream={OUTPUT_SLOTS} peak_buffers={} peak_output_bytes={} peak_output_slots={} input_output_json_bytes={} wire_bytes={}",
        budget.metrics.buffers.peak.load(Ordering::SeqCst),
        budget.metrics.output_bytes.peak.load(Ordering::SeqCst),
        budget.metrics.output_slots.peak.load(Ordering::SeqCst),
        serde_json::to_vec(&result(SEGMENT + 1).output).unwrap().len(), wire.len());
    drop(bodies);
    for caller in callers { assert!(caller.await.unwrap().is_err()); }
    for job in jobs { tokio::time::timeout(WAIT, job).await.unwrap().unwrap(); }
    assert_eq!(budget.metrics.bytes.current.load(Ordering::SeqCst), 0);
    assert_eq!(budget.metrics.leases.current.load(Ordering::SeqCst), 0);
    eprintln!("output milestones={:?}; process_before={before}; stalled={stalled_memory}; after={}",
        milestones.lock().unwrap(), process_snapshot());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn p2_stalled_peer_does_not_take_another_streams_encoder() { profile(1).await; }
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn p5_four_stalled_peers_allow_the_fifth_to_finish() { profile(4).await; }
#[tokio::test]
async fn dropped_caller_keeps_a_join_owner_for_blocked_serialization() {
    let budget = OutputBudget::new(1);
    let (sink, body, job) = start(&budget).await;
    let caller = tokio::spawn(async move { framer_core::port::emit_assembled(result(SEGMENT * 20), &sink).await });
    reached(&budget.metrics.buffers, 5).await;
    caller.abort();
    assert!(caller.await.unwrap_err().is_cancelled());
    assert_eq!(budget.metrics.jobs.current.load(Ordering::SeqCst), 1);
    drop(body);
    tokio::time::timeout(WAIT, job).await.unwrap().unwrap();
    assert_eq!(budget.metrics.jobs.current.load(Ordering::SeqCst), 0);
    assert_eq!(budget.metrics.bytes.current.load(Ordering::SeqCst), 0);
}
async fn fault_case(fault: Fault, partial: bool) {
    let budget = OutputBudget::new(1); budget.inject(fault);
    let (emitter, sink, mut body) = output(budget.reserve().unwrap(), "fixture", "logical");
    let published = emitter.publisher.content_published.clone();
    let task = tokio::spawn(emitter.run());
    let caller = tokio::spawn(async move { framer_core::port::emit_assembled(result(SEGMENT * 4), &sink).await });
    let mut wire = Vec::new(); let mut aborted = false;
    while let Some(frame) = tokio::time::timeout(WAIT, body.next()).await.unwrap() {
        match frame { Ok(bytes) => wire.extend_from_slice(&bytes), Err(_) => { aborted = true; break; } }
    }
    drop(body);
    assert!(matches!(caller.await.unwrap(), Err(ResponseError::Output(OutputError::Encoding))));
    tokio::time::timeout(WAIT, task).await.unwrap().unwrap();
    assert_eq!(aborted, partial);
    assert_eq!(published.load(Ordering::SeqCst), partial);
    let wire = String::from_utf8(wire).unwrap();
    assert_eq!(wire.contains("response.failed"), !partial);
    assert!(!wire.contains("response.completed"));
    assert_eq!(budget.metrics.jobs.current.load(Ordering::SeqCst), 0);
    assert_eq!(budget.metrics.bytes.current.load(Ordering::SeqCst), 0);
}
#[tokio::test]
async fn failure_before_content_uses_fixed_boundary_error() { fault_case(Fault::BeforeContent, false).await; }
#[tokio::test]
async fn failure_after_partial_content_aborts_without_success_or_replay() { fault_case(Fault::AfterFirstSegment, true).await; }
#[tokio::test]
async fn serializer_panic_is_joined_and_fails_at_the_accepted_boundary() { fault_case(Fault::Panic, false).await; }
#[tokio::test]
async fn unavailable_output_admission_starts_no_job() {
    let budget = OutputBudget::new(0);
    assert!(budget.reserve().is_err());
    assert_eq!(budget.metrics.jobs.peak.load(Ordering::SeqCst), 0);
    assert_eq!(budget.metrics.bytes.peak.load(Ordering::SeqCst), 0);
}
fn process_snapshot() -> Value {
    fn fields(path: &str, names: &[&str]) -> Option<Vec<String>> {
        std::fs::read_to_string(path).ok().map(|text| text.lines()
            .filter(|line| names.iter().any(|name| line.starts_with(name)))
            .map(str::to_owned).collect())
    }
    json!({"status":fields("/proc/self/status", &["Threads:","VmSize:"]),
        "memory":fields("/proc/self/smaps_rollup", &["Rss:","Pss:","Private_Clean:","Private_Dirty:"]),
        "kind":"point sample, not peak or encoder-only memory"})
}

#[tokio::test]
async fn cancellation_before_job_admission_starts_no_serializer() {
    let budget = OutputBudget::new(1);
    let (emitter, sink, body) = output(budget.reserve().unwrap(), "fixture", "logical");
    drop(body);
    emitter.run().await;
    drop(sink);
    assert_eq!(budget.metrics.jobs.peak.load(Ordering::SeqCst), 0);
    assert_eq!(budget.metrics.bytes.peak.load(Ordering::SeqCst), 0);
    assert_eq!(budget.counts(), (0, 0, 0));
}

#[tokio::test]
async fn segment_writer_accounts_boundaries_and_rejects_empty_physical_events() {
    for size in [0, 1, SEGMENT - 1, SEGMENT, SEGMENT + 1, 2 * SEGMENT] {
        let metrics = Arc::new(Metrics::default());
        let counted = metrics.clone();
        let (sender, mut receiver) = mpsc::channel(1);
        let job = tokio::task::spawn_blocking(move || {
            let mut writer = SegmentWriter { sender, buffer: None, metrics: counted,
                sequence: 0, content: false, staging_wait: Duration::ZERO,
                fault: None, written_content: 0 };
            writer.begin_event(7, true).unwrap();
            writer.write_all(&vec![b'x'; size]).unwrap();
            assert_eq!(writer.end_event().is_ok(), size != 0);
        });
        let mut bytes = Vec::new();
        let mut ends = Vec::new();
        while let Some(segment) = receiver.recv().await {
            assert_eq!(segment.sequence, 7);
            assert!(segment.content);
            assert_eq!(segment.buffer.bytes.capacity(), SEGMENT);
            bytes.extend_from_slice(&segment.buffer.bytes);
            ends.push(segment.last);
        }
        job.await.unwrap();
        assert_eq!(bytes, vec![b'x'; size]);
        assert_eq!(ends.len(), size.div_ceil(SEGMENT));
        assert_eq!(ends.iter().filter(|&&last| last).count(), usize::from(size != 0));
        assert!(size == 0 || ends.last() == Some(&true));
        assert_eq!(metrics.bytes.current.load(Ordering::SeqCst), 0);
        assert!(metrics.bytes.peak.load(Ordering::SeqCst) <= 3 * SEGMENT);
    }
}

#[tokio::test]
async fn retaining_two_polled_buffers_stalls_then_resumes_without_losing_events() {
    let budget = OutputBudget::new(1);
    let (sink, mut body, emitter) = start(&budget).await;
    let producer = tokio::spawn(async move {
        let result = framer_core::port::emit_assembled(result(SEGMENT * 8), &sink).await.unwrap();
        sink.request(Input::Finish(result)).await.unwrap();
    });
    let first = body.next().await.unwrap().unwrap();
    let second = body.next().await.unwrap().unwrap();
    reached(&budget.metrics.buffers, 5).await;
    assert_eq!(budget.metrics.output_slots.current.load(Ordering::SeqCst), 2);
    assert!(!producer.is_finished());
    let mut wire = first.to_vec();
    wire.extend_from_slice(&second);
    drop(first); drop(second);
    while let Some(bytes) = tokio::time::timeout(WAIT, body.next()).await.unwrap() {
        wire.extend_from_slice(&bytes.unwrap());
    }
    producer.await.unwrap(); emitter.await.unwrap();
    let wire = String::from_utf8(wire).unwrap();
    let events: Vec<Value> = wire.lines().filter_map(|line| line.strip_prefix("data: "))
        .map(|line| serde_json::from_str(line).unwrap()).collect();
    assert!(events.iter().enumerate().all(|(i, event)| event["sequence_number"] == i + 2));
    assert_eq!(events.last().unwrap()["type"], "response.completed");
    assert_eq!(events.last().unwrap()["response"]["output"][0]["arguments"], "x".repeat(SEGMENT * 8));
    assert_eq!(budget.counts(), (0, 0, 0));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn output_profile_observer_control() {
    let started = Instant::now();
    let before = process_snapshot();
    let empty = process_snapshot();
    let after = process_snapshot();
    eprintln!("output observer-only runtime=multi_thread workers=2 elapsed_us={} before={before} empty={empty} after={after}",
        started.elapsed().as_micros());
}

struct TrackedBytes { bytes: Vec<u8>, drops: Arc<AtomicUsize> }
impl AsRef<[u8]> for TrackedBytes { fn as_ref(&self) -> &[u8] { &self.bytes } }
impl Drop for TrackedBytes {
    fn drop(&mut self) { self.drops.fetch_add(1, Ordering::SeqCst); }
}
#[tokio::test]
async fn copying_consumer_releases_each_frame_including_empty_data() {
    let drops = Arc::new(AtomicUsize::new(0));
    let counted = drops.clone();
    let frames = [b"ab".as_slice(), b"".as_slice(), b"cd".as_slice()];
    let stream = futures::stream::iter(frames.into_iter().enumerate().map(move |(index, bytes)| {
        assert_eq!(counted.load(Ordering::SeqCst), index);
        Ok::<_, io::Error>(bytes::Bytes::from_owner(TrackedBytes {
            bytes: bytes.to_vec(), drops: counted.clone() }))
    }));
    let bytes = consume_body(axum::body::Body::from_stream(stream), 4).await.unwrap();
    assert_eq!(bytes.as_ref(), b"abcd");
    assert_eq!(drops.load(Ordering::SeqCst), 3);
}

#[tokio::test]
async fn copying_consumer_preserves_aggregate_limit_and_body_errors() {
    let chunks = [Ok::<_, io::Error>("ab"), Ok("cd")];
    let body = axum::body::Body::from_stream(futures::stream::iter(chunks));
    let limited = consume_body(body, 3).await.unwrap_err();
    let original = axum::body::to_bytes(axum::body::Body::from("abcd"), 3).await.unwrap_err();
    assert_eq!(limited.into_inner().to_string(), original.into_inner().to_string());
    let body = axum::body::Body::from_stream(futures::stream::iter([
        Err::<bytes::Bytes, _>(io::Error::new(io::ErrorKind::InvalidData, "fixture source error"))
    ]));
    let error = consume_body(body, 1024).await.unwrap_err().into_inner();
    let error = error.downcast::<io::Error>().unwrap();
    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    assert_eq!(error.to_string(), "fixture source error");
}

#[tokio::test]
async fn cancelling_copying_consumer_drops_its_pending_source() {
    let drops = Arc::new(AtomicUsize::new(0));
    let entered = Arc::new(Notify::new());
    let notify = entered.clone();
    let guard = TrackedBytes { bytes: Vec::new(), drops: drops.clone() };
    let stream = futures::stream::unfold(guard, move |guard| {
        let notify = notify.clone();
        async move {
            notify.notify_one();
            std::future::pending::<()>().await;
            Some((Ok::<_, io::Error>(bytes::Bytes::new()), guard))
        }
    });
    let task = tokio::spawn(consume_body(axum::body::Body::from_stream(stream), 1024));
    tokio::time::timeout(WAIT, entered.notified()).await.unwrap();
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    assert_eq!(drops.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn blocked_publication_coalesces_heartbeat_at_an_event_boundary() {
    let budget = OutputBudget::new(1);
    let (mut emitter, sink, mut body) = output(budget.reserve().unwrap(), "fixture", "logical");
    emitter.heartbeat = tokio::time::interval_at(
        tokio::time::Instant::now() + Duration::from_millis(2), Duration::from_millis(2));
    emitter.heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let job = tokio::spawn(emitter.run());
    for _ in 0..2 { drop(body.next().await.unwrap().unwrap()); }
    let caller = tokio::spawn(async move {
        let result = framer_core::port::emit_assembled(result(SEGMENT * 8), &sink).await.unwrap();
        sink.request(Input::Finish(result)).await.unwrap();
    });
    reached(&budget.metrics.buffers, 5).await;
    tokio::time::sleep(Duration::from_millis(10)).await;
    let wire = drain_wire(&mut body).await;
    caller.await.unwrap(); job.await.unwrap();
    assert_heartbeat_boundary(&wire);
    assert_eq!(budget.counts(), (0, 0, 0));
}

async fn drain_wire(body: &mut OutputBody) -> Vec<u8> {
    let mut wire = Vec::new();
    while let Some(frame) = tokio::time::timeout(WAIT, body.next()).await.unwrap() {
        wire.extend_from_slice(&frame.unwrap());
    }
    wire
}
fn assert_heartbeat_boundary(wire: &[u8]) {
    let events: Vec<Value> = std::str::from_utf8(wire).unwrap().lines()
        .filter_map(|line| line.strip_prefix("data: "))
        .map(|line| serde_json::from_str(line).unwrap()).collect();
    let last_content = events.iter().rposition(|e| e["type"] == "response.output_item.done").unwrap();
    assert!(events[last_content + 1..].iter().any(|e| e["type"] == "response.in_progress"));
    assert_eq!(events.last().unwrap()["type"], "response.completed");
    assert!(events.windows(2).all(|pair| pair[1]["sequence_number"].as_u64().unwrap()
        == pair[0]["sequence_number"].as_u64().unwrap() + 1));
}

#[tokio::test]
async fn retained_polled_bytes_cannot_keep_body_cancellation_open() {
    let budget = OutputBudget::new(1);
    let (sink, mut body, emitter) = start(&budget).await;
    let caller = tokio::spawn(async move {
        framer_core::port::emit_assembled(result(SEGMENT * 20), &sink).await
    });
    let retained = body.next().await.unwrap().unwrap();
    reached(&budget.metrics.buffers, 5).await;
    drop(body);
    assert!(tokio::time::timeout(WAIT, caller).await.unwrap().unwrap().is_err());
    assert_eq!(budget.metrics.jobs.current.load(Ordering::SeqCst), 0);
    assert!(!emitter.is_finished());
    assert!(budget.reserve().is_err());
    drop(retained);
    tokio::time::timeout(WAIT, emitter).await.unwrap().unwrap();
    assert_eq!(budget.counts(), (0, 0, 0));
}

#[tokio::test(start_paused = true)]
async fn blocked_publication_records_ticks_before_output_credit_returns() {
    let budget = OutputBudget::new(1);
    let (mut emitter, sink, mut body) = output(budget.reserve().unwrap(), "fixture", "logical");
    let held = emitter.publisher.lease.output.clone()
        .acquire_many_owned(OUTPUT_SLOTS as u32).await.unwrap();
    let mut buffer = Buffer::new(budget.metrics.clone());
    buffer.bytes.extend_from_slice(b"data: fixture\n\n");
    assert!(!emitter.pending_heartbeat);
    {
        let publication = emitter.publish_segment(Segment {
            buffer, sequence: 0, content: false, last: true,
        });
        tokio::pin!(publication);
        assert!(futures::poll!(&mut publication).is_pending());
        tokio::time::advance(Duration::from_secs(24)).await;
        assert!(futures::poll!(&mut publication).is_pending());
        drop(held);
        assert!(matches!(futures::poll!(&mut publication), Poll::Ready(Ok(()))));
    }
    assert!(emitter.pending_heartbeat, "ticks must be recorded while credit is unavailable");
    assert_eq!(body.next().await.unwrap().unwrap().as_ref(), b"data: fixture\n\n");
    drop(body); drop(sink); drop(emitter);
    assert_eq!(budget.counts(), (0, 0, 0));
}

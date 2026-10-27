//! Bounded free Responses SSE. Publisher owns acceptance; emitter owns policy.
use super::*;
use framer_core::convert::{AssembledResponses, ResponseEncoder, ResponseWriter};
use framer_core::port::{OutputError, ResponseError, ResponseResult, ResponsesSink, ValidatedChunk};
use futures::{future::BoxFuture, Stream};
use std::{io::{self, Write}, pin::Pin, sync::{Mutex, atomic::{AtomicUsize, Ordering}},
    task::{Context, Poll}, time::{Duration, Instant}};
use tokio::sync::{mpsc, oneshot, watch, Notify, OwnedSemaphorePermit, Semaphore};
use framer_worker_wire::{credit::{Capacity, Window, data_cost, DATA_WIRE_RESERVE},
    frame::{self, WriteSlot}, message::{self as wire, Message}, WireError};

const SEGMENT: usize = 16 * 1024;
const OUTPUT_SLOTS: usize = 2;
pub(crate) const WORKER_OUTPUT: Capacity = Capacity {
    wire_bytes: DATA_WIRE_RESERVE * OUTPUT_SLOTS as u64,
    slots: OUTPUT_SLOTS as u64, downstream_bytes: (SEGMENT * OUTPUT_SLOTS) as u64,
};
pub(crate) const DEFAULT_UNOWNED_OUTPUT_LIMIT: usize = 256;

#[derive(Default)]
struct Counter { current: AtomicUsize, peak: AtomicUsize, changed: Notify }
impl Counter {
    fn add(&self, value: usize) {
        let next = self.current.fetch_add(value, Ordering::SeqCst) + value;
        self.peak.fetch_max(next, Ordering::SeqCst);
        self.changed.notify_one();
    }
    fn sub(&self, value: usize) {
        self.current.fetch_sub(value, Ordering::SeqCst);
        self.changed.notify_one();
    }
}
#[derive(Default)]
struct Metrics {
    leases: Counter, buffers: Counter, bytes: Counter, jobs: Counter,
    output_bytes: Counter, output_slots: Counter,
}

#[cfg(test)]
#[derive(Clone, Copy)]
pub(crate) enum Fault { BeforeContent, AfterFirstSegment, Panic }
pub(crate) struct OutputBudget {
    slots: Arc<Semaphore>, metrics: Arc<Metrics>,
    #[cfg(test)] fault: Mutex<Option<Fault>>,
}
impl OutputBudget {
    // Called only while the runtime is exclusively owned, before any request.
    pub(crate) fn construction_limit(&self) -> usize { self.slots.available_permits() }
    pub(crate) fn new(limit: usize) -> Self {
        Self { slots: Arc::new(Semaphore::new(limit)), metrics: Arc::new(Metrics::default()),
            #[cfg(test)] fault: Mutex::new(None) }
    }
    fn reserve(&self) -> Result<Arc<Lease>, ApiError> {
        let permit = self.slots.clone().try_acquire_owned()
            .map_err(|_| ApiError::ServingCapacity("response output capacity unavailable".into()))?;
        self.metrics.leases.add(1);
        Ok(Arc::new(Lease { _permit: permit, metrics: self.metrics.clone(),
            output: Arc::new(Semaphore::new(OUTPUT_SLOTS)), live_output: AtomicUsize::new(0),
            disposed: Notify::new(), timings: Arc::new(Mutex::new(Timings::default())), started: Instant::now(),
            #[cfg(test)] fault: *self.fault.lock().unwrap() }))
    }
    #[cfg(test)]
    pub(crate) fn inject(&self, fault: Fault) { *self.fault.lock().unwrap() = Some(fault); }
    #[cfg(test)]
    pub(crate) fn counts(&self) -> (usize, usize, usize) {
        (self.metrics.leases.current.load(Ordering::SeqCst),
            self.metrics.jobs.current.load(Ordering::SeqCst),
            self.metrics.bytes.current.load(Ordering::SeqCst))
    }
    #[cfg(test)]
    pub(crate) fn output_counts(&self) -> (usize, usize) {
        (self.metrics.output_bytes.current.load(Ordering::SeqCst),
            self.metrics.output_slots.current.load(Ordering::SeqCst))
    }
}
#[derive(Default, Debug)]
struct Timings {
    validated: Option<Duration>, job_started: Option<Duration>, first_enqueued: Option<Duration>,
    last_enqueued: Option<Duration>, joined: Option<Duration>, polled: Option<Duration>,
    disposed: Option<Duration>, credit_wait: Duration,
    serialization_wall: Duration, staging_wait: Duration,
}
struct Lease {
    _permit: OwnedSemaphorePermit, metrics: Arc<Metrics>, output: Arc<Semaphore>,
    live_output: AtomicUsize, disposed: Notify, timings: Arc<Mutex<Timings>>, started: Instant,
    #[cfg(test)] fault: Option<Fault>,
}
impl Drop for Lease { fn drop(&mut self) { self.metrics.leases.sub(1); } }
impl Lease {
    async fn wait_disposed(&self) {
        loop {
            let notified = self.disposed.notified();
            if self.live_output.load(Ordering::SeqCst) == 0 { return; }
            notified.await;
        }
    }
}

#[derive(Clone)]
pub(super) struct Cancel(watch::Receiver<bool>);
impl Cancel {
    pub(super) fn is_cancelled(&self) -> bool { *self.0.borrow() || self.0.has_changed().is_err() }
    pub(super) async fn cancelled(&self) {
        let mut receiver = self.0.clone();
        while !*receiver.borrow() { if receiver.changed().await.is_err() { return; } }
    }
}
struct Queued { data: bytes::Bytes, content: bool, lease: Arc<Lease> }
pub(crate) struct OutputBody {
    receiver: mpsc::Receiver<Queued>, cancel: watch::Sender<bool>,
    aborted: Arc<std::sync::atomic::AtomicBool>, failed: bool,
}
impl Stream for OutputBody {
    type Item = Result<bytes::Bytes, io::Error>;
    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let item = self.receiver.poll_recv(cx);
        if matches!(item, Poll::Ready(None)) && !self.failed && self.aborted.load(Ordering::SeqCst) {
            self.failed = true;
            return Poll::Ready(Some(Err(io::Error::other("incomplete response event"))));
        }
        item.map(|item| item.map(|item| {
            if item.content {
                item.lease.timings.lock().unwrap().polled.get_or_insert(item.lease.started.elapsed());
            }
            Ok(item.data)
        }))
    }
}
impl Drop for OutputBody { fn drop(&mut self) { self.cancel.send_replace(true); } }

struct Buffer { bytes: Vec<u8>, metrics: Arc<Metrics> }
impl Buffer {
    fn new(metrics: Arc<Metrics>) -> Self {
        let bytes = Vec::with_capacity(SEGMENT);
        metrics.bytes.add(bytes.capacity()); metrics.buffers.add(1);
        Self { bytes, metrics }
    }
}
impl Drop for Buffer {
    fn drop(&mut self) { self.metrics.bytes.sub(self.bytes.capacity()); self.metrics.buffers.sub(1); }
}
struct Segment { buffer: Buffer, sequence: u64, content: bool, last: bool }

struct OwnedBytes { buffer: Option<Buffer>, lease: Arc<Lease>, slot: Option<OwnedSemaphorePermit> }
struct WorkerBytes {
    // Field order disposes bytes/slot before returning credit and dropping the pin.
    inner: OwnedBytes, _credit: CreditReturn, _pin: crate::policy_generations::GenerationPin,
}
impl AsRef<[u8]> for WorkerBytes { fn as_ref(&self) -> &[u8] { self.inner.as_ref() } }
impl AsRef<[u8]> for OwnedBytes { fn as_ref(&self) -> &[u8] { &self.buffer.as_ref().unwrap().bytes } }
impl Drop for OwnedBytes {
    fn drop(&mut self) {
        let capacity = self.buffer.as_ref().unwrap().bytes.capacity();
        drop(self.buffer.take());
        drop(self.slot.take());
        self.lease.metrics.output_bytes.sub(capacity);
        self.lease.metrics.output_slots.sub(1);
        if self.lease.live_output.fetch_sub(1, Ordering::SeqCst) == 1 {
            self.lease.timings.lock().unwrap().disposed = Some(self.lease.started.elapsed());
            self.lease.disposed.notify_one();
        }
    }
}
struct Publisher {
    sender: Option<mpsc::Sender<Queued>>, lease: Arc<Lease>, cancel: Cancel,
    next_sequence: u64, partial: bool, content_published: Arc<std::sync::atomic::AtomicBool>,
    aborted: Arc<std::sync::atomic::AtomicBool>,
}
impl Publisher {
    async fn publish(&mut self, segment: Segment) -> Result<(), OutputError> {
        if segment.sequence != self.next_sequence { return Err(OutputError::Encoding); }
        let start = Instant::now();
        let slot = tokio::select! {
            biased;
            _ = self.cancel.cancelled() => return Err(OutputError::Cancelled),
            slot = self.lease.output.clone().acquire_owned() => slot.map_err(|_| OutputError::Closed)?,
        };
        self.lease.timings.lock().unwrap().credit_wait += start.elapsed();
        self.accept(segment, slot)
    }
    fn accept(&mut self, segment: Segment, slot: OwnedSemaphorePermit) -> Result<(), OutputError> {
        self.accept_retained(segment, slot, None)
    }
    fn accept_retained(&mut self, segment: Segment, slot: OwnedSemaphorePermit,
        retention: Option<(CreditReturn, crate::policy_generations::GenerationPin)>)
        -> Result<(), OutputError> {
        let (content, last) = (segment.content, segment.last);
        self.lease.metrics.output_bytes.add(segment.buffer.bytes.capacity());
        self.lease.metrics.output_slots.add(1);
        self.lease.live_output.fetch_add(1, Ordering::SeqCst);
        let owner = OwnedBytes {
            buffer: Some(segment.buffer), lease: self.lease.clone(), slot: Some(slot) };
        let data = match retention {
            Some((credit, pin)) => bytes::Bytes::from_owner(WorkerBytes {
                inner: owner, _credit: credit, _pin: pin }),
            None => bytes::Bytes::from_owner(owner),
        };
        self.sender.as_ref().ok_or(OutputError::Closed)?.try_send(Queued {
            data, content, lease: self.lease.clone(),
        }).map_err(|_| OutputError::Closed)?;
        self.partial = !last;
        if last { self.next_sequence += 1; }
        if content {
            self.content_published.store(true, Ordering::SeqCst);
            let mut timings = self.lease.timings.lock().unwrap();
            timings.first_enqueued.get_or_insert(self.lease.started.elapsed());
            timings.last_enqueued = Some(self.lease.started.elapsed());
        }
        Ok(())
    }
}

struct SegmentWriter {
    sender: mpsc::Sender<Segment>, buffer: Option<Buffer>, metrics: Arc<Metrics>,
    sequence: u64, content: bool, staging_wait: Duration,
    #[cfg(test)] fault: Option<Fault>,
    #[cfg(test)] written_content: usize,
}
impl SegmentWriter {
    fn send(&mut self, last: bool) -> io::Result<()> {
        let Some(buffer) = self.buffer.take() else { return Ok(()); };
        let started = Instant::now();
        let result = self.sender.blocking_send(Segment { buffer, sequence: self.sequence,
            content: self.content, last }).map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "output closed"));
        self.staging_wait += started.elapsed();
        result
    }
}
impl Write for SegmentWriter {
    fn write(&mut self, mut bytes: &[u8]) -> io::Result<usize> {
        let length = bytes.len();
        while !bytes.is_empty() {
            #[cfg(test)]
            let allowance = self.fault_allowance()?;
            if self.buffer.as_ref().is_some_and(|b| b.bytes.len() == SEGMENT) { self.send(false)?; }
            let buffer = self.buffer.get_or_insert_with(|| Buffer::new(self.metrics.clone()));
            let count = bytes.len().min(SEGMENT - buffer.bytes.len());
            #[cfg(test)]
            let count = count.min(allowance);
            buffer.bytes.extend_from_slice(&bytes[..count]);
            #[cfg(test)]
            if self.content { self.written_content += count; }
            bytes = &bytes[count..];
        }
        Ok(length)
    }
    fn flush(&mut self) -> io::Result<()> { Ok(()) }
}
impl ResponseWriter for SegmentWriter {
    fn begin_event(&mut self, sequence: u64, content: bool) -> io::Result<()> {
        #[cfg(test)]
        { self.written_content = 0; }
        self.sequence = sequence; self.content = content; Ok(())
    }
    fn end_event(&mut self) -> io::Result<()> {
        if self.buffer.is_none() { return Err(io::Error::other("empty SSE event")); }
        self.send(true)
    }
}

enum Input {
    Open, Heartbeat, Chunk(ValidatedChunk), Finish(AssembledResponses), Fail(String),
    FailureAt(u64, String),
}
struct Command { input: Input, reply: oneshot::Sender<Result<Option<ValidatedChunk>, OutputError>> }
struct Sink {
    sender: mpsc::Sender<Command>, serial: tokio::sync::Mutex<()>, lease: Arc<Lease>,
}
impl Sink {
    async fn request(&self, input: Input) -> Result<Option<ValidatedChunk>, OutputError> {
        let _command = self.serial.lock().await;
        if matches!(input, Input::Chunk(_)) {
            self.lease.timings.lock().unwrap().validated.get_or_insert(self.lease.started.elapsed());
        }
        let (reply, receive) = oneshot::channel();
        self.sender.send(Command { input, reply }).await.map_err(|_| OutputError::Closed)?;
        receive.await.map_err(|_| OutputError::Closed)?
    }
}
impl ResponsesSink for Sink {
    fn emit<'s, 'a>(&'s self, chunk: ValidatedChunk)
        -> BoxFuture<'a, Result<ValidatedChunk, OutputError>> where 's: 'a, Self: 'a {
        Box::pin(async move { self.request(Input::Chunk(chunk)).await?
            .ok_or(OutputError::Encoding) })
    }
}
struct AttemptSink<'a> {
    output: &'a Sink, cancel: &'a Cancel,
    completed_on_disconnect: std::sync::atomic::AtomicBool,
}
impl ResponsesSink for AttemptSink<'_> {
    fn emit<'s, 'a>(&'s self, chunk: ValidatedChunk)
        -> BoxFuture<'a, Result<ValidatedChunk, OutputError>> where 's: 'a, Self: 'a {
        Box::pin(async move {
            let complete = chunk.completes_chain();
            let result = self.output.emit(chunk).await;
            if complete && self.cancel.is_cancelled()
                && matches!(&result, Err(OutputError::Cancelled | OutputError::Closed)) {
                self.completed_on_disconnect.store(true, Ordering::SeqCst);
            }
            result
        })
    }
}
struct Emitter {
    publisher: Publisher, encoder: Option<ResponseEncoder>, commands: mpsc::Receiver<Command>,
    heartbeat: tokio::time::Interval, pending_heartbeat: bool,
}
struct JobCount(Arc<Metrics>);
impl Drop for JobCount { fn drop(&mut self) { self.0.jobs.sub(1); } }
impl Emitter {
    async fn publish_segment(&mut self, segment: Segment) -> Result<(), OutputError> {
        let publish = self.publisher.publish(segment);
        tokio::pin!(publish);
        loop {
            tokio::select! {
                result = &mut publish => return result,
                _ = self.heartbeat.tick() => self.pending_heartbeat = true,
            }
        }
    }
    async fn encode(&mut self, input: Input) -> Result<Input, OutputError> {
        if self.publisher.cancel.is_cancelled() { return Err(OutputError::Cancelled); }
        let mut encoder = self.encoder.take().ok_or(OutputError::Encoding)?;
        let (sender, mut receiver) = mpsc::channel(1);
        let metrics = self.publisher.lease.metrics.clone();
        metrics.jobs.add(1);
        let count = JobCount(metrics.clone());
        let lease = self.publisher.lease.clone();
        let job = tokio::task::spawn_blocking(move || {
            let _count = count;
            let started = Instant::now();
            if matches!(input, Input::Chunk(_)) {
                lease.timings.lock().unwrap().job_started.get_or_insert(lease.started.elapsed());
            }
            let mut writer = SegmentWriter { sender, buffer: None, metrics, sequence: 0,
                content: false, staging_wait: Duration::ZERO,
                #[cfg(test)] fault: lease.fault, #[cfg(test)] written_content: 0 };
            let result = match &input {
                Input::Open => encoder.open(&mut writer),
                Input::Heartbeat => encoder.heartbeat(&mut writer),
                Input::Chunk(chunk) => encoder.push_items(&mut writer, chunk.items(), chunk.continues_text()),
                Input::Finish(result) => encoder.finish(&mut writer, result),
                Input::Fail(error) => encoder.fail(&mut writer, error),
                Input::FailureAt(sequence, error) => ResponseEncoder::fail_at(&mut writer, *sequence, error),
            };
            let mut timings = lease.timings.lock().unwrap();
            timings.serialization_wall += started.elapsed();
            timings.staging_wait += writer.staging_wait;
            drop(timings);
            (encoder, input, result)
        });
        let mut output = Ok(());
        loop {
            tokio::select! {
                biased;
                _ = self.publisher.cancel.cancelled() => { output = Err(OutputError::Cancelled); break; }
                segment = receiver.recv() => match segment {
                    Some(segment) => {
                        let accepted = self.publish_segment(segment).await;
                        if let Err(error) = accepted { output = Err(error); break; }
                    },
                    None => break,
                },
                _ = self.heartbeat.tick() => self.pending_heartbeat = true,
            }
        }
        // Drop queued/local buffers and wake a blocked writer before joining it.
        drop(receiver);
        let joined = job.await.map_err(|_| OutputError::Encoding);
        self.publisher.lease.timings.lock().unwrap().joined = Some(self.publisher.lease.started.elapsed());
        let (encoder, input, encoded) = joined?;
        self.encoder = Some(encoder);
        output?;
        encoded.map_err(|_| OutputError::Encoding)?;
        Ok(input)
    }
    async fn run(mut self) {
        if self.encode(Input::Open).await.is_ok() { self.commands().await; }
        else { self.fail_fixed("response output encoding failed").await; }
        // EOF depends only on this close, never on the following disposal wait.
        self.commands.close();
        while self.commands.try_recv().is_ok() {}
        self.publisher.sender.take();
        self.publisher.lease.wait_disposed().await;
    }
    async fn fail_fixed(&mut self, message: &str) {
        if self.publisher.cancel.is_cancelled() { return; }
        if self.publisher.partial {
            self.publisher.aborted.store(true, Ordering::SeqCst);
            return;
        }
        self.encoder.get_or_insert_with(|| ResponseEncoder::new("", ""));
        let input = Input::FailureAt(self.publisher.next_sequence, message.to_owned());
        if self.encode(input).await.is_err() { self.publisher.aborted.store(true, Ordering::SeqCst); }
    }
    async fn commands(&mut self) {
        loop {
            let command = tokio::select! {
                biased;
                _ = self.publisher.cancel.cancelled() => return,
                command = self.commands.recv() => match command {
                    Some(command) => command,
                    None => { self.fail_fixed("upstream operation task failed").await; return; }
                },
                _ = self.heartbeat.tick() => {
                    if self.encode(Input::Heartbeat).await.is_err() {
                        self.fail_fixed("response output encoding failed").await; return;
                    }
                    continue;
                },
            };
            let terminal = matches!(command.input, Input::Finish(_) | Input::Fail(_));
            let result = self.encode(command.input).await;
            let failed = result.is_err();
            let _ = command.reply.send(result.map(|input| match input {
                Input::Chunk(chunk) => Some(chunk), _ => None,
            }));
            if failed { self.fail_fixed("response output encoding failed").await; return; }
            if terminal { self.pending_heartbeat = false; return; }
            if self.pending_heartbeat {
                self.pending_heartbeat = false;
                if self.encode(Input::Heartbeat).await.is_err() {
                    self.fail_fixed("response output encoding failed").await; return;
                }
            }
        }
    }
}

fn output(lease: Arc<Lease>, model: &str, id: &str) -> (Emitter, Sink, OutputBody) {
    let (sender, receiver) = mpsc::channel(OUTPUT_SLOTS);
    let (commands, inbox) = mpsc::channel(1);
    let (cancel, cancelled) = watch::channel(false);
    let aborted = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let publisher = Publisher { sender: Some(sender), lease: lease.clone(), cancel: Cancel(cancelled),
        next_sequence: 0, partial: false, content_published: Arc::new(false.into()), aborted: aborted.clone() };
    let mut heartbeat = tokio::time::interval_at(tokio::time::Instant::now() + Duration::from_secs(8),
        Duration::from_secs(8));
    heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    (Emitter { publisher, encoder: Some(ResponseEncoder::new(model, id)), commands: inbox,
        heartbeat, pending_heartbeat: false }, Sink { sender: commands, serial: tokio::sync::Mutex::new(()), lease },
        OutputBody { receiver, cancel, aborted, failed: false })
}

pub(super) struct FreeRequest {
    pub state: AppState, pub account: ManagedAccount, pub residency: RequestResidency,
    pub session: framer_core::port::Session, pub project: framer_core::ids::ProjectId,
    pub config: framer_core::port::ChainConfig, pub payload: Value,
    pub gate_model: String, pub gate_key: String, pub first_permit: Option<OwnedSemaphorePermit>,
    pub chat_session: String, pub explicit_session: Option<String>, pub allow_repick: bool,
    pub client_model: String, pub logical_id: String,
}
pub(super) type InvocationResult<T> = (Result<T, ResponseError>, bool);
/// Only policy invocation varies. The one account/health/repick loop stays here.
pub(super) trait Invocation: Send {
    type Output: Send;
    fn invoke<'a>(&'a mut self, request: &'a FreeRequest, cancel: &'a Cancel)
        -> BoxFuture<'a, InvocationResult<Self::Output>>;
}
struct LegacyInvocation<'a>(&'a Sink);
impl Invocation for LegacyInvocation<'_> {
    type Output = AssembledResponses;
    fn invoke<'a>(&'a mut self, request: &'a FreeRequest, cancel: &'a Cancel)
        -> BoxFuture<'a, InvocationResult<Self::Output>> {
        Box::pin(async move {
            let output = AttemptSink { output: self.0, cancel, completed_on_disconnect: false.into() };
            let result = request.state.client.native_long_with_sink(&request.session,
                request.state.browser.as_ref(), &request.account.id, &request.project,
                request.payload.clone(), &request.config, &request.chat_session, &output).await;
            (result, output.completed_on_disconnect.load(Ordering::SeqCst))
        })
    }
}
pub(super) async fn serve(mut request: FreeRequest) -> Result<axum::response::Response, ApiError> {
    if request.config.is_cancelled() {
        let (status, body) = convert::error_json(ClientDialect::OpenAiResponses, "client cancelled");
        return Ok(operation_ownership::completed_response(axum::response::IntoResponse::into_response(
            ShapedApiError(StatusCode::from_u16(status).unwrap(), body)), true));
    }
    let lease = request.state.response_output.reserve()?;
    let (emitter, sink, body) = output(lease, &request.client_model, &request.logical_id);
    let cancelled = emitter.publisher.cancel.clone();
    let published = emitter.publisher.content_published.clone();
    let cancel = cancelled.clone();
    request.config.cancelled = Some(Arc::new(move || cancel.is_cancelled()));
    let session_header = request.chat_session.clone();
    let residency = request.residency.clone();
    let scope = request.state.operation_ownership.clone();
    let (handle, completion) = operation_ownership::spawn_observed(scope.as_ref(),
        framer_core::operation_gate::OperationKind::Inference, async move {
            // This owner survives a dropped request future and joins all serializers.
            let encoding = tokio::spawn(emitter.run());
            let started = Instant::now();
            let (result, cleanup) = request.drive(&mut LegacyInvocation(&sink), &cancelled, &published).await;
            request.log(&result, started.elapsed());
            let result = match result {
                Ok(result) => sink.request(Input::Finish(result)).await.map(|_| ()).map_err(|e| e.to_string()),
                Err(ResponseError::Upstream(error)) => {
                    let _ = sink.request(Input::Fail(error.clone())).await;
                    Err(error)
                }
                Err(ResponseError::Output(error)) => Err(error.to_string()),
            };
            drop(sink);
            let joined = encoding.await.is_ok();
            (result, cleanup && joined)
        }).map_err(|_| ApiError::ServingCapacity("operation admission closed".into()))?;
    drop(handle); // The shared completion still observes the original producer.
    let mut response = axum::response::Response::new(axum::body::Body::from_stream(
        retain_owner(body, residency)));
    response.extensions_mut().insert(completion);
    response.headers_mut().insert(axum::http::header::CONTENT_TYPE,
        axum::http::HeaderValue::from_static("text/event-stream"));
    response.headers_mut().insert(axum::http::header::CACHE_CONTROL,
        axum::http::HeaderValue::from_static("no-cache"));
    response.headers_mut().insert("x-framerize-session",
        session_header.parse().unwrap_or(axum::http::HeaderValue::from_static("")));
    Ok(response)
}
impl FreeRequest {
    pub(super) async fn drive<I: Invocation>(&mut self, invocation: &mut I, cancel: &Cancel,
        published: &std::sync::atomic::AtomicBool) -> (Result<I::Output, ResponseError>, bool) {
        let mut tried = Vec::new();
        let mut attempts = 0u32;
        let mut cleanup = true;
        let mut cooldown = account_wait::MAX_WAIT;
        let result = loop {
            if let Err(error) = self.config.check_cancelled() { break Err(error.into()); }
            let permit = match self.first_permit.take() {
                Some(permit) => permit,
                None => tokio::select! {
                    _ = cancel.cancelled() => break Err("client cancelled".into()),
                    permit = self.state.gate.acquire(&self.gate_key) => permit,
                },
            };
            if let Err(error) = self.config.check_cancelled() { break Err(error.into()); }
            let admission = current_account(&self.state, self.account.id).await;
            let admitted = admission.is_ok();
            let (result, completed_on_disconnect) = match admission {
                Err(error) => (Err(chain_admission_error(error).into()), false),
                Ok(_) => invocation.invoke(self, cancel).await,
            };
            drop(permit);
            // Keep a joined terminal success when delivery is cancelled.
            // Local encoding failures and intermediate chunks cannot earn it.
            let result = match result {
                Err(ResponseError::Output(error)) => {
                    if admitted && completed_on_disconnect
                        && matches!(error, OutputError::Cancelled | OutputError::Closed) {
                        record_account_health(&self.state, &self.account, &Ok::<(), String>(())).await;
                    }
                    break Err(error.into());
                }
                Err(ResponseError::Upstream(error)) => Err(error),
                Ok(value) => Ok(value),
            };
            cleanup &= !matches!(&result, Err(error) if pool_control::cleanup_unconfirmed(error));
            let result = self.residency.observe_result(self.account.id, result);
            if admitted && !matches!(&result, Err(error) if error == "client cancelled") {
                record_account_health(&self.state, &self.account, &result).await;
            }
            if self.config.is_cancelled() { break result.map_err(Into::into); }
            let refusal = matches!(&result, Err(error) if should_retry_account(error));
            if !(self.allow_repick && refusal && !published.load(Ordering::SeqCst) && attempts < 2) {
                break result.map_err(Into::into);
            }
            let local = matches!(&result, Err(error) if AccountCooldown::from_error(error).is_some());
            if !local { tried.push(self.account.id.0.to_string()); }
            drop(self.residency.take());
            let acquired = tokio::select! {
                _ = cancel.cancelled() => break Err("client cancelled".into()),
                value = account_wait::pick(&self.state, &tried, local && cleanup,
                    &mut cooldown) => value,
            };
            cleanup &= acquired.cleanup_confirmed;
            let (next, lease) = match acquired.result {
                Ok(value) => value,
                Err(_) => break result.map_err(Into::into),
            };
            self.residency.replace(lease);
            framer_core::observability::Event::new("ai.chain.repick")
                .account(&self.account.id).field("to", &next.id.0.to_string()[..8])
                .field("reason", result.as_ref().err().map(|e| error_excerpt(e, 120)).unwrap_or(""))
                .emit();
            self.account = next;
            self.session = match self.state.sessions.load(&self.account.framer_user_id).await {
                Ok(Some(session)) => session, _ => break result.map_err(Into::into),
            };
            let scope = match pick_scope(&self.state, &self.account) {
                Ok(scope) => scope, Err(_) => break result.map_err(Into::into),
            };
            self.project = scope.project.clone();
            self.gate_key = format!("{}:{}", scope.team.0, self.gate_model);
            self.chat_session = self.explicit_session.clone().unwrap_or_else(|| stable_session(&self.account.id));
            let team = scope.team.0.clone();
            let rotation = self.state.serving_accounts.rotation().clone();
            self.config.on_429 = Some(Arc::new(move || rotation.mark_saturated(&team, 45_000)));
            attempts += 1;
        };
        (result, cleanup)
    }
    fn log(&self, result: &ResponseResult, elapsed: Duration) {
        self.log_fields(result.as_ref().map(|result| (result.chunks, result.metered_chunks,
            result.input_tokens, result.output_tokens, result.status.as_str())), elapsed);
    }
    pub(super) fn log_worker(&self, result: &Result<wire::Summary, ResponseError>, elapsed: Duration) {
        self.log_fields(result.as_ref().map(|result| (result.chunks, result.metered_chunks,
            result.input_tokens, result.output_tokens, match result.finish {
                wire::FinishReason::Completed => "completed", wire::FinishReason::Incomplete => "incomplete",
            })), elapsed);
    }
    fn log_fields(&self, result: Result<(u32, u32, u64, u64, &str), &ResponseError>, elapsed: Duration) {
        let event = operation_ownership::correlate_chain(framer_core::observability::Event::new("ai.chain"))
            .account(&self.account.id).field("model", self.gate_model.as_str())
            .field("dialect", "OpenAiResponses").duration_ms(elapsed.as_millis());
        match result {
            Ok((chunks, metered, input, output, status)) => event.field("chunks", chunks).field("metered_chunks", metered)
                .field("prompt_tokens", input).field("completion_tokens", output)
                .field("finish_reason", status).outcome("ok").emit(),
            Err(error) => event.error(&error.to_string()).emit(),
        }
    }
}

struct CreditState {
    window: Window, pending: Capacity, accepted: u64, sent: u64,
    done: bool, failed: bool,
}
struct WorkerOutputState {
    publisher: Option<Publisher>, received: u64, kind: Option<wire::DataKind>,
    head: bool, finish_allowed: bool, terminal: bool,
}
pub(crate) struct WorkerPublisher {
    state: Mutex<WorkerOutputState>, pub credit: Arc<WorkerCredit>, lease: Arc<Lease>,
    pin: crate::policy_generations::GenerationPin,
}
pub(crate) struct WorkerOutput {
    pub publisher: Arc<WorkerPublisher>, pub body: OutputBody,
    pub cancel: Cancel, pub published: Arc<std::sync::atomic::AtomicBool>,
}
impl WorkerOutputState {
    fn receive(&mut self, event: wire::DataKind, seq: u64, last: bool) -> Result<(), WireError> {
        if !self.head || self.terminal || seq != self.received
            || self.kind.is_some_and(|kind| kind != event)
            || (event == wire::DataKind::Opening) != (seq < 2)
            || (event == wire::DataKind::Terminal && !self.finish_allowed) {
            return Err(WireError::Protocol);
        }
        if last { self.received = seq.checked_add(1).ok_or(WireError::Protocol)?; }
        self.kind = if last { None } else { Some(event) };
        Ok(())
    }
}
impl WorkerPublisher {
    pub(crate) fn terminal(&self) -> Result<bool, WireError> {
        Ok(self.state.lock().map_err(|_| WireError::Protocol)?.terminal)
    }
    /// Called by the single reader. Never waits for the body or a credit write.
    pub(crate) fn data(&self, event: wire::DataKind, seq: u64, bytes: wire::Segment,
        last: bool, wire_bytes: u64) -> Result<(), WireError> {
        let mut state = self.state.lock().map_err(|_| WireError::Protocol)?;
        state.receive(event, seq, last)?;
        let credit = self.credit.receive(wire_bytes)?;
        let publisher = state.publisher.as_mut().ok_or(WireError::Protocol)?;
        if publisher.cancel.is_cancelled() {
            drop(bytes);
            drop(credit);
            return Ok(());
        }
        let bytes = bytes.into_vec();
        if bytes.capacity() > SEGMENT { return Err(WireError::Credit); }
        let slot = self.lease.output.clone().try_acquire_owned().map_err(|_| WireError::Credit)?;
        self.lease.metrics.bytes.add(bytes.capacity());
        self.lease.metrics.buffers.add(1);
        let content = event == wire::DataKind::Content && !bytes.is_empty();
        let segment = Segment { buffer: Buffer { bytes, metrics: self.lease.metrics.clone() },
            sequence: seq, content, last };
        match publisher.accept_retained(segment, slot, Some((credit, self.pin.clone()))) {
            Ok(()) => {
                if last { self.credit.accepted(publisher.next_sequence)?; }
                if event == wire::DataKind::Terminal && last { state.terminal = true; }
                Ok(())
            }
            Err(_) if publisher.cancel.is_cancelled() => Ok(()),
            Err(_) => Err(WireError::Protocol),
        }
    }
    pub(crate) fn head(&self, status: u16, content_type: &str) -> Result<(), WireError> {
        let mut state = self.state.lock().map_err(|_| WireError::Protocol)?;
        if state.head || status != 200 || content_type != "text/event-stream" {
            return Err(WireError::Protocol);
        }
        state.head = true;
        Ok(())
    }
    pub(crate) fn allow_finish(&self) -> Result<(), WireError> {
        let mut state = self.state.lock().map_err(|_| WireError::Protocol)?;
        if !state.head || state.finish_allowed { return Err(WireError::Protocol); }
        state.finish_allowed = true;
        Ok(())
    }
    pub(crate) async fn disposed(&self) { self.lease.wait_disposed().await; }
    pub(crate) fn done(&self, outcome: wire::DoneOutcome) -> Result<(), WireError> {
        let mut state = self.state.lock().map_err(|_| WireError::Protocol)?;
        if !state.head || outcome == wire::DoneOutcome::Panicked
            || (outcome == wire::DoneOutcome::Completed && !state.terminal) {
            return Err(WireError::Protocol);
        }
        let publisher = state.publisher.as_mut().ok_or(WireError::Protocol)?;
        if publisher.sender.take().is_none() { return Err(WireError::Protocol); }
        // Body EOF is independent of both final credit writes and Bytes disposal.
        self.credit.done();
        Ok(())
    }
    pub(crate) async fn channel_lost(&self) {
        self.credit.fail();
        let (publisher, terminal) = {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            (state.publisher.take(), state.terminal)
        };
        let Some(publisher) = publisher else { return; };
        let (_, commands) = mpsc::channel(1);
        let mut heartbeat = tokio::time::interval_at(tokio::time::Instant::now()
            + Duration::from_secs(8), Duration::from_secs(8));
        heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut emitter = Emitter { publisher, encoder: None, commands, heartbeat,
            pending_heartbeat: false };
        if !terminal { emitter.fail_fixed("upstream operation task failed").await; }
        emitter.publisher.sender.take();
    }
}
impl WorkerOutput {
    pub(crate) fn reserve(budget: &OutputBudget, pin: crate::policy_generations::GenerationPin)
        -> Result<Self, ApiError> {
        let lease = budget.reserve()?;
        let (sender, receiver) = mpsc::channel(OUTPUT_SLOTS);
        let (cancel, cancelled) = watch::channel(false);
        let cancelled = Cancel(cancelled);
        let aborted = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let published = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let publisher = Publisher { sender: Some(sender), lease: lease.clone(), cancel: cancelled.clone(),
            next_sequence: 0, partial: false, content_published: published.clone(), aborted: aborted.clone() };
        Ok(Self { body: OutputBody { receiver, cancel, aborted, failed: false },
            cancel: cancelled, published, publisher: Arc::new(WorkerPublisher {
                state: Mutex::new(WorkerOutputState { publisher: Some(publisher), received: 0,
                    kind: None, head: false, finish_allowed: false, terminal: false }),
                credit: WorkerCredit::new(), lease, pin,
            }) })
    }
}
pub(crate) struct WorkerCredit { state: Mutex<CreditState>, changed: Notify }
impl WorkerCredit {
    #[cfg(test)]
    pub(crate) fn accounting(&self) -> (Capacity, Capacity, u64) {
        let state = self.state.lock().unwrap();
        (state.window.outstanding(), state.pending, state.accepted)
    }
    async fn send_returns(&self, op: u64, slot: &WriteSlot, alive: &mut watch::Receiver<bool>)
        -> Result<(), WireError> {
        loop {
            let changed = self.changed.notified();
            let pending = {
                let state = self.state.lock().map_err(|_| WireError::Protocol)?;
                if state.failed || !*alive.borrow() { return Err(WireError::Protocol); }
                if state.done && state.window.outstanding() == Capacity::default() { return Ok(()); }
                state.pending != Capacity::default() || state.sent != state.accepted
            };
            if !pending {
                tokio::select! { _ = changed => {}, _ = alive.changed() => {} }
                continue;
            }
            let permit = tokio::select! {
                permit = slot.reserve() => permit?,
                _ = alive.changed() => continue,
            };
            let receipt = {
                let mut state = self.state.lock().map_err(|_| WireError::Protocol)?;
                if state.failed || !*alive.borrow() { return Err(WireError::Protocol); }
                let returned = state.pending;
                let bytes = frame::encode(&Message::Credit { op, returned,
                    accepted_event_seq: state.accepted })?;
                let receipt = permit.enqueue_encoded(bytes)?;
                state.window.release(returned)?;
                state.pending = Capacity::default();
                state.sent = state.accepted;
                receipt
            };
            receipt.wait().await?; // The owning writer completes every accepted frame.
        }
    }
    fn accepted(&self, next: u64) -> Result<(), WireError> {
        let mut state = self.state.lock().map_err(|_| WireError::Protocol)?;
        if next != state.accepted.checked_add(1).ok_or(WireError::Credit)? {
            return Err(WireError::Credit);
        }
        state.accepted = next;
        self.changed.notify_one();
        Ok(())
    }
    fn done(&self) {
        self.state.lock().unwrap_or_else(|e| e.into_inner()).done = true;
        self.changed.notify_one();
    }
    pub(crate) fn fail(&self) {
        self.state.lock().unwrap_or_else(|e| e.into_inner()).failed = true;
        self.changed.notify_one();
    }
    pub(crate) async fn run(&self, op: u64, slot: WriteSlot, mut alive: watch::Receiver<bool>)
        -> Result<(), WireError> {
        let result = self.send_returns(op, &slot, &mut alive).await;
        if result.is_err() { self.fail(); }
        result
    }
    fn new() -> Arc<Self> {
        Arc::new(Self { state: Mutex::new(CreditState {
            window: Window::new(WORKER_OUTPUT).expect("fixed nonzero credit"),
            pending: Capacity::default(), accepted: 0, sent: 0, done: false, failed: false,
        }), changed: Notify::new() })
    }
    fn receive(self: &Arc<Self>, wire_bytes: u64) -> Result<CreditReturn, WireError> {
        let mut state = self.state.lock().map_err(|_| WireError::Protocol)?;
        if state.done || state.failed || wire_bytes == 0 || wire_bytes > DATA_WIRE_RESERVE {
            return Err(WireError::Credit);
        }
        state.window.reserve(data_cost(wire_bytes))?;
        state.pending.wire_bytes += wire_bytes;
        self.changed.notify_one();
        Ok(CreditReturn(self.clone()))
    }
}
struct CreditReturn(Arc<WorkerCredit>);
impl Drop for CreditReturn {
    fn drop(&mut self) {
        let mut state = self.0.state.lock().unwrap_or_else(|e| e.into_inner());
        state.pending.slots += 1;
        state.pending.downstream_bytes += SEGMENT as u64;
        self.0.changed.notify_one();
    }
}

#[cfg(test)]
#[path = "response_output_tests.rs"]
mod tests;

#[cfg(test)]
impl SegmentWriter {
    fn fault_allowance(&self) -> io::Result<usize> {
        if !self.content { return Ok(usize::MAX); }
        let limit = match self.fault {
            None => return Ok(usize::MAX),
            Some(Fault::BeforeContent) => 0,
            Some(Fault::AfterFirstSegment) => SEGMENT + 16,
            Some(Fault::Panic) => panic!("injected encoder panic"),
        };
        if self.written_content >= limit { return Err(io::Error::other("injected encoder failure")); }
        Ok(limit - self.written_content)
    }
}

#[cfg(test)]
pub(crate) async fn consume_body(body: axum::body::Body, limit: usize)
    -> Result<bytes::Bytes, axum::Error> {
    use futures::StreamExt;
    let mut body = body.into_data_stream();
    let mut collected = Vec::new();
    while let Some(chunk) = body.next().await {
        let chunk = chunk?;
        if chunk.len() > limit.saturating_sub(collected.len()) {
            // Produce the existing axum length-limit error without a large allocation.
            return axum::body::to_bytes(axum::body::Body::from("x"), 0).await;
        }
        collected.extend_from_slice(&chunk);
    }
    Ok(bytes::Bytes::from(collected))
}

/// Fixture-only partial consumption. Retain the original SSE body after return.
#[cfg(test)]
pub(crate) async fn pump_until_producer_returns(
    response: axum::response::Response, returned: impl Fn() -> bool,
) -> axum::response::Response {
    use futures::{FutureExt, StreamExt};
    let (parts, body) = response.into_parts();
    let mut body = body.into_data_stream();
    let (mut prefix, mut frames) = (Vec::with_capacity(65_536), 0);
    tokio::time::timeout(Duration::from_secs(2), async {
        while !returned() {
            assert!(frames < 64, "producer did not return within 64 frames");
            if let Some(frame) = body.next().now_or_never() {
                let bytes = frame.expect("unexpected EOF before producer return")
                    .expect("body failed before producer return");
                assert!(bytes.len() <= 65_536 - prefix.len(), "producer prefix exceeds 65536 bytes");
                prefix.extend_from_slice(&bytes);
                drop(bytes);
                frames += 1;
            }
            tokio::task::yield_now().await;
        }
    }).await.expect("producer did not return within 2 seconds");
    let prefix = (!prefix.is_empty()).then(|| Ok::<_, axum::Error>(bytes::Bytes::from(prefix)));
    let stream = futures::stream::iter(prefix).chain(body);
    axum::response::Response::from_parts(parts, axum::body::Body::from_stream(stream))
}

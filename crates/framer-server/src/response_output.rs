//! Bounded free Responses SSE. Publisher owns acceptance; emitter owns policy.
use super::*;
use framer_core::convert::{AssembledResponses, ResponseEncoder, ResponseWriter};
use framer_core::port::{OutputError, ResponseError, ResponseResult, ResponsesSink, ValidatedChunk};
use futures::{future::BoxFuture, Stream};
use std::{io::{self, Write}, pin::Pin, sync::{Mutex, atomic::{AtomicUsize, Ordering}},
    task::{Context, Poll}, time::{Duration, Instant}};
use tokio::sync::{mpsc, oneshot, watch, Notify, OwnedSemaphorePermit, Semaphore};

const SEGMENT: usize = 16 * 1024;
const OUTPUT_SLOTS: usize = 2;
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
struct Cancel(watch::Receiver<bool>);
impl Cancel {
    fn is_cancelled(&self) -> bool { *self.0.borrow() || self.0.has_changed().is_err() }
    async fn cancelled(&self) {
        let mut receiver = self.0.clone();
        while !*receiver.borrow() { if receiver.changed().await.is_err() { return; } }
    }
}
struct Queued { data: bytes::Bytes, content: bool, lease: Arc<Lease> }
struct OutputBody {
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
        let (content, last) = (segment.content, segment.last);
        self.lease.metrics.output_bytes.add(segment.buffer.bytes.capacity());
        self.lease.metrics.output_slots.add(1);
        self.lease.live_output.fetch_add(1, Ordering::SeqCst);
        let data = bytes::Bytes::from_owner(OwnedBytes {
            buffer: Some(segment.buffer), lease: self.lease.clone(), slot: Some(slot) });
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
    pub wait_budget: Duration,
}
pub(super) async fn wait(state: AppState, payload: Value, client_model: String,
    max_tokens: Option<u32>, explicit_session: Option<String>)
    -> Result<axum::response::Response, ApiError> {
    let residency = RequestResidency::empty();
    let id = format!("{:08x}{:08x}", rand_u32(), rand_u32());
    serve_initialized(state.clone(), client_model.clone(), id.clone(),
        explicit_session.clone(), residency.clone(), move |cancel| async move {
            let mut budget = account_wait::MAX_WAIT;
            let acquired = account_wait::pick_cancellable(&state, &[], account_wait::WaitFor::Capacity, &mut budget,
                || cancel.is_cancelled()).await;
            let cleanup_confirmed = acquired.cleanup_confirmed;
            let result = async {
                let (account, lease) = acquired.result?;
                residency.replace(lease);
                if cancel.is_cancelled() { return Err(ApiError::Internal("client cancelled".into())); }
                let session = state.sessions.load(&account.framer_user_id).await
                    .map_err(ApiError::Internal)?
                    .ok_or_else(|| ApiError::Internal("account has no session".into()))?;
                let scope = pick_scope(&state, &account)?;
                let gate_model = convert::to_openrouter_id(&client_model);
                let chat_session = explicit_session.clone().unwrap_or_else(|| stable_session(&account.id));
                let config = waiting_config(&state, max_tokens, scope.team.0.clone());
                Ok(FreeRequest { state, account, residency, session, project: scope.project,
                    config, payload, gate_key: format!("{}:{gate_model}", scope.team.0),
                    gate_model, first_permit: None, chat_session, explicit_session,
                    allow_repick: true, client_model, logical_id: id, wait_budget: budget })
            }.await;
            pool_control::CleanupOutcome { result, cleanup_confirmed }
        }).await
}
fn waiting_config(state: &AppState, max_tokens: Option<u32>, team: String)
    -> framer_core::port::ChainConfig {
    let rotation = state.serving_accounts.rotation().clone();
    framer_core::port::ChainConfig {
        max_total_tokens: max_tokens,
        metered_fallback: std::env::var("FRAMERIZE_ALLOW_METERED").is_ok_and(|v| v == "1"),
        on_429: Some(Arc::new(move || rotation.mark_saturated(&team, 45_000))),
        ..Default::default()
    }
}

pub(super) async fn serve(request: FreeRequest) -> Result<axum::response::Response, ApiError> {
    if request.config.is_cancelled() {
        let (status, body) = convert::error_json(ClientDialect::OpenAiResponses, "client cancelled");
        return Ok(operation_ownership::completed_response(axum::response::IntoResponse::into_response(
            ShapedApiError(StatusCode::from_u16(status).unwrap(), body)), true));
    }
    serve_initialized(request.state.clone(), request.client_model.clone(),
        request.logical_id.clone(), Some(request.chat_session.clone()), request.residency.clone(),
        move |_| async move { pool_control::CleanupOutcome {
            result: Ok(request), cleanup_confirmed: true,
        }}).await
}

async fn serve_initialized<F, Fut>(state: AppState, model: String, id: String,
    session_header: Option<String>, residency: RequestResidency, initialize: F)
    -> Result<axum::response::Response, ApiError>
where F: FnOnce(Cancel) -> Fut + Send + 'static,
    Fut: std::future::Future<Output = pool_control::CleanupOutcome<FreeRequest>> + Send {
    let lease = state.response_output.reserve()?;
    let (emitter, sink, body) = output(lease, &model, &id);
    let cancelled = emitter.publisher.cancel.clone();
    let published = emitter.publisher.content_published.clone();
    let scope = state.operation_ownership.clone();
    let (handle, completion) = operation_ownership::spawn_observed(scope.as_ref(),
        framer_core::operation_gate::OperationKind::Inference, async move {
            // This owner survives a dropped request future and joins all serializers.
            let encoding = tokio::spawn(emitter.run());
            let started = Instant::now();
            let initialized = initialize(cancelled.clone()).await;
            let (result, cleanup) = match initialized.result {
                Ok(mut request) => {
                    let cancel = cancelled.clone();
                    request.config.cancelled = Some(Arc::new(move || cancel.is_cancelled()));
                    let (result, cleanup) = request.drive(&sink, &cancelled, &published).await;
                    request.log(&result, started.elapsed());
                    (result, cleanup && initialized.cleanup_confirmed)
                }
                Err(error) => {
                    let error = chain_admission_error(error);
                    operation_ownership::correlate_chain(framer_core::observability::Event::new("ai.chain"))
                        .field("model", model.as_str()).field("dialect", "OpenAiResponses")
                        .duration_ms(started.elapsed().as_millis()).error(&error).emit();
                    (Err(error.into()), initialized.cleanup_confirmed)
                }
            };
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
    if let Some(session) = session_header {
        response.headers_mut().insert("x-framerize-session",
            session.parse().unwrap_or(axum::http::HeaderValue::from_static("")));
    }
    Ok(response)
}
impl FreeRequest {
    async fn drive(&mut self, sink: &Sink, cancel: &Cancel,
        published: &std::sync::atomic::AtomicBool) -> (ResponseResult, bool) {
        let mut tried = Vec::new();
        let mut attempts = 0u32;
        let mut cleanup = true;
        let mut cooldown = self.wait_budget;
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
            let output = AttemptSink { output: sink, cancel,
                completed_on_disconnect: false.into() };
            let result = match admission {
                Err(error) => Err(chain_admission_error(error).into()),
                Ok(_) => self.state.client.native_long_with_sink(&self.session,
                    self.state.browser.as_ref(), &self.account.id, &self.project,
                    self.payload.clone(), &self.config, &self.chat_session, &output).await,
            };
            drop(permit);
            // Keep a joined terminal success when delivery is cancelled.
            // Local encoding failures and intermediate chunks cannot earn it.
            let result = match result {
                Err(ResponseError::Output(error)) => {
                    if admitted && output.completed_on_disconnect.load(Ordering::SeqCst)
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
            let local = matches!(&result, Err(error) if AccountCooldown::from_error(error).is_some()
                || is_server_cooling(error));
            if !local { tried.push(self.account.id.0.to_string()); }
            drop(self.residency.take());
            let acquired = tokio::select! {
                _ = cancel.cancelled() => break Err("client cancelled".into()),
                value = account_wait::pick(&self.state, &tried,
                    account_wait::WaitFor::after_attempt(local, cleanup),
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
        let event = operation_ownership::correlate_chain(framer_core::observability::Event::new("ai.chain"))
            .account(&self.account.id).field("model", self.gate_model.as_str())
            .field("dialect", "OpenAiResponses").duration_ms(elapsed.as_millis());
        match result {
            Ok(result) => event.field("chunks", result.chunks).field("metered_chunks", result.metered_chunks)
                .field("prompt_tokens", result.input_tokens).field("completion_tokens", result.output_tokens)
                .field("finish_reason", result.status.as_str()).outcome("ok").emit(),
            Err(error) => event.error(&error.to_string()).emit(),
        }
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

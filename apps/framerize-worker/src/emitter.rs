//! One encoder owner, joined serializers, and kernel-confirmed publication.
use framer_protocol::{convert::{AssembledResponses, ResponseEncoder, ResponseWriter},
    responses_sink::{OutputError, ResponsesSink, ValidatedChunk}};
use framer_worker_wire::{credit::{data_cost, Capacity, Frontier, Window, DATA_WIRE_RESERVE},
    frame::{self, WriteSlot}, message::{DataKind, Message, PublicFailure, Segment},
    WireError, SEGMENT_LIMIT};
use std::{io::{self, Write}, sync::{Arc, Mutex, atomic::{AtomicUsize, Ordering}}, time::Duration};
use tokio::sync::{mpsc, oneshot, Notify};

struct CreditState { window: Window, frontier: Frontier, peak: Capacity, cancelled: bool, failed: bool }
struct OutputSegment { bytes: Vec<u8>, sequence: u64, content: bool, last: bool }
struct SegmentWriter {
    sender: mpsc::Sender<OutputSegment>, bytes: Vec<u8>, sequence: u64, content: bool,
    content_next: u64, event_next: u64,
}
struct Job(Arc<OutputControl>);
impl Drop for Job { fn drop(&mut self) { self.0.jobs.fetch_sub(1, Ordering::SeqCst); } }
fn encode_input(encoder: &mut ResponseEncoder, writer: &mut SegmentWriter, input: &Input)
    -> io::Result<()> {
    match input {
        Input::Open => encoder.open(writer),
        Input::Heartbeat => encoder.heartbeat(writer),
        Input::Chunk(chunk) => encoder.push_items(writer, chunk.items(), chunk.continues_text()),
        Input::Finish(result) => encoder.finish(writer, result),
        Input::Fail(error) => encoder.fail_shaped(writer, &serde_json::json!({
            "message": error.message.as_str(), "type": error.error_type.as_str(), "code": error.code.as_str(),
        })),
    }
}
impl SegmentWriter {
    fn send(&mut self, last: bool) -> io::Result<()> {
        let bytes = std::mem::take(&mut self.bytes);
        self.sender.blocking_send(OutputSegment { bytes, sequence: self.sequence,
            content: self.content, last }).map_err(|_| io::Error::other("output closed"))
    }
}
impl Write for SegmentWriter {
    fn write(&mut self, mut bytes: &[u8]) -> io::Result<usize> {
        let length = bytes.len();
        while !bytes.is_empty() {
            if self.bytes.len() == SEGMENT_LIMIT { self.send(false)?; }
            if self.bytes.capacity() == 0 { self.bytes = Vec::with_capacity(SEGMENT_LIMIT); }
            let count = bytes.len().min(SEGMENT_LIMIT - self.bytes.len());
            self.bytes.extend_from_slice(&bytes[..count]);
            bytes = &bytes[count..];
        }
        Ok(length)
    }
    fn flush(&mut self) -> io::Result<()> { Ok(()) }
}
impl ResponseWriter for SegmentWriter {
    fn begin_event(&mut self, sequence: u64, content: bool) -> io::Result<()> {
        self.sequence = sequence; self.content = content; Ok(())
    }
    fn end_event(&mut self) -> io::Result<()> {
        self.send(true)?;
        self.event_next = self.sequence.checked_add(1).ok_or_else(|| io::Error::other("sequence"))?;
        if self.content { self.content_next = self.event_next; }
        Ok(())
    }
}
pub(crate) struct OutputControl {
    state: Mutex<CreditState>, changed: Notify, jobs: AtomicUsize, peak_jobs: AtomicUsize,
}
impl OutputControl {
    pub fn new(capacity: Capacity) -> Result<Arc<Self>, WireError> {
        if capacity.wire_bytes < DATA_WIRE_RESERVE || capacity.downstream_bytes < SEGMENT_LIMIT as u64 {
            return Err(WireError::Credit);
        }
        Ok(Arc::new(Self { state: Mutex::new(CreditState { window: Window::new(capacity)?,
            frontier: Frontier::default(), peak: Capacity::default(), cancelled: false, failed: false }),
            changed: Notify::new(), jobs: AtomicUsize::new(0), peak_jobs: AtomicUsize::new(0) }))
    }
    fn lock(&self) -> std::sync::MutexGuard<'_, CreditState> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }
    pub fn credit(&self, returned: Capacity, accepted: u64) -> Result<(), WireError> {
        let mut state = self.lock();
        let result = state.frontier.acknowledge(accepted)
            .and_then(|()| state.window.release(returned));
        if result.is_err() { state.failed = true; }
        drop(state);
        self.changed.notify_one();
        result
    }
    pub fn cancel(&self) { self.lock().cancelled = true; self.changed.notify_one(); }
    pub fn fail(&self) { self.lock().failed = true; self.changed.notify_one(); }
    pub fn cancelled(&self) -> bool { self.lock().cancelled }
    fn check(state: &CreditState) -> Result<(), OutputError> {
        if state.failed { Err(OutputError::Closed) }
        else if state.cancelled { Err(OutputError::Cancelled) } else { Ok(()) }
    }
    async fn interrupted(&self) -> OutputError {
        loop {
            let changed = self.changed.notified();
            if let Err(error) = Self::check(&self.lock()) { return error; }
            changed.await;
        }
    }
    async fn accepted(&self, next: u64) -> Result<(), OutputError> {
        loop {
            let changed = self.changed.notified();
            {
                let state = self.lock();
                Self::check(&state)?;
                if state.frontier.accepted() >= next { return Ok(()); }
            }
            changed.await;
        }
    }
    pub(crate) async fn reserve(&self) -> Result<(), OutputError> {
        loop {
            let changed = self.changed.notified();
            {
                let mut state = self.lock();
                Self::check(&state)?;
                if state.window.reserve(data_cost(DATA_WIRE_RESERVE)).is_ok() {
                    let current = state.window.outstanding();
                    state.peak.wire_bytes = state.peak.wire_bytes.max(current.wire_bytes);
                    state.peak.slots = state.peak.slots.max(current.slots);
                    state.peak.downstream_bytes = state.peak.downstream_bytes.max(current.downstream_bytes);
                    return Ok(());
                }
            }
            changed.await;
        }
    }
    pub fn outstanding(&self) -> Capacity { self.lock().window.outstanding() }
    pub async fn disposed(&self) -> Result<(), WireError> {
        loop {
            let changed = self.changed.notified();
            {
                let state = self.lock();
                if state.failed { return Err(WireError::Protocol); }
                if state.window.outstanding() == Capacity::default() { return Ok(()); }
            }
            changed.await;
        }
    }
    pub fn jobs(&self) -> usize { self.jobs.load(Ordering::SeqCst) }
    pub fn peaks(&self) -> (Capacity, usize) {
        (self.lock().peak, self.peak_jobs.load(Ordering::SeqCst))
    }
}

enum Input { Open, Heartbeat, Chunk(ValidatedChunk), Finish(AssembledResponses), Fail(PublicFailure) }
struct Command { input: Input, reply: oneshot::Sender<Result<Option<ValidatedChunk>, OutputError>> }
#[derive(Clone)]
pub(crate) struct Sink { sender: mpsc::Sender<Command> }
pub(crate) struct Emitter {
    op: u64, slot: WriteSlot, control: Arc<OutputControl>, encoder: Option<ResponseEncoder>,
    commands: mpsc::Receiver<Command>, heartbeat: tokio::time::Interval, pending_heartbeat: bool,
}
pub(crate) fn channel(op: u64, slot: WriteSlot, control: Arc<OutputControl>,
    model: &str, id: &str) -> (Sink, Emitter) {
    let (sender, commands) = mpsc::channel(1);
    (Sink { sender }, Emitter { op, slot, control, commands,
        encoder: Some(ResponseEncoder::new(model, id)), pending_heartbeat: false,
        heartbeat: tokio::time::interval_at(tokio::time::Instant::now() + Duration::from_secs(8),
            Duration::from_secs(8)) })
}

impl Sink {
    async fn request(&self, input: Input) -> Result<Option<ValidatedChunk>, OutputError> {
        let (reply, receive) = oneshot::channel();
        self.sender.send(Command { input, reply }).await.map_err(|_| OutputError::Closed)?;
        receive.await.map_err(|_| OutputError::Closed)?
    }
    pub async fn finish(&self, result: AssembledResponses) -> Result<(), OutputError> {
        self.request(Input::Finish(result)).await.map(|_| ())
    }
    pub async fn fail(&self, error: PublicFailure) -> Result<(), OutputError> {
        self.request(Input::Fail(error)).await.map(|_| ())
    }
}
#[async_trait::async_trait]
impl ResponsesSink for Sink {
    async fn emit(&self, chunk: ValidatedChunk) -> Result<ValidatedChunk, OutputError> {
        self.request(Input::Chunk(chunk)).await?.ok_or(OutputError::Encoding)
    }
}
pub(crate) struct AttemptSink {
    pub sink: Sink, pub control: Arc<OutputControl>,
    pub completed: std::sync::atomic::AtomicBool,
}
#[async_trait::async_trait]
impl ResponsesSink for AttemptSink {
    async fn emit(&self, chunk: ValidatedChunk) -> Result<ValidatedChunk, OutputError> {
        let completed = chunk.completes_chain();
        let result = self.sink.emit(chunk).await;
        if completed && self.control.cancelled()
            && matches!(result, Err(OutputError::Cancelled | OutputError::Closed)) {
            self.completed.store(true, Ordering::SeqCst);
        }
        result
    }
}

impl Emitter {
    pub async fn run(mut self) -> Result<(), OutputError> {
        let result = self.commands().await;
        self.commands.close();
        while let Ok(command) = self.commands.try_recv() {
            let _ = command.reply.send(Err(result.as_ref().err().copied().unwrap_or(OutputError::Closed)));
        }
        result
    }
    async fn commands(&mut self) -> Result<(), OutputError> {
        self.encode(Input::Open).await?;
        loop {
            let command = tokio::select! {
                biased;
                error = self.control.interrupted() => return Err(error),
                command = self.commands.recv() => command.ok_or(OutputError::Closed)?,
                _ = self.heartbeat.tick() => {
                    self.encode(Input::Heartbeat).await?; self.pending_heartbeat = false; continue;
                },
            };
            let terminal = matches!(command.input, Input::Finish(_) | Input::Fail(_));
            let result = self.encode(command.input).await;
            let failed = result.as_ref().err().copied();
            let _ = command.reply.send(result.map(|input| match input {
                Input::Chunk(chunk) => Some(chunk), _ => None,
            }));
            if let Some(error) = failed { return Err(error); }
            if terminal { self.pending_heartbeat = false; return Ok(()); }
            if self.pending_heartbeat {
                self.pending_heartbeat = false;
                self.encode(Input::Heartbeat).await?;
            }
        }
    }
    async fn encode(&mut self, input: Input) -> Result<Input, OutputError> {
        OutputControl::check(&self.control.lock())?;
        let mut encoder = self.encoder.take().ok_or(OutputError::Encoding)?;
        let kind = match &input {
            Input::Open => DataKind::Opening, Input::Heartbeat => DataKind::Heartbeat,
            Input::Chunk(_) => DataKind::Content, _ => DataKind::Terminal,
        };
        let terminal = matches!(input, Input::Finish(_) | Input::Fail(_));
        let (sender, mut receiver) = mpsc::channel(1);
        let jobs = self.control.jobs.fetch_add(1, Ordering::SeqCst) + 1;
        self.control.peak_jobs.fetch_max(jobs, Ordering::SeqCst);
        let count = Job(self.control.clone());
        let job = tokio::task::spawn_blocking(move || {
            let _count = count;
            let mut writer = SegmentWriter { sender, bytes: Vec::new(), sequence: 0,
                content: false, content_next: 0, event_next: 0 };
            let result = encode_input(&mut encoder, &mut writer, &input);
            let next = if terminal { writer.event_next } else { writer.content_next };
            (encoder, input, result, next)
        });
        let output = self.drain_segments(&mut receiver, kind).await;
        drop(receiver); // Close staging before a blocked serializer is joined.
        let (encoder, input, encoded, next) = job.await.map_err(|_| OutputError::Encoding)?;
        self.encoder = Some(encoder);
        output?;
        encoded.map_err(|_| OutputError::Encoding)?;
        if next != 0 { self.control.accepted(next).await?; }
        Ok(input)
    }
    async fn drain_segments(&mut self, receiver: &mut mpsc::Receiver<OutputSegment>, kind: DataKind)
        -> Result<(), OutputError> {
        loop {
            let segment = tokio::select! {
                biased;
                error = self.control.interrupted() => return Err(error),
                segment = receiver.recv() => match segment { Some(s) => s, None => return Ok(()) },
                _ = self.heartbeat.tick() => { self.pending_heartbeat = true; continue; }
            };
            self.publish_segment(segment, kind).await?;
        }
    }
    async fn publish_segment(&mut self, segment: OutputSegment, kind: DataKind) -> Result<(), OutputError> {
        let publish = Self::publish(self.op, &self.slot, &self.control, segment, kind);
        tokio::pin!(publish);
        loop {
            tokio::select! {
                result = &mut publish => return result,
                _ = self.heartbeat.tick() => self.pending_heartbeat = true,
            }
        }
    }
    async fn publish(op: u64, slot: &WriteSlot, control: &OutputControl,
        segment: OutputSegment, kind: DataKind) -> Result<(), OutputError> {
        let bytes = Segment::new(segment.bytes).map_err(|_| OutputError::Encoding)?;
        control.reserve().await?;
        let cost = data_cost(DATA_WIRE_RESERVE);
        let permit = tokio::select! {
            biased;
            error = control.interrupted() => Err(error),
            permit = slot.reserve() => permit.map_err(|_| OutputError::Closed),
        };
        let permit = match permit {
            Ok(permit) => permit,
            Err(error) => {
                control.lock().window.release(cost).expect("unused reservation");
                return Err(error);
            }
        };
        let (seq, last) = (segment.sequence, segment.last);
        let message = Message::Data { op, seq, last,
            event: if segment.content { DataKind::Content } else { kind },
            bytes };
        let encoded = frame::encode(&message);
        drop(message); // Producer bytes never remain in a pipe-write waiter.
        enqueue_reserved(control, permit, encoded, seq, last)?
            .wait().await.map_err(|_| OutputError::Closed)
    }
}

/// The caller holds one full Data reservation and one writer permit.
pub(crate) fn enqueue_reserved(control: &OutputControl, permit: frame::WritePermit,
    encoded: Result<Vec<u8>, WireError>, seq: u64, last: bool)
    -> Result<frame::WriteReceipt, OutputError> {
        let cost = data_cost(DATA_WIRE_RESERVE);
        let receipt = {
            let mut state = control.lock();
            let encoded = match encoded {
                Ok(bytes) if bytes.len() as u64 <= DATA_WIRE_RESERVE => bytes,
                _ => { state.window.release(cost).expect("unused reservation"); return Err(OutputError::Encoding); }
            };
            if let Err(error) = OutputControl::check(&state) {
                state.window.release(cost).expect("unused reservation"); return Err(error);
            }
            if seq != state.frontier.completed() || (last && seq == u64::MAX) {
                state.window.release(cost).expect("unused reservation"); return Err(OutputError::Encoding);
            }
            let actual = encoded.len() as u64;
            let receipt = match permit.enqueue_encoded(encoded) {
                Ok(receipt) => receipt,
                Err(_) => {
                    state.window.release(cost).expect("unaccepted reservation");
                    return Err(OutputError::Closed);
                }
            };
            state.frontier.segment(seq, last).expect("checked sequence");
            state.window.release(Capacity { wire_bytes: DATA_WIRE_RESERVE - actual,
                ..Capacity::default() }).expect("unused wire reservation");
            receipt
        };
        Ok(receipt)
}

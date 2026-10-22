use bytes::Bytes;
use framer_core::native_dispatch::{Cancellation, Cleanup, ComposedAttemptMetadata,
    ComposedStreamCompletion, ComposedStreamHead, ComposedStreamLimits, DispatchError,
    FailureKind, RawStreamHeaders, stream_frames::{self, Field}};
use futures::{task::AtomicWaker, Future, Stream};
use std::{collections::VecDeque, pin::Pin, sync::{Arc, Mutex, MutexGuard},
    task::{Context, Poll}};
use tokio::{sync::{oneshot, watch, OwnedSemaphorePermit, Semaphore}, time::Instant};

pub(super) fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

pub(super) struct State {
    queue: VecDeque<Bytes>,
    terminal: Option<Bytes>,
    failure_framed: bool,
    pub attempts: Vec<ComposedAttemptMetadata>,
    pub cleanups: Vec<(Cleanup, Arc<dyn Cancellation>)>,
    pub headers: Option<Arc<RawStreamHeaders>>,
    pub result: Option<Result<(), FailureKind>>,
    aborted: Option<FailureKind>,
    produced: usize,
    emitted: usize,
    header_bytes: usize,
    next_sequence: u64,
    pub body_done: bool,
}

pub(super) struct Control {
    pub state: Mutex<State>,
    pub bytes: Arc<Semaphore>,
    pub frames: Arc<Semaphore>,
    pub terminal: Arc<Semaphore>,
    pub json: Arc<Semaphore>,
    pub limits: ComposedStreamLimits,
    pub started: Instant,
    pub deadline: Instant,
    pub stop: watch::Sender<Option<FailureKind>>,
    ended: watch::Sender<bool>,
    body_waker: AtomicWaker,
    head_waker: AtomicWaker,
    max_attempts: usize,
}

impl Control {
    pub fn new(limits: ComposedStreamLimits, deadline: Instant, max_attempts: usize) -> Arc<Self> {
        let (stop, _) = watch::channel(None);
        let (ended, _) = watch::channel(false);
        Arc::new(Self {
            state: Mutex::new(State {
                queue: VecDeque::with_capacity(limits.retained_frames), terminal: None,
                failure_framed: false,
                attempts: Vec::with_capacity(max_attempts),
                cleanups: Vec::with_capacity(max_attempts), headers: None,
                result: None, aborted: None, produced: 0, emitted: 0,
                header_bytes: 0, next_sequence: 0, body_done: false,
            }),
            bytes: Arc::new(Semaphore::new(limits.retained_bytes)),
            frames: Arc::new(Semaphore::new(limits.retained_frames)),
            terminal: Arc::new(Semaphore::new(limits.terminal_bytes)),
            json: Arc::new(Semaphore::new(limits.json_bytes)),
            limits, started: Instant::now(), deadline, stop, ended,
            body_waker: AtomicWaker::new(), head_waker: AtomicWaker::new(), max_attempts,
        })
    }

    pub fn abort(&self, kind: FailureKind) {
        let (queue, terminal, cancellations) = {
            let mut s = lock(&self.state);
            if s.body_done { return; }
            s.aborted = Some(kind);
            s.result = Some(Err(kind));
            s.body_done = true;
            (std::mem::take(&mut s.queue), s.terminal.take(),
                s.cleanups.iter().map(|(_, c)| c.clone()).collect::<Vec<_>>())
        };
        self.stop.send_replace(Some(kind));
        for cancel in cancellations { cancel.cancel(); }
        drop((queue, terminal));
        self.ended.send_replace(true);
        self.body_waker.wake();
        self.head_waker.wake();
    }

    pub fn check(&self) -> Result<(), FailureKind> {
        if Instant::now() >= self.deadline { self.abort(FailureKind::Deadline); }
        match lock(&self.state).aborted { Some(kind) => Err(kind), None => Ok(()) }
    }

    pub async fn stopped(&self) -> FailureKind {
        let mut rx = self.stop.subscribe();
        loop {
            if let Some(kind) = *rx.borrow_and_update() { return kind; }
            if rx.changed().await.is_err() { return FailureKind::Internal; }
        }
    }

    pub async fn body_finished(&self) {
        let mut rx = self.ended.subscribe();
        loop {
            if *rx.borrow_and_update() { return; }
            if rx.changed().await.is_err() { return; }
        }
    }

    pub fn register(&self, cleanup: Cleanup, cancellation: Arc<dyn Cancellation>)
        -> Result<(), FailureKind> {
        let mut s = lock(&self.state);
        if s.cleanups.len() == self.max_attempts {
            cancellation.cancel();
            return Err(FailureKind::Internal);
        }
        if s.aborted.is_some() { cancellation.cancel(); }
        s.cleanups.push((cleanup, cancellation));
        Ok(())
    }
}

impl Cancellation for Control {
    fn cancel(&self) { self.abort(FailureKind::Cancelled); }
}

// Backing storage drops before its reservations. Clones retain the owner.
struct Allocation {
    data: Box<[u8]>, len: usize,
    _bytes: OwnedSemaphorePermit, _frame: Option<OwnedSemaphorePermit>,
}
impl AsRef<[u8]> for Allocation {
    fn as_ref(&self) -> &[u8] { &self.data[..self.len] }
}

// A first refusal can outlive the producer and the header future. Its fixed
// wire buffer must keep the invocation's JSON allowance until the last owner
// disappears, just as a yielded frame keeps its output allowance. The buffer
// drops before the lease because Rust drops fields in declaration order.
pub(super) struct JsonAllocation {
    pub data: Box<[u8]>,
    pub len: usize,
    _json: OwnedSemaphorePermit,
}
impl AsRef<[u8]> for JsonAllocation {
    fn as_ref(&self) -> &[u8] { &self.data[..self.len] }
}

impl Control {
    pub async fn json_allocation(&self) -> Result<JsonAllocation, FailureKind> {
        let credit = self.json.clone().acquire_many_owned(self.limits.json_bytes as u32)
            .await.map_err(|_| FailureKind::Internal)?;
        self.check()?;
        Ok(JsonAllocation { data: vec![0; self.limits.json_bytes].into_boxed_slice(),
            len: 0, _json: credit })
    }

    async fn allocation(&self) -> Result<Allocation, FailureKind> {
        self.allocation_frames(1).await
    }

    async fn allocation_frames(&self, count: u32) -> Result<Allocation, FailureKind> {
        let bytes = self.bytes.clone().acquire_many_owned(self.limits.frame_bytes as u32)
            .await.map_err(|_| FailureKind::Internal)?;
        let frame = self.frames.clone().acquire_many_owned(count).await.map_err(|_| FailureKind::Internal)?;
        self.check()?;
        Ok(Allocation { data: vec![0; self.limits.frame_bytes].into_boxed_slice(),
            len: 0, _bytes: bytes, _frame: Some(frame) })
    }

    fn enqueue(&self, allocation: Allocation, response_event: bool) -> Result<(), FailureKind> {
        self.check()?;
        let mut s = lock(&self.state);
        if let Some(kind) = s.aborted { return Err(kind); }
        s.produced = s.produced.checked_add(allocation.len)
            .filter(|n| *n <= self.limits.total_bytes - self.limits.terminal_bytes)
            .ok_or(FailureKind::BodyTooLarge)?;
        if s.queue.len() >= self.limits.retained_frames { return Err(FailureKind::Internal); }
        if response_event { s.next_sequence += 1; }
        s.queue.push_back(Bytes::from_owner(allocation));
        drop(s);
        self.body_waker.wake();
        Ok(())
    }

    pub async fn send(&self, event: Option<&str>, fields: &[(&str, Field<'_>)])
        -> Result<(), FailureKind> {
        let mut allocation = self.allocation().await?;
        let response_event = event.is_some_and(|name| name.starts_with("response."));
        let sequence = response_event.then(|| lock(&self.state).next_sequence);
        allocation.len = stream_frames::frame(&mut allocation.data, event, fields, sequence)?;
        self.enqueue(allocation, response_event)
    }

    pub async fn chat_terminal(&self, finish: &[(&str, Field<'_>)],
        usage: Option<&[(&str, Field<'_>)]>) -> Result<(), FailureKind> {
        // Admit, serialize and enqueue the complete success suffix together.
        // A budget failure cannot follow an already emitted finish_reason.
        // The shared allocation holds one credit for every logical frame.
        let mut allocation = self.allocation_frames(if usage.is_some() { 3 } else { 2 }).await?;
        let mut len = stream_frames::frame(&mut allocation.data, None, finish, None)?;
        if let Some(usage) = usage {
            len += stream_frames::frame(&mut allocation.data[len..], None, usage, None)?;
        }
        len += stream_frames::done(&mut allocation.data[len..])?;
        allocation.len = len;
        self.enqueue(allocation, false)
    }

    pub fn failure_frame(&self, dialect: framer_core::convert::ClientDialect, kind: FailureKind) {
        use framer_core::convert::ClientDialect;
        // Chat's chunk schema has no error event. Preserve the typed body
        // failure instead of manufacturing a successful finish or [DONE].
        if dialect == ClientDialect::OpenAiChat { return; }
        if self.check().is_err() || lock(&self.state).headers.is_none() { return; }
        let Ok(credit) = self.terminal.clone()
            .try_acquire_many_owned(self.limits.terminal_bytes as u32) else { return; };
        let mut allocation = Allocation {
            data: vec![0; self.limits.terminal_bytes].into_boxed_slice(), len: 0,
            _bytes: credit, _frame: None,
        };
        let message = match kind {
            FailureKind::BodyTooLarge => "stream budget exceeded",
            FailureKind::UnsupportedResponse => "unsupported upstream response",
            FailureKind::Upstream => "upstream rejected continuation",
            FailureKind::TerminalFailure => "upstream failed",
            _ => "stream failed",
        };
        let error = [("type", Field::Str("api_error")), ("message", Field::Str(message))];
        let anthropic = [("error", Field::Object(&error))];
        let responses = [("code", Field::Str("server_error")),
            ("message", Field::Str(message)), ("param", Field::Null)];
        let is_response = dialect == ClientDialect::OpenAiResponses;
        let fields: &[(&str, Field<'_>)] = if is_response { &responses } else { &anthropic };
        let sequence = is_response.then(|| lock(&self.state).next_sequence);
        let Ok(len) = stream_frames::frame(&mut allocation.data, Some("error"), fields, sequence)
            else { return; };
        allocation.len = len;
        let mut s = lock(&self.state);
        if s.aborted.is_some() || s.failure_framed { return; }
        if is_response { s.next_sequence += 1; }
        s.produced += len;
        s.terminal = Some(Bytes::from_owner(allocation));
        s.failure_framed = true;
        drop(s);
        self.body_waker.wake();
    }
}

struct Body { control: Arc<Control>, done: bool }
impl Stream for Body {
    type Item = Result<Bytes, FailureKind>;
    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        if this.done { return Poll::Ready(None); }
        let control = this.control.clone();
        control.body_waker.register(cx.waker());
        let _ = control.check();
        let mut s = lock(&control.state);
        if let Some(kind) = s.aborted {
            this.done = true;
            return Poll::Ready(Some(Err(kind)));
        }
        if let Some(bytes) = s.queue.pop_front().or_else(|| s.terminal.take()) {
            s.emitted += bytes.len();
            return Poll::Ready(Some(Ok(bytes)));
        }
        if let Some(result) = s.result {
            this.done = true;
            s.body_done = true;
            control.ended.send_replace(true);
            return Poll::Ready(match result {
                Err(kind) if !s.failure_framed => Some(Err(kind)),
                _ => None,
            });
        }
        Poll::Pending
    }
}
impl Drop for Body {
    fn drop(&mut self) {
        if !self.done { self.control.cancel(); }
        drop(self.control.body_waker.take());
    }
}

pub(super) struct HeadReceiver {
    pub receiver: oneshot::Receiver<Result<ComposedStreamHead, DispatchError>>,
    pub control: Arc<Control>,
    pub done: bool,
}
impl Future for HeadReceiver {
    type Output = Result<ComposedStreamHead, DispatchError>;
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.control.head_waker.register(cx.waker());
        if let Err(kind) = self.control.check() {
            self.done = true;
            return Poll::Ready(Err(self.control.error(kind)));
        }
        match Pin::new(&mut self.receiver).poll(cx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(value) => {
                self.done = true;
                if let Err(kind) = self.control.check() {
                    drop(value);
                    return Poll::Ready(Err(self.control.error(kind)));
                }
                Poll::Ready(value.unwrap_or_else(|_| Err(self.control.error(FailureKind::Internal))))
            }
        }
    }
}
impl Drop for HeadReceiver {
    fn drop(&mut self) {
        if !self.done { self.control.cancel(); }
        drop(self.control.head_waker.take());
    }
}

impl Control {
    pub fn remaining_headers(&self) -> Result<usize, FailureKind> {
        self.limits.header_bytes.checked_sub(lock(&self.state).header_bytes)
            .filter(|bytes| *bytes > 0).ok_or(FailureKind::BodyTooLarge)
    }

    pub fn record(&self, headers: Arc<RawStreamHeaders>, step_index: u32)
        -> Result<(), FailureKind> {
        let mut s = lock(&self.state);
        let size = headers.usage_header.as_ref().map_or(0, |v| v.len())
            .checked_add(headers.original_content_type.as_ref().map_or(0, |v| v.len()))
            .and_then(|n| n.checked_add(if headers.original_content_type.is_none() {
                headers.content_type.len()
            } else { 0 })).ok_or(FailureKind::BodyTooLarge)?;
        let total = s.header_bytes.checked_add(size)
            .filter(|n| *n <= self.limits.header_bytes).ok_or(FailureKind::BodyTooLarge)?;
        if s.attempts.len() == self.max_attempts { return Err(FailureKind::Internal); }
        s.header_bytes = total;
        s.attempts.push(ComposedAttemptMetadata {
            elapsed: headers.elapsed, headers, step_index, usage: None, response_id: None,
        });
        Ok(())
    }

    pub fn head(self: &Arc<Self>, headers: Arc<RawStreamHeaders>) -> ComposedStreamHead {
        lock(&self.state).headers = Some(headers.clone());
        ComposedStreamHead { first_attempt: headers, content_type: "text/event-stream",
            body: Box::pin(Body { control: self.clone(), done: false }) }
    }

    pub fn complete(&self, result: Result<(), FailureKind>) {
        let mut s = lock(&self.state);
        if s.aborted.is_none() { s.result = Some(result); }
        if s.headers.is_none() { s.body_done = true; self.ended.send_replace(true); }
        drop(s);
        self.body_waker.wake();
    }

    pub fn summary(&self) -> ComposedStreamCompletion {
        let s = lock(&self.state);
        ComposedStreamCompletion {
            result: s.result.unwrap_or(Err(FailureKind::Internal)),
            attempts: Arc::from(s.attempts.clone()), emitted_bytes: s.emitted,
            elapsed: self.started.elapsed(),
        }
    }

    pub fn error(&self, kind: FailureKind) -> DispatchError {
        DispatchError { kind, upstream: None, attempts: self.progress() }
    }

    pub fn progress(&self) -> Vec<framer_core::native_dispatch::AttemptMetadata> {
        lock(&self.state).attempts.iter().map(|a| {
            framer_core::native_dispatch::AttemptMetadata {
                status: a.headers.status,
                usage_header: a.headers.usage_header.as_ref().map(|s| s.to_string()),
                original_content_type: a.headers.original_content_type.as_ref().map(|s| s.to_string()),
                step_index: a.step_index, elapsed: a.elapsed,
            }
        }).collect()
    }
}

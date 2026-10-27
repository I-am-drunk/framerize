//! One invocation's borrowed-authority adapter. The kernel owns every effect.
use async_trait::async_trait;
use bytes::Bytes;
use framer_protocol::{forward::{AuthorizedForward, ChainControl, ForwardReply, PolicyDiag},
    forward_error::{ControlError, ErrorClass, ForwardError, PolicyError}};
use framer_worker_wire::{frame::{Outbox, WriteSlot}, message::{self as wire, Message},
    WireError, SEGMENT_LIMIT};
use serde_json::Value;
use std::{io::{self, Write}, sync::{Arc, Mutex, atomic::{AtomicBool, Ordering}}};
use tokio::sync::{mpsc, oneshot, Notify};

// Same byte bound as the runtime browser reader. Queue UTF-16 units are separate.
const REPLY_BODY_LIMIT: usize = 8 * 1024 * 1024;

struct ReplyBody { status: u16, content_type: wire::Text, queue_ms: u64, bytes: Vec<u8> }
enum Pending {
    Forward { id: u64, chunk: u32, sent: bool, body: Option<ReplyBody>,
        reply: oneshot::Sender<Result<ForwardReply, PolicyError>> },
    Saturated { id: u64, reply: oneshot::Sender<()> },
}
#[derive(Default)]
struct State {
    next: u64, busy: bool, failed: bool, pending: Option<Pending>,
    saturation: Option<u64>, last_failure: Option<(u64, u32)>,
}
pub(crate) struct ForwardClient {
    op: u64, n: u32, category: String, outbox: Outbox, slot: WriteSlot,
    state: Mutex<State>, cancelled: AtomicBool, pub diagnostics: Arc<Diagnostics>,
}
impl ForwardClient {
    pub fn new(op: u64, n: u32, category: String, outbox: Outbox, slot: WriteSlot,
        diagnostics: Arc<Diagnostics>)
        -> Arc<Self> {
        Arc::new(Self { op, n, category, outbox, slot, state: Mutex::new(State::default()),
            cancelled: AtomicBool::new(false), diagnostics })
    }
    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }
    fn protocol_error(&self) -> ControlError { self.outbox.poison(); ControlError }
    pub fn cancel(&self) { self.cancelled.store(true, Ordering::SeqCst); }
    /// Called synchronously by the sole reader. Never waits on policy or output.
    pub fn dispatch(&self, message: Message) -> Result<(), WireError> {
        let result = self.dispatch_inner(message);
        if result.is_err() { self.outbox.poison(); }
        result
    }
    fn dispatch_inner(&self, message: Message) -> Result<(), WireError> {
        let mut state = self.lock();
        match message {
            Message::ForwardError { op, n, fwd, class, cooldown_seconds }
                if op == self.op && n == self.n => {
                let Some(Pending::Forward { id, chunk, sent: true, body: None, reply })
                    = state.pending.take() else { return Err(WireError::Protocol); };
                if id != fwd { return Err(WireError::Protocol); }
                let class = from_class(class);
                let error = ForwardError::from_facts(class, cooldown_seconds, id)
                    .map_err(|_| WireError::Protocol)?;
                state.saturation = (class == ErrorClass::Retryable429).then_some(id);
                state.last_failure = Some((id, chunk));
                let _ = reply.send(Err(error.into()));
                Ok(())
            }
            Message::ForwardReply { op, n, fwd, status, content_type, queue_wait_ms, body }
                if op == self.op && n == self.n => {
                receive_part(&mut state, fwd, status, content_type, queue_wait_ms, body)
            }
            Message::SaturatedAck { op, n, fwd } if op == self.op && n == self.n => {
                let Some(Pending::Saturated { id, reply }) = state.pending.take()
                    else { return Err(WireError::Protocol); };
                if id != fwd { return Err(WireError::Protocol); }
                let _ = reply.send(());
                Ok(())
            }
            _ => Err(WireError::Protocol),
        }
    }
}

struct Call<'a>(&'a ForwardClient);
fn receive_part(state: &mut State, fwd: u64, status: u16,
    content_type: wire::Text, queue_ms: u64, part: wire::Part) -> Result<(), WireError> {
    let Some(Pending::Forward { id, sent: true, body, .. }) = state.pending.as_mut()
        else { return Err(WireError::Protocol); };
    if *id != fwd || (!part.last && part.bytes.as_slice().is_empty()) {
        return Err(WireError::Protocol);
    }
    if body.is_none() {
        if part.offset != 0 { return Err(WireError::Protocol); }
        *body = Some(ReplyBody { status, content_type: content_type.clone(),
            queue_ms, bytes: Vec::new() });
    }
    let body = body.as_mut().expect("initialized reply");
    if body.status != status || body.content_type != content_type || body.queue_ms != queue_ms
        || part.offset != body.bytes.len() as u64 {
        return Err(WireError::Protocol);
    }
    let next = body.bytes.len().checked_add(part.bytes.as_slice().len())
        .filter(|length| *length <= REPLY_BODY_LIMIT).ok_or(WireError::Protocol)?;
    if next > body.bytes.capacity() {
        let capacity = next.max(SEGMENT_LIMIT).next_power_of_two().min(REPLY_BODY_LIMIT);
        body.bytes.try_reserve_exact(capacity - body.bytes.len()).map_err(|_| WireError::Allocation)?;
        if body.bytes.capacity() > REPLY_BODY_LIMIT { return Err(WireError::Allocation); }
    }
    body.bytes.extend_from_slice(part.bytes.as_slice());
    if part.last {
        let Some(Pending::Forward { body: Some(body), reply, .. }) = state.pending.take()
            else { unreachable!("validated pending reply"); };
        let _ = reply.send(Ok(ForwardReply { status, content_type: content_type.into_string(),
            queue_wait_ms: queue_ms, body: Bytes::from(body.bytes) }));
    }
    Ok(())
}

impl ChainControl for ForwardClient {
    fn is_cancelled(&self) -> bool { self.cancelled.load(Ordering::SeqCst) }
    fn on_429(&self) { self.outbox.poison(); }
    fn diag(&self, event: PolicyDiag) { self.diagnostics.push(to_diag(event)); }
}

struct PartWriter { sender: mpsc::Sender<wire::Part>, bytes: Vec<u8>, offset: u64 }
impl PartWriter {
    fn send(&mut self, last: bool) -> io::Result<()> {
        let bytes = std::mem::take(&mut self.bytes);
        let length = bytes.len() as u64;
        let part = wire::Part { offset: self.offset,
            bytes: wire::Segment::new(bytes).map_err(io::Error::other)?, last };
        self.sender.blocking_send(part).map_err(|_| io::Error::other("writer closed"))?;
        self.offset = self.offset.checked_add(length).ok_or_else(|| io::Error::other("offset"))?;
        Ok(())
    }
}
impl Write for PartWriter {
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

fn serialize_payload(payload: Value, sender: mpsc::Sender<wire::Part>) -> io::Result<()> {
    let mut writer = PartWriter { sender, bytes: Vec::new(), offset: 0 };
    serde_json::to_writer(&mut writer, &payload)?;
    writer.send(true)
}

impl ForwardClient {
    async fn send_payload(&self, id: u64, chunk: u32, step: wire::Text, payload: Value)
        -> Result<(), ControlError> {
        let (sender, mut receiver) = mpsc::channel(1);
        let job = tokio::task::spawn_blocking(move || serialize_payload(payload, sender));
        let mut result = Ok(());
        while let Some(payload) = receiver.recv().await {
            if payload.last {
                if let Err(error) = self.mark_sent(id) { result = Err(error); break; }
            }
            let message = Message::Forward { op: self.op, n: self.n, fwd: id,
                dialect: wire::Dialect::OpenRouterResponses, step: step.clone(),
                step_index: chunk, payload };
            if self.slot.send(&message).await.is_err() {
                result = Err(ControlError); break;
            }
        }
        // Wake a producer blocked on its single staging slot before joining.
        drop(receiver);
        let joined = job.await;
        if result.is_err() || !matches!(joined, Ok(Ok(()))) {
            self.channel_lost();
            return Err(self.protocol_error());
        }
        Ok(())
    }
}

pub(crate) fn from_class(class: wire::ErrorClass) -> ErrorClass {
    use wire::ErrorClass as W;
    match class {
        W::Cancelled => ErrorClass::Cancelled, W::RiskRefused => ErrorClass::RiskRefused,
        W::Unauthorized => ErrorClass::Unauthorized, W::CreditsExhausted => ErrorClass::CreditsExhausted,
        W::UpstreamForbidden => ErrorClass::UpstreamForbidden, W::Retryable429 => ErrorClass::Retryable429,
        W::Retryable4 => ErrorClass::Retryable4, W::Cooldown => ErrorClass::Cooldown, W::Other => ErrorClass::Other,
    }
}
pub(crate) fn to_class(class: ErrorClass) -> wire::ErrorClass {
    use wire::ErrorClass as W;
    match class {
        ErrorClass::Cancelled => W::Cancelled, ErrorClass::RiskRefused => W::RiskRefused,
        ErrorClass::Unauthorized => W::Unauthorized, ErrorClass::CreditsExhausted => W::CreditsExhausted,
        ErrorClass::UpstreamForbidden => W::UpstreamForbidden, ErrorClass::Retryable429 => W::Retryable429,
        ErrorClass::Retryable4 => W::Retryable4, ErrorClass::Cooldown => W::Cooldown, ErrorClass::Other => W::Other,
    }
}
fn to_diag(event: PolicyDiag) -> wire::Diagnostic {
    use PolicyDiag as P; use wire::Diagnostic as D;
    match event {
        P::ChunkStart { chunk, max_tokens } => D::ChunkStart { chunk, max_tokens },
        P::Backoff { chunk, attempt, wait_ms } => D::Backoff { chunk, attempt, wait_ms },
        P::RetryHalved { chunk, elapsed_ms, max_tokens, cause } => D::RetryHalved { chunk, elapsed_ms, max_tokens, cause: to_class(cause) },
        P::MeteredFallback { chunk, cause } => D::MeteredFallback { chunk, cause: to_class(cause) },
        P::UpstreamQueued { chunk, queue_ms, wall_ms } => D::UpstreamQueued { chunk, queue_ms, wall_ms },
        P::ChunkDone { chunk, tokens, elapsed_ms } => D::ChunkDone { chunk, tokens, elapsed_ms },
        P::ToolBudgetWithheld { chunk } => D::ToolBudgetWithheld { chunk },
        P::ToolRetry { chunk, max_tokens } => D::ToolRetry { chunk, max_tokens },
        P::NamespaceBackfill { count } => D::NamespaceBackfill { count },
    }
}

#[derive(Default)]
struct DiagnosticState {
    pending: Option<wire::Diagnostic>, sending: bool, sealed: bool, dropped: u32,
}
/// One reserved low-priority event, including a frame currently being written.
#[derive(Default)]
pub(crate) struct Diagnostics { state: Mutex<DiagnosticState>, wake: Notify }
impl Diagnostics {
    fn lock(&self) -> std::sync::MutexGuard<'_, DiagnosticState> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }
    fn push(&self, event: wire::Diagnostic) {
        let mut state = self.lock();
        if state.sealed || state.pending.is_some() {
            state.dropped = state.dropped.saturating_add(1);
        } else { state.pending = Some(event); }
        drop(state);
        self.wake.notify_one();
    }
    pub fn begin(&self) -> Option<wire::Diagnostic> {
        let mut state = self.lock();
        if state.sending || state.sealed { return None; }
        let event = state.pending?;
        state.sending = true;
        Some(event)
    }
    pub fn finish(&self) {
        let mut state = self.lock();
        assert!(state.sending, "diagnostic writer owns its started frame");
        state.sending = false;
        state.pending = None;
        drop(state);
        self.wake.notify_one();
    }
    pub fn seal(&self) {
        let mut state = self.lock();
        state.sealed = true;
        if !state.sending && state.pending.take().is_some() {
            state.dropped = state.dropped.saturating_add(1);
        }
        drop(state);
        self.wake.notify_one();
    }
    pub fn snapshot(&self) -> Result<u32, ControlError> {
        let state = self.lock();
        if !state.sealed || state.sending { return Err(ControlError); }
        Ok(state.dropped)
    }
    pub fn reopen(&self) -> Result<(), ControlError> {
        let mut state = self.lock();
        if !state.sealed || state.sending || state.pending.is_some() { return Err(ControlError); }
        state.sealed = false;
        Ok(())
    }
    pub async fn changed(&self) { self.wake.notified().await; }
}

#[cfg(test)]
#[path = "forward_tests.rs"]
mod tests;
#[async_trait]
impl AuthorizedForward for ForwardClient {
    fn category(&self) -> &str { &self.category }
    async fn forward(&self, step: &str, payload: Value, chunk: u32)
        -> Result<ForwardReply, String> {
        self.forward_typed(step, payload, chunk).await.map_err(|e| e.to_string())
    }
    async fn forward_typed(&self, step: &str, payload: Value, chunk: u32)
        -> Result<ForwardReply, PolicyError> {
        let _call = self.begin()?;
        self.check_cancelled()?;
        let step = wire::Text::new(step.into()).map_err(|_| self.protocol_error())?;
        let (reply, receive) = oneshot::channel();
        let id = {
            let mut state = self.lock();
            state.next = state.next.checked_add(1).ok_or(ControlError)?;
            let id = state.next;
            state.saturation = None;
            state.last_failure = None;
            state.pending = Some(Pending::Forward { id, chunk, sent: false, body: None, reply });
            id
        };
        self.send_payload(id, chunk, step, payload).await?;
        receive.await.map_err(|_| ControlError)?
    }
    async fn saturated(&self) -> Result<(), ControlError> {
        let _call = self.begin()?;
        let (reply, receive) = oneshot::channel();
        let id = {
            let mut state = self.lock();
            let Some(id) = state.saturation.take() else {
                drop(state);
                return Err(self.protocol_error());
            };
            state.pending = Some(Pending::Saturated { id, reply }); id
        };
        self.slot.send(&Message::Saturated { op: self.op, n: self.n, fwd: id })
            .await.map_err(|_| self.protocol_error())?;
        receive.await.map_err(|_| ControlError)
    }
}
impl Drop for Call<'_> {
    fn drop(&mut self) {
        let mut state = self.0.lock();
        state.busy = false;
        // The runner owns the policy task through join. Unexpected loss cannot
        // permit a replacement call to overtake its accepted Forward.
        if state.pending.is_some() { state.failed = true; }
    }
}
impl ForwardClient {
    pub fn channel_lost(&self) {
        let mut state = self.lock();
        state.failed = true;
        state.pending.take();
        self.diagnostics.seal();
    }
    fn begin(&self) -> Result<Call<'_>, ControlError> {
        let mut state = self.lock();
        if state.busy || state.pending.is_some() || state.failed {
            drop(state);
            return Err(self.protocol_error());
        }
        state.busy = true;
        Ok(Call(self))
    }
    pub fn last_failure(&self) -> Option<(u64, u32)> { self.lock().last_failure }
    fn mark_sent(&self, id: u64) -> Result<(), ControlError> {
        match self.lock().pending.as_mut() {
            Some(Pending::Forward { id: pending, sent, .. }) if *pending == id => {
                *sent = true; Ok(())
            }
            _ => Err(ControlError),
        }
    }
}

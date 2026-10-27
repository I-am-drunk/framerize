//! One pipe reader, one writer, and joined policy/emitter siblings per operation.
use crate::{emitter::{self, AttemptSink, OutputControl, Sink}, forward::{Diagnostics, ForwardClient}};
use framer_protocol::{convert::AssembledResponses, forward::ChainLimits,
    forward_error::{ErrorRecipe, LocalFailure, PolicyError}, responses_sink::OutputError};
use framer_worker_wire::{frame::{self, Outbox, WriteSlot}, message::{self as wire, Message}, WireError};
use std::{collections::BTreeMap, sync::{Arc, Mutex, atomic::{AtomicBool, Ordering}}};
use tokio::{io::{AsyncRead, AsyncWrite}, sync::{mpsc, watch, Notify}, task::JoinSet};

const MAX_OPERATIONS: usize = 65536; // Same ceiling as explicit kernel construction.
// A 64 MiB ingress can expand when re-encoded. This is not a history/event cap.
const REQUEST_BYTES: usize = 6 * 64 * 1024 * 1024;
struct Invocation { n: u32, category: wire::Text, limits: wire::Limits, bytes: Vec<u8> }
enum Command { Invoke(Invocation), Finish, Fail(wire::PublicFailure) }
enum Phase {
    Initial, Receiving(Invocation), Queued(u32), Running(u32, Arc<ForwardClient>),
    Decision { n: u32, success: bool }, Finalizing, Done(wire::DoneOutcome),
}
struct State { phase: Phase, cancelled: bool, lost: bool }
struct Operation {
    op: u64, state: Mutex<State>, changed: Notify, output: Arc<OutputControl>,
    commands: mpsc::Sender<Command>, outbox: Outbox,
    fatal: watch::Sender<bool>,
    control: WriteSlot, data: WriteSlot, diag: WriteSlot, diagnostics: Arc<Diagnostics>,
}
struct Work { owner: Arc<Operation>, commands: mpsc::Receiver<Command>, model: String, id: String }
impl Operation {
    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }
    fn cancel(&self, lost: bool) {
        let mut state = self.lock();
        state.cancelled = true; state.lost |= lost;
        if let Phase::Running(_, client) = &state.phase {
            client.cancel();
            if lost { client.channel_lost(); }
        }
        self.output.cancel();
        if lost { self.output.fail(); }
        self.changed.notify_one();
    }
    fn dispatch(&self, message: Message) -> Result<(), WireError> {
        if let Message::Cancel { .. } = message { self.cancel(false); return Ok(()); }
        if let Message::Credit { returned, accepted_event_seq, .. } = message {
            return self.output.credit(returned, accepted_event_seq);
        }
        let mut state = self.lock();
        match message {
            Message::Invoke { n, category, limits, request, .. } if !state.cancelled => {
                let start = match &state.phase {
                    Phase::Initial => n == 1,
                    Phase::Decision { n: previous, success: false } => previous.checked_add(1) == Some(n),
                    _ => false,
                };
                if start {
                    state.phase = Phase::Receiving(Invocation { n, category: category.clone(),
                        limits: limits.clone(), bytes: Vec::new() });
                }
                let Phase::Receiving(input) = &mut state.phase else { return Err(WireError::Protocol); };
                if input.n != n || input.category != category || input.limits != limits {
                    return Err(WireError::Protocol);
                }
                append_request(&mut input.bytes, &request)?;
                if request.last {
                    let Phase::Receiving(input) = std::mem::replace(&mut state.phase, Phase::Queued(n))
                        else { unreachable!() };
                    self.commands.try_send(Command::Invoke(input)).map_err(|_| WireError::Protocol)?;
                }
                Ok(())
            }
            Message::Finish { n, .. } => {
                if !matches!(&state.phase, Phase::Decision { n: expected, success: true } if *expected == n) {
                    return Err(WireError::Protocol);
                }
                state.phase = Phase::Finalizing;
                self.commands.try_send(Command::Finish).map_err(|_| WireError::Protocol)
            }
            Message::Fail { n, error, .. } => {
                if !matches!(&state.phase, Phase::Decision { n: expected, .. } if *expected == n) {
                    return Err(WireError::Protocol);
                }
                state.phase = Phase::Finalizing;
                self.commands.try_send(Command::Fail(error)).map_err(|_| WireError::Protocol)
            }
            message @ (Message::ForwardReply { .. } | Message::ForwardError { .. }
                | Message::SaturatedAck { .. }) => {
                let Phase::Running(_, client) = &state.phase else { return Err(WireError::Protocol); };
                client.dispatch(message)
            }
            _ => Err(WireError::Protocol),
        }
    }
    async fn next(&self, commands: &mut mpsc::Receiver<Command>) -> Option<Command> {
        loop {
            let changed = self.changed.notified();
            if self.lock().cancelled { return None; }
            tokio::select! {
                command = commands.recv() => return command,
                _ = changed => {},
            }
        }
    }
}
fn append_request(bytes: &mut Vec<u8>, part: &wire::Part) -> Result<(), WireError> {
    if part.offset != bytes.len() as u64 || (!part.last && part.bytes.as_slice().is_empty()) {
        return Err(WireError::Protocol);
    }
    let length = bytes.len().checked_add(part.bytes.as_slice().len())
        .filter(|n| *n <= REQUEST_BYTES).ok_or(WireError::Protocol)?;
    if length > bytes.capacity() {
        let capacity = length.max(framer_worker_wire::SEGMENT_LIMIT).next_power_of_two().min(REQUEST_BYTES);
        bytes.try_reserve_exact(capacity - bytes.len()).map_err(|_| WireError::Allocation)?;
        if bytes.capacity() > REQUEST_BYTES { return Err(WireError::Allocation); }
    }
    bytes.extend_from_slice(part.bytes.as_slice()); Ok(())
}
impl Work {
    async fn run(mut self) -> Result<(), WireError> {
        let owner = self.owner.clone();
        owner.control.send(&Message::Head { op: owner.op, status: 200,
            content_type: wire::Text::new("text/event-stream".into())? }).await?;
        let (sink, emitter) = emitter::channel(owner.op, owner.data.clone(), owner.output.clone(),
            &self.model, &self.id);
        self.model = String::new(); self.id = String::new();
        let mut emitter = tokio::spawn(emitter.run());
        let (result, emitted) = {
            let policy = self.policy_loop(sink);
            tokio::pin!(policy);
            tokio::select! {
                result = &mut policy => {
                    if !matches!(&result, Ok(wire::DoneOutcome::Completed)) { owner.output.cancel(); }
                    (result, emitter.await)
                }
                emitted = &mut emitter => {
                    if !matches!(&emitted, Ok(Ok(()))) && !owner.lock().cancelled {
                        owner.outbox.poison();
                        owner.fatal.send_replace(true);
                    }
                    (policy.await, emitted)
                }
            }
        };
        self.commands.close();
        while self.commands.try_recv().is_ok() {}
        // Every ordinary exit joins the encoder, including a policy panic.
        let outcome = result?;
        if matches!(outcome, wire::DoneOutcome::Completed) && !matches!(emitted, Ok(Ok(()))) {
            return Err(WireError::Protocol);
        }
        owner.diagnostics.seal();
        let diag_dropped = owner.diagnostics.snapshot().map_err(|_| WireError::Protocol)?;
        owner.lock().phase = Phase::Done(outcome);
        owner.control.send(&Message::Done { op: owner.op, outcome, diag_dropped }).await?;
        // A policy panic ends this generation. Only the kernel can establish
        // retirement after its Forward verdicts, output disposal, and reaping.
        if outcome == wire::DoneOutcome::Panicked { return Err(WireError::Protocol); }
        // Done never waits on D. The thin record stays valid for in-flight returns.
        owner.output.disposed().await?;
        Ok(())
    }
    async fn policy_loop(&mut self, sink: Sink) -> Result<wire::DoneOutcome, WireError> {
        let owner = self.owner.clone();
        let mut assembled = None;
        while let Some(command) = owner.next(&mut self.commands).await {
            match command {
                Command::Finish => {
                    let value = assembled.take().ok_or(WireError::Protocol)?;
                    if sink.finish(value).await.is_err() {
                        if owner.lock().cancelled { return Ok(wire::DoneOutcome::Cancelled); }
                        return Err(WireError::Protocol);
                    }
                    return Ok(wire::DoneOutcome::Completed);
                }
                Command::Fail(error) => {
                    drop(assembled.take());
                    if sink.fail(error).await.is_err() {
                        if owner.lock().cancelled { return Ok(wire::DoneOutcome::Cancelled); }
                        return Err(WireError::Protocol);
                    }
                    return Ok(wire::DoneOutcome::Completed);
                }
                Command::Invoke(input) => {
                    drop(assembled.take());
                    if input.n > 1 { owner.diagnostics.reopen().map_err(|_| WireError::Protocol)?; }
                    let client = ForwardClient::new(owner.op, input.n, input.category.as_str().into(),
                        owner.outbox.clone(), owner.control.clone(), owner.diagnostics.clone());
                    {
                        let mut state = owner.lock();
                        if !matches!(&state.phase, Phase::Queued(n) if *n == input.n) {
                            return Err(WireError::Protocol);
                        }
                        if state.cancelled { client.cancel(); }
                        if state.lost { client.channel_lost(); }
                        state.phase = Phase::Running(input.n, client.clone());
                    }
                    let attempt = Arc::new(AttemptSink { sink: sink.clone(), control: owner.output.clone(),
                        completed: AtomicBool::new(false) });
                    let diagnostics = tokio::spawn(send_diagnostics(owner.clone(), input.n));
                    let policy = tokio::spawn(invoke(input, client.clone(), attempt.clone()));
                    let result = policy.await;
                    owner.diagnostics.seal();
                    let diag_dropped = diagnostics.await.map_err(|_| WireError::Protocol)??;
                    let (n, result) = match result {
                        Ok(result) => result,
                        Err(_) => { owner.cancel(false); return Ok(wire::DoneOutcome::Panicked); }
                    };
                    let outcome = result_outcome(owner.op, n, &result, &client,
                        attempt.completed.load(Ordering::SeqCst))?;
                    if let Ok(value) = result { assembled = Some(value); }
                    owner.result(n, outcome, diag_dropped).await?;
                }
            }
        }
        Ok(wire::DoneOutcome::Cancelled)
    }
}
impl Operation {
    async fn result(&self, n: u32, outcome: wire::Outcome, diag_dropped: u32) -> Result<(), WireError> {
        let success = matches!(outcome, wire::Outcome::Success { .. });
        let permit = self.control.reserve().await?;
        let bytes = frame::encode(&Message::Result { op: self.op, n, outcome, diag_dropped })?;
        let receipt = {
            let mut state = self.lock();
            if !matches!(&state.phase, Phase::Running(current, _) if *current == n) {
                return Err(WireError::Protocol);
            }
            let receipt = permit.enqueue_encoded(bytes)?;
            state.phase = Phase::Decision { n, success };
            receipt
        };
        receipt.wait().await
    }
}
async fn send_diagnostics(owner: Arc<Operation>, n: u32) -> Result<u32, WireError> {
    loop {
        if let Some(event) = owner.diagnostics.begin() {
            let result = owner.diag.send(&Message::Diag { op: owner.op, n, event }).await;
            owner.diagnostics.finish();
            result?;
        } else if let Ok(count) = owner.diagnostics.snapshot() { return Ok(count); }
        else { owner.diagnostics.changed().await; }
    }
}
async fn invoke(input: Invocation, client: Arc<ForwardClient>, sink: Arc<AttemptSink>)
    -> (u32, Result<AssembledResponses, PolicyError>) {
    let n = input.n;
    let parsed = tokio::task::spawn_blocking(move || {
        let payload = serde_json::from_slice::<serde_json::Value>(&input.bytes);
        (input.limits, payload)
    }).await;
    let (limits, payload) = match parsed {
        Ok((limits, Ok(payload))) => (limits, payload),
        _ => return (n, Err(PolicyError::Local { kind: LocalFailure::InvalidResponse,
            message: "invalid policy input".into() })),
    };
    let cfg = ChainLimits { chunk_tokens: limits.chunk_tokens, max_chunks: limits.max_chunks,
        max_total_tokens: limits.max_total_tokens, metered_fallback: limits.metered_fallback,
        continue_prompt: limits.continue_prompt.into_string() };
    let payload = framer_protocol::responses_input::prepare_worker_payload(payload);
    (n, framer_protocol::responses::complete_long_typed(client.as_ref(), payload, &cfg, sink.as_ref()).await)
}
fn result_outcome(op: u64, n: u32, result: &Result<AssembledResponses, PolicyError>,
    client: &ForwardClient, completed: bool) -> Result<wire::Outcome, WireError> {
    use wire::{Outcome as W, FinishReason as F};
    Ok(match result {
        Ok(value) => W::Success { summary: wire::Summary {
            input_tokens: value.input_tokens, output_tokens: value.output_tokens,
            chunks: value.chunks, metered_chunks: value.metered_chunks,
            finish: match value.status.as_str() { "completed" => F::Completed,
                "incomplete" => F::Incomplete, _ => return Err(WireError::Protocol) },
        } },
        Err(PolicyError::Control(_)) => W::Control,
        Err(PolicyError::Output(error)) => W::Output { completed, failure: match error {
            OutputError::Cancelled => wire::OutputFailure::Cancelled,
            OutputError::Encoding => wire::OutputFailure::Encoding,
            OutputError::Closed => wire::OutputFailure::Closed,
            OutputError::Unsupported => wire::OutputFailure::Unsupported,
        } },
        Err(PolicyError::Local { kind, .. }) => W::Local { failure: match kind {
            LocalFailure::Policy => wire::LocalFailure::Policy,
            LocalFailure::InvalidResponse => wire::LocalFailure::InvalidResponse,
            LocalFailure::IncompleteTool => wire::LocalFailure::IncompleteTool,
            LocalFailure::Cancelled => wire::LocalFailure::Cancelled,
        } },
        Err(PolicyError::Forward(error)) => {
            let (fwd, chunk) = client.last_failure().ok_or(WireError::Protocol)?;
            if error.cause != Some(fwd) { return Err(WireError::Protocol); }
            let tag = match error.recipe {
                ErrorRecipe::Direct => wire::FailureTag::Direct,
                ErrorRecipe::SaturationExhausted { chunk: c, retries: 9 } if c == chunk =>
                    wire::FailureTag::SaturationExhausted,
                ErrorRecipe::FallbackOff { chunk: c } if c == chunk => wire::FailureTag::FallbackOff,
                _ => return Err(WireError::Protocol),
            };
            W::Forward { failure: wire::FailureRecipe { cause: wire::Cause { op, n, fwd }, tag, chunk } }
        }
    })
}

struct SessionState { closed: bool, operations: BTreeMap<u64, Arc<Operation>> }
struct Session {
    state: Mutex<SessionState>, outbox: Outbox, starts: mpsc::Sender<Work>, fatal: watch::Sender<bool>,
}
impl Session {
    fn lock(&self) -> std::sync::MutexGuard<'_, SessionState> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }
    fn close(&self) {
        let mut state = self.lock();
        state.closed = true;
        for owner in state.operations.values() { owner.cancel(true); }
        self.outbox.poison();
    }
    fn dispatch(&self, message: Message) -> Result<(), WireError> {
        if let Message::Start { op, logical_id, model, mode: wire::Mode::Sse, output } = message {
            return self.start(op, logical_id, model, output);
        }
        let op = match &message {
            Message::Invoke { op, .. } | Message::ForwardReply { op, .. } |
            Message::ForwardError { op, .. } | Message::SaturatedAck { op, .. } |
            Message::Finish { op, .. } | Message::Fail { op, .. } |
            Message::Cancel { op } | Message::Credit { op, .. } => *op,
            _ => return Err(WireError::Protocol),
        };
        let owner = self.lock().operations.get(&op).cloned().ok_or(WireError::Protocol)?;
        owner.dispatch(message)
    }
    fn start(&self, op: u64, id: wire::Text, model: wire::Text,
        capacity: framer_worker_wire::credit::Capacity) -> Result<(), WireError> {
        let mut state = self.lock();
        if state.closed || op == 0 || state.operations.contains_key(&op)
            || state.operations.len() >= MAX_OPERATIONS { return Err(WireError::Protocol); }
        let key = op.checked_mul(3).ok_or(WireError::Protocol)?;
        let output = OutputControl::new(capacity)?;
        let control = self.outbox.register(key - 2)?;
        let data = match self.outbox.register(key - 1) {
            Ok(slot) => slot,
            Err(error) => { let _ = self.outbox.remove(&control); return Err(error); }
        };
        let diag = match self.outbox.register(key) {
            Ok(slot) => slot,
            Err(error) => { let _ = self.outbox.remove(&control); let _ = self.outbox.remove(&data);
                return Err(error); }
        };
        let (commands, receive) = mpsc::channel(1);
        let owner = Arc::new(Operation { op, state: Mutex::new(State { phase: Phase::Initial,
            cancelled: false, lost: false }), changed: Notify::new(), output, commands,
            outbox: self.outbox.clone(), fatal: self.fatal.clone(),
            control, data, diag, diagnostics: Arc::default() });
        if self.starts.try_send(Work { owner: owner.clone(), commands: receive,
            model: model.into_string(), id: id.into_string() }).is_err() {
            let _ = self.outbox.remove(&owner.control);
            let _ = self.outbox.remove(&owner.data);
            let _ = self.outbox.remove(&owner.diag);
            return Err(WireError::Protocol);
        }
        state.operations.insert(op, owner);
        Ok(())
    }
    fn remove(&self, op: u64) -> Result<(), WireError> {
        let owner = self.lock().operations.remove(&op).ok_or(WireError::Protocol)?;
        self.outbox.remove(&owner.control)?;
        self.outbox.remove(&owner.data)?;
        self.outbox.remove(&owner.diag)
    }
}
async fn read_session<R: AsyncRead + Unpin>(mut reader: R, session: Arc<Session>) -> Result<(), WireError> {
    while let Some(frame) = frame::read::<_, Message>(&mut reader).await? {
        session.dispatch(frame.message)?;
    }
    let settled = session.lock().operations.values().all(|owner|
        matches!(&owner.lock().phase, Phase::Done(outcome) if *outcome != wire::DoneOutcome::Panicked)
            && owner.output.outstanding() == framer_worker_wire::credit::Capacity::default());
    if settled { Ok(()) } else { Err(WireError::Protocol) }
}
/// Own both channels until closure; no task or serializer is detached on return.
pub async fn run<R, W>(mut reader: R, writer: W) -> Result<(), WireError>
where R: AsyncRead + Unpin + Send + 'static, W: AsyncWrite + Unpin + Send + 'static {
    let (outbox, frames) = Outbox::new(MAX_OPERATIONS * 3 + 1)?;
    let hello = outbox.register(0)?;
    let mut writer = tokio::spawn(frames.run(writer));
    let handshake = async {
        hello.send(&Message::Hello { wire: framer_worker_wire::VERSION,
            build: wire::Text::new(concat!("framerize-worker/", env!("CARGO_PKG_VERSION")).into())?,
            routes: vec![wire::Route::FreeResponses] }).await?;
        match frame::read::<_, Message>(&mut reader).await? {
            Some(frame::Decoded { message: Message::Ready { generation }, .. }) if generation != 0 => Ok(()),
            _ => Err(WireError::Protocol),
        }
    }.await;
    if let Err(error) = handshake {
        outbox.poison(); writer.abort(); let _ = writer.await;
        return Err(error);
    }
    if let Err(error) = outbox.remove(&hello) {
        outbox.poison(); writer.abort(); let _ = writer.await;
        return Err(error);
    }
    let (starts, mut pending) = mpsc::channel(MAX_OPERATIONS);
    let (fatal, mut failed) = watch::channel(false);
    let session = Arc::new(Session { state: Mutex::new(SessionState {
        closed: false, operations: BTreeMap::new() }), outbox, starts, fatal });
    let mut reader = tokio::spawn(read_session(reader, session.clone()));
    let mut tasks = JoinSet::new();
    let (result, reader_done, writer_done) = loop {
        tokio::select! {
            _ = failed.changed() => break (Err(WireError::Protocol), false, false),
            work = pending.recv() => {
                let Some(work) = work else { break (Err(WireError::Protocol), false, false); };
                tasks.spawn(async move { (work.owner.op, work.run().await) });
            }
            joined = tasks.join_next(), if !tasks.is_empty() => {
                match joined {
                    Some(Ok((op, Ok(())))) => if let Err(e) = session.remove(op) { break (Err(e), false, false); },
                    _ => break (Err(WireError::Protocol), false, false),
                }
            }
            result = &mut reader => break (result.unwrap_or(Err(WireError::Protocol)), true, false),
            _ = &mut writer => break (Err(WireError::Protocol), false, true),
        }
    };
    // Whole-channel teardown may cut a partial frame. Operation cancellation cannot.
    session.close();
    if !reader_done { reader.abort(); let _ = reader.await; }
    if !writer_done { writer.abort(); let _ = writer.await; }
    while let Ok(work) = pending.try_recv() {
        tasks.spawn(async move { (work.owner.op, work.run().await) });
    }
    while let Some(joined) = tasks.join_next().await {
        if let Ok((op, _)) = joined { let _ = session.remove(op); }
    }
    result
}

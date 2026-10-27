//! Kernel-owned invocation facts. Workers never supply authority or raw causes.
use framer_core::{convert::{self, ClientDialect}, port::{ErrorClass, ErrorOrigin,
    ErrorRecipe, ForwardError, OutputError, ResponseError}};
use framer_worker_wire::{message::{self as wire, Message}, WireError};
use std::sync::Arc;
use std::{io::{self, Write}, sync::Mutex};
use tokio::sync::{mpsc, oneshot, watch};
use framer_worker_wire::frame::{self, Decoded, WriteSlot};
use crate::{policy_generations::{GenerationPin, OperationLog}, policy_process::Dispatch,
    response_output::{Cancel, FreeRequest, Invocation, InvocationResult, WorkerPublisher}};

const REQUEST_BYTES: usize = 6 * 64 * 1024 * 1024;
/// A preflight-only selection has never sent Start and needs no worker cleanup.
pub(super) struct Selection { pin: Option<GenerationPin>, log: Arc<OperationLog> }
impl Drop for Selection {
    fn drop(&mut self) { if let Some(pin) = &self.pin { pin.confirm(); } }
}
pub(super) fn select(state: &crate::AppState) -> Result<Option<Selection>, crate::ApiError> {
    let Some(manager) = state.policy_generations() else { return Ok(None); };
    let refused = || crate::ApiError::ServingCapacity("policy generation unavailable".into());
    let Some(pin) = manager.select().map_err(|_| refused())? else { return Ok(None); };
    let context = match crate::operation_ownership::associate_generation(pin.clone()) {
        Ok(context) => context,
        Err(_) => { pin.confirm(); return Err(refused()); }
    };
    let log = match pin.logger(context) {
        Ok(log) => log,
        Err(_) => { pin.confirm(); return Err(refused()); }
    };
    Ok(Some(Selection { pin: Some(pin), log }))
}
pub(super) async fn serve(mut request: FreeRequest, mut selection: Selection)
    -> Result<axum::response::Response, crate::ApiError> {
    if request.config.is_cancelled() {
        // Reuse the unchanged pre-producer cancellation response.
        return crate::response_output::serve(request).await;
    }
    let pin = selection.pin.as_ref().expect("unstarted selection").clone();
    let output = crate::response_output::WorkerOutput::reserve(&request.state.response_output, pin.clone())?;
    let original = std::mem::take(&mut request.payload);
    let (invocation, head) = WorkerInvocation::new(pin, selection.log.clone(),
        output.publisher, original).map_err(|_| crate::ApiError::ServingCapacity(
            "policy generation unavailable".into()))?;
    let cancel = output.cancel.clone();
    request.config.cancelled = Some(Arc::new(move || cancel.is_cancelled()));
    let session_header = request.chat_session.clone();
    let residency = request.residency.clone();
    let scope = request.state.operation_ownership.clone();
    let (handle, completion) = crate::operation_ownership::spawn_observed(scope.as_ref(),
        framer_core::operation_gate::OperationKind::Inference, async move {
            let _started = selection.pin.take().expect("one Start owner");
            invocation.run(request, head, output.cancel, output.published).await
        }).map_err(|_| crate::ApiError::ServingCapacity("operation admission closed".into()))?;
    drop(handle);
    let mut response = axum::response::Response::new(axum::body::Body::from_stream(
        crate::retain_owner(output.body, residency)));
    response.extensions_mut().insert(completion);
    response.headers_mut().insert(axum::http::header::CONTENT_TYPE,
        axum::http::HeaderValue::from_static("text/event-stream"));
    response.headers_mut().insert(axum::http::header::CACHE_CONTROL,
        axum::http::HeaderValue::from_static("no-cache"));
    response.headers_mut().insert("x-framerize-session",
        session_header.parse().unwrap_or(axum::http::HeaderValue::from_static("")));
    Ok(response)
}
struct InputWriter { sender: mpsc::Sender<wire::Part>, bytes: Vec<u8>, offset: u64 }
impl InputWriter {
    fn send(&mut self, last: bool) -> io::Result<()> {
        let bytes = std::mem::take(&mut self.bytes);
        let length = bytes.len() as u64;
        if self.offset + length > REQUEST_BYTES as u64 { return Err(io::Error::other("input bound")); }
        let part = wire::Part { offset: self.offset, last,
            bytes: wire::Segment::new(bytes).map_err(io::Error::other)? };
        self.sender.blocking_send(part).map_err(|_| io::Error::other("input closed"))?;
        self.offset = self.offset.checked_add(length).ok_or_else(|| io::Error::other("input offset"))?;
        Ok(())
    }
}
impl Write for InputWriter {
    fn write(&mut self, mut bytes: &[u8]) -> io::Result<usize> {
        let length = bytes.len();
        while !bytes.is_empty() {
            if self.bytes.len() == framer_worker_wire::SEGMENT_LIMIT { self.send(false)?; }
            if self.bytes.capacity() == 0 { self.bytes = Vec::with_capacity(framer_worker_wire::SEGMENT_LIMIT); }
            let count = bytes.len().min(framer_worker_wire::SEGMENT_LIMIT - self.bytes.len());
            self.bytes.extend_from_slice(&bytes[..count]);
            bytes = &bytes[count..];
        }
        Ok(length)
    }
    fn flush(&mut self) -> io::Result<()> { Ok(()) }
}
struct ForwardInput { cause: wire::Cause, step: wire::Text, chunk: u32, bytes: Vec<u8> }
enum Action { Forward(ForwardInput), Ack { cause: wire::Cause, message: Message }, Result(wire::Outcome) }
struct OperationState {
    invocation: Option<InvocationLedger>, receiving: Option<ForwardInput>,
    head: Option<oneshot::Sender<()>>, cancelled: bool, done: bool,
}
struct KernelOperation {
    state: Mutex<OperationState>, actions: mpsc::Sender<Action>,
    input_gate: tokio::sync::Mutex<()>,
    output: Arc<WorkerPublisher>, pin: GenerationPin, log: Arc<OperationLog>,
    done: watch::Sender<Option<wire::DoneOutcome>>,
}
struct WorkerInvocation {
    operation: Arc<KernelOperation>, actions: mpsc::Receiver<Action>,
    control: WriteSlot, payload: WriteSlot, _dispatch: Dispatch,
    original: Arc<serde_json::Value>, n: u32, cleanup: bool,
    public_failure: Option<wire::PublicFailure>, last_error: Option<String>,
}
impl Invocation for WorkerInvocation {
    type Output = wire::Summary;
    fn invoke<'a>(&'a mut self, request: &'a FreeRequest, cancel: &'a Cancel)
        -> futures::future::BoxFuture<'a, InvocationResult<Self::Output>> {
        Box::pin(async move {
            match self.invoke_bound(request, cancel).await {
                Ok(decision) => {
                    self.last_error = decision.result.as_ref().err().map(ToString::to_string);
                    self.public_failure = decision.public_failure;
                    (decision.result, decision.completed_on_disconnect)
                }
                Err(_) => {
                    self.operation.pin.unconfirmed(); self.operation.pin.poison();
                    (Err(OutputError::Closed.into()), false)
                }
            }
        })
    }
}
impl WorkerInvocation {
    async fn run(self, request: FreeRequest, head: oneshot::Receiver<()>,
        cancel: Cancel, published: Arc<std::sync::atomic::AtomicBool>) -> (Result<(), String>, bool) {
        let operation = self.operation.clone();
        let _dispatch = self._dispatch.clone();
        let _residency = request.residency.clone();
        if self.start(&request, head).await.is_err() {
            drop(self);
            drop(request);
            operation.fail().await;
            operation.output.disposed().await;
            return (Err("upstream operation task failed".into()), false);
        }
        let control = self.control.clone();
        let driver = tokio::spawn(crate::operation_ownership::scope_current(
            self.drive(request, cancel.clone(), published)));
        let returned = async {
            let result = operation.output.credit.run(operation.pin.operation(),
                control.clone(), operation.pin.subscribe()).await;
            if result.is_err() { operation.pin.unconfirmed(); operation.pin.poison(); }
            result
        };
        let cancelled = async {
            let result = operation.watch_cancel(&control, &cancel).await;
            if result.is_err() { operation.pin.unconfirmed(); operation.pin.poison(); }
        };
        let ((result, cleanup), returned, ()) = tokio::join!(
            operation.observe_driver(driver), returned, cancelled);
        operation.output.disposed().await;
        let confirmed = cleanup && returned.is_ok();
        if confirmed { operation.pin.confirm(); } else { operation.pin.unconfirmed(); }
        (result, confirmed)
    }
    async fn start(&self, request: &FreeRequest, head: oneshot::Receiver<()>)
        -> Result<(), WireError> {
        self.control.send(&Message::Start {
            op: self.operation.pin.operation(),
            logical_id: wire::Text::new(request.logical_id.clone())?,
            model: wire::Text::new(request.client_model.clone())?,
            mode: wire::Mode::Sse, output: crate::response_output::WORKER_OUTPUT,
        }).await?;
        let mut alive = self.operation.pin.subscribe();
        if !*alive.borrow() { return Err(WireError::Protocol); }
        tokio::select! {
            result = head => result.map_err(|_| WireError::Protocol),
            _ = alive.changed() => Err(WireError::Protocol),
        }
    }
    async fn drive(mut self, mut request: FreeRequest, cancel: Cancel,
        published: Arc<std::sync::atomic::AtomicBool>)
        -> Result<(Result<(), String>, bool), WireError> {
        let started = std::time::Instant::now();
        let (result, cleanup) = request.drive(&mut self, &cancel, &published).await;
        request.log_worker(&result, started.elapsed());
        self.finish(&result, &cancel).await?;
        Ok((result.map(|_| ()).map_err(|e| e.to_string()), cleanup && self.cleanup))
    }
    async fn finish(&mut self, result: &Result<wire::Summary, ResponseError>, cancel: &Cancel)
        -> Result<(), WireError> {
        if cancel.is_cancelled() {
            return self.operation.cancel(&self.control).await;
        }
        if matches!(result, Err(ResponseError::Output(_))) { return Err(WireError::Protocol); }
        let op = self.operation.pin.operation();
        let message = match result {
            Ok(_) => Message::Finish { op, n: self.n },
            Err(error) => {
                let error = error.to_string();
                let public = if self.last_error.as_ref() == Some(&error) {
                    self.public_failure.take()
                } else { None };
                let error = match public {
                    Some(public) => public,
                    None => public_failure(&error, ErrorOrigin::Local)?,
                };
                Message::Fail { op, n: self.n, error }
            }
        };
        let _ordered = self.operation.input_gate.lock().await;
        let permit = self.control.reserve().await?;
        let receipt = {
            let state = self.operation.lock()?;
            if state.done || state.cancelled { return Ok(()); }
            self.operation.output.allow_finish()?;
            permit.enqueue_encoded(frame::encode(&message)?)?
        };
        receipt.wait().await
    }
    async fn send_invocation(&self, category: wire::Text, limits: wire::Limits)
        -> Result<(), WireError> {
        let (sender, mut parts) = mpsc::channel(1);
        let original = self.original.clone();
        let job = tokio::task::spawn_blocking(move || {
            let mut writer = InputWriter { sender, bytes: Vec::new(), offset: 0 };
            serde_json::to_writer(&mut writer, original.as_ref()).map_err(io::Error::other)?;
            writer.send(true)
        });
        let sent: Result<bool, WireError> = async {
            while let Some(part) = parts.recv().await {
                let _ordered = self.operation.input_gate.lock().await;
                if self.operation.lock()?.cancelled { return Ok(false); }
                let permit = self.payload.reserve().await?;
                let message = Message::Invoke { op: self.operation.pin.operation(), n: self.n,
                    category: category.clone(), limits: limits.clone(), request: part };
                let bytes = frame::encode(&message)?;
                drop(message);
                permit.send_encoded(bytes).await?;
            }
            Ok(true)
        }.await;
        // Unblock a serializer before joining, on cancellation and every error.
        parts.close();
        drop(parts);
        let encoded = job.await.map_err(|_| WireError::Protocol)?;
        if sent? { encoded.map_err(|_| WireError::Protocol)?; }
        Ok(())
    }
    async fn send_reply(&self, cause: wire::Cause, reply: framer_core::port::ForwardReply)
        -> Result<(), WireError> {
        if reply.body.len() > 8 * 1024 * 1024 { return Err(WireError::Protocol); }
        let content_type = wire::Text::new(reply.content_type)?;
        let mut offset = 0;
        loop {
            let end = (offset + framer_worker_wire::SEGMENT_LIMIT).min(reply.body.len());
            let last = end == reply.body.len();
            let permit = self.payload.reserve().await?;
            let message = Message::ForwardReply { op: cause.op, n: cause.n, fwd: cause.fwd,
                status: reply.status, content_type: content_type.clone(), queue_wait_ms: reply.queue_wait_ms,
                body: wire::Part { offset: offset as u64, last,
                    bytes: wire::Segment::new(reply.body[offset..end].to_vec())? } };
            let encoded = frame::encode(&message)?;
            drop(message);
            permit.send_encoded(encoded).await?;
            if last { return Ok(()); }
            offset = end;
        }
    }
    async fn forward(&mut self, binding: &dyn framer_core::port::AuthorizedForward,
        input: ForwardInput, cancel: &Cancel) -> Result<(), WireError> {
        use framer_core::port::PolicyError;
        let ForwardInput { cause, step, chunk, bytes } = input;
        let payload = tokio::task::spawn_blocking(move ||
            serde_json::from_slice::<serde_json::Value>(&bytes))
            .await.map_err(|_| WireError::Protocol)?.map_err(|_| WireError::Protocol)?;
        let result = if cancel.is_cancelled() {
            Err(PolicyError::Forward(ForwardError::from_legacy("client cancelled".into())))
        } else { binding.forward_typed(step.as_str(), payload, chunk).await };
        let (reply, failure) = match result {
            Ok(reply) => (Some(reply), None),
            Err(PolicyError::Forward(error)) => (None, Some(error)),
            Err(PolicyError::Control(_)) => return Err(WireError::Protocol),
            Err(error) => (None, Some(ForwardError::from_legacy(error.to_string()))),
        };
        let error = self.operation.lock()?.invocation.as_mut().ok_or(WireError::Protocol)?
            .forward_finished(cause, failure)?;
        match (reply, error) {
            (Some(reply), None) => self.send_reply(cause, reply).await,
            (None, Some(error)) => self.payload.send(&error).await,
            _ => Err(WireError::Protocol),
        }
    }
    async fn ack(&self, cause: wire::Cause, message: Message) -> Result<(), WireError> {
        let permit = self.control.reserve().await?;
        let bytes = frame::encode(&message)?;
        let receipt = {
            let mut state = self.operation.lock()?;
            let receipt = permit.enqueue_encoded(bytes)?;
            // Ordered acceptance precedes the reader seeing the next Forward.
            // The same 429 record remains consumed after this point.
            state.invocation.as_mut().ok_or(WireError::Protocol)?.acknowledged(cause)?;
            receipt
        };
        receipt.wait().await
    }
    async fn decide(&mut self, binding: &dyn framer_core::port::AuthorizedForward, cancel: &Cancel)
        -> Result<Decision, WireError> {
        let mut alive = self.operation.pin.subscribe();
        let mut done = self.operation.done.subscribe();
        done.mark_changed();
        loop {
            if !*alive.borrow() { return Err(WireError::Protocol); }
            let action = tokio::select! {
                biased;
                action = self.actions.recv() => action.ok_or(WireError::Protocol)?,
                _ = alive.changed() => continue,
                _ = done.changed() => {
                    if done.borrow().is_none() { continue; }
                    if *done.borrow() == Some(wire::DoneOutcome::Cancelled) {
                        if let Some(ledger) = self.operation.lock()?.invocation.take() {
                            self.cleanup &= ledger.cleanup_confirmed();
                        }
                        return Ok(Decision { result: Err(OutputError::Cancelled.into()),
                            public_failure: None, completed_on_disconnect: false });
                    }
                    return Err(WireError::Protocol);
                }
            };
            match action {
                Action::Forward(input) => self.forward(binding, input, cancel).await?,
                Action::Ack { cause, message } => self.ack(cause, message).await?,
                Action::Result(outcome) => {
                    let ledger = self.operation.lock()?.invocation.take().ok_or(WireError::Protocol)?;
                    self.cleanup &= ledger.cleanup_confirmed();
                    return ledger.resolve(outcome, cancel.is_cancelled());
                }
            }
        }
    }
    async fn invoke_bound(&mut self, request: &FreeRequest, cancel: &Cancel) -> Result<Decision, WireError> {
        if cancel.is_cancelled() {
            return Ok(Decision { result: Err(OutputError::Cancelled.into()),
                public_failure: None, completed_on_disconnect: false });
        }
        let Some(binding) = request.state.client.bind_responses(&request.session,
            request.state.browser.as_ref(), &request.account.id, &request.project,
            &request.config, &request.chat_session) else {
            return Ok(Decision { result: Err(OutputError::Unsupported.into()),
                public_failure: None, completed_on_disconnect: false });
        };
        self.n = self.n.checked_add(1).ok_or(WireError::Protocol)?;
        self.public_failure = None;
        self.last_error = None;
        let limits = wire::Limits { chunk_tokens: request.config.chunk_tokens,
            max_chunks: request.config.max_chunks, max_total_tokens: request.config.max_total_tokens,
            metered_fallback: request.config.metered_fallback,
            continue_prompt: wire::Text::new(request.config.continue_prompt.clone())? };
        self.operation.lock()?.invocation = Some(InvocationLedger::new(
            self.operation.pin.operation(), self.n, limits.max_chunks, request.config.on_429.clone())?);
        self.send_invocation(wire::Text::new(binding.category().into())?, limits).await?;
        self.decide(binding.as_ref(), cancel).await
    }
    fn new(pin: GenerationPin, log: Arc<OperationLog>, output: Arc<WorkerPublisher>,
        original: serde_json::Value) -> Result<(Self, oneshot::Receiver<()>), WireError> {
        let (head, receive) = oneshot::channel();
        let (actions, inbox) = mpsc::channel(2);
        let (done, _) = watch::channel(None);
        let operation = Arc::new(KernelOperation {
            state: Mutex::new(OperationState { invocation: None, receiving: None,
                head: Some(head), cancelled: false, done: false }),
            actions, output, pin: pin.clone(), log, done, input_gate: tokio::sync::Mutex::new(()),
        });
        let dispatch: Dispatch = { let op = operation.clone(); Arc::new(move |frame| op.dispatch(frame)) };
        let (control, payload) = pin.route(&dispatch).map_err(|_| WireError::Protocol)?;
        Ok((Self { operation, actions: inbox, control, payload, _dispatch: dispatch,
            original: Arc::new(original), n: 0, cleanup: true, public_failure: None,
            last_error: None }, receive))
    }
}
impl KernelOperation {
    async fn observe_driver(&self, driver: tokio::task::JoinHandle<
        Result<(Result<(), String>, bool), WireError>>) -> (Result<(), String>, bool) {
        let result = match driver.await {
            Ok(Ok(result)) if self.wait_done().await.is_ok() => result,
            _ => {
                self.fail().await;
                (Err("upstream operation task failed".into()), false)
            }
        };
        {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            state.invocation.take();
            state.receiving.take();
        }
        result
    }
    async fn fail(&self) {
        self.pin.unconfirmed();
        self.pin.poison();
        self.log.worker_lost();
        // Local joins never clear the generation or operation fence.
        self.pin.join_lost().await;
        self.output.channel_lost().await;
    }
    async fn cancel(&self, control: &WriteSlot) -> Result<(), WireError> {
        let _ordered = self.input_gate.lock().await;
        let permit = control.reserve().await?;
        let receipt = {
            let mut state = self.lock()?;
            if state.done || state.cancelled || self.output.terminal()? { return Ok(()); }
            let bytes = frame::encode(&Message::Cancel { op: self.pin.operation() })?;
            let receipt = permit.enqueue_encoded(bytes)?;
            state.cancelled = true;
            receipt
        };
        receipt.wait().await
    }
    async fn watch_cancel(&self, control: &WriteSlot, cancel: &Cancel) -> Result<(), WireError> {
        let mut done = self.done.subscribe();
        let mut alive = self.pin.subscribe();
        loop {
            if done.borrow().is_some() { return Ok(()); }
            if !*alive.borrow() { return Err(WireError::Protocol); }
            tokio::select! {
                _ = cancel.cancelled() => return self.cancel(control).await,
                _ = done.changed() => {},
                _ = alive.changed() => {},
            }
        }
    }
    async fn wait_done(&self) -> Result<wire::DoneOutcome, WireError> {
        let mut done = self.done.subscribe();
        let mut alive = self.pin.subscribe();
        loop {
            if !*alive.borrow() { return Err(WireError::Protocol); }
            if let Some(outcome) = *done.borrow() { return Ok(outcome); }
            tokio::select! { _ = done.changed() => {}, _ = alive.changed() => {} }
        }
    }
    fn forward_part(&self, state: &mut OperationState, cause: wire::Cause,
        step: wire::Text, chunk: u32, part: wire::Part) -> Result<(), WireError> {
        if state.receiving.is_none() {
            state.invocation.as_mut().ok_or(WireError::Protocol)?.accept_forward(cause, chunk)?;
            state.receiving = Some(ForwardInput { cause, step: step.clone(), chunk, bytes: Vec::new() });
        }
        let input = state.receiving.as_mut().ok_or(WireError::Protocol)?;
        if input.cause != cause || input.chunk != chunk || input.step != step {
            return Err(WireError::Protocol);
        }
        append_part(&mut input.bytes, &part)?;
        if part.last {
            self.actions.try_send(Action::Forward(state.receiving.take().unwrap()))
                .map_err(|_| WireError::Protocol)?;
        }
        Ok(())
    }
    fn dispatch_end(&self, state: &mut OperationState, message: Message) -> Result<(), WireError> {
        match message {
            Message::Diag { n, event, .. } => {
                let ledger = state.invocation.as_ref().ok_or(WireError::Protocol)?;
                if ledger.n != n || ledger.closed { return Err(WireError::Protocol); }
                self.log.diagnostic(n, event);
                Ok(())
            }
            Message::Done { outcome, diag_dropped, .. } => {
                if state.receiving.is_some()
                    || state.invocation.as_ref().is_some_and(|ledger| !ledger.settled()) {
                    return Err(WireError::Protocol);
                }
                if outcome == wire::DoneOutcome::Cancelled && !state.cancelled {
                    return Err(WireError::Protocol);
                }
                if outcome == wire::DoneOutcome::Panicked {
                    state.done = true;
                    self.done.send_replace(Some(outcome));
                    return Err(WireError::Protocol);
                }
                self.output.done(outcome)?;
                self.log.dropped(diag_dropped)?;
                state.done = true;
                self.done.send_replace(Some(outcome));
                Ok(())
            }
            _ => Err(WireError::Protocol),
        }
    }
    fn dispatch_inner(&self, frame: Decoded<Message>) -> Result<(), WireError> {
        let op = match &frame.message {
            Message::Head { op, .. } | Message::Data { op, .. } | Message::Forward { op, .. }
            | Message::Saturated { op, .. } | Message::Result { op, .. }
            | Message::Diag { op, .. } | Message::Done { op, .. } => *op,
            _ => return Err(WireError::Protocol),
        };
        if op != self.pin.operation() { return Err(WireError::Protocol); }
        let mut state = self.lock()?;
        if state.done { return Err(WireError::Protocol); }
        match frame.message {
            Message::Head { status, content_type, .. } => {
                self.output.head(status, content_type.as_str())?;
                let _ = state.head.take().ok_or(WireError::Protocol)?.send(());
                Ok(())
            }
            _ if state.head.is_some() => Err(WireError::Protocol),
            Message::Data { event, seq, bytes, last, .. } =>
                self.output.data(event, seq, bytes, last, frame.wire_bytes),
            message => self.dispatch_control(&mut state, message),
        }
    }
    fn lock(&self) -> Result<std::sync::MutexGuard<'_, OperationState>, WireError> {
        self.state.lock().map_err(|_| WireError::Protocol)
    }
    fn dispatch_control(&self, state: &mut OperationState, message: Message) -> Result<(), WireError> {
        match message {
            Message::Forward { op, n, fwd, step, step_index, payload, .. } =>
                self.forward_part(state, wire::Cause { op, n, fwd }, step, step_index, payload),
            Message::Saturated { op, n, fwd } => {
                let cause = wire::Cause { op, n, fwd };
                let message = state.invocation.as_mut().ok_or(WireError::Protocol)?.saturated(cause)?;
                self.actions.try_send(Action::Ack { cause, message }).map_err(|_| WireError::Protocol)
            }
            Message::Result { op, n, outcome, diag_dropped } => {
                if state.receiving.is_some() { return Err(WireError::Protocol); }
                state.invocation.as_mut().ok_or(WireError::Protocol)?.close_result(op, n)?;
                self.log.dropped(diag_dropped)?;
                self.actions.try_send(Action::Result(outcome)).map_err(|_| WireError::Protocol)
            }
            other => self.dispatch_end(state, other),
        }
    }
    fn dispatch(&self, frame: Decoded<Message>) -> Result<(), WireError> {
        let result = self.dispatch_inner(frame);
        if result.is_err() { self.pin.unconfirmed(); self.pin.poison(); }
        result
    }
}

fn append_part(bytes: &mut Vec<u8>, part: &wire::Part) -> Result<(), WireError> {
    if part.offset != bytes.len() as u64 || (!part.last && part.bytes.as_slice().is_empty()) {
        return Err(WireError::Protocol);
    }
    let next = bytes.len().checked_add(part.bytes.as_slice().len())
        .filter(|n| *n <= REQUEST_BYTES).ok_or(WireError::Protocol)?;
    if next > bytes.capacity() {
        let capacity = next.max(framer_worker_wire::SEGMENT_LIMIT).next_power_of_two().min(REQUEST_BYTES);
        bytes.try_reserve_exact(capacity - bytes.len()).map_err(|_| WireError::Allocation)?;
        if bytes.capacity() > REQUEST_BYTES { return Err(WireError::Allocation); }
    }
    bytes.extend_from_slice(part.bytes.as_slice());
    Ok(())
}

fn to_wire_class(class: ErrorClass) -> wire::ErrorClass {
    use wire::ErrorClass as W;
    match class {
        ErrorClass::Cancelled => W::Cancelled, ErrorClass::RiskRefused => W::RiskRefused,
        ErrorClass::Unauthorized => W::Unauthorized, ErrorClass::CreditsExhausted => W::CreditsExhausted,
        ErrorClass::UpstreamForbidden => W::UpstreamForbidden, ErrorClass::Retryable429 => W::Retryable429,
        ErrorClass::Retryable4 => W::Retryable4, ErrorClass::Cooldown => W::Cooldown,
        ErrorClass::Other => W::Other,
    }
}

type Saturation = Option<Arc<dyn Fn() + Send + Sync>>;
fn public_failure(rendered: &str, origin: ErrorOrigin) -> Result<wire::PublicFailure, WireError> {
    public_failure_with_display(rendered, if origin == ErrorOrigin::Http {
        None
    } else { Some("upstream operation failed") })
}
fn public_failure_with_display(rendered: &str, fixed: Option<&str>)
    -> Result<wire::PublicFailure, WireError> {
    // Classification precedes redaction, once, with the exact legacy recipe.
    let (status, shaped) = convert::error_json(ClientDialect::OpenAiResponses, rendered);
    let field = |name: &str| shaped["error"][name].as_str().ok_or(WireError::Protocol);
    Ok(wire::PublicFailure {
        status, message: wire::Text::new(fixed.unwrap_or(field("message")?).into())?,
        error_type: wire::Text::new(field("type")?.into())?,
        code: wire::Text::new(field("code")?.into())?,
    })
}
fn resolve_local(decision: &mut Decision, outcome: wire::Outcome, cancelled: bool)
    -> Result<(), WireError> {
    match outcome {
        wire::Outcome::Local { failure } => {
            let message = match failure {
                wire::LocalFailure::Cancelled => "client cancelled",
                wire::LocalFailure::InvalidResponse => "invalid upstream response",
                wire::LocalFailure::IncompleteTool => "incomplete upstream tool call",
                wire::LocalFailure::Policy => "upstream operation failed",
            };
            decision.public_failure = Some(public_failure_with_display(message, Some(message))?);
            decision.result = Err(ResponseError::Upstream(message.into()));
        }
        wire::Outcome::Output { failure, completed } => {
            let error = match failure {
                wire::OutputFailure::Cancelled => OutputError::Cancelled,
                wire::OutputFailure::Closed => OutputError::Closed,
                wire::OutputFailure::Encoding => OutputError::Encoding,
                wire::OutputFailure::Unsupported => OutputError::Unsupported,
            };
            decision.completed_on_disconnect = completed && cancelled
                && matches!(error, OutputError::Cancelled | OutputError::Closed);
            decision.result = Err(error.into());
        }
        // This bypasses every string-based account and transport retry branch.
        wire::Outcome::Control => return Err(WireError::Protocol),
        _ => return Err(WireError::Protocol),
    }
    Ok(())
}
struct AcceptedForward { cause: wire::Cause, chunk: u32, saturation: Saturation }
struct FailedForward { accepted: AcceptedForward, error: ForwardError, consumed: bool }

/// One invocation, inside the existing FreeRequest account loop.
pub(crate) struct InvocationLedger {
    op: u64, n: u32, max_chunks: u32, next_forward: u64,
    saturation: Saturation, active: Option<AcceptedForward>,
    failure: Option<FailedForward>, ack: Option<wire::Cause>,
    saturations: u8, closed: bool, cleanup: bool, last_succeeded: bool,
}

impl InvocationLedger {
    pub(crate) fn resolve(self, outcome: wire::Outcome, cancelled: bool)
        -> Result<Decision, WireError> {
        if !self.closed || !self.settled() { return Err(WireError::Protocol); }
        let mut decision = Decision { result: Err(OutputError::Closed.into()),
            public_failure: None, completed_on_disconnect: false };
        match outcome {
            wire::Outcome::Success { summary } => {
                if summary.chunks > self.max_chunks || summary.metered_chunks > summary.chunks {
                    return Err(WireError::Protocol);
                }
                decision.result = Ok(summary);
            }
            wire::Outcome::Forward { failure } => {
                let record = self.failure.ok_or(WireError::Protocol)?;
                if record.accepted.cause != failure.cause || record.accepted.chunk != failure.chunk
                    || failure.chunk >= self.max_chunks { return Err(WireError::Protocol); }
                let recipe = match failure.tag {
                    wire::FailureTag::Direct => ErrorRecipe::Direct,
                    wire::FailureTag::FallbackOff => ErrorRecipe::FallbackOff { chunk: failure.chunk },
                    wire::FailureTag::SaturationExhausted if self.saturations == 9
                        && record.error.class == ErrorClass::Retryable429 && !record.consumed =>
                        ErrorRecipe::SaturationExhausted { chunk: failure.chunk, retries: 9 },
                    _ => return Err(WireError::Protocol),
                };
                let origin = record.error.origin;
                let rendered = record.error.with_recipe(recipe).to_string();
                decision.public_failure = Some(public_failure(&rendered, origin)?);
                decision.result = Err(ResponseError::Upstream(rendered));
            }
            other => resolve_local(&mut decision, other, cancelled && self.last_succeeded)?,
        }
        Ok(decision)
    }
}
pub(crate) struct Decision {
    pub result: Result<wire::Summary, ResponseError>,
    pub public_failure: Option<wire::PublicFailure>,
    pub completed_on_disconnect: bool,
}
impl InvocationLedger {
    pub(crate) fn new(op: u64, n: u32, max_chunks: u32, saturation: Saturation)
        -> Result<Self, WireError> {
        if op == 0 || n == 0 { return Err(WireError::Protocol); }
        Ok(Self { op, n, max_chunks, next_forward: 1, saturation, active: None,
            failure: None, ack: None, saturations: 0, closed: false, cleanup: true,
            last_succeeded: false })
    }
    pub(crate) fn cleanup_confirmed(&self) -> bool { self.cleanup }
    pub(crate) fn settled(&self) -> bool { self.active.is_none() && self.ack.is_none() }
    /// Synchronous receive-side effect, before the Ack can be enqueued.
    pub(crate) fn saturated(&mut self, cause: wire::Cause) -> Result<Message, WireError> {
        if self.closed || !self.settled() || self.saturations >= 9 {
            return Err(WireError::Protocol);
        }
        let record = self.failure.as_mut().ok_or(WireError::Protocol)?;
        if record.accepted.cause != cause || record.consumed
            || cause.fwd.checked_add(1) != Some(self.next_forward)
            || record.error.class != ErrorClass::Retryable429 {
            return Err(WireError::Protocol);
        }
        record.consumed = true;
        self.ack = Some(cause);
        self.saturations += 1;
        if let Some(mark) = &record.accepted.saturation { mark(); }
        Ok(Message::SaturatedAck { op: cause.op, n: cause.n, fwd: cause.fwd })
    }
    pub(crate) fn acknowledged(&mut self, cause: wire::Cause) -> Result<(), WireError> {
        if self.ack != Some(cause) { return Err(WireError::Protocol); }
        self.ack = None;
        Ok(())
    }
    /// Receipt closes dispatch immediately. Resolution waits for accepted work.
    pub(crate) fn close_result(&mut self, op: u64, n: u32) -> Result<(), WireError> {
        if self.closed || op != self.op || n != self.n { return Err(WireError::Protocol); }
        self.closed = true;
        Ok(())
    }
    pub(crate) fn accept_forward(&mut self, cause: wire::Cause, chunk: u32)
        -> Result<(), WireError> {
        if self.closed || !self.settled() || cause.op != self.op || cause.n != self.n
            || cause.fwd != self.next_forward || chunk >= self.max_chunks {
            return Err(WireError::Protocol);
        }
        self.next_forward = cause.fwd.checked_add(1).ok_or(WireError::Protocol)?;
        self.last_succeeded = false;
        self.active = Some(AcceptedForward { cause, chunk, saturation: self.saturation.clone() });
        Ok(())
    }
    /// Called only after the runtime Forward future and its cleanup have joined.
    pub(crate) fn forward_finished(&mut self, cause: wire::Cause, error: Option<ForwardError>)
        -> Result<Option<Message>, WireError> {
        let accepted = self.active.take().ok_or(WireError::Protocol)?;
        if accepted.cause != cause { return Err(WireError::Protocol); }
        match error {
            None => { self.last_succeeded = true; Ok(None) },
            Some(error) => {
                self.cleanup &= !crate::pool_control::cleanup_unconfirmed(error.original_message());
                let message = Message::ForwardError { op: cause.op, n: cause.n, fwd: cause.fwd,
                    class: to_wire_class(error.class), cooldown_seconds: error.cooldown_seconds };
                self.failure = Some(FailedForward { accepted, error, consumed: false });
                Ok(Some(message))
            }
        }
    }
}

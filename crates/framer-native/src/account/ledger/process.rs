use super::DeviceFailure;
use serde_json::{json, Value};
use std::{collections::HashMap, path::Path, process::Stdio, sync::{Arc,
    atomic::{AtomicBool, Ordering}}};
use tokio::{io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    process::{Child, ChildStdin, ChildStdout, Command},
    sync::{mpsc, oneshot, watch, Notify}, time::{timeout, Duration}};

struct State {
    failed: AtomicBool,
    closing: AtomicBool,
    wake: Notify,
}
impl State {
    fn fail(&self) {
        self.failed.store(true, Ordering::Release);
        self.wake.notify_one();
    }
}

#[derive(Clone)]
pub(super) struct Connection {
    sender: mpsc::Sender<Request>,
    state: Arc<State>,
    done: watch::Receiver<Option<Result<(), DeviceFailure>>>,
}

pub(super) enum Undo { None, Open(Value), Mint(Value) }
struct Request {
    op: &'static str,
    args: Value,
    reply: Option<oneshot::Sender<Result<Reply, DeviceFailure>>>,
    undo: Undo,
}
pub(super) struct Reply {
    value: Option<Value>, undo: Undo,
    sender: mpsc::WeakSender<Request>, state: Arc<State>,
}

async fn read_frame(reader: &mut BufReader<ChildStdout>, bytes: &mut Vec<u8>) -> Result<Value, ()> {
    let remaining = 4097usize.checked_sub(bytes.len()).ok_or(())?;
    (&mut *reader).take(remaining as u64).read_until(b'\n', bytes).await.map_err(|_| ())?;
    if bytes.len() > 4096 || bytes.last() != Some(&b'\n') { return Err(()); }
    let value = serde_json::from_slice(bytes).map_err(|_| ())?;
    bytes.clear();
    Ok(value)
}

fn deliver(value: Value, pending: &mut HashMap<u64, Request>,
    sender: &mpsc::WeakSender<Request>, state: &Arc<State>) -> Result<(), ()> {
    let object = value.as_object().ok_or(())?;
    let id = object.get("id").and_then(Value::as_u64).ok_or(())?;
    let request = pending.remove(&id).ok_or(())?;
    if object.len() != 3 { return Err(()); }
    let result = match object.get("ok").and_then(Value::as_bool) {
        Some(true) => Ok(object.get("result").filter(|v| v.is_object()).ok_or(())?.clone()),
        Some(false) => Err(match object.get("error").and_then(Value::as_str) {
            Some("rejected") => DeviceFailure::Rejected,
            Some("unconfirmed") => DeviceFailure::Unconfirmed,
            _ => return Err(()),
        }),
        None => return Err(()),
    };
    if let Some(reply) = request.reply {
        let result = result.map(|value| Reply { value: Some(value), undo: request.undo,
            sender: sender.clone(), state: state.clone() });
        let _ = reply.send(result);
    } else {
        let value = result.map_err(|_| ())?;
        let field = if request.op == "close" { "closed" } else { "confirmed" };
        if value.get(field).and_then(Value::as_bool) != Some(true) { return Err(()); }
    }
    Ok(())
}

async fn run_actor(mut child: Child, mut input: ChildStdin, mut output: BufReader<ChildStdout>,
    mut receiver: mpsc::Receiver<Request>, sender: mpsc::WeakSender<Request>, state: Arc<State>)
    -> Result<(), DeviceFailure> {
    let mut pending = HashMap::new();
    let mut bytes = Vec::new();
    let mut sequence = 0u64;
    let orderly = loop {
        if state.failed.load(Ordering::Acquire) { break false; }
        tokio::select! {
            _ = state.wake.notified() => break false,
            request = receiver.recv(), if pending.len() < 64 => {
                let Some(request) = request else { break false; };
                let Some(next) = sequence.checked_add(1) else { break false; };
                sequence = next;
                let Ok(mut frame) = serde_json::to_vec(&json!({"id": sequence,
                    "op": request.op, "args": &request.args})) else { break false; };
                if frame.len() >= 4096 { break false; }
                frame.push(b'\n');
                pending.insert(sequence, request);
                if input.write_all(&frame).await.is_err() { break false; }
            }
            frame = read_frame(&mut output, &mut bytes) => match frame {
                Ok(value) => if deliver(value, &mut pending, &sender, &state).is_err() { break false; },
                Err(()) => break bytes.is_empty() && pending.is_empty()
                    && state.closing.load(Ordering::Acquire),
            }
        }
    };
    drop(input);
    if !orderly { let _ = child.kill().await; }
    let status = timeout(Duration::from_secs(5), child.wait()).await;
    if status.is_err() {
        // Keep session ownership until the timed-out helper has actually exited.
        let _ = child.kill().await;
        let _ = child.wait().await;
    }
    if orderly && matches!(status, Ok(Ok(status)) if status.success()) { Ok(()) }
    else { Err(DeviceFailure::Unconfirmed) }
}
impl Reply {
    pub(super) fn value(&self) -> &Value { self.value.as_ref().expect("unconsumed reply") }
    pub(super) fn accept(mut self) -> Value {
        self.undo = Undo::None;
        self.value.take().expect("unconsumed reply")
    }
    pub(super) fn invalid(&self) -> DeviceFailure { self.state.fail(); DeviceFailure::Unconfirmed }
}
impl Drop for Reply {
    fn drop(&mut self) {
        let Some(value) = &self.value else { return; };
        let operation = match std::mem::replace(&mut self.undo, Undo::None) {
            Undo::None => return,
            Undo::Open(mut args) => {
                let Some(generation) = value.get("generation").and_then(Value::as_str) else {
                    self.state.fail(); return;
                };
                args["generation"] = json!(generation);
                args["joined"] = json!(true);
                args["risk_refused"] = json!(false);
                ("close", args)
            }
            Undo::Mint(mut args) => {
                let Some(sequence) = value.get("sequence").and_then(Value::as_u64) else {
                    self.state.fail(); return;
                };
                args["sequence"] = json!(sequence);
                args["joined"] = json!(true);
                ("finish", args)
            }
        };
        let request = Request { op: operation.0, args: operation.1, reply: None, undo: Undo::None };
        if self.sender.upgrade().is_none_or(|sender| sender.try_send(request).is_err()) {
            self.state.fail();
        }
    }
}

impl Connection {
    pub(super) async fn request(&self, op: &'static str, args: Value, undo: Undo)
        -> Result<Reply, DeviceFailure> {
        if self.state.closing.load(Ordering::Acquire) { return Err(DeviceFailure::Unconfirmed); }
        self.call(op, args, undo).await
    }

    async fn call(&self, op: &'static str, args: Value, undo: Undo) -> Result<Reply, DeviceFailure> {
        if self.state.failed.load(Ordering::Acquire) { return Err(DeviceFailure::Unconfirmed); }
        let (sender, receiver) = oneshot::channel();
        let request = Request { op, args, reply: Some(sender), undo };
        let result = timeout(Duration::from_secs(20), async {
            self.sender.send(request).await.map_err(|_| DeviceFailure::Unconfirmed)?;
            receiver.await.map_err(|_| DeviceFailure::Unconfirmed)?
        }).await;
        match result {
            Ok(result) => result,
            Err(_) => { self.state.fail(); Err(DeviceFailure::Unconfirmed) }
        }
    }

    pub(super) async fn shutdown(&self) -> Result<(), DeviceFailure> {
        if self.state.closing.swap(true, Ordering::AcqRel) { return self.wait_closed().await; }
        let clean = match self.call("shutdown", json!({}), Undo::None).await {
            Ok(reply) if reply.value() == &json!({"clean": true}) => { reply.accept(); true }
            _ => { self.state.fail(); false }
        };
        let terminal = self.wait_closed().await;
        if clean { terminal } else { Err(DeviceFailure::Unconfirmed) }
    }

    async fn wait_closed(&self) -> Result<(), DeviceFailure> {
        timeout(Duration::from_secs(5), self.wait_for_exit()).await
            .map_err(|_| DeviceFailure::Unconfirmed)?
    }

    pub(super) async fn wait_for_exit(&self) -> Result<(), DeviceFailure> {
        let mut done = self.done.clone();
        loop {
            if let Some(result) = *done.borrow_and_update() { return result; }
            done.changed().await.map_err(|_| DeviceFailure::Unconfirmed)?;
        }
    }

    pub(super) async fn start(python: &Path, script: &Path, root: &Path,
        sessions: Arc<dyn framer_core::port::SessionStore>) -> Result<Self, DeviceFailure> {
        if !python.is_absolute() || !script.is_absolute() || !root.is_absolute() {
            return Err(DeviceFailure::Rejected);
        }
        let mut child = Command::new(python).arg("-B").arg(script).arg("--root").arg(root)
            .process_group(0)
            .env_clear().env("PATH", "/usr/bin:/bin")
            .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null())
            .kill_on_drop(true).spawn().map_err(|_| DeviceFailure::Rejected)?;
        let input = child.stdin.take().ok_or(DeviceFailure::Unconfirmed)?;
        let mut output = BufReader::new(child.stdout.take().ok_or(DeviceFailure::Unconfirmed)?);
        let ready = timeout(Duration::from_secs(5), read_frame(&mut output, &mut Vec::new())).await;
        if !matches!(ready, Ok(Ok(ref value)) if value == &json!({"ready": 1})) {
            let _ = child.kill().await;
            let _ = child.wait().await;
            return Err(DeviceFailure::Rejected);
        }
        let state = Arc::new(State { failed: AtomicBool::new(false),
            closing: AtomicBool::new(false), wake: Notify::new() });
        let (sender, receiver) = mpsc::channel(64);
        let weak = sender.downgrade();
        let (done_tx, done) = watch::channel(None);
        let owned = state.clone();
        tokio::spawn(async move {
            let _sessions = sessions;
            let result = run_actor(child, input, output, receiver, weak, owned.clone()).await;
            drop(_sessions);
            owned.failed.store(true, Ordering::Release);
            let _ = done_tx.send(Some(result));
        });
        Ok(Self { sender, state, done })
    }
}

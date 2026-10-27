//! Two-slot generation ownership. No account state or autonomous restarts.
use crate::policy_process::{Dispatch, Executable, PolicyProcess, ProcessControl};
use framer_worker_wire::{frame::{Decoded, Outbox, WriteSlot}, message::Message, WireError};
use futures::{future::{BoxFuture, Shared}, FutureExt};
use std::{collections::BTreeMap, sync::{Arc, Mutex, Weak, atomic::{AtomicBool, Ordering}}};
use tokio::sync::{oneshot, watch};

/// Configure after operation ownership, before sharing or admitting requests.
/// No child is created until stage(), and no worker is selected until activate().
pub fn with_policy_workers(mut state: crate::AppState) -> Result<crate::AppState, GenerationError> {
    let runtime = state.runtime_mut().ok_or(GenerationError::RuntimeShared)?;
    if runtime.policy_generations.is_some() { return Err(GenerationError::AlreadyConfigured); }
    let ownership = runtime.operation_ownership.clone().ok_or(GenerationError::OwnershipRequired)?;
    runtime.policy_generations = Some(GenerationManager::new(
        runtime.response_output.construction_limit(), ownership)?);
    Ok(state)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GenerationPhase { Starting, Ready, Draining, ReapedUnconfirmed }
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GenerationSnapshot {
    pub generation: u64, pub phase: GenerationPhase, pub pins: usize,
    pub unconfirmed: bool, pub active: bool, pub alive: bool,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GenerationError { Closed, Full, Unknown, NotReady, Protocol, Process, InvalidLimit,
    Unconfirmed, OwnershipRequired, RuntimeShared, AlreadyConfigured }

type Completion = Shared<BoxFuture<'static, ()>>;
type Retirement = Shared<BoxFuture<'static, Result<(), GenerationError>>>;
struct State {
    next: u64, next_op: u64, active: Option<u64>, closed: bool, forced: bool,
    slots: BTreeMap<u64, Slot>, shutdown: Option<Retirement>,
}
struct Slot {
    phase: GenerationPhase, pins: usize, unconfirmed: bool,
    worker: Option<Arc<Worker>>, starting: Option<Completion>, retiring: Option<Retirement>,
    cancel_stage: watch::Sender<bool>,
}
struct Worker {
    process: tokio::sync::Mutex<PolicyProcess>, outbox: Outbox,
    control: ProcessControl,
    alive: watch::Receiver<bool>, inbox: Arc<Inbox>,
}
impl Worker {
    fn new(process: PolicyProcess, inbox: Arc<Inbox>) -> Arc<Self> {
        Arc::new(Self { outbox: process.outbox.clone(), alive: process.subscribe(),
            control: process.control(), process: tokio::sync::Mutex::new(process), inbox })
    }
    async fn end(&self, graceful: bool) -> bool {
        if graceful { self.control.request_drain(); } else { self.control.request_stop(); }
        self.process.lock().await.join().await.is_ok_and(|result| result.reaped)
    }
}
struct InboxState { hello: Option<oneshot::Sender<()>>, ready: bool,
    operations: BTreeMap<u64, Weak<dyn Fn(Decoded<Message>) -> Result<(), WireError> + Send + Sync>> }
struct Inbox { state: Mutex<InboxState>, limit: usize }
pub struct GenerationManager {
    state: Mutex<State>, changed: watch::Sender<()>, limit: usize,
    ownership: Arc<crate::operation_ownership::ServingOwnership>,
}
impl GenerationManager {
    fn new(limit: usize, ownership: Arc<crate::operation_ownership::ServingOwnership>)
        -> Result<Arc<Self>, GenerationError> {
        if limit == 0 || limit > 65536 { return Err(GenerationError::InvalidLimit); }
        let (changed, _) = watch::channel(());
        Ok(Arc::new(Self { state: Mutex::new(State {
            next: 1, next_op: 1, active: None, closed: false, forced: false,
            slots: BTreeMap::new(), shutdown: None,
        }), changed, limit, ownership }))
    }
    fn lock(&self) -> Result<std::sync::MutexGuard<'_, State>, GenerationError> {
        self.state.lock().map_err(|_| GenerationError::Protocol)
    }
    pub fn snapshot(&self) -> Result<Vec<GenerationSnapshot>, GenerationError> {
        let state = self.lock()?;
        Ok(state.slots.iter().map(|(&generation, slot)| GenerationSnapshot {
            generation, phase: slot.phase, pins: slot.pins, unconfirmed: slot.unconfirmed,
            active: state.active == Some(generation),
            alive: slot.worker.as_ref().is_some_and(|w| *w.alive.borrow()),
        }).collect())
    }
}
struct StageGuard { owner: Weak<GenerationManager>, generation: u64, finished: bool }
impl Drop for StageGuard {
    fn drop(&mut self) {
        if self.finished { return; }
        if let Some(manager) = self.owner.upgrade() {
            if let Ok(mut state) = manager.lock() {
                if let Some(slot) = state.slots.get_mut(&self.generation) {
                    slot.phase = GenerationPhase::Draining;
                    slot.unconfirmed = true;
                    if let Some(worker) = &slot.worker { worker.control.request_stop(); }
                }
            }
            manager.changed.send_replace(());
        }
    }
}
impl Drop for GenerationManager {
    fn drop(&mut self) {
        // A library caller must await shutdown. Drop requests stop but cannot
        // claim reaping; each process monitor still owns its child and pipes.
        let state = self.state.get_mut().unwrap_or_else(|e| e.into_inner());
        for slot in state.slots.values() {
            if let Some(worker) = &slot.worker { worker.control.request_stop(); }
        }
    }
}
#[derive(Clone)]
pub(crate) struct GenerationPin(Arc<PinLease>);
struct PinLease { manager: Weak<GenerationManager>, generation: u64, op: u64,
    worker: Arc<Worker>, confirmed: AtomicBool, channels: Mutex<Option<(WriteSlot, WriteSlot)>> }
impl GenerationPin {
    pub fn generation(&self) -> u64 { self.0.generation }
    pub fn operation(&self) -> u64 { self.0.op }
    /// Only after Done (or joined channel loss), all accepted Forward verdicts
    /// and local jobs. Each retained output owner must still hold a pin clone.
    pub(crate) fn confirm(&self) { self.0.confirmed.store(true, Ordering::Release); }
    pub(crate) fn route(&self, handler: &Dispatch) -> Result<(WriteSlot, WriteSlot), GenerationError> {
        let mut channels = self.0.channels.lock().map_err(|_| GenerationError::Protocol)?;
        let mut inbox = self.0.worker.inbox.state.lock().map_err(|_| GenerationError::Protocol)?;
        if channels.is_some() || !inbox.ready || !*self.0.worker.alive.borrow()
            || inbox.operations.len() >= self.0.worker.inbox.limit {
            return Err(GenerationError::Protocol);
        }
        let control = self.0.worker.outbox.register(self.0.op * 2 - 1)
            .map_err(|_| GenerationError::Protocol)?;
        let payload = match self.0.worker.outbox.register(self.0.op * 2) {
            Ok(slot) => slot,
            Err(_) => { let _ = self.0.worker.outbox.remove(&control);
                return Err(GenerationError::Protocol); }
        };
        inbox.operations.insert(self.0.op, Arc::downgrade(handler));
        *channels = Some((control.clone(), payload.clone()));
        Ok((control, payload))
    }
    pub(crate) fn subscribe(&self) -> watch::Receiver<bool> { self.0.worker.alive.clone() }
    pub(crate) fn poison(&self) { self.0.worker.control.request_stop(); }
    pub fn unconfirmed(&self) {
        if let Some(manager) = self.0.manager.upgrade() {
            if let Ok(mut state) = manager.lock() {
                if let Some(slot) = state.slots.get_mut(&self.0.generation) { slot.unconfirmed = true; }
            }
            manager.changed.send_replace(());
        }
    }
}
impl Drop for PinLease {
    fn drop(&mut self) {
        if let Some((control, payload)) = self.channels.get_mut().unwrap_or_else(|e| e.into_inner()).take() {
            // Done and last-owner settlement must precede removal. A busy slot
            // is a local integrity failure, never permission to cut its frame.
            let control_removed = self.worker.outbox.remove(&control).is_ok();
            let payload_removed = self.worker.outbox.remove(&payload).is_ok();
            if !control_removed || !payload_removed {
                self.confirmed.store(false, Ordering::Release);
                self.worker.control.request_stop();
            }
        }
        if let Ok(mut inbox) = self.worker.inbox.state.lock() { inbox.operations.remove(&self.op); }
        if let Some(manager) = self.manager.upgrade() {
            if let Ok(mut state) = manager.lock() {
                if let Some(slot) = state.slots.get_mut(&self.generation) {
                    slot.unconfirmed |= !self.confirmed.load(Ordering::Acquire);
                    if slot.pins == 0 { slot.unconfirmed = true; } else { slot.pins -= 1; }
                }
            }
            manager.changed.send_replace(());
        }
    }
}
impl GenerationManager {
    pub fn activate(&self, generation: u64) -> Result<(), GenerationError> {
        let mut state = self.lock()?;
        if state.closed { return Err(GenerationError::Closed); }
        if !self.ownership.accepting_requests() || state.slots.values().any(|s| s.unconfirmed) {
            return Err(GenerationError::Unconfirmed);
        }
        let slot = state.slots.get(&generation).ok_or(GenerationError::Unknown)?;
        if slot.unconfirmed || slot.phase != GenerationPhase::Ready
            || !slot.worker.as_ref().is_some_and(|w| *w.alive.borrow()) {
            return Err(GenerationError::NotReady);
        }
        state.active = Some(generation);
        Ok(())
    }
    pub async fn stage(self: &Arc<Self>, path: std::path::PathBuf, sha256: [u8; 32])
        -> Result<u64, GenerationError> {
        let result = {
            let mut state = self.lock()?;
            if state.closed { return Err(GenerationError::Closed); }
            if state.slots.len() >= 2 { return Err(GenerationError::Full); }
            let generation = state.next;
            state.next = generation.checked_add(1).ok_or(GenerationError::Full)?;
            let (cancel_stage, closing) = watch::channel(false);
            state.slots.insert(generation, Slot { phase: GenerationPhase::Starting,
                pins: 0, unconfirmed: false, worker: None, starting: None, retiring: None,
                cancel_stage });
            let (reply, result) = oneshot::channel();
            let owner = Arc::downgrade(self);
            let limit = self.limit;
            let job = tokio::spawn(async move {
                let mut guard = StageGuard { owner: owner.clone(), generation, finished: false };
                let result = stage_worker(owner, generation, Executable { path, sha256 },
                    limit, closing).await;
                guard.finished = true;
                let _ = reply.send(result.map(|()| generation));
            });
            state.slots.get_mut(&generation).unwrap().starting =
                Some(async move { let _ = job.await; }.boxed().shared());
            result
        };
        result.await.map_err(|_| GenerationError::Process)?
    }
    pub(crate) fn select(self: &Arc<Self>) -> Result<Option<GenerationPin>, GenerationError> {
        let mut state = self.lock()?;
        if state.closed { return Err(GenerationError::Closed); }
        if !self.ownership.accepting_requests() || state.slots.values().any(|s| s.unconfirmed) {
            return Err(GenerationError::Unconfirmed);
        }
        let Some(generation) = state.active else { return Ok(None); };
        let slot = state.slots.get(&generation).ok_or(GenerationError::Unknown)?;
        let worker = slot.worker.clone().ok_or(GenerationError::NotReady)?;
        if slot.unconfirmed || slot.phase != GenerationPhase::Ready || !*worker.alive.borrow() {
            state.active = None;
            return Ok(None);
        }
        if state.slots.values().map(|slot| slot.pins).sum::<usize>() >= self.limit {
            return Err(GenerationError::Full);
        }
        let op = state.next_op;
        op.checked_mul(2).ok_or(GenerationError::Full)?;
        state.next_op = op.checked_add(1).ok_or(GenerationError::Full)?;
        state.slots.get_mut(&generation).unwrap().pins += 1;
        Ok(Some(GenerationPin(Arc::new(PinLease {
            manager: Arc::downgrade(self), generation, op, worker,
            confirmed: AtomicBool::new(false), channels: Mutex::new(None),
        }))))
    }
}
impl GenerationManager {
    /// No new pins after this transition. Existing operations keep their worker.
    pub async fn retire(self: &Arc<Self>, generation: u64) -> Result<(), GenerationError> {
        let completion = {
            let mut state = self.lock()?;
            let slot = state.slots.get_mut(&generation).ok_or(GenerationError::Unknown)?;
            if let Some(completion) = &slot.retiring { completion.clone() } else {
                if slot.phase == GenerationPhase::ReapedUnconfirmed {
                    return Err(GenerationError::Unconfirmed);
                }
                slot.phase = GenerationPhase::Draining;
                slot.cancel_stage.send_replace(true);
                let starting = slot.starting.clone();
                let owner = self.clone();
                let job = tokio::spawn(async move {
                    if let Some(starting) = starting { starting.await; }
                    owner.retire_worker(generation).await
                });
                let completion = async move { job.await.unwrap_or(Err(GenerationError::Process)) }
                    .boxed().shared();
                slot.retiring = Some(completion.clone());
                if state.active == Some(generation) { state.active = None; }
                completion
            }
        };
        self.changed.send_replace(());
        completion.await
    }
}

impl GenerationManager {
    async fn retire_worker(&self, generation: u64) -> Result<(), GenerationError> {
        let mut changed = self.changed.subscribe();
        let (worker, forced) = loop {
            {
                let state = self.lock()?;
                let Some(slot) = state.slots.get(&generation) else { return Ok(()); };
                if slot.pins == 0 || state.forced {
                    break (slot.worker.clone(), state.forced);
                }
            }
            changed.changed().await.map_err(|_| GenerationError::Closed)?;
        };
        let reaped = match worker { Some(worker) => worker.end(!forced).await, None => true };
        let mut state = self.lock()?;
        let Some(slot) = state.slots.get_mut(&generation) else { return Ok(()); };
        if !reaped || slot.pins != 0 || slot.unconfirmed {
            slot.unconfirmed = true;
            slot.phase = if reaped { GenerationPhase::ReapedUnconfirmed }
                else { GenerationPhase::Draining };
            return Err(GenerationError::Unconfirmed);
        }
        state.slots.remove(&generation);
        Ok(())
    }
}

impl GenerationManager {
    /// Normal shutdown follows application drain. Error shutdown still reaps
    /// children, but cannot turn outstanding pins or cleanup into confirmation.
    pub async fn shutdown(self: &Arc<Self>, force: bool) -> Result<(), GenerationError> {
        let completion = {
            let mut state = self.lock()?;
            state.closed = true;
            state.active = None;
            state.forced |= force;
            if force {
                for slot in state.slots.values() {
                    if let Some(worker) = &slot.worker { worker.control.request_stop(); }
                }
            }
            if let Some(completion) = &state.shutdown { completion.clone() } else {
                let generations: Vec<_> = state.slots.keys().copied().collect();
                let owner = self.clone();
                let job = tokio::spawn(async move {
                    let results = futures::future::join_all(generations.into_iter()
                        .map(|generation| owner.retire(generation))).await;
                    results.into_iter().filter(|r| *r != Err(GenerationError::Unknown))
                        .collect::<Result<Vec<_>, _>>().map(|_| ())
                });
                let completion = async move { job.await.unwrap_or(Err(GenerationError::Process)) }
                    .boxed().shared();
                state.shutdown = Some(completion.clone());
                completion
            }
        };
        self.changed.send_replace(());
        completion.await
    }
}

async fn stage_worker(owner: Weak<GenerationManager>, generation: u64,
    executable: Executable, limit: usize, mut closing: watch::Receiver<bool>)
    -> Result<(), GenerationError> {
    let (inbox, hello) = Inbox::new(limit);
    let dispatch_inbox = inbox.clone();
    let dispatch: Dispatch = Arc::new(move |frame| dispatch_inbox.dispatch(frame));
    let process = match PolicyProcess::spawn(executable, limit * 2 + 1, dispatch).await {
        Ok(process) => process,
        Err(_) => { finish_failed_stage(&owner, generation, true); return Err(GenerationError::Process); }
    };
    let worker = Worker::new(process, inbox);
    let installed = owner.upgrade().ok_or(GenerationError::Closed)
        .and_then(|manager| manager.install(generation, worker.clone()));
    let result = match installed {
        Ok(()) => prepare_ready(&owner, generation, &worker, hello, &mut closing).await,
        Err(error) => Err(error),
    };
    if result.is_err() {
        let reaped = worker.end(false).await;
        finish_failed_stage(&owner, generation, reaped);
    }
    result
}

impl GenerationManager {
    fn install(&self, generation: u64, worker: Arc<Worker>) -> Result<(), GenerationError> {
        let mut state = self.lock()?;
        if state.closed { return Err(GenerationError::Closed); }
        let slot = state.slots.get_mut(&generation).ok_or(GenerationError::Unknown)?;
        if slot.phase != GenerationPhase::Starting { return Err(GenerationError::NotReady); }
        slot.worker = Some(worker);
        Ok(())
    }
    fn publish(&self, generation: u64) -> Result<(), GenerationError> {
        let mut state = self.lock()?;
        if state.closed { return Err(GenerationError::Closed); }
        let slot = state.slots.get_mut(&generation).ok_or(GenerationError::Unknown)?;
        if slot.phase != GenerationPhase::Starting
            || !slot.worker.as_ref().is_some_and(|w| *w.alive.borrow()) {
            return Err(GenerationError::NotReady);
        }
        slot.phase = GenerationPhase::Ready;
        Ok(())
    }
}
async fn prepare_ready(owner: &Weak<GenerationManager>, generation: u64,
    worker: &Worker, hello: oneshot::Receiver<()>, closing: &mut watch::Receiver<bool>)
    -> Result<(), GenerationError> {
    let mut alive = worker.alive.clone();
    if *closing.borrow() || !*alive.borrow() { return Err(GenerationError::Closed); }
    tokio::select! {
        biased;
        _ = closing.changed() => return Err(GenerationError::Closed),
        _ = alive.changed() => return Err(GenerationError::Process),
        result = hello => result.map_err(|_| GenerationError::Protocol)?,
    }
    let slot = worker.outbox.register(0).map_err(|_| GenerationError::Protocol)?;
    let message = Message::Ready { generation };
    tokio::select! {
        biased;
        _ = closing.changed() => return Err(GenerationError::Closed),
        _ = alive.changed() => return Err(GenerationError::Process),
        result = slot.send(&message) => result.map_err(|_| GenerationError::Process)?,
    }
    worker.outbox.remove(&slot).map_err(|_| GenerationError::Protocol)?;
    worker.inbox.state.lock().map_err(|_| GenerationError::Protocol)?.ready = true;
    owner.upgrade().ok_or(GenerationError::Closed)?.publish(generation)
}

fn finish_failed_stage(owner: &Weak<GenerationManager>, generation: u64, reaped: bool) {
    if let Some(manager) = owner.upgrade() {
        if let Ok(mut state) = manager.lock() {
            if reaped { state.slots.remove(&generation); }
            else if let Some(slot) = state.slots.get_mut(&generation) {
                slot.phase = GenerationPhase::Draining;
                slot.unconfirmed = true;
            }
        }
        manager.changed.send_replace(());
    }
}
impl Inbox {
    fn new(limit: usize) -> (Arc<Self>, oneshot::Receiver<()>) {
        let (sender, receiver) = oneshot::channel();
        (Arc::new(Self { state: Mutex::new(InboxState {
            hello: Some(sender), ready: false, operations: BTreeMap::new(),
        }), limit }), receiver)
    }
    fn dispatch(&self, frame: Decoded<Message>) -> Result<(), WireError> {
        let mut state = self.state.lock().map_err(|_| WireError::Protocol)?;
        if let Some(sender) = state.hello.take() {
            match &frame.message {
                Message::Hello { wire, build, routes } if *wire == framer_worker_wire::VERSION
                    && !build.as_str().is_empty() && routes.as_slice() == [framer_worker_wire::message::Route::FreeResponses] => {
                    sender.send(()).map_err(|_| WireError::Protocol)?;
                    return Ok(());
                }
                _ => return Err(WireError::Protocol),
            }
        }
        if !state.ready { return Err(WireError::Protocol); }
        let op = match &frame.message {
            Message::Head { op, .. } | Message::Forward { op, .. }
            | Message::Saturated { op, .. } | Message::Data { op, .. }
            | Message::Diag { op, .. } | Message::Result { op, .. }
            | Message::Done { op, .. } => op,
            _ => return Err(WireError::Protocol),
        };
        let handler = state.operations.get(op).and_then(Weak::upgrade)
            .ok_or(WireError::Protocol)?;
        drop(state); // No lifecycle or routing lock is held during delivery.
        handler(frame)
    }
}

#[cfg(test)]
#[path = "policy_generation_tests.rs"]
mod tests;

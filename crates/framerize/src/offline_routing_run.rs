use super::*;
use port::{Cancellation, InvocationRun};
use framer_api::experimental_native::native_account::PreparationControl;
use std::sync::Mutex;
use tokio::{sync::{oneshot, watch}, time::Instant};

pub(super) struct Control {
    stop: watch::Sender<bool>,
    inner: Mutex<Option<Arc<dyn Cancellation>>>,
    progress: Mutex<Option<port::AttemptProgress>>,
    failure: Mutex<FailureKind>,
    preparation: Mutex<Option<PreparationControl>>,
}
impl Control {
    pub fn new() -> Arc<Self> {
        Arc::new(Self { stop: watch::channel(false).0, inner: Mutex::new(None),
            progress: Mutex::new(None), failure: Mutex::new(FailureKind::Cleanup),
            preparation: Mutex::new(None) })
    }
    pub fn fail(&self, kind: FailureKind) {
        *self.failure.lock().unwrap_or_else(|e| e.into_inner()) = kind;
    }
    pub fn failure(&self) -> FailureKind {
        *self.failure.lock().unwrap_or_else(|e| e.into_inner())
    }
    pub fn stopped(&self, deadline: Instant) -> Result<(), DispatchError> {
        let kind = if Instant::now() >= deadline { Some(FailureKind::Deadline) }
            else if *self.stop.borrow() { Some(FailureKind::Cancelled) } else { None };
        kind.map_or(Ok(()), |kind| Err(DispatchError::new(kind)))
    }
    pub fn attach<T>(&self, run: &InvocationRun<T>) {
        *self.progress.lock().unwrap_or_else(|e| e.into_inner()) = run.completed_attempts.clone();
        *self.inner.lock().unwrap_or_else(|e| e.into_inner()) = Some(run.cancellation.clone());
        if *self.stop.borrow() { run.cancellation.cancel(); }
    }

    pub fn attach_preparation(&self, work: PreparationControl) {
        *self.preparation.lock().unwrap_or_else(|e| e.into_inner()) = Some(work.clone());
        if *self.stop.borrow() { work.cancel(); }
    }
}

impl Cancellation for Control {
    fn cancel(&self) {
        self.stop.send_replace(true);
        if let Some(work) = self.preparation.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
            work.cancel();
        }
        let inner = self.inner.lock().unwrap_or_else(|e| e.into_inner()).clone();
        if let Some(inner) = inner { inner.cancel(); }
    }
}

struct CancelOnDrop { control: Arc<Control>, armed: bool }
#[cfg(test)]
#[path = "offline_routing_forward_tests.rs"]
mod tests;
impl Drop for CancelOnDrop {
    fn drop(&mut self) { if self.armed { self.control.cancel(); } }
}

pub(super) fn launch<T, F>(prepared: Box<Prepared>, payload: Value, valid: bool,
    control: Arc<Control>, operation: F) -> InvocationRun<T>
where T: owned::Delivered + Send + 'static,
    F: FnOnce(Box<dyn OwnedInvocation>, Value) -> InvocationRun<T> + Send + 'static,
{
    let ownership = prepared.shared.state.operation_ownership.clone();
    let (sender, receiver) = oneshot::channel();
    let task = framer_server::operation_ownership::spawn(ownership.as_ref(),
        framer_core::operation_gate::OperationKind::Inference,
        owned::run(*prepared, payload, valid, control.clone(), operation, sender));
    let cleanup = match task {
        Ok(task) => async move { task.await.unwrap_or(Err(FailureKind::Cleanup)) }.boxed(),
        Err(_) => {
            control.fail(FailureKind::Ineligible);
            async { Ok(()) }.boxed()
        }
    }.shared();
    result(receiver, cleanup, control)
}

fn result<T: Send + 'static>(receiver: oneshot::Receiver<Result<T, DispatchError>>,
    cleanup: port::Cleanup, control: Arc<Control>) -> InvocationRun<T> {
    let snapshot = control.clone();
    let completed_attempts: port::AttemptProgress = Arc::new(move || {
        let inner = snapshot.progress.lock().unwrap_or_else(|e| e.into_inner()).clone();
        inner.map(|get| get()).unwrap_or_default()
    });
    let guard = CancelOnDrop { control: control.clone(), armed: true };
    let result = async move {
        let mut guard = guard;
        let value = receiver.await.unwrap_or_else(|_| {
            Err(DispatchError::new(guard.control.failure()))
        });
        guard.armed = false;
        value
    }.boxed();
    InvocationRun { result, cleanup, cancellation: control,
        completed_attempts: Some(completed_attempts) }
}

//! Own Rust transport tasks without attesting remote or profile cleanup.
use std::future::Future;
use std::sync::Arc;
use tokio::sync::{watch, Mutex};
use tokio::task::JoinSet;

const TASK_LIMIT: usize = 1024;

#[cfg(test)]
#[path = "sidecar_task_tests.rs"]
mod tests;

pub(super) struct TransportTasks {
    state: Mutex<State>,
    stop: watch::Sender<bool>,
}

struct State {
    sealed: bool,
    failed: bool,
    launching: bool,
    started: u64,
    joined: u64,
    tasks: JoinSet<()>,
}

impl TransportTasks {
    pub(super) fn new() -> Arc<Self> {
        let (stop, _) = watch::channel(false);
        Arc::new(Self { stop, state: Mutex::new(State {
            sealed: false, failed: false, launching: false,
            started: 0, joined: 0, tasks: JoinSet::new(),
        }) })
    }

    pub(super) fn stopping(&self) -> watch::Receiver<bool> {
        self.stop.subscribe()
    }

    pub(super) async fn spawn(
        &self, task: impl Future<Output = ()> + Send + 'static,
    ) -> Result<(), String> {
        let mut stopped = self.stop.subscribe();
        if *stopped.borrow() { return Err("host task admission closed".into()); }
        let mut state = tokio::select! {
            biased;
            _ = stopped.changed() => return Err("host task admission closed".into()),
            state = self.state.lock() => state,
        };
        state.reap();
        if state.failed { return Err("host task ownership unconfirmed".into()); }
        if state.sealed { return Err("host task admission closed".into()); }
        if state.tasks.len() >= TASK_LIMIT {
            return Err("host task capacity reached".into());
        }
        let next = state.started.checked_add(1).ok_or("host task ids exhausted")?;
        // A panic during spawn leaves this reservation visibly unresolved.
        state.started = next;
        state.launching = true;
        state.tasks.spawn(task);
        state.launching = false;
        Ok(())
    }

    pub(super) async fn seal(&self) {
        let mut state = self.state.lock().await;
        state.sealed = true;
        self.stop.send_replace(true);
    }

    /// Cancellation drops only the waiter; the set retains every live task.
    pub(super) async fn join(&self) -> Result<u64, String> {
        let mut state = self.state.lock().await;
        if !state.sealed { return Err("host tasks must be sealed before join".into()); }
        while let Some(result) = state.tasks.join_next().await {
            state.joined += 1;
            state.failed |= result.is_err();
        }
        state.reap();
        if state.failed { return Err("host task ownership unconfirmed".into()); }
        Ok(state.joined)
    }
}

impl State {
    fn reap(&mut self) {
        while let Some(result) = self.tasks.try_join_next() {
            self.joined += 1;
            self.failed |= result.is_err();
        }
        if self.launching || self.started != self.joined + self.tasks.len() as u64 {
            self.failed = true;
        }
    }
}

pub(super) async fn launch(
    owner: Option<&Arc<TransportTasks>>,
    task: impl Future<Output = ()> + Send + 'static,
) -> Result<(), String> {
    match owner {
        Some(owner) => owner.spawn(task).await,
        None => { tokio::spawn(task); Ok(()) }
    }
}

pub(super) async fn stopped(signal: &mut Option<watch::Receiver<bool>>) {
    let Some(signal) = signal else { return std::future::pending().await; };
    loop {
        if *signal.borrow_and_update() { return; }
        if signal.changed().await.is_err() { return; }
    }
}

use framer_core::AccountId;
use std::sync::{Arc, Mutex};
use tokio::{process::Child, task::JoinHandle};
use crate::cdp_process_group::{self, ProcessGroup};

type Outcome = Result<(), String>;

impl Cleanup {
    async fn join(&self) -> Outcome {
        let mut state = self.state.lock().await;
        let outcome = match &mut *state {
            State::Complete(outcome) => return outcome.clone(),
            State::Running(task) => {
                match tokio::time::timeout(std::time::Duration::from_secs(30), task).await {
                    Err(_) => return Err("browser startup cleanup timed out".into()),
                    Ok(Err(_)) => Err("browser startup cleanup task failed".into()),
                    Ok(Ok(outcome)) => outcome,
                }
            }
        };
        *state = State::Complete(outcome.clone());
        outcome
    }
}

enum State {
    Running(JoinHandle<Outcome>),
    Complete(Outcome),
}

struct Cleanup {
    // A version probe has no account and cannot confirm a browser's exit.
    account: Option<AccountId>,
    state: tokio::sync::Mutex<State>,
}

/// Retains failed-start cleanup and its result across abandoned shutdown calls.
#[derive(Default)]
pub(super) struct StartupChildren(Mutex<Vec<Arc<Cleanup>>>);

pub(super) struct StartingChild<'a> {
    owner: &'a StartupChildren,
    account: Option<AccountId>,
    child: Option<Child>,
    group: Option<Arc<ProcessGroup>>,
    runtime: tokio::runtime::Handle,
}

impl StartupChildren {
    /// The browser must have been spawned in its own process group on Unix.
    pub(super) fn track(&self, account: AccountId, child: Child) -> StartingChild<'_> {
        let group = ProcessGroup::for_child(&child);
        StartingChild {
            owner: self,
            account: Some(account),
            child: Some(child),
            group,
            runtime: tokio::runtime::Handle::current(),
        }
    }

    pub(super) fn track_probe(&self, child: Child) -> StartingChild<'_> {
        StartingChild {
            owner: self,
            account: None,
            child: Some(child),
            group: None,
            runtime: tokio::runtime::Handle::current(),
        }
    }

    fn register(&self, account: Option<AccountId>, task: JoinHandle<Outcome>) {
        let cleanup = Arc::new(Cleanup {
            account,
            state: tokio::sync::Mutex::new(State::Running(task)),
        });
        self.0.lock().unwrap_or_else(|error| error.into_inner()).push(cleanup);
    }

    pub(super) async fn join(&self) -> Vec<(Option<AccountId>, Outcome)> {
        let pending = self.0.lock().unwrap_or_else(|error| error.into_inner()).clone();
        let mut outcomes = Vec::with_capacity(pending.len());
        for cleanup in pending {
            outcomes.push((cleanup.account, cleanup.join().await));
        }
        outcomes
    }
}

async fn pipe_bytes<T: tokio::io::AsyncRead + Unpin>(mut pipe: Option<T>)
    -> std::io::Result<Vec<u8>> {
    use tokio::io::AsyncReadExt;
    let mut bytes = Vec::new();
    if let Some(pipe) = pipe.as_mut() {
        pipe.read_to_end(&mut bytes).await?;
    }
    Ok(bytes)
}

impl StartingChild<'_> {
    pub(super) fn group(&self) -> Option<Arc<ProcessGroup>> { self.group.clone() }

    /// Preserve output semantics while retaining the Child across cancellation.
    pub(super) async fn output(mut self) -> std::io::Result<std::process::Output> {
        let child = self.child.as_mut().expect("startup child already published");
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        let (status, stdout, stderr) = tokio::try_join!(
            child.wait(), pipe_bytes(stdout), pipe_bytes(stderr))?;
        drop(self.child.take());
        Ok(std::process::Output { status, stdout, stderr })
    }

    /// The caller must already hold the instance-publication lock.
    pub(super) fn into_child(mut self) -> Child {
        self.child.take().expect("startup child already published")
    }
}

impl Drop for StartingChild<'_> {
    fn drop(&mut self) {
        let Some(mut child) = self.child.take() else { return; };
        let group = self.group.take();
        if let Some(group) = group.as_ref() { group.request_kill(&child); }
        let _ = child.start_kill();
        let task = self.runtime.spawn(async move {
            // A kill request is not an exit receipt. Keep the Child until wait.
            cdp_process_group::wait_for_exit(&mut child, group.as_deref()).await.map(|_| ())
                .map_err(|error| format!("wait for browser startup child: {error}"))
        });
        self.owner.register(self.account, task);
    }
}

#[cfg(test)]
#[path = "cdp_startup_children_tests.rs"]
mod tests;

//! Local admission closure only. No process or profile cleanup is implied.
use super::SidecarBridge;
use framer_core::operation_gate::{ApplicationDrained, GateError, HostRetirement, OperationGate};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RetirementError {
    Gate(GateError), Closed, Busy, Poisoned, Exhausted, Unsupported,
    ProcessUnconfirmed, TasksUnconfirmed, GenerationMismatch,
}

impl RetirementError {
    pub fn explanation(self) -> &'static str {
        match self {
            Self::Gate(error) => error.explanation(),
            Self::Closed => "the browser host admission gate is closed for shutdown",
            Self::Busy => "active browser host commands have not finished",
            Self::Poisoned => "browser host ownership became uncertain after a state-lock failure",
            Self::Exhausted => "the browser host command counter is exhausted",
            Self::Unsupported => "this host has no registered shutdown owner",
            Self::ProcessUnconfirmed => "browser host startup or process termination did not complete",
            Self::TasksUnconfirmed => "browser host transport tasks did not finish successfully",
            Self::GenerationMismatch => "the browser host generation changed during shutdown",
        }
    }
}

struct State {
    closed: bool,
    active: usize,
    uncertain: bool,
    startup: Startup,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Startup { Unused, Incomplete, Ready }

pub(super) struct AdmissionBoundary {
    gate: OperationGate,
    state: Mutex<State>,
}

pub(super) struct LocalCall {
    boundary: Arc<AdmissionBoundary>,
}

impl Drop for LocalCall {
    fn drop(&mut self) {
        let mut state = match self.boundary.state.lock() {
            Ok(state) => state,
            Err(error) => {
                let mut state = error.into_inner();
                state.uncertain = true;
                state
            }
        };
        match state.active.checked_sub(1) {
            Some(active) => state.active = active,
            None => state.uncertain = true,
        }
    }
}

/// Created together with a new bridge. Existing unmanaged hosts cannot be adopted.
pub struct RetirementControl {
    bridge: Arc<SidecarBridge>,
}

/// Keeps the original bridge alive. Does not attest task or process termination.
#[must_use = "admission closure is not a cleanup proof"]
pub struct SealedHostAdmission {
    bridge: Arc<SidecarBridge>,
    incarnation: Option<uuid::Uuid>,
    authority: HostRetirement,
}

/// Local Rust joins and leader exit only. Descendants and profile writes are
/// deliberately outside this observation; it cannot complete the host barrier.
#[must_use = "local task joins are not remote cleanup"]
pub struct JoinedHostTasks<'owner> {
    owner: &'owner SealedHostAdmission,
    tasks: u64,
    leader_status: Option<std::process::ExitStatus>,
}

impl JoinedHostTasks<'_> {
    pub fn task_count(&self) -> u64 { self.tasks }
    pub fn leader_status(&self) -> Option<std::process::ExitStatus> { self.leader_status }
    pub fn incarnation(&self) -> Option<uuid::Uuid> { self.owner.incarnation }
}

impl SidecarBridge {
    /// Allocate a fresh bridge and bind its future retirement to this exact gate.
    /// Does not start a host, adopt a process, or create a host cgroup.
    pub fn new_retirable(
        state_root: impl Into<PathBuf>, gate: OperationGate,
    ) -> (Arc<Self>, RetirementControl) {
        let mut bridge = Self::new(state_root);
        bridge.retirement = Some(Arc::new(AdmissionBoundary::new(gate)));
        bridge.transport_tasks = Some(super::tasks::TransportTasks::new());
        let bridge = Arc::new(bridge);
        (bridge.clone(), RetirementControl { bridge })
    }
}

impl RetirementControl {
    pub async fn seal_admission(
        &self, proof: &ApplicationDrained<'_>,
    ) -> Result<SealedHostAdmission, RetirementError> {
        // Serialize with the final startup check and process creation.
        let inner = self.bridge.inner.lock().await;
        let boundary = self.bridge.retirement.as_ref()
            .ok_or(RetirementError::Unsupported)?;
        let authority = boundary.close(proof)?;
        Ok(SealedHostAdmission {
            bridge: self.bridge.clone(),
            incarnation: inner.as_ref().map(|host| host.incarnation),
            authority,
        })
    }
}

impl SealedHostAdmission {
    /// Does not signal or kill a process. The keeper must arrange its exit.
    /// A cancelled waiter leaves the child and all task handles owned here.
    pub async fn wait_local_tasks(&self) -> Result<JoinedHostTasks<'_>, RetirementError> {
        let boundary = self.bridge.retirement.as_ref().ok_or(RetirementError::Unsupported)?;
        boundary.gate.check_host_retirement(&self.authority).map_err(RetirementError::Gate)?;
        let status = {
            let mut inner = self.bridge.inner.lock().await;
            if inner.as_ref().map(|host| host.incarnation) != self.incarnation {
                return Err(RetirementError::GenerationMismatch);
            }
            match inner.as_mut() {
                Some(host) => {
                    let status = host.child.wait().await.map_err(|_| RetirementError::ProcessUnconfirmed)?;
                    host.healthy.store(false, std::sync::atomic::Ordering::Release);
                    super::fail_pending(&host.pending, "retired sidecar leader exited");
                    Some(status)
                }
                None => None,
            }
        };
        let tasks = self.bridge.transport_tasks.as_ref().ok_or(RetirementError::Unsupported)?;
        tasks.seal().await;
        let joined = tasks.join().await.map_err(|_| RetirementError::TasksUnconfirmed)?;
        boundary.gate.check_host_retirement(&self.authority).map_err(RetirementError::Gate)?;
        Ok(JoinedHostTasks { owner: self, tasks: joined, leader_status: status })
    }

    /// The process generation seen at closure, if one had been started.
    /// Its presence or absence is not evidence that any process has exited.
    pub fn observed_incarnation(&self) -> Option<uuid::Uuid> {
        self.incarnation
    }

    pub fn matches_bridge(&self, bridge: &Arc<SidecarBridge>) -> bool {
        Arc::ptr_eq(&self.bridge, bridge)
    }
}

#[cfg(test)]
#[path = "sidecar_retirement_tests.rs"]
mod tests;

impl AdmissionBoundary {
    fn startup(&self, expected: Startup, next: Startup) -> Result<(), RetirementError> {
        let mut state = self.state.lock().map_err(|_| RetirementError::Poisoned)?;
        if state.uncertain { return Err(RetirementError::Poisoned); }
        if state.closed { return Err(RetirementError::Closed); }
        if state.startup != expected { return Err(RetirementError::ProcessUnconfirmed); }
        state.startup = next;
        Ok(())
    }

    pub(super) fn begin_start(&self) -> Result<(), RetirementError> {
        self.startup(Startup::Unused, Startup::Incomplete)
    }

    pub(super) fn finish_start(&self) -> Result<(), RetirementError> {
        self.startup(Startup::Incomplete, Startup::Ready)
    }

    pub(super) fn check_ready(&self) -> Result<(), RetirementError> {
        self.startup(Startup::Ready, Startup::Ready)
    }

    pub(super) fn check_unused(&self) -> Result<(), RetirementError> {
        self.startup(Startup::Unused, Startup::Unused)
    }

    pub(super) fn check_open(&self) -> Result<(), RetirementError> {
        let state = self.state.lock().map_err(|_| RetirementError::Poisoned)?;
        if state.uncertain { return Err(RetirementError::Poisoned); }
        if state.closed { return Err(RetirementError::Closed); }
        Ok(())
    }

    pub(super) fn enter(self: &Arc<Self>) -> Result<LocalCall, RetirementError> {
        let mut state = self.state.lock().map_err(|_| RetirementError::Poisoned)?;
        if state.uncertain { return Err(RetirementError::Poisoned); }
        if state.closed { return Err(RetirementError::Closed); }
        state.active = state.active.checked_add(1).ok_or(RetirementError::Exhausted)?;
        Ok(LocalCall { boundary: self.clone() })
    }

    fn close(&self, proof: &ApplicationDrained<'_>) -> Result<HostRetirement, RetirementError> {
        let mut state = self.state.lock().map_err(|_| RetirementError::Poisoned)?;
        if state.uncertain { return Err(RetirementError::Poisoned); }
        if state.closed { return Err(RetirementError::Closed); }
        if state.active != 0 { return Err(RetirementError::Busy); }
        if state.startup == Startup::Incomplete { return Err(RetirementError::ProcessUnconfirmed); }
        let authority = self.gate.begin_host_retirement(proof).map_err(RetirementError::Gate)?;
        state.closed = true;
        Ok(authority)
    }

    fn new(gate: OperationGate) -> Self {
        Self { gate, state: Mutex::new(State {
            closed: false, active: 0, uncertain: false, startup: Startup::Unused,
        }) }
    }
}

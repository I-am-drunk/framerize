//! Explicit private child ownership. No default runtime or source selection.
use super::{BrowserControlMint, BrowserCredentialControl, OfflineMintTarget};
use super::browser_control_wire::{self as wire, BrowserControlBinding, Grant, Reason, Reply};
use crate::account::{client::{AccountScope, PreparationControl, PreparationFailure,
    PreparationResult}, device::DeviceGrant};
use framer_core::port::{CastleToken, Session};
use futures::{future::BoxFuture, FutureExt};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{ffi::OsString, io::Read, path::{Path, PathBuf}, process::Stdio, time::Duration};
use tokio::{io::{AsyncReadExt, BufReader}, process::{Child, ChildStdin, ChildStdout, Command},
    sync::Mutex, time::{timeout, Instant}};

/// Caller-published immutable files. Credentials never belong in arguments.
pub struct BrowserControlLaunch {
    pub python: PathBuf,
    pub python_sha256: String,
    pub script: PathBuf,
    pub script_sha256: String,
    pub arguments: Vec<OsString>,
}

pub struct BrowserControlProcess {
    launch: BrowserControlLaunch,
    binding: BrowserControlBinding,
    state: Mutex<State>,
    prepare_limit: Duration,
    cleanup_limit: Duration,
}

#[derive(PartialEq, Eq)]
enum Phase { Fresh, Uncertain, Ready, Held, Closed }

struct State {
    phase: Phase,
    process: Option<Process>,
    grant: Option<Grant>,
    closed: Option<bool>,
}

struct Process {
    child: Child,
    group: i32,
    input: Option<ChildStdin>,
    output: BufReader<ChildStdout>,
    buffer: Vec<u8>,
}

fn pinned(path: &Path, expected: &str) -> bool {
    use std::os::unix::fs::OpenOptionsExt;
    // Reject special files without blocking before the opened-handle check.
    let Ok(file) = std::fs::OpenOptions::new().read(true)
        .custom_flags(libc::O_NONBLOCK).open(path) else { return false; };
    let Ok(metadata) = file.metadata() else { return false; };
    if !metadata.is_file() || metadata.len() > 64 * 1024 * 1024 { return false; }
    let mut hash = Sha256::new();
    let mut reader = file.take(64 * 1024 * 1024 + 1);
    let mut part = [0u8; 64 * 1024];
    loop {
        match reader.read(&mut part) {
            Ok(0) => break,
            Ok(count) => hash.update(&part[..count]),
            Err(_) => return false,
        }
    }
    format!("{:x}", hash.finalize()) == expected
}

impl BrowserControlProcess {
    /// Inert construction. Only prepare starts the explicitly pinned child.
    pub fn new(launch: BrowserControlLaunch, binding: BrowserControlBinding)
        -> Result<Self, &'static str> {
        let hash = |s: &str| s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit());
        if !binding.valid() || !launch.python.is_absolute() || !launch.script.is_absolute()
            || !hash(&launch.python_sha256) || !hash(&launch.script_sha256) {
            return Err("invalid browser control descriptor");
        }
        Ok(Self { launch, binding, state: Mutex::new(State { phase: Phase::Fresh,
            process: None, grant: None, closed: None }),
            prepare_limit: Duration::from_secs(180), cleanup_limit: Duration::from_secs(30) })
    }

    async fn end_command(&self, state: &mut State, command: serde_json::Value, known: bool) -> bool {
        state.phase = Phase::Uncertain;
        let reply = match state.process.as_mut() {
            Some(process) => timeout(self.cleanup_limit, async {
                process.write(command).await?;
                process.read().await
            }).await.ok().and_then(Result::ok),
            None => None,
        };
        let claim = known && reply.and_then(|reply| closed_reply(reply, &state.grant))
            .is_some_and(|(confirmed, reason)| confirmed && reason == Reason::Finished);
        state.close(claim, self.cleanup_limit).await
    }
}

async fn stopped(control: &PreparationControl) {
    while !control.is_cancelled() { tokio::time::sleep(Duration::from_millis(10)).await; }
}

fn stop_reason(control: &PreparationControl) -> PreparationFailure {
    if Instant::now() >= control.deadline() { PreparationFailure::Deadline }
    else { PreparationFailure::Cancelled }
}

enum MintWait { Reply(Result<Reply, ()>), Stop(PreparationFailure) }

impl BrowserControlProcess {
    async fn mint_reply(&self, state: &mut State, reply: Reply,
        started: std::time::Instant, control: &PreparationControl)
        -> PreparationResult<BrowserControlMint> {
        match reply {
            Reply::Minted { version: 1, grant, token } if Some(&grant) == state.grant.as_ref()
                && !token.is_empty() && token.len() <= 16 * 1024 => {
                if control.is_cancelled() { return self.cancel_mint(state, stop_reason(control)).await; }
                state.phase = Phase::Held;
                PreparationResult::finished(Ok(BrowserControlMint {
                    account: grant.account, user: self.binding.user.clone(),
                    generation: grant.native_generation, host_generation: grant.host_generation,
                    lease: uuid::Uuid::parse_str(&grant.lease).expect("locally constructed grant"),
                    device: grant.device, page_url: self.binding.page_url.clone(),
                    token: CastleToken(token), received_at: started,
                }))
            }
            other => {
                let detail = closed_reply(other, &state.grant);
                let claim = detail.is_some_and(|(known, reason)| known
                    && matches!(reason, Reason::Deadline | Reason::Rejected));
                let closed = state.close(claim, self.cleanup_limit).await;
                if !closed { return PreparationResult::unconfirmed(); }
                let reason = if control.is_cancelled() { stop_reason(control) }
                    else if detail.is_some_and(|(_, r)| r == Reason::Deadline) { PreparationFailure::Deadline }
                    else { PreparationFailure::Rejected };
                PreparationResult::finished(Err(reason))
            }
        }
    }

    async fn cancel_mint(&self, state: &mut State, reason: PreparationFailure)
        -> PreparationResult<BrowserControlMint> {
        let grant = state.grant.clone();
        let process = state.process.as_mut().expect("mint owns its child");
        let reply = timeout(self.cleanup_limit, async {
            let sent = process.write(json!({"version": 1, "op": "cancel", "grant": &grant})).await;
            let first = process.read().await?;
            let last = match first {
                Reply::Minted { version: 1, grant: actual, token }
                    if Some(&actual) == grant.as_ref() && !token.is_empty()
                        && token.len() <= 16 * 1024 && sent.is_ok() => process.read().await?,
                other => other,
            };
            closed_reply(last, &grant).ok_or(())
        }).await;
        let claim = matches!(reply, Ok(Ok((true, Reason::Cancelled | Reason::Deadline | Reason::Rejected))));
        if state.close(claim, self.cleanup_limit).await {
            PreparationResult::finished(Err(reason))
        } else { PreparationResult::unconfirmed() }
    }
}

#[cfg(test)]
#[path = "browser_control_process_tests.rs"]
mod tests;

impl BrowserControlProcess {
    async fn mint_owned(&self, scope: &AccountScope, session: &Session,
        grant: &DeviceGrant, target: &OfflineMintTarget, budget_ms: u64,
        control: &PreparationControl) -> PreparationResult<BrowserControlMint> {
        if !self.binding.matches(scope, session, grant, target) || !(1..=16_000).contains(&budget_ms) {
            return PreparationResult::finished(Err(PreparationFailure::Rejected));
        }
        let Some(key) = Grant::new(scope, grant) else {
            return PreparationResult::finished(Err(PreparationFailure::Rejected));
        };
        let mut state = self.state.lock().await;
        if control.is_cancelled() { return PreparationResult::finished(Err(stop_reason(control))); }
        if state.phase != Phase::Ready {
            return PreparationResult::finished(Err(PreparationFailure::Rejected));
        }
        let remaining = control.deadline().saturating_duration_since(Instant::now()).as_millis();
        if !(1..=180_000).contains(&remaining) {
            return PreparationResult::finished(Err(PreparationFailure::Rejected));
        }
        let budget = budget_ms.min(remaining as u64);
        let deadline = Instant::now() + Duration::from_millis(budget);
        let started = std::time::Instant::now();
        state.phase = Phase::Uncertain;
        state.grant = Some(key.clone());
        let command = json!({"version": 1, "op": "mint", "grant": &key,
            "budget_ms": budget, "request_ms": remaining as u64});
        self.exchange_mint(&mut state, command, deadline, started, control).await
    }

    async fn exchange_mint(&self, state: &mut State, command: serde_json::Value,
        deadline: Instant, started: std::time::Instant, control: &PreparationControl)
        -> PreparationResult<BrowserControlMint> {
        let process = state.process.as_mut().expect("ready owns its child");
        if !matches!(tokio::time::timeout_at(deadline, process.write(command)).await, Ok(Ok(()))) {
            state.close(false, self.cleanup_limit).await;
            return PreparationResult::unconfirmed();
        }
        let outcome = tokio::select! {
            biased;
            _ = stopped(control) => MintWait::Stop(stop_reason(control)),
            reply = tokio::time::timeout_at(deadline, process.read()) => match reply {
                Ok(reply) => MintWait::Reply(reply),
                Err(_) => MintWait::Stop(PreparationFailure::Deadline),
            },
        };
        match outcome {
            MintWait::Stop(reason) => self.cancel_mint(state, reason).await,
            MintWait::Reply(Ok(reply)) => self.mint_reply(state, reply, started, control).await,
            MintWait::Reply(Err(())) => {
                state.close(false, self.cleanup_limit).await;
                PreparationResult::unconfirmed()
            }
        }
    }
}

impl Process {
    fn start(launch: &BrowserControlLaunch) -> Result<Self, ()> {
        if !pinned(&launch.python, &launch.python_sha256)
            || !pinned(&launch.script, &launch.script_sha256) { return Err(()); }
        let mut child = Command::new(&launch.python).arg("-B").arg(&launch.script)
            .args(&launch.arguments).current_dir(launch.script.parent().ok_or(())?)
            .env_clear().env("PATH", "/usr/bin:/bin").env("PYTHONUNBUFFERED", "1")
            .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null())
            .process_group(0).kill_on_drop(true).spawn().map_err(|_| ())?;
        let group = child.id().expect("fresh child has a process id") as i32;
        let input = child.stdin.take().expect("piped child stdin");
        let output = BufReader::new(child.stdout.take().expect("piped child stdout"));
        Ok(Self { child, group, input: Some(input), output, buffer: Vec::new() })
    }

    async fn write(&mut self, value: serde_json::Value) -> Result<(), ()> {
        wire::write(self.input.as_mut().ok_or(())?, &value).await
    }

    async fn read(&mut self) -> Result<Reply, ()> {
        wire::read(&mut self.output, &mut self.buffer).await
    }

    fn kill(&mut self) {
        if self.group > 0 { unsafe { libc::kill(-self.group, libc::SIGKILL); } }
        let _ = self.child.start_kill();
    }

    async fn end(&mut self, claimed: bool, grace: Duration) -> bool {
        self.input.take();
        let natural = claimed && matches!(timeout(grace, async {
            if !self.buffer.is_empty() { return false; }
            let mut byte = [0u8; 1];
            if !matches!(self.output.read(&mut byte).await, Ok(0)) { return false; }
            self.child.wait().await.is_ok_and(|status| status.success())
        }).await, Ok(true));
        let group_gone = unsafe { libc::kill(-self.group, 0) } == -1
            && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH);
        if !natural || !group_gone {
            self.kill();
            let _ = self.child.wait().await;
        }
        self.group = 0;
        natural && group_gone
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        // Drop cannot prove cleanup. Explicit shutdown must join the child.
        if self.group > 0 && self.child.id().is_some() { self.kill(); }
    }
}

impl State {
    async fn close(&mut self, claim: bool, grace: Duration) -> bool {
        self.phase = Phase::Uncertain;
        let closed = match self.process.as_mut() {
            Some(process) => process.end(claim, grace).await,
            None => claim,
        };
        self.process.take();
        self.closed = Some(closed);
        self.phase = Phase::Closed;
        closed
    }
}

fn closed_reply(reply: Reply, grant: &Option<Grant>) -> Option<(bool, Reason)> {
    match reply {
        Reply::Closed { version: 1, grant: actual, cleanup_confirmed, reason }
            if &actual == grant => Some((cleanup_confirmed, reason)),
        _ => None,
    }
}

impl BrowserCredentialControl for BrowserControlProcess {
    fn prepare(&self) -> BoxFuture<'_, bool> {
        async move {
            let mut state = self.state.lock().await;
            if state.phase != Phase::Fresh { return false; }
            state.phase = Phase::Uncertain;
            state.process = Process::start(&self.launch).ok();
            let Some(process) = state.process.as_mut() else { return false; };
            let ready = timeout(self.prepare_limit, async {
                process.write(json!({"version": 1, "op": "prepare", "binding": &self.binding})).await?;
                process.read().await
            }).await;
            if matches!(ready, Ok(Ok(Reply::Ready { version: 1, host_generation,
                incarnation, proxy_revision: _ })) if wire::generation_valid(&host_generation)
                    && incarnation > 0) {
                state.phase = Phase::Ready;
                true
            } else { state.close(false, self.cleanup_limit).await; false }
        }.boxed()
    }

    fn mint<'a>(&'a self, scope: &'a AccountScope, session: &'a Session,
        grant: &'a DeviceGrant, target: &'a OfflineMintTarget, budget_ms: u64,
        control: &'a PreparationControl) -> BoxFuture<'a, PreparationResult<BrowserControlMint>> {
        self.mint_owned(scope, session, grant, target, budget_ms, control).boxed()
    }

    fn finish<'a>(&'a self, grant: &'a DeviceGrant, joined: bool, risk_refused: bool)
        -> BoxFuture<'a, bool> {
        async move {
            let mut state = self.state.lock().await;
            if !state.grant.as_ref().is_some_and(|key| key.matches(grant)) { return false; }
            if state.phase == Phase::Closed { return joined && state.closed == Some(true); }
            if state.phase != Phase::Held { return state.close(false, self.cleanup_limit).await; }
            let command = json!({"version": 1, "op": "finish", "grant": &state.grant,
                "joined": joined, "risk_refused": risk_refused});
            self.end_command(&mut state, command, joined).await
        }.boxed()
    }

    fn shutdown(&self) -> BoxFuture<'_, bool> {
        async move {
            let mut state = self.state.lock().await;
            match state.phase {
                Phase::Closed => state.closed == Some(true),
                Phase::Fresh => state.close(true, self.cleanup_limit).await,
                Phase::Ready => self.end_command(&mut state,
                    json!({"version": 1, "op": "shutdown"}), true).await,
                Phase::Held => {
                    let command = json!({"version": 1, "op": "finish", "grant": &state.grant,
                        "joined": false, "risk_refused": false});
                    self.end_command(&mut state, command, false).await
                }
                Phase::Uncertain => state.close(false, self.cleanup_limit).await,
            }
        }.boxed()
    }
}

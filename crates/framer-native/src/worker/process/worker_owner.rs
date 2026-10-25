//! Inert configuration, then one owned SDK process. No HTTP or account lookup.
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use std::time::Instant;
use std::os::fd::AsRawFd;
use std::os::unix::fs::OpenOptionsExt;
use std::io::Read;
use std::process::Stdio;
use std::sync::atomic::AtomicU32;
use std::sync::atomic::Ordering;
use futures::future::BoxFuture;
use futures::future::Shared;
use futures::FutureExt;
use tokio::process::Command;
use tokio::sync::oneshot;
use tokio::sync::watch;
use sha2::Digest;
use sha2::Sha256;

#[path = "worker_io.rs"] mod pipe;
#[path = "worker_sandbox.rs"] mod sandbox;
#[path = "worker_metadata.rs"] mod metadata;
#[path = "worker_failure.rs"] mod failure;
pub use failure::WorkerStage;
#[path = "worker_v2_metadata.rs"] pub mod v2_metadata;
#[path = "worker_v2.rs"] mod v2;
pub use v2::V2Worker;
pub use v2::V2Metadata;
pub use v2::V2WorkerStage;
#[path = "worker_device.rs"] mod device;
pub use metadata::Metadata;
pub use device::SavedDevice;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Failure { Configuration, Runtime, Deadline, Cancelled, Input, Identity, Spawn, Io, Worker,
    WorkerStage(WorkerStage), V2WorkerStage(V2WorkerStage), Protocol }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cleanup { NotStarted, Joined { pid: u32 }, Unconfirmed }

pub struct FilePin { pub path: PathBuf, pub sha256: [u8; 32] }
pub struct Specification { pub node: PathBuf, pub entry: PathBuf, pub pins: Vec<FilePin> }
pub struct Target { pub project_id: String, pub path: String, pub page_url: String }
pub struct Worker {
    specification: Arc<Specification>,
    #[cfg(test)]
    first_read_delay: Option<Duration>,
}

pub struct Minted<M = Metadata> {
    token: String,
    /// Conservative lower bound on SDK generation, captured before spawn.
    pub freshness_origin: Instant,
    /// Read-time diagnostic only. Buffered bytes can be older than this time.
    pub received_at: Instant,
    pub metadata: M,
}
impl<M> Minted<M> {
    pub fn into_token(self) -> String { self.token }
}

#[derive(Clone)]
pub struct Completion(Shared<BoxFuture<'static, Cleanup>>);
impl Completion { pub async fn wait(self) -> Cleanup { self.0.await } }

pub struct Run<M = Metadata> {
    deadline: Instant,
    pid: Arc<AtomicU32>,
    result: Option<oneshot::Receiver<Result<Minted<M>, Failure>>>,
    completion: Completion,
    cancel: watch::Sender<bool>,
}
struct CancelOnDrop(Option<watch::Sender<bool>>);
impl Drop for CancelOnDrop {
    fn drop(&mut self) { if let Some(sender) = &self.0 { sender.send_replace(true); } }
}
impl<M> Drop for Run<M> { fn drop(&mut self) { self.cancel(); } }
impl<M> Run<M> {
    /// Diagnostic only. Ownership stays with the private Child handle.
    pub fn started_pid(&self) -> Option<u32> {
        match self.pid.load(Ordering::Acquire) { 0 => None, pid => Some(pid) }
    }
    pub fn cancel(&self) { self.cancel.send_replace(true); }
    pub fn completion(&self) -> Completion { self.completion.clone() }
    pub async fn result(&mut self) -> Result<Minted<M>, Failure> {
        let receive = self.result.take().ok_or(Failure::Runtime)?;
        let mut guard = CancelOnDrop(Some(self.cancel.clone()));
        let result = receive.await.unwrap_or(Err(Failure::Worker));
        let cleanup = self.completion.clone().wait().await;
        guard.0 = None;
        if *self.cancel.borrow() { return Err(Failure::Cancelled); }
        if Instant::now() >= self.deadline { return Err(Failure::Deadline); }
        if result.is_ok() && !matches!(cleanup, Cleanup::Joined { .. }) {
            return Err(Failure::Worker);
        }
        result
    }
}

impl Worker {
    pub fn new(specification: Specification) -> Result<Self, Failure> {
        let s = &specification;
        if !s.node.is_absolute() || !s.entry.is_absolute() || s.pins.len() < 2
            || s.pins.len() > 64 || s.pins.iter().any(|pin| !pin.path.is_absolute())
            || !s.pins.iter().any(|pin| pin.path == s.node)
            || !s.pins.iter().any(|pin| pin.path == s.entry) { return Err(Failure::Configuration); }
        let mut paths = std::collections::BTreeSet::new();
        if s.pins.iter().any(|pin| !paths.insert(&pin.path)) { return Err(Failure::Configuration); }
        Ok(Self { specification: Arc::new(specification),
            #[cfg(test)] first_read_delay: None })
    }

    pub fn begin(&self, target: Target, deadline: Instant) -> Result<Run, Failure> {
        self.begin_input(target, None, deadline)
    }

    pub fn begin_with_device(&self, target: Target, device: &SavedDevice,
        deadline: Instant) -> Result<Run, Failure> {
        self.begin_input(target, Some(device), deadline)
    }

    fn begin_input(&self, target: Target, device: Option<&SavedDevice>,
        deadline: Instant) -> Result<Run, Failure> {
        self.begin_protocol(target, device, deadline, metadata::accept_mode, failure::from_exit)
    }

    fn begin_protocol<M: Send + 'static>(&self, target: Target,
        device: Option<&SavedDevice>, deadline: Instant,
        accept: fn(&[u8], u64, bool) -> Option<M>,
        failed: fn(Option<i32>, &[u8]) -> Failure) -> Result<Run<M>, Failure> {
        let now = Instant::now();
        let deadline = deadline.min(now + Duration::from_secs(16));
        let budget_ms = deadline.saturating_duration_since(now).as_millis() as u64;
        if budget_ms == 0 { return Err(Failure::Deadline); }
        let input = device::encode(target, device, budget_ms)?;
        let device_required = device.is_some();
        let runtime = tokio::runtime::Handle::try_current().map_err(|_| Failure::Runtime)?;
        let (cancel, receiver) = watch::channel(false);
        let (sender, result) = oneshot::channel();
        let spec = self.specification.clone();
        let pid = Arc::new(AtomicU32::new(0));
        #[cfg(test)]
        let first_read_delay = self.first_read_delay;
        #[cfg(not(test))]
        let first_read_delay = None;
        let task = runtime.spawn(supervise(spec, input, budget_ms, deadline,
            receiver, sender, pid.clone(), first_read_delay, device_required, accept, failed));
        let completion = Completion(async move {
            task.await.unwrap_or(Cleanup::Unconfirmed)
        }.boxed().shared());
        Ok(Run { result: Some(result), completion, cancel, deadline, pid })
    }
}

async fn cancelled(receiver: &mut watch::Receiver<bool>) {
    loop {
        if *receiver.borrow_and_update() { return; }
        if receiver.changed().await.is_err() { return; }
    }
}

async fn verify(specification: &Specification) -> Result<(), Failure> {
    let mut buffer = [0u8; 32768];
    for pin in &specification.pins {
        // Regular, bounded reads stay in this task. Cancelling verification
        // cannot leave a tokio::fs blocking-pool operation running behind it.
        let mut file = std::fs::OpenOptions::new().read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(&pin.path).map_err(|_| Failure::Identity)?;
        let info = file.metadata().map_err(|_| Failure::Identity)?;
        if !info.is_file() || info.len() > 256 * 1024 * 1024 {
            return Err(Failure::Identity);
        }
        let mut hash = Sha256::new();
        let mut length = 0u64;
        loop {
            let count = file.read(&mut buffer).map_err(|_| Failure::Identity)?;
            if count == 0 { break; }
            length += count as u64;
            if length > info.len() { return Err(Failure::Identity); }
            hash.update(&buffer[..count]);
            tokio::task::yield_now().await;
        }
        let actual: [u8; 32] = hash.finalize().into();
        if length != info.len() || actual != pin.sha256 { return Err(Failure::Identity); }
    }
    Ok(())
}

struct Spawned {
    child: tokio::process::Child,
    input: tokio::io::unix::AsyncFd<std::os::fd::OwnedFd>,
    token: tokio::io::unix::AsyncFd<std::os::fd::OwnedFd>,
    metadata: tokio::io::unix::AsyncFd<std::os::fd::OwnedFd>,
}

fn spawn(spec: &Specification) -> std::io::Result<Spawned> {
    let (input_read, input_write) = pipe::pipe()?;
    let (token_read, token_write) = pipe::pipe()?;
    let (metadata_read, metadata_write) = pipe::pipe()?;
    let copies = [pipe::child_copy(&input_read)?, pipe::child_copy(&token_write)?,
        pipe::child_copy(&metadata_write)?];
    let channels = copies.each_ref().map(|fd| fd.as_raw_fd());
    let input = pipe::asynchronous(input_write)?;
    let token = pipe::asynchronous(token_read)?;
    let metadata = pipe::asynchronous(metadata_read)?;
    let mut command = Command::new(&spec.node);
    command.arg("--jitless");
    command.arg("--max-old-space-size=96").arg("--experimental-vm-modules")
        .arg(&spec.entry).env_clear().env("LANG", "C.UTF-8").env("TZ", "UTC")
        .stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null())
        .kill_on_drop(true);
    if let Some(directory) = spec.entry.parent() { command.current_dir(directory); }
    unsafe { command.pre_exec(move || sandbox::prepare(channels)); }
    let child = command.spawn()?;
    // Keep all original pipe ends alive through spawn. They reserve the low
    // descriptors and keep the Command exec-error channel out of FD 3..5.
    drop((copies, input_read, token_write, metadata_write));
    Ok(Spawned { child, input, token, metadata })
}

async fn supervise<M: Send + 'static>(spec: Arc<Specification>, input: Vec<u8>, budget_ms: u64,
    deadline: Instant, mut cancel: watch::Receiver<bool>,
    sender: oneshot::Sender<Result<Minted<M>, Failure>>, started: Arc<AtomicU32>,
    _first_read_delay: Option<Duration>, device_required: bool,
    accept: fn(&[u8], u64, bool) -> Option<M>,
    failed: fn(Option<i32>, &[u8]) -> Failure) -> Cleanup {
    let until = tokio::time::Instant::from_std(deadline);
    let verified = tokio::select! {
        biased;
        _ = cancelled(&mut cancel) => Err(Failure::Cancelled),
        _ = tokio::time::sleep_until(until) => Err(Failure::Deadline),
        result = verify(&spec) => result,
    };
    if let Err(error) = verified { let _ = sender.send(Err(error)); return Cleanup::NotStarted; }
    if *cancel.borrow() || Instant::now() >= deadline {
        let error = if *cancel.borrow() { Failure::Cancelled } else { Failure::Deadline };
        let _ = sender.send(Err(error)); return Cleanup::NotStarted;
    }
    let freshness_origin = Instant::now();
    let Spawned { mut child, input: input_fd, token, metadata } = match spawn(&spec) {
        Ok(value) => value,
        Err(_) => { let _ = sender.send(Err(Failure::Spawn)); return Cleanup::NotStarted; }
    };
    let pid = child.id().unwrap_or(0);
    started.store(pid, Ordering::Release);
    let exchange = async {
        let token = async move {
            #[cfg(test)]
            if let Some(delay) = _first_read_delay {
                hold_first_read(&token, delay).await?;
            }
            pipe::read_with_receipt(token, 16384 + 12).await
        };
        tokio::try_join!(pipe::write_all(input_fd, input), token,
            pipe::read_bounded(metadata, 16384), child.wait())
    };
    let outcome = tokio::select! {
        biased;
        _ = cancelled(&mut cancel) => Err(Failure::Cancelled),
        _ = tokio::time::sleep_until(until) => Err(Failure::Deadline),
        result = exchange => result.map_err(|_| Failure::Io),
    };
    let (result, cleanup) = match outcome {
        Ok(((), (bytes, received_at), safe, status)) => {
            let cleanup = if pid == 0 { Cleanup::Unconfirmed } else { Cleanup::Joined { pid } };
            let result = (|| {
                if *cancel.borrow() { return Err(Failure::Cancelled); }
                if Instant::now() >= deadline { return Err(Failure::Deadline); }
                if pid == 0 { return Err(Failure::Worker); }
                if !status.success() {
                    return Err(failed(status.code(), &safe));
                }
                let metadata = accept(&safe, budget_ms, device_required)
                    .ok_or(Failure::Protocol)?;
                let token = metadata::token(bytes).ok_or(Failure::Protocol)?;
                let received_at = received_at.ok_or(Failure::Protocol)?;
                Ok(Minted { token, freshness_origin, received_at, metadata })
            })();
            (result, cleanup)
        }
        Err(error) => (Err(error), stop_and_join(&mut child, pid).await),
    };
    let _ = sender.send(result);
    cleanup
}

#[cfg(test)]
async fn hold_first_read(fd: &tokio::io::unix::AsyncFd<std::os::fd::OwnedFd>,
    delay: Duration) -> std::io::Result<()> {
    drop(fd.readable().await?);
    let mut pending: libc::c_int = 0;
    let result = unsafe { libc::ioctl(fd.get_ref().as_raw_fd(), libc::FIONREAD, &mut pending) };
    // The existing fake worker writes this complete small frame atomically.
    if result != 0 || pending != 12 + b"fake-fixture-token".len() as libc::c_int {
        return Err(std::io::ErrorKind::InvalidData.into());
    }
    tokio::time::sleep(delay).await;
    Ok(())
}

async fn stop_and_join(child: &mut tokio::process::Child, pid: u32) -> Cleanup {
    if pid == 0 { return Cleanup::Unconfirmed; }
    if matches!(child.try_wait(), Ok(Some(_))) { return Cleanup::Joined { pid }; }
    // A kill request is not a cleanup receipt. Always await the exact child.
    let _ = child.start_kill();
    match tokio::time::timeout(Duration::from_secs(2), child.wait()).await {
        Ok(Ok(_)) => Cleanup::Joined { pid },
        _ => Cleanup::Unconfirmed,
    }
}

#[cfg(test)]
#[path = "worker_tests.rs"]
mod tests;

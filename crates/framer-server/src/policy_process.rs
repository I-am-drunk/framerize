//! Owned child transport. The generation manager supplies admission and routing.
use framer_worker_wire::{frame::{self, Decoded, Outbox}, message::Message, WireError};
use sha2::{Digest, Sha256};
use std::{fs::File, io::Read, path::PathBuf, process::Stdio, sync::Arc};
use tokio::{io::{AsyncRead, AsyncReadExt}, process::Child, sync::{oneshot, watch}, task::JoinHandle};
use std::sync::atomic::{AtomicU64, Ordering};

const MAX_EXECUTABLE_BYTES: u64 = 512 * 1024 * 1024;
const STDERR_RESERVE: usize = 4096;

/// The publisher must keep these inode contents immutable through exec.
#[derive(Clone, Debug)]
pub(crate) struct Executable { pub path: PathBuf, pub sha256: [u8; 32] }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ProcessError {
    Unsupported, Executable, Channel, Join,
    Os { stage: ProcessStage, code: Option<i32> },
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ProcessStage { Open, Inspect, Read, Duplicate, Spawn }
fn os_error(stage: ProcessStage, error: std::io::Error) -> ProcessError {
    ProcessError::Os { stage, code: error.raw_os_error() }
}

#[derive(Clone, Debug)]
pub(crate) struct ProcessExit {
    pub reaped: bool,
    pub channel_failed: bool,
    pub cause: ProcessCause,
    pub stderr_bytes: u64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ChannelEnd { Eof, Io, Protocol, Task }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ProcessCause {
    Stopped, Drained, ChildExited, WaitError, Reader(ChannelEnd), Writer(ChannelEnd),
}

/// Callback implementations must only perform bounded synchronous dispatch.
pub(crate) type Dispatch = Arc<dyn Fn(Decoded<Message>) -> Result<(), WireError> + Send + Sync>;
pub(crate) struct PolicyProcess {
    pub outbox: Outbox,
    control: ProcessControl,
    joined: JoinHandle<ProcessExit>,
    result: Option<Result<ProcessExit, ProcessError>>,
}
#[derive(Clone)]
pub(crate) struct ProcessControl {
    alive: watch::Sender<bool>,
    stop: Arc<std::sync::Mutex<Option<oneshot::Sender<()>>>>,
    drain: Arc<std::sync::Mutex<Option<oneshot::Sender<()>>>>,
}

fn verify(executable: &Executable) -> Result<File, ProcessError> {
    if !executable.path.is_absolute() { return Err(ProcessError::Executable); }
    let mut file = File::open(&executable.path).map_err(|e| os_error(ProcessStage::Open, e))?;
    let metadata = file.metadata().map_err(|e| os_error(ProcessStage::Inspect, e))?;
    if !metadata.is_file() || metadata.len() < 4 || metadata.len() > MAX_EXECUTABLE_BYTES {
        return Err(ProcessError::Executable);
    }
    let mut magic = [0; 4];
    file.read_exact(&mut magic).map_err(|e| os_error(ProcessStage::Read, e))?;
    if &magic != b"\x7fELF" { return Err(ProcessError::Executable); }
    let mut buffer = [0; 65536];
    let mut hash = Sha256::new();
    hash.update(magic);
    let mut length = 4u64;
    loop {
        let count = file.read(&mut buffer).map_err(|e| os_error(ProcessStage::Read, e))?;
        if count == 0 { break; }
        length += count as u64;
        if length > MAX_EXECUTABLE_BYTES { return Err(ProcessError::Executable); }
        hash.update(&buffer[..count]);
    }
    let actual: [u8; 32] = hash.finalize().into();
    if length != metadata.len() || actual != executable.sha256 {
        return Err(ProcessError::Executable);
    }
    above_stdio(file)
}

#[cfg(target_os = "linux")]
fn above_stdio(mut file: File) -> Result<File, ProcessError> {
    use std::os::fd::AsRawFd;
    // Keep low descriptors occupied until a CLOEXEC duplicate is above stderr.
    let mut held = Vec::with_capacity(3);
    while file.as_raw_fd() <= 2 {
        let duplicate = file.try_clone().map_err(|e| os_error(ProcessStage::Duplicate, e))?;
        held.push(file);
        file = duplicate;
    }
    Ok(file)
}
#[cfg(not(target_os = "linux"))]
fn above_stdio(_: File) -> Result<File, ProcessError> { Err(ProcessError::Unsupported) }

#[cfg(target_os = "linux")]
fn command(file: &File) -> std::process::Command {
    use std::os::{fd::AsRawFd, unix::process::CommandExt};
    let mut command = std::process::Command::new(format!("/proc/self/fd/{}", file.as_raw_fd()));
    command.env_clear().current_dir("/").stdin(Stdio::piped())
        .stdout(Stdio::piped()).stderr(Stdio::piped()).process_group(0);
    // After fork: only the close_range syscall and errno capture. CLOEXEC
    // leaves the ELF descriptor usable until exec, unlike closing it here.
    unsafe { command.pre_exec(|| {
        let rc = libc::syscall(libc::SYS_close_range, 3u32, u32::MAX,
            libc::CLOSE_RANGE_CLOEXEC);
        if rc == 0 { Ok(()) } else { Err(std::io::Error::last_os_error()) }
    }); }
    command
}

impl PolicyProcess {
    fn attach(mut child: Child, outbox: Outbox, writer: frame::FrameWriter,
        dispatch: Dispatch) -> Self {
        // Only our piped command reaches here; none of these handles escaped.
        let stdin = child.stdin.take().expect("piped worker stdin");
        let stdout = child.stdout.take().expect("piped worker stdout");
        let stderr = child.stderr.take().expect("piped worker stderr");
        let reader = tokio::spawn(read_frames(stdout, dispatch));
        let writer = tokio::spawn(writer.run(stdin));
        let counts = Arc::new(StderrCount::default());
        let stderr = tokio::spawn(drain_stderr(stderr, counts.clone()));
        let (stop, stopped) = oneshot::channel();
        let (drain, draining) = oneshot::channel();
        let (alive, _) = watch::channel(true);
        let joined = tokio::spawn(monitor(child, reader, writer, stderr,
            counts, outbox.clone(), alive.clone(), stopped, draining));
        let control = ProcessControl { alive,
            stop: Arc::new(std::sync::Mutex::new(Some(stop))),
            drain: Arc::new(std::sync::Mutex::new(Some(drain))) };
        Self { outbox, control, joined, result: None }
    }
    pub fn is_alive(&self) -> bool { self.control.is_alive() }
    pub fn subscribe(&self) -> watch::Receiver<bool> { self.control.alive.subscribe() }
    pub fn control(&self) -> ProcessControl { self.control.clone() }
    pub fn request_stop(&mut self) {
        self.control.request_stop();
    }
    pub fn request_drain(&mut self) {
        self.control.request_drain();
    }
    pub async fn join(&mut self) -> Result<ProcessExit, ProcessError> {
        if let Some(result) = &self.result { return result.clone(); }
        let result = (&mut self.joined).await.map_err(|_| ProcessError::Join);
        self.result = Some(result.clone());
        result
    }
    /// The manager must reserve its Starting slot before calling this method.
    #[cfg(target_os = "linux")]
    pub async fn spawn(executable: Executable, records: usize, dispatch: Dispatch)
        -> Result<Self, ProcessError> {
        let (outbox, writer) = Outbox::new(records).map_err(|_| ProcessError::Channel)?;
        let file = tokio::task::spawn_blocking(move || verify(&executable))
            .await.map_err(|_| ProcessError::Join)??;
        let mut command = tokio::process::Command::from(command(&file));
        command.kill_on_drop(true);
        let child = command.spawn().map_err(|e| os_error(ProcessStage::Spawn, e))?;
        drop(file); // The opened, hashed inode was retained through exec.
        Ok(Self::attach(child, outbox, writer, dispatch))
    }
    #[cfg(not(target_os = "linux"))]
    pub async fn spawn(_: Executable, _: usize, _: Dispatch) -> Result<Self, ProcessError> {
        Err(ProcessError::Unsupported)
    }
}

impl Drop for PolicyProcess {
    fn drop(&mut self) { self.request_stop(); }
}
impl ProcessControl {
    pub fn is_alive(&self) -> bool { *self.alive.borrow() }
    pub fn request_stop(&self) {
        self.alive.send_replace(false);
        if let Some(sender) = self.stop.lock().unwrap_or_else(|e| e.into_inner()).take() {
            let _ = sender.send(());
        }
    }
    pub fn request_drain(&self) {
        if let Some(sender) = self.drain.lock().unwrap_or_else(|e| e.into_inner()).take() {
            let _ = sender.send(());
        }
    }
}

enum End {
    Stop,
    Drained,
    Child(std::io::Result<std::process::ExitStatus>),
    Reader(ChannelEnd),
    Writer(ChannelEnd),
}
fn channel_end(result: Result<Result<(), WireError>, tokio::task::JoinError>) -> ChannelEnd {
    match result {
        Ok(Ok(())) => ChannelEnd::Eof,
        Ok(Err(WireError::Io(_))) => ChannelEnd::Io,
        Ok(Err(_)) => ChannelEnd::Protocol,
        Err(_) => ChannelEnd::Task,
    }
}

async fn monitor(mut child: Child, mut reader: JoinHandle<Result<(), WireError>>,
    mut writer: JoinHandle<Result<(), WireError>>, stderr: JoinHandle<()>,
    counts: Arc<StderrCount>, outbox: Outbox, alive: watch::Sender<bool>,
    mut stopped: oneshot::Receiver<()>, draining: oneshot::Receiver<()>) -> ProcessExit {
    let end = tokio::select! {
        biased;
        _ = &mut stopped => End::Stop,
        _ = draining => drain_writer(&outbox, &mut child, &mut reader,
            &mut writer, &mut stopped).await,
        result = child.wait() => End::Child(result),
        result = &mut reader => End::Reader(channel_end(result)),
        result = &mut writer => End::Writer(channel_end(result)),
    };
    alive.send_replace(false);
    outbox.poison();
    settle(child, reader, writer, stderr, counts, end).await
}

async fn drain_writer(outbox: &Outbox, child: &mut Child,
    reader: &mut JoinHandle<Result<(), WireError>>,
    writer: &mut JoinHandle<Result<(), WireError>>, stopped: &mut oneshot::Receiver<()>) -> End {
    outbox.close();
    tokio::select! {
        biased;
        _ = stopped => End::Stop,
        result = writer => match channel_end(result) {
            ChannelEnd::Eof => End::Drained, cause => End::Writer(cause),
        },
        result = child.wait() => End::Child(result),
        result = reader => End::Reader(channel_end(result)),
    }
}

async fn settle(mut child: Child, reader: JoinHandle<Result<(), WireError>>,
    writer: JoinHandle<Result<(), WireError>>, stderr: JoinHandle<()>,
    counts: Arc<StderrCount>, end: End) -> ProcessExit {
    let reaped = if matches!(&end, End::Child(Ok(_))) { true } else {
        let _ = child.start_kill();
        child.wait().await.is_ok()
    };
    // Cancellation here belongs to a dead/poisoned channel, never one op.
    if !matches!(&end, End::Reader(_)) { reader.abort(); let _ = reader.await; }
    if !matches!(&end, End::Writer(_) | End::Drained) { writer.abort(); let _ = writer.await; }
    stderr.abort();
    let _ = stderr.await;
    let observed = counts.read.load(Ordering::Relaxed);
    let cause = match &end {
        End::Stop => ProcessCause::Stopped,
        End::Drained => ProcessCause::Drained,
        End::Child(Ok(_)) => ProcessCause::ChildExited,
        End::Child(Err(_)) => ProcessCause::WaitError,
        End::Reader(cause) => ProcessCause::Reader(*cause),
        End::Writer(cause) => ProcessCause::Writer(*cause),
    };
    ProcessExit { reaped, channel_failed: !matches!(end, End::Stop | End::Drained),
        cause, stderr_bytes: observed }
}

async fn read_frames<R: AsyncRead + Unpin>(mut reader: R, dispatch: Dispatch)
    -> Result<(), WireError> {
    while let Some(frame) = frame::read::<_, Message>(&mut reader).await? {
        dispatch(frame)?;
    }
    Ok(())
}

#[derive(Default)]
struct StderrCount { read: AtomicU64 }

async fn drain_stderr<R: AsyncRead + Unpin>(mut reader: R, count: Arc<StderrCount>) {
    // Fixed scratch only. No stderr payload is retained, relayed or logged.
    let mut bytes = [0; STDERR_RESERVE];
    loop {
        match reader.read(&mut bytes).await {
            Ok(0) | Err(_) => return,
            Ok(n) => { let _ = count.read.fetch_update(Ordering::Relaxed,
                Ordering::Relaxed, |old| Some(old.saturating_add(n as u64))); }
        }
    }
}

#[cfg(test)]
#[path = "policy_worker_tests.rs"]
mod tests;

#[cfg(test)]
impl PolicyProcess {
    // Manager state-machine fixture only. No child exists; `reaped` is a
    // supplied outcome, not evidence about the production process monitor.
    pub(crate) fn fixture(records: usize) -> (Self, oneshot::Sender<()>) {
        let (outbox, writer) = Outbox::new(records).unwrap();
        let (stop, stopped) = oneshot::channel();
        let (drain, draining) = oneshot::channel();
        let (exit, exited) = oneshot::channel();
        let (alive, _) = watch::channel(true);
        let control = ProcessControl { alive: alive.clone(),
            stop: Arc::new(std::sync::Mutex::new(Some(stop))),
            drain: Arc::new(std::sync::Mutex::new(Some(drain))) };
        let queue = outbox.clone();
        let joined = tokio::spawn(async move {
            let cause = tokio::select! {
                _ = stopped => ProcessCause::Stopped,
                _ = draining => ProcessCause::Drained,
                _ = exited => ProcessCause::ChildExited,
            };
            alive.send_replace(false);
            queue.close();
            let _ = writer.run(tokio::io::sink()).await;
            ProcessExit { reaped: true, channel_failed: cause == ProcessCause::ChildExited,
                cause, stderr_bytes: 0 }
        });
        (Self { outbox, control, joined, result: None }, exit)
    }
}

//! Source-only, future-startup private control. No HTTP route is installed.
//! Wire: one big-endian u32 byte length, then one JSON object.
//! The complete frame, including its four-byte prefix, is at most 96 KiB.
//! Keep both socket directions open until the identically framed facts arrive.
//! EOF, extra bytes or a read error requests cancellation, never settlement.
//! Linux pins the existing 0700 parent through /proc/self/fd; other OSes refuse.
use super::resident_control::{self, Failure, Report, StartupOwner};
use framer_api::{experimental_native::application::AttemptContext,
    native_dispatch::complete_json::{FreeRoute, Plan, Specification}};
use framer_core::{ids::{AccountId, FramerUserId, ProjectId, TeamId},
    native_dispatch::Identity};
use framer_server::AppState;
use futures::{stream::FuturesUnordered, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{io, net::SocketAddr, path::Path, sync::Arc, time::Duration};
use tokio::{io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    sync::watch, time::Instant};

#[cfg(target_os = "linux")]
pub(crate) use platform::BoundSocket;

/// Unregistered supervisor: shutdown waits on its workers, never on itself.
/// Dropping this caller detaches the retained supervisor rather than aborting it.
#[cfg(target_os = "linux")]
pub(crate) async fn serve(state: AppState, owner: Arc<StartupOwner>, bound: BoundSocket,
    web_dir: &str, address: SocketAddr) -> io::Result<()> {
    let web_dir = web_dir.to_owned();
    tokio::spawn(async move { platform::supervise(state, owner, bound, web_dir, address).await })
        .await.map_err(|_| refused())?
}

#[cfg(not(target_os = "linux"))]
pub(crate) struct BoundSocket;
#[cfg(not(target_os = "linux"))]
impl BoundSocket {
    pub(crate) fn bind(_: &Path) -> io::Result<Self> {
        Err(io::Error::new(io::ErrorKind::Unsupported, "private socket requires Linux"))
    }
}
#[cfg(not(target_os = "linux"))]
pub(crate) async fn serve(_: AppState, _: Arc<StartupOwner>, _: BoundSocket,
    _: &str, _: SocketAddr) -> io::Result<()> { Err(refused()) }

const MAX_FRAME: usize = 96 * 1024;
const MAX_PAYLOAD: usize = MAX_FRAME - 4;
const MAX_CONNECTIONS: usize = 4;
const READ_WINDOW: Duration = Duration::from_secs(5);
const WRITE_WINDOW: Duration = Duration::from_secs(5);

#[cfg(all(test, target_os = "linux"))]
#[path = "resident_socket_tests.rs"]
mod tests;

#[cfg(target_os = "linux")]
mod platform {
    use super::*;
    use std::{fs::{self, File, OpenOptions, Metadata},
        os::{fd::AsRawFd, unix::{ffi::OsStrExt, fs::{MetadataExt, OpenOptionsExt, PermissionsExt}}},
        path::{Component, PathBuf}};
    use tokio::net::{UnixListener, UnixStream};

    #[derive(Clone, Copy, PartialEq, Eq)]
    pub(super) struct ObjectId { device: u64, inode: u64 }
    impl ObjectId {
        pub(super) fn of(metadata: &Metadata) -> Self {
            Self { device: metadata.dev(), inode: metadata.ino() }
        }
    }
    pub(super) fn private_parent(metadata: &Metadata, uid: u32) -> bool {
        metadata.is_dir() && metadata.uid() == uid && metadata.mode() & 0o7777 == 0o700
    }
    pub(super) fn same_peer(expected_uid: u32, observed_uid: u32) -> bool { expected_uid == observed_uid }

    pub(crate) struct BoundSocket {
        listener: UnixListener,
        _path: SocketPath,
        uid: u32,
    }
    impl BoundSocket {
        pub(crate) fn bind(socket: &Path) -> io::Result<Self> {
            // No process-wide umask change, mkdir, stale-path removal or retry.
            // Clients use this path, not the shorter /proc alias used to bind.
            // Linux sun_path holds 108 bytes, including the trailing NUL.
            if !socket.is_absolute() || socket.as_os_str().as_bytes().len() > 107
                || socket.components().any(|c|
                matches!(c, Component::ParentDir | Component::CurDir)) { return Err(refused()); }
            let parent = socket.parent().ok_or_else(refused)?;
            let name = socket.file_name().ok_or_else(refused)?;
            if fs::canonicalize(parent).map_err(|_| refused())? != parent { return Err(refused()); }
            // SAFETY: geteuid takes no arguments and does not access Rust memory.
            let uid = unsafe { libc::geteuid() };
            let before = fs::symlink_metadata(parent).map_err(|_| refused())?;
            if !private_parent(&before, uid) { return Err(refused()); }
            let directory = OpenOptions::new().read(true)
                .custom_flags(libc::O_PATH | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
                .open(parent).map_err(|_| refused())?;
            let actual = directory.metadata().map_err(|_| refused())?;
            if !private_parent(&actual, uid) || ObjectId::of(&before) != ObjectId::of(&actual) {
                return Err(refused());
            }
            let pinned = PathBuf::from(format!("/proc/self/fd/{}", directory.as_raw_fd())).join(name);
            match fs::symlink_metadata(&pinned) {
                Err(error) if error.kind() == io::ErrorKind::NotFound => {},
                _ => return Err(refused()),
            }
            Self::bind_pinned(socket, parent, directory, pinned, uid, ObjectId::of(&actual))
        }
    }

    impl BoundSocket {
        fn bind_pinned(socket: &Path, parent: &Path, directory: File, pinned: PathBuf,
            uid: u32, parent_id: ObjectId) -> io::Result<Self> {
            let listener = UnixListener::bind(&pinned).map_err(|_| refused())?;
            let metadata = fs::symlink_metadata(&pinned).map_err(|_| refused())?;
            let id = ObjectId::of(&metadata);
            if !own_socket(&metadata, id) || metadata.uid() != uid { return Err(refused()); }
            let guard = SocketPath { _parent: directory, pinned, id };
            let node = OpenOptions::new().read(true)
                .custom_flags(libc::O_PATH | libc::O_NOFOLLOW | libc::O_CLOEXEC)
                .open(&guard.pinned).map_err(|_| refused())?;
            if !own_socket(&node.metadata().map_err(|_| refused())?, id) { return Err(refused()); }
            // chmod through the pinned node cannot follow a swapped pathname.
            fs::set_permissions(format!("/proc/self/fd/{}", node.as_raw_fd()),
                fs::Permissions::from_mode(0o600)).map_err(|_| refused())?;
            let visible_parent = fs::symlink_metadata(parent).map_err(|_| refused())?;
            let visible = fs::symlink_metadata(socket).map_err(|_| refused())?;
            if fs::canonicalize(parent).map_err(|_| refused())? != parent
                || !private_parent(&visible_parent, uid) || ObjectId::of(&visible_parent) != parent_id
                || !own_socket(&visible, id) || visible.uid() != uid
                || visible.mode() & 0o7777 != 0o600 { return Err(refused()); }
            Ok(Self { listener, _path: guard, uid })
        }
    }

    async fn connection(mut socket: UnixStream, state: AppState, owner: Arc<StartupOwner>,
        mut stop: watch::Receiver<bool>, admitted: Instant) {
        let deadline = admitted + READ_WINDOW;
        if *stop.borrow() { return; }
        let frame = tokio::select! {
            biased;
            _ = stop.changed() => return,
            frame = read_frame(&mut socket, deadline) => frame,
        };
        let parsed = frame.and_then(|bytes| parse(&bytes, Instant::now()));
        if *stop.borrow() || Instant::now() >= deadline { return; }
        let (spec, baseline) = match parsed {
            Ok(value) => value,
            Err(_) => { write_report(&mut socket, &preflight(Failure::Context)).await; return; }
        };
        // A second frame, EOF or error already observable before launch refuses.
        let mut extra = [0];
        if !matches!(socket.try_read(&mut extra), Err(e) if e.kind() == io::ErrorKind::WouldBlock) {
            return;
        }
        let report = match resident_control::launch(state, spec, baseline, owner) {
            Ok(run) => finish_run(&mut socket, run).await,
            Err(failure) => preflight(failure),
        };
        write_report(&mut socket, &report).await;
    }

    pub(super) async fn supervise(state: AppState, owner: Arc<StartupOwner>,
        bound: BoundSocket, web_dir: String, address: SocketAddr) -> io::Result<()> {
        let BoundSocket { listener, _path: path, uid } = bound;
        let (stop, receiver) = watch::channel(false);
        let mut connections = FuturesUnordered::new();
        let http = framer_server::serve(state.clone(), &web_dir, address);
        let signal = shutdown_signal();
        tokio::pin!(http, signal);
        let mut http_finished = false;
        let mut result = loop {
            tokio::select! {
                biased;
                result = &mut signal => break result,
                result = &mut http => {
                    http_finished = true;
                    break result.map_err(|_| refused());
                }
                _ = connections.next(), if !connections.is_empty() => {},
                accepted = listener.accept(), if connections.len() < MAX_CONNECTIONS => {
                    let (socket, _) = match accepted {
                        Ok(value) => value, Err(_) => break Err(refused()),
                    };
                    // SAFETY: geteuid is a memory-independent query.
                    if unsafe { libc::geteuid() } != uid { continue; }
                    if !socket.peer_cred().is_ok_and(|p| same_peer(uid, p.uid())) { continue; }
                    connections.push(connection(socket, state.clone(), owner.clone(),
                        receiver.clone(), Instant::now()));
                }
            }
        };

        stop.send_replace(true);
        drop(listener);
        drop(path);
        // Poll both sides: application drain may be waiting on these workers.
        // Keep normal HTTP bodies flowing until that drain has completed.
        let finishing = async {
            let existing = async { while connections.next().await.is_some() {} };
            let (_, outcome) = tokio::join!(existing, owner.shutdown(&state));
            outcome
        };
        tokio::pin!(finishing);
        let outcome = if http_finished { finishing.await } else {
            tokio::select! {
                outcome = &mut finishing => outcome,
                ended = &mut http => {
                    http_finished = true;
                    if result.is_ok() { result = ended.map_err(|_| refused()); }
                    finishing.await
                }
            }
        };
        if outcome.is_err() {
            // No proof permits returning to main's process exit. This hold
            // requires external owner resolution, not a forced cleanup timeout.
            eprintln!("resident shutdown unconfirmed; retaining application runtime");
            if !http_finished { let _ = http.await; }
            std::future::pending::<()>().await;
        }
        result
    }

    async fn shutdown_signal() -> io::Result<()> {
        use tokio::signal::unix::{signal, SignalKind};
        let mut terminate = signal(SignalKind::terminate()).map_err(|_| refused())?;
        let mut interrupt = signal(SignalKind::interrupt()).map_err(|_| refused())?;
        tokio::select! {
            value = terminate.recv() => value.ok_or_else(refused),
            value = interrupt.recv() => value.ok_or_else(refused),
        }
    }

    struct SocketPath { _parent: File, pinned: PathBuf, id: ObjectId }
    impl Drop for SocketPath {
        fn drop(&mut self) {
            // Check the pinned parent's leaf without following symlinks. The
            // private directory is a trusted same-UID boundary; Linux has no
            // atomic compare-inode-and-unlink operation against a hostile peer.
            if fs::symlink_metadata(&self.pinned).is_ok_and(|m| own_socket(&m, self.id)) {
                let _ = fs::remove_file(&self.pinned);
            }
        }
    }
    pub(super) fn own_socket(metadata: &Metadata, id: ObjectId) -> bool {
        use std::os::unix::fs::FileTypeExt;
        metadata.file_type().is_socket() && ObjectId::of(metadata) == id
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Input {
    account_id: uuid::Uuid, user_id: String, team_id: String, project_id: String,
    conversation_id: String, route: String, feature_category: String,
    feature_name: String, step: String, turn_id: String, baseline_turn_id: String,
    user_prompt: String, timeout_ms: u64, token_limit: u32, request: Value,
}

fn refused() -> io::Error { io::Error::other("private resident control refused") }

fn preflight(failure: Failure) -> Report {
    Report { failure: Some(failure), cleanup_confirmed: true, ..Report::default() }
}

fn join_cleanup(mut report: Report, joined: bool) -> Report {
    // A transport close never upgrades the worker's own settlement evidence.
    if !joined { report.cleanup_confirmed = false; report.failure = Some(Failure::Cleanup); }
    report
}

async fn finish_run<R: AsyncRead + Unpin>(reader: &mut R, mut run: resident_control::Run)
    -> Report {
    let mut extra = [0];
    let result = tokio::select! {
        biased;
        _ = reader.read(&mut extra) => None,
        report = &mut run.result => Some(report),
    };
    let report = match result {
        Some(report) => report,
        None => { run.cancel(); (&mut run.result).await }
    };
    let joined = tokio::select! {
        biased;
        _ = reader.read(&mut extra) => { run.cancel(); run.cleanup.clone().await.is_ok() }
        outcome = run.cleanup.clone() => outcome.is_ok(),
    };
    join_cleanup(report, joined)
}

async fn read_frame<R: AsyncRead + Unpin>(reader: &mut R, deadline: Instant)
    -> io::Result<Vec<u8>> {
    tokio::time::timeout_at(deadline, async {
        let length = reader.read_u32().await.map_err(|_| refused())? as usize;
        if length == 0 || length > MAX_PAYLOAD { return Err(refused()); }
        let mut bytes = vec![0; length];
        reader.read_exact(&mut bytes).await.map_err(|_| refused())?;
        Ok(bytes)
    }).await.map_err(|_| refused())?
}

async fn write_report<W: AsyncWrite + Unpin>(writer: &mut W, report: &Report) {
    let Ok(bytes) = serde_json::to_vec(&facts(report)) else { return; };
    let _ = tokio::time::timeout(WRITE_WINDOW, async {
        writer.write_u32(bytes.len() as u32).await?;
        writer.write_all(&bytes).await?;
        writer.shutdown().await
    }).await;
}

#[derive(Serialize)]
struct Facts {
    failure: Option<&'static str>, baseline_accepted: bool,
    baseline_status: Option<u16>, baseline_body_complete: bool,
    baseline_json_object: bool, native_invoked: bool, status: Option<u16>,
    body_complete: bool, json_object: bool, refusal_policy: Option<&'static str>,
    refusal_basis: Option<&'static str>, cleanup_confirmed: bool, host_fenced: bool,
}

fn failure_name(failure: Failure) -> &'static str {
    match failure {
        Failure::MissingOwner => "missing_owner", Failure::Admission => "admission",
        Failure::Context => "context", Failure::Acquire => "acquire",
        Failure::Bearer => "bearer", Failure::Baseline => "baseline",
        Failure::Mint => "mint", Failure::Native => "native",
        Failure::Cancelled => "cancelled", Failure::Deadline => "deadline",
        Failure::Publication => "publication", Failure::Cleanup => "cleanup",
        Failure::OwnerLost => "owner_lost",
    }
}

fn facts(report: &Report) -> Facts {
    use framer_api::classify::{Refusal, RefusalBasis};
    Facts { failure: report.failure.map(failure_name),
        baseline_accepted: report.baseline_accepted, baseline_status: report.baseline_status,
        baseline_body_complete: report.baseline_body_complete,
        baseline_json_object: report.baseline_json_object, native_invoked: report.native_invoked,
        status: report.status, body_complete: report.body_complete, json_object: report.json_object,
        cleanup_confirmed: report.cleanup_confirmed, host_fenced: report.host_fenced,
        refusal_policy: report.refusal.map(|r| match r.policy {
            Refusal::Castle => "castle", Refusal::Upstream => "upstream",
        }),
        refusal_basis: report.refusal.map(|r| match r.basis {
            RefusalBasis::ProviderMarker => "provider_marker",
            RefusalBasis::FramerSecurityEnvelope => "framer_security_envelope",
            RefusalBasis::FramerMarker => "framer_marker",
            RefusalBasis::ElapsedFallback => "elapsed_fallback",
        }),
    }
}

// No Debug for caller input. Free text is never echoed or logged. This rejects
// structured credentials, including nested fields, not secrets hidden in prose.
fn credential_key(key: &str) -> bool {
    let key: String = key.chars().filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_lowercase()).collect();
    matches!(key.as_str(), "authorization" | "authentication" | "auth" |
        "bearer" | "cookie" | "cookies" | "cookieheader" | "setcookie" |
        "token" | "accesstoken" | "refreshtoken" | "idtoken" | "castletoken" |
        "apikey" | "secret" | "clientsecret" | "password" | "credentials" |
        "headers" | "proxy" | "proxyurl")
}

fn parse(frame: &[u8], now: Instant) -> io::Result<(Specification, String)> {
    if frame.is_empty() || frame.len() > MAX_PAYLOAD { return Err(refused()); }
    let StrictJson(value) = serde_json::from_slice(frame).map_err(|_| refused())?;
    let input: Input = serde_json::from_value(value).map_err(|_| refused())?;
    if input.account_id.is_nil() || !(1..=60_000).contains(&input.timeout_ms)
        || input.baseline_turn_id.is_empty() || input.baseline_turn_id.len() > 256
        || input.baseline_turn_id.chars().any(char::is_control)
        || input.baseline_turn_id == input.turn_id { return Err(refused()); }
    let route = match input.route.as_str() {
        "/ai/v3/responses/" => FreeRoute::Responses,
        "/ai/v3/messages/" => FreeRoute::Messages,
        _ => return Err(refused()),
    };
    let spec = Specification {
        identity: Identity { account: AccountId(input.account_id),
            user: FramerUserId(input.user_id), team: TeamId(input.team_id),
            project: ProjectId(input.project_id), chat_session: input.conversation_id },
        context: AttemptContext { feature_category: input.feature_category,
            feature_name: input.feature_name, step: input.step,
            turn_id: input.turn_id, user_prompt: input.user_prompt },
        route, payload: input.request, token_limit: input.token_limit,
        deadline: now + Duration::from_millis(input.timeout_ms),
    };
    Plan::validate(&spec).map_err(|_| refused())?;
    Ok((spec, input.baseline_turn_id))
}

struct StrictJson(Value);
impl<'de> Deserialize<'de> for StrictJson {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct Visitor;
        impl<'de> serde::de::Visitor<'de> for Visitor {
            type Value = StrictJson;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("bounded credential-free JSON")
            }
            fn visit_bool<E: serde::de::Error>(self, v: bool) -> Result<Self::Value, E> {
                Ok(StrictJson(v.into()))
            }
            fn visit_i64<E: serde::de::Error>(self, v: i64) -> Result<Self::Value, E> {
                Ok(StrictJson(v.into()))
            }
            fn visit_u64<E: serde::de::Error>(self, v: u64) -> Result<Self::Value, E> {
                Ok(StrictJson(v.into()))
            }
            fn visit_f64<E: serde::de::Error>(self, v: f64) -> Result<Self::Value, E> {
                serde_json::Number::from_f64(v).map(|n| StrictJson(Value::Number(n)))
                    .ok_or_else(|| E::custom("nonfinite number"))
            }
            fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<Self::Value, E> {
                Ok(StrictJson(v.into()))
            }
            fn visit_string<E: serde::de::Error>(self, v: String) -> Result<Self::Value, E> {
                Ok(StrictJson(v.into()))
            }
            fn visit_unit<E: serde::de::Error>(self) -> Result<Self::Value, E> {
                Ok(StrictJson(Value::Null))
            }
            fn visit_seq<A: serde::de::SeqAccess<'de>>(self, mut a: A)
                -> Result<Self::Value, A::Error> {
                let mut values = Vec::new();
                while let Some(StrictJson(value)) = a.next_element()? { values.push(value); }
                Ok(StrictJson(Value::Array(values)))
            }
            fn visit_map<A: serde::de::MapAccess<'de>>(self, mut a: A)
                -> Result<Self::Value, A::Error> {
                let mut values = serde_json::Map::new();
                while let Some(key) = a.next_key::<String>()? {
                    if values.contains_key(&key) || credential_key(&key) {
                        return Err(serde::de::Error::custom("refused JSON field"));
                    }
                    let StrictJson(value) = a.next_value()?;
                    values.insert(key, value);
                }
                Ok(StrictJson(Value::Object(values)))
            }
        }
        d.deserialize_any(Visitor)
    }
}

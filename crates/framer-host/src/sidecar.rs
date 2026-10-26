//! SidecarBridge — BrowserBridge over the Camoufox stealth sidecar.
//!
//! Camoufox (stealth Firefox, C++-level spoofing: no CDP artifacts, no
//! "Headless" UA, realistic fonts/navigator/screen/WebGL) is the engine the
//! user's previous program passed Castle with. Headless Chromium builds —
//! even full Chrome with stealth flags — present container tells (SwiftShader
//! WebGL, 800x600 ozone screen, hardwareConcurrency=1) that Castle scores
//! down; verified by fingerprint diff against a known-good environment
//! (2026-10-02).
//!
//! The sidecar is a release-owned Python package speaking NDJSON over stdio. One
//! Camoufox process per account; the profile dir is the account identity
//! (session cookie + Castle __cuid persist across restarts).

use framer_core::ids::AccountId;
use framer_core::port::{BrowserBridge, CastleToken};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use tokio::io::{AsyncBufReadExt, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::{mpsc, oneshot, Mutex};

type PendingReplies =
    Arc<StdMutex<HashMap<u64, oneshot::Sender<Result<Value, String>>>>>;

fn bundled_host(executable: &Path) -> Result<PathBuf, String> {
    let executable = executable.canonicalize().map_err(|error| error.to_string())?;
    let root = executable.parent().and_then(Path::parent)
        .ok_or("cannot resolve application package")?;
    let script = root.join("runtime/host.py");
    for path in [&script, &root.join("runtime/framerize_host/__main__.py"),
        &root.join("runtime/runtime-manifest.json"),
        &root.join("runtime/framerize_host/app.py"), &root.join("runtime/framerize_host/castle_forge.py")] {
        if !path.is_file() {
            return Err("incomplete application package; install the full archive or pass --sidecar-script for development".into());
        }
    }
    script.canonicalize().map_err(|error| error.to_string())
}

#[path = "sidecar_retirement.rs"]
pub mod retirement;
#[path = "sidecar_tasks.rs"]
mod tasks;

struct PendingCall {
    id: u64,
    pending: PendingReplies,
    cancel: mpsc::Sender<u64>,
}

impl Drop for PendingCall {
    fn drop(&mut self) {
        if self.pending.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&self.id).is_some() {
            // A dropped or timed-out caller must release its reply slot and
            // ask the browser host to cancel the abandoned work.
            let _ = self.cancel.try_send(self.id);
        }
    }
}

fn fail_pending(pending: &PendingReplies, message: &str) {
    for (_, tx) in pending.lock().unwrap_or_else(std::sync::PoisonError::into_inner).drain() {
        let _ = tx.send(Err(message.to_owned()));
    }
}


/// In-page AI calls wait out a full generation (window ceiling ~109 s); mints
/// on a cold engine can take 60 s+. 300 s covers both with margin.
// 900s: a browser LAUNCH happens inside whichever command triggers it
// (set_cookie/first ai_post of a daemon boot), and under heavy machine load
// the handshake needs >300s (measured on the iMac under its chromium build:
// 300.8s deaths with the browser still booting). The hung-command guard stays
// — just looser.
const REQ_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(900);

#[path = "sidecar_ai_result.rs"]
mod ai_result;

#[path = "sidecar_browser_lifecycle.rs"]
mod browser_lifecycle;

#[path = "sidecar_proxy_registration.rs"]
mod proxy_registration;
use proxy_registration::ProxyState;

fn take_reply_result(mut reply: Value) -> Result<Value, String> {
    if reply["ok"].as_bool() == Some(true) {
        // Move the owned payload out of its envelope, including nested strings.
        Ok(reply.get_mut("result").map(Value::take).unwrap_or(Value::Null))
    } else {
        Err(reply["error"].as_str().unwrap_or("sidecar error").to_owned())
    }
}

async fn write_command(writer: &mut (impl AsyncWrite + Unpin), fields: Value) -> std::io::Result<()> {
    let line = format!("{fields}\n");
    // Only the encoded line is needed while the pipe applies backpressure.
    drop(fields);
    writer.write_all(line.as_bytes()).await
}

/// Release cancelled work while queued; finish an already-started frame.
async fn write_owned_command<W>(
    stdin: Arc<Mutex<W>>,
    fields: Value,
) -> std::io::Result<()>
where
    W: AsyncWrite + Unpin + Send + 'static,
{
    write_registered_command(stdin, fields, None).await
}

async fn write_registered_command<W>(
    stdin: Arc<Mutex<W>>,
    fields: Value,
    owner: Option<&Arc<tasks::TransportTasks>>,
) -> std::io::Result<()>
where
    W: AsyncWrite + Unpin + Send + 'static,
{
    let (mut completed, received) = oneshot::channel();
    tasks::launch(owner, async move {
        let mut writer = tokio::select! {
            biased;
            _ = completed.closed() => return,
            writer = stdin.lock() => writer,
        };
        if completed.is_closed() {
            return;
        }
        // This task owns the write even if its caller disconnects. Cancelling
        // write_all after a short write would splice the next JSON command
        // into the unfinished line.
        let result = write_command(&mut *writer, fields).await;
        drop(writer);
        let _ = completed.send(result);
    }).await.map_err(|error| std::io::Error::new(std::io::ErrorKind::BrokenPipe, error))?;
    received.await.unwrap_or_else(|_| {
        Err(std::io::Error::new(
            std::io::ErrorKind::BrokenPipe,
            "sidecar writer stopped",
        ))
    })
}

#[cfg(test)]
#[path = "sidecar_memory_tests.rs"]
mod memory_tests;

#[cfg(test)]
#[path = "sidecar_write_tests.rs"]
mod write_tests;

#[cfg(test)]
#[path = "sidecar_cancel_write_tests.rs"]
mod cancel_write_tests;

#[cfg(test)]
#[path = "sidecar_test_support.rs"]
mod test_support;
#[cfg(test)]
#[path = "sidecar_history_tests.rs"]
mod history_tests;

#[cfg(test)]
#[path = "sidecar_lifecycle_tests.rs"]
mod lifecycle_tests;

#[cfg(test)]
#[path = "sidecar_deadline_tests.rs"]
mod deadline_tests;

#[cfg(test)]
#[path = "sidecar_proxy_tests.rs"]
mod proxy_tests;

#[cfg(test)]
#[path = "sidecar_pool_merge_tests.rs"]
mod pool_merge_tests;

#[cfg(test)]
#[path = "sidecar_script_tests.rs"]
mod script_tests;

pub struct SidecarBridge {
    stopping: AtomicBool,
    retirement: Option<Arc<retirement::AdmissionBoundary>>,
    transport_tasks: Option<Arc<tasks::TransportTasks>>,
    state_root: PathBuf,
    script_override: Option<PathBuf>,
    inner: Mutex<Option<Sidecar>>,
    routes: Mutex<HashMap<AccountId, Arc<Mutex<ProxyState>>>>,
    /// Per-account queue wait from the most recent `ai_post`, in ms. The host
    /// reports how long the request waited for a free page; callers subtract it
    /// before estimating upstream tok/s.
    queue_wait: Mutex<HashMap<AccountId, u64>>,
}

#[cfg(test)]
#[path = "sidecar_shutdown_tests.rs"]
mod shutdown_tests;

struct Sidecar {
    healthy: Arc<std::sync::atomic::AtomicBool>,
    incarnation: uuid::Uuid,
    child: Child,
    stdin: Arc<Mutex<tokio::process::ChildStdin>>,
    pending: PendingReplies,
    cancel: mpsc::Sender<u64>,
    next_id: Arc<AtomicU64>,
}

impl SidecarBridge {
    pub fn new(state_root: impl Into<PathBuf>) -> Self {
        Self {
            stopping: AtomicBool::new(false),
            retirement: None,
            transport_tasks: None,
            state_root: state_root.into(),
            script_override: None,
            inner: Mutex::new(None),
            routes: Mutex::new(HashMap::new()),
            queue_wait: Mutex::new(HashMap::new()),
        }
    }

    /// Select an instance's prepared host without using the shared home override.
    /// This inspects source metadata only and never starts or rewrites the host.
    pub fn with_script(mut self, script: impl Into<PathBuf>) -> Result<Self, String> {
        let script = script.into();
        if !script.is_absolute()
            || script.canonicalize().map_err(|_| "cannot resolve sidecar script")? != script
        {
            return Err("sidecar script must be an absolute canonical path".into());
        }
        let metadata = script.metadata().map_err(|_| "cannot inspect sidecar script")?;
        if !metadata.is_file() || metadata.len() == 0 || metadata.len() > 4 * 1024 * 1024 {
            return Err("sidecar script must be a bounded regular file".into());
        }
        self.script_override = Some(script);
        Ok(self)
    }

    fn admit_unscoped(&self) -> Result<Option<retirement::LocalCall>, String> {
        self.retirement.as_ref().map(|boundary| boundary.enter())
            .transpose().map_err(|error| format!("host admission: {error:?}"))
    }

    // Call only while holding inner. A missing Child after failed startup is
    // not proof that no process or browser work was started.
    fn check_unused_startup(&self) -> Result<(), String> {
        self.retirement.as_ref().map(|boundary| boundary.check_unused())
            .transpose().map(|_| ())
            .map_err(|error| format!("host startup: {error:?}"))
    }

    fn script_path(&self) -> Result<PathBuf, String> {
        if let Some(script) = &self.script_override { return Ok(script.clone()); }
        bundled_host(&std::env::current_exe().map_err(|error| error.to_string())?)
    }

    /// Start the first host only. A lost host cannot certify that its browser
    /// descendants stopped or transfer its cooldown ledger to a new process.
    async fn sidecar(&self) -> Result<(), String> {
        let mut guard = self.inner.lock().await;
        if self.stopping.load(Ordering::Acquire) {
            return Err("browser host is shutting down".into());
        }
        if let Some(boundary) = &self.retirement {
            boundary.check_open().map_err(|error| format!("host admission: {error:?}"))?;
        }
        if let Some(s) = guard.as_mut() {
            let ready = self.retirement.as_ref().map_or(true, |b| b.check_ready().is_ok());
            match (ready && s.healthy.load(Ordering::Acquire), s.child.try_wait()) {
                (true, Ok(None)) => return Ok(()),
                _ => {
                    s.healthy.store(false, Ordering::Release);
                    fail_pending(&s.pending, "host-lifecycle-unconfirmed: sidecar exited");
                    return Err("host-lifecycle-unconfirmed: sidecar exited; cleanup must be confirmed before recovery".into());
                }
            }
        }
        if let Some(boundary) = &self.retirement {
            boundary.begin_start().map_err(|error| format!("host startup: {error:?}"))?;
        }
        let script = self.script_path()?;

        // Python resolution: explicit override, then the installer's venv,
        // then PATH. The venv is where install-camoufox.sh puts everything.
        let python = std::env::var("FRAMERIZE_PYTHON").unwrap_or_else(|_| {
            let venv = dirs_home().join(".framerize/venv/bin/python3");
            if venv.exists() { venv.to_string_lossy().into_owned() } else { "python3".into() }
        });
        let incarnation = uuid::Uuid::new_v4();
        let healthy = Arc::new(std::sync::atomic::AtomicBool::new(true));
        let mut command = Command::new(&python);
        // The daemon handles terminal signals and drains before closing us.
        #[cfg(unix)]
        command.process_group(0);
        command.arg("-B").arg(&script)
            .env("FRAMERIZE_SIDECAR_INCARNATION", incarnation.to_string())
            .env("FRAMERIZE_STATE", self.state_root.join("camoufox"))
            // Structured sidecar events land in the SAME NDJSON stream as the
            // daemon's (docs/logging.md).
            .env(
                "FRAMERIZE_LOG_DIR",
                self.state_root
                    .parent()
                    .map(|p| p.join(".logs"))
                    .unwrap_or_else(|| self.state_root.join(".logs")),
            )
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit()); // sidecar traces land in the daemon log
        let mut child = command.spawn()
            .map_err(|e| {
                format!(
                    "spawn camoufox sidecar: {e} — install it first: \
                     scripts/linux/install-camoufox.sh"
                )
            })?;
        let stdin = Arc::new(Mutex::new(child.stdin.take().ok_or("sidecar stdin")?));
        let stdout = child.stdout.take().ok_or("sidecar stdout")?;
        let pending: PendingReplies = Arc::new(StdMutex::new(HashMap::new()));
        let (cancel, mut cancellations) = mpsc::channel::<u64>(512);
        // Own the generation before registering transport tasks can yield.
        *guard = Some(Sidecar {
            child, stdin: stdin.clone(), pending: pending.clone(),
            cancel: cancel.clone(), incarnation, healthy: healthy.clone(),
            next_id: Arc::new(AtomicU64::new(0)),
        });
        {
            let stdin = stdin.clone();
            let owner = self.transport_tasks.as_ref().map(Arc::downgrade);
            let mut stop = self.transport_tasks.as_ref().map(|tasks| tasks.stopping());
            tasks::launch(self.transport_tasks.as_ref(), async move {
                loop {
                    let id = tokio::select! {
                        biased;
                        _ = tasks::stopped(&mut stop) => break,
                        id = cancellations.recv() => match id { Some(id) => id, None => break },
                    };
                    let retained = owner.as_ref().and_then(std::sync::Weak::upgrade);
                    if owner.is_some() && retained.is_none() { break; }
                    let write = write_registered_command(
                        stdin.clone(), json!({"cmd": "cancel", "request_id": id}), retained.as_ref(),
                    );
                    // Stop enqueueing after a stall. A started frame still
                    // finishes under its independent writer ownership.
                    if !matches!(
                        tokio::time::timeout(std::time::Duration::from_secs(2), write).await,
                        Ok(Ok(()))
                    ) {
                        break;
                    }
                }
            }).await?;
        }
        // Reader task: route response lines to their oneshot by id.
        {
            let pending = pending.clone();
            let healthy = healthy.clone();
            let mut stop = self.transport_tasks.as_ref().map(|tasks| tasks.stopping());
            tasks::launch(self.transport_tasks.as_ref(), async move {
                let mut lines = BufReader::new(stdout).lines();
                loop {
                    let line = tokio::select! {
                        biased;
                        _ = tasks::stopped(&mut stop) => break,
                        line = lines.next_line() => match line {
                            Ok(Some(line)) => line,
                            _ => break,
                        },
                    };
                    let v = match serde_json::from_str::<Value>(&line) {
                        Ok(v) => v,
                        Err(_) => continue,
                    };
                    let Some(id) = v["id"].as_u64() else {
                        continue;
                    };
                    let tx = pending.lock().unwrap().remove(&id);
                    if let Some(tx) = tx {
                        let _ = tx.send(take_reply_result(v));
                    }
                }
                healthy.store(false, Ordering::Release);
                fail_pending(&pending, "host-lifecycle-unconfirmed: sidecar output closed");
            }).await?;
        }
        if let Some(boundary) = &self.retirement {
            boundary.finish_start().map_err(|error| format!("host startup: {error:?}"))?;
        }
        Ok(())
    }

    async fn call(&self, cmd: &str, fields: Value) -> Result<Value, String> {
        self.call_with_timeout(cmd, fields, REQ_TIMEOUT).await
    }

    async fn call_with_timeout(
        &self,
        cmd: &str,
        fields: Value,
        timeout: std::time::Duration,
    ) -> Result<Value, String> {
        self.call_with_policy(cmd, fields, timeout, true).await
    }

    async fn call_with_policy(
        &self,
        cmd: &str,
        fields: Value,
        timeout: std::time::Duration,
        start_missing: bool,
    ) -> Result<Value, String> {
        self.call_controlled(cmd, fields, timeout, start_missing, None).await
    }

    /// Experimental supplied-token host. Cancellation retains the real reply.
    pub async fn prepare_supplied(&self) -> Result<Value, String> {
        self.call_with_timeout("prepare_supplied", json!({}), std::time::Duration::from_secs(60)).await
    }

    /// Only the separate experimental host implements this operation.
    pub async fn post_supplied(&self, fields: Value, timeout: std::time::Duration,
        cancelled: &(dyn Fn() -> bool + Send + Sync)) -> Result<Value, String> {
        self.call_controlled("send_supplied", fields, timeout, true, Some(cancelled)).await
    }

    async fn call_controlled(&self, cmd: &str, mut fields: Value,
        timeout: std::time::Duration, start_missing: bool,
        cancelled: Option<&(dyn Fn() -> bool + Send + Sync)>) -> Result<Value, String> {
        let _admission = self.admit_unscoped()?;
        let started = std::time::Instant::now();
        // Apply one deadline to setup, queueing, writing and the reply.
        // A started write retains ownership when this caller times out.
        let request = async {
            if start_missing {
                self.sidecar().await?;
            }
            let mut guard = self.inner.lock().await;
            if self.stopping.load(Ordering::Acquire) {
                return Err("browser host is shutting down".into());
            }
            let s = guard.as_mut().ok_or("sidecar unavailable")?;
            if !s.healthy.load(Ordering::Acquire) || !matches!(s.child.try_wait(), Ok(None)) {
                return Err("host-lifecycle-unconfirmed: sidecar exited; browser cleanup is unconfirmed".into());
            }
            if let Some(boundary) = &self.retirement {
                boundary.check_ready().map_err(|error| format!("host startup: {error:?}"))?;
            }
            let id = s.next_id.fetch_add(1, Ordering::Relaxed) + 1;
            fields["id"] = json!(id);
            fields["cmd"] = json!(cmd);
            let (tx, mut rx) = oneshot::channel();
            s.pending.lock().unwrap().insert(id, tx);
            let _pending_call = PendingCall {
                id,
                pending: s.pending.clone(),
                cancel: s.cancel.clone(),
            };
            let stdin = s.stdin.clone();
            // A full command pipe must not hold the process lifecycle lock.
            // The cloned pipe belongs to this generation, and its own mutex
            // still keeps complete command frames in order.
            drop(guard);
            write_registered_command(stdin.clone(), fields, self.transport_tasks.as_ref())
                .await
                .map_err(|e| format!("sidecar write: {e}"))?;
            if let Some(cancelled) = cancelled {
                while !cancelled() {
                    tokio::select! {
                        reply = &mut rx => return reply.map_err(|_| "sidecar dropped the request".to_owned())?,
                        _ = tokio::time::sleep(std::time::Duration::from_millis(10)) => {},
                    }
                }
                write_registered_command(stdin, json!({"cmd": "cancel", "request_id": id}),
                    self.transport_tasks.as_ref()).await.map_err(|_| "sidecar cancellation write failed")?;
            }
            rx.await.map_err(|_| "sidecar dropped the request".to_owned())?
        };
        match tokio::time::timeout(timeout, request).await {
            Ok(result) => result,
            Err(_) => Err(format!("sidecar timeout after {:?} (cmd {cmd})", started.elapsed())),
        }
    }

    async fn ai_post_result(&self, account: &AccountId, value: Value)
        -> Result<(u16, Option<String>, String), String> {
        let ai_result::OwnedAiPostResult { status, usage, text, queue_wait_ms } =
            ai_result::take_ai_post_result(value);
        self.queue_wait.lock().await.insert(*account, queue_wait_ms);
        if status == 0 {
            if text.contains("velocity-paused") {
                return Ok((0, usage, text));
            }
            return Err("castle-v3-unavailable".into());
        }
        if status == -1 {
            return Err(format!("in-page fetch failed: {text}"));
        }
        Ok((status as u16, usage, text))
    }

    async fn call_account(&self, account: &AccountId, cmd: &str, extra: Value) -> Result<Value, String> {
        // Own route selection too: it may start the host before the command.
        let _admission = self.admit_unscoped()?;
        tokio::time::timeout(REQ_TIMEOUT, async {
            let route = self.route_for_command(account).await?;
            let mut fields = extra;
            fields["account"] = json!(account.0.to_string());
            fields["proxy_revision"] = json!(route.revision);
            if matches!(cmd, "begin" | "complete" | "mint" | "ai_post" | "set_cookie" | "eval") {
                // Null explicitly selects the host's global/default egress.
                fields["proxy"] = json!(route.proxy);
            }
            self.call(cmd, fields).await
        }).await.map_err(|_| format!("sidecar account command timed out (cmd {cmd})"))?
    }
}

#[async_trait::async_trait]
impl BrowserBridge for SidecarBridge {
    async fn check_account_history(&self, accounts: &[AccountId]) -> Result<(), String> {
        let accounts: Vec<_> = accounts.iter().map(|account| account.0.to_string()).collect();
        let value = self.call_with_timeout("history_status", json!({"accounts": accounts}),
            std::time::Duration::from_secs(5)).await?;
        if value == json!({"ready": true, "accounts": accounts.len()}) {
            Ok(())
        } else {
            Err("invalid account history readiness reply".into())
        }
    }

    async fn shutdown(&self) -> Result<(), String> {
        let mut guard = self.inner.lock().await;
        self.stopping.store(true, Ordering::Release);
        let Some(host) = guard.as_mut() else { return Ok(()); };
        if !host.pending.lock().map_err(|_| "host replies poisoned")?.is_empty() {
            return Err("host still has active requests".into());
        }
        write_registered_command(host.stdin.clone(), json!({"cmd": "shutdown"}),
                                 self.transport_tasks.as_ref())
            .await.map_err(|error| format!("host shutdown request: {error}"))?;
        let status = host.child.wait().await.map_err(|error| error.to_string())?;
        if !status.success() {
            return Err(format!("browser host shutdown failed: {status}"));
        }
        *guard = None;
        Ok(())
    }

    fn supports_browser_lifecycle(&self) -> bool { true }

    async fn browser_status(&self, account: &AccountId)
        -> Result<framer_core::browser_lifecycle::BrowserAccountStatus, String> {
        self.inspect_browser(account).await
    }

    async fn close_idle_browser(&self, account: &AccountId,
        expected: &framer_core::browser_lifecycle::BrowserAccountStatus)
        -> Result<framer_core::browser_lifecycle::IdleCloseResult, String> {
        self.close_observed_browser(account, expected).await
    }

    async fn castle_token(&self, account: &AccountId) -> Result<CastleToken, String> {
        let v = self.call_account(account, "mint", json!({})).await?;
        v.as_str()
            .map(|s| CastleToken(s.to_string()))
            .ok_or_else(|| "sidecar mint: bad result".into())
    }

    async fn auth_begin(&self, account: &AccountId, email: &str) -> Result<(), String> {
        self.call_account(account, "begin", json!({ "email": email })).await?;
        Ok(())
    }

    async fn auth_complete(
        &self,
        account: &AccountId,
        email_token: &str,
    ) -> Result<(String, Value), String> {
        let v = self
            .call_account(account, "complete", json!({ "token": email_token }))
            .await?;
        let cookie = v["cookie_header"]
            .as_str()
            .ok_or("sidecar complete: no cookie_header")?
            .to_string();
        Ok((cookie, v["me"].clone()))
    }

    async fn seed_session(&self, account: &AccountId, cookie_header: &str) -> Result<(), String> {
        let Some((name, value)) = cookie_header.split_once('=') else {
            return Err("bad cookie header".into());
        };
        self.call_account(
            account,
            "set_cookie",
            json!({ "name": name.trim(), "value": value.trim() }),
        )
        .await?;
        Ok(())
    }

    async fn ai_post(
        &self,
        account: &AccountId,
        path: &str,
        bearer: &str,
        body: &Value,
    ) -> Result<(u16, Option<String>, String), String> {
        let v = self
            .call_account(
                account,
                "ai_post",
                json!({ "path": path, "bearer": bearer, "body": body }),
            )
            .await?;
        self.ai_post_result(account, v).await
    }

    async fn ai_post_cancellable(
        &self, account: &AccountId, path: &str, bearer: &str, body: &Value,
        cancelled: Option<&(dyn Fn() -> bool + Send + Sync)>,
    ) -> Result<(u16, Option<String>, String), String> {
        let Some(cancelled) = cancelled else {
            return self.ai_post(account, path, bearer, body).await;
        };
        let request = async {
            for attempt_index in 0..3 {
                if cancelled() { return Err("client cancelled".into()); }
                // One host fetch at a time. Keep its real reply owned, then check
                // cancellation before authorizing a device retry in another call.
                let value = self.call_account(account, "ai_post", json!({
                    "path": path, "bearer": bearer, "body": body,
                    "defer_retries": true, "attempt_index": attempt_index,
                })).await?;
                if value.get("_retry_device").and_then(Value::as_bool) != Some(true) {
                    return self.ai_post_result(account, value).await;
                }
            }
            Err("host-lifecycle-unconfirmed: invalid browser retry budget".into())
        };
        tokio::time::timeout(REQ_TIMEOUT, request).await
            .map_err(|_| "sidecar account command timed out (cmd ai_post)".to_owned())?
    }

    async fn last_queue_wait_ms(&self, account: &AccountId) -> u64 {
        self.queue_wait.lock().await.get(account).copied().unwrap_or(0)
    }

    async fn set_proxy(&self, account: &AccountId, proxy: Option<String>) -> Result<(), String> {
        let _admission = self.admit_unscoped()?;
        if self.route_snapshot(account).await?.proxy == proxy { return Ok(()); }
        // Do not report a new egress while active requests prevented closure.
        self.call_account(account, "close", json!({})).await?;
        self.register_proxy(account, proxy).await
    }

    async fn register_account_proxy(&self, account: &AccountId, proxy: Option<String>)
        -> Result<(), String> {
        self.register_proxy(account, proxy).await
    }

    async fn rotate_identity(&self, account: &AccountId) -> Result<(), String> {
        self.call_account(account, "rotate", json!({})).await?;
        Ok(())
    }
}

/// Default state root helper shared with main.rs.
pub fn default_sidecar_root(profiles: &str) -> PathBuf {
    Path::new(profiles).join(".sidecar")
}

fn dirs_home() -> PathBuf {
    std::env::var("HOME").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from("/root"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dropped_call_releases_reply_and_requests_cancellation() {
        let pending: PendingReplies = Arc::new(StdMutex::new(HashMap::new()));
        let (tx, _rx) = oneshot::channel();
        pending.lock().unwrap().insert(7, tx);
        let (cancel, mut cancellations) = mpsc::channel(1);
        drop(PendingCall { id: 7, pending: pending.clone(), cancel });
        assert!(pending.lock().unwrap().is_empty());
        assert_eq!(cancellations.try_recv().unwrap(), 7);
    }

    #[test]
    fn completed_call_does_not_cancel_another_request() {
        let pending: PendingReplies = Arc::new(StdMutex::new(HashMap::new()));
        let (tx, _rx) = oneshot::channel();
        pending.lock().unwrap().insert(8, tx);
        let (cancel, mut cancellations) = mpsc::channel(1);
        drop(PendingCall { id: 7, pending: pending.clone(), cancel });
        assert!(pending.lock().unwrap().contains_key(&8));
        assert!(cancellations.try_recv().is_err());
    }

    #[test]
    fn closed_output_fails_all_waiters_immediately() {
        let pending: PendingReplies = Arc::new(StdMutex::new(HashMap::new()));
        let mut receivers = Vec::new();
        for id in 0..3 {
            let (tx, rx) = oneshot::channel();
            pending.lock().unwrap().insert(id, tx);
            receivers.push(rx);
        }
        fail_pending(&pending, "output closed");
        assert!(pending.lock().unwrap().is_empty());
        for mut rx in receivers {
            assert_eq!(rx.try_recv().unwrap(), Err("output closed".to_owned()));
        }
    }
}

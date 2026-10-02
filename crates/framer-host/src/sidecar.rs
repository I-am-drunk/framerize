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
//! The sidecar is a Python process (scripts/camoufox_host.py, embedded via
//! include_str! and extracted on first use) speaking NDJSON over stdio. One
//! Camoufox process per account; the profile dir is the account identity
//! (session cookie + Castle __cuid persist across restarts).

use framer_core::ids::AccountId;
use framer_core::port::{BrowserBridge, CastleToken};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::{oneshot, Mutex};

const SCRIPT: &str = include_str!("../../../scripts/camoufox_host.py");

/// In-page AI calls wait out a full generation (window ceiling ~109 s); mints
/// on a cold engine can take 60 s+. 300 s covers both with margin.
const REQ_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(300);

pub struct SidecarBridge {
    state_root: PathBuf,
    inner: Mutex<Option<Sidecar>>,
    proxies: Mutex<HashMap<AccountId, String>>,
}

struct Sidecar {
    child: Child,
    stdin: tokio::process::ChildStdin,
    pending: Arc<Mutex<HashMap<u64, oneshot::Sender<Result<Value, String>>>>>,
    next_id: AtomicU64,
}

impl SidecarBridge {
    pub fn new(state_root: impl Into<PathBuf>) -> Self {
        Self {
            state_root: state_root.into(),
            inner: Mutex::new(None),
            proxies: Mutex::new(HashMap::new()),
        }
    }

    fn script_path(&self) -> PathBuf {
        // External override wins: sidecar fixes ship via `git pull` + a copy to
        // ~/.framerize/camoufox_host.py instead of a full binary rebuild.
        // (Learned the hard way: include_str! alone makes every python tweak
        // a 20-minute release cycle.)
        let ext = dirs_home().join(".framerize/camoufox_host.py");
        if ext.exists() {
            return ext;
        }
        self.state_root.join(".bin").join("camoufox_host.py")
    }

    /// Spawn the sidecar if needed; RESPAWN if it died. A dead python still
    /// has a Some(child.id()) — id() is not a liveness check (a silent
    /// sidecar death on a constrained box hung every later call for 300s).
    async fn sidecar(&self) -> Result<(), String> {
        let mut guard = self.inner.lock().await;
        let mut dead = false;
        if let Some(s) = guard.as_mut() {
            match s.child.try_wait() {
                Ok(None) => return Ok(()), // alive
                _ => {
                    eprintln!("[sidecar] python exited — respawning on next request");
                    dead = true;
                }
            }
        }
        if dead {
            if let Some(s) = guard.take() {
                // fail every waiter; the caller's error surfaces immediately
                let mut pend = s.pending.lock().await;
                for (_, tx) in pend.drain() {
                    let _ = tx.send(Err("sidecar died — retry the request".into()));
                }
            }
        }
        let script = self.script_path();
        if !script.exists() {
            if let Some(parent) = script.parent() {
                std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            std::fs::write(&script, SCRIPT).map_err(|e| e.to_string())?;
        }

        // Python resolution: explicit override, then the installer's venv,
        // then PATH. The venv is where install-camoufox.sh puts everything.
        let python = std::env::var("FRAMERIZE_PYTHON").unwrap_or_else(|_| {
            let venv = dirs_home().join(".framerize/venv/bin/python3");
            if venv.exists() { venv.to_string_lossy().into_owned() } else { "python3".into() }
        });
        let mut child = Command::new(&python)
            .arg(&script)
            .env("FRAMERIZE_STATE", self.state_root.join("camoufox"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit()) // sidecar traces land in the daemon log
            .spawn()
            .map_err(|e| {
                format!(
                    "spawn camoufox sidecar: {e} — install it first: \
                     scripts/linux/install-camoufox.sh"
                )
            })?;
        let stdin = child.stdin.take().ok_or("sidecar stdin")?;
        let stdout = child.stdout.take().ok_or("sidecar stdout")?;
        let pending: Arc<Mutex<HashMap<u64, oneshot::Sender<Result<Value, String>>>>> =
            Arc::new(Mutex::new(HashMap::new()));
        // Reader task: route response lines to their oneshot by id.
        {
            let pending = pending.clone();
            tokio::spawn(async move {
                let mut lines = BufReader::new(stdout).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    let Ok(v) = serde_json::from_str::<Value>(&line) else { continue };
                    let Some(id) = v["id"].as_u64() else { continue };
                    let tx = pending.lock().await.remove(&id);
                    if let Some(tx) = tx {
                        if v["ok"].as_bool() == Some(true) {
                            let _ = tx.send(Ok(v["result"].clone()));
                        } else {
                            let _ = tx.send(Err(v["error"].as_str().unwrap_or("sidecar error").to_string()));
                        }
                    }
                }
            });
        }
        *guard = Some(Sidecar { child, stdin, pending, next_id: AtomicU64::new(0) });
        Ok(())
    }

    async fn call(&self, cmd: &str, mut fields: Value) -> Result<Value, String> {
        self.sidecar().await?;
        let mut guard = self.inner.lock().await;
        let s = guard.as_mut().ok_or("sidecar unavailable")?;
        let id = s.next_id.fetch_add(1, Ordering::Relaxed) + 1;
        fields["id"] = json!(id);
        fields["cmd"] = json!(cmd);
        let (tx, rx) = oneshot::channel();
        s.pending.lock().await.insert(id, tx);
        s.stdin
            .write_all(format!("{}\n", fields).as_bytes())
            .await
            .map_err(|e| format!("sidecar write: {e}"))?;
        let started = std::time::Instant::now();
        drop(guard); // don't hold the process lock while a generation runs
        match tokio::time::timeout(REQ_TIMEOUT, rx).await {
            Ok(Ok(r)) => r,
            Ok(Err(_)) => Err("sidecar dropped the request".into()),
            Err(_) => Err(format!("sidecar timeout after {:?} (cmd {cmd})", started.elapsed())),
        }
    }

    async fn call_account(&self, account: &AccountId, cmd: &str, extra: Value) -> Result<Value, String> {
        let mut fields = extra;
        fields["account"] = json!(account.0.to_string());
        self.call(cmd, fields).await
    }
}

#[async_trait::async_trait]
impl BrowserBridge for SidecarBridge {
    async fn castle_token(&self, account: &AccountId) -> Result<CastleToken, String> {
        let v = self.call_account(account, "mint", json!({})).await?;
        v.as_str()
            .map(|s| CastleToken(s.to_string()))
            .ok_or_else(|| "sidecar mint: bad result".into())
    }

    async fn auth_begin(&self, account: &AccountId, email: &str) -> Result<(), String> {
        let proxy = self.proxies.lock().await.get(account).cloned();
        let mut body = json!({ "email": email });
        if let Some(p) = proxy {
            body["proxy"] = json!(p);
        }
        self.call_account(account, "begin", body).await?;
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
            return Err(format!("bad cookie header: {cookie_header:?}"));
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
        let status = v["status"].as_i64().unwrap_or(-1);
        let usage = v["usage"].as_str().map(|s| s.to_string());
        let text = v["body"].as_str().unwrap_or("").to_string();
        if status == 0 {
            return Err("castle-v3-unavailable".into());
        }
        if status == -1 {
            return Err(format!("in-page fetch failed: {text}"));
        }
        Ok((status as u16, usage, text))
    }

    async fn set_proxy(&self, account: &AccountId, proxy: Option<String>) -> Result<(), String> {
        match proxy {
            Some(p) => self.proxies.lock().await.insert(*account, p),
            None => self.proxies.lock().await.remove(account),
        };
        // Relaunch the account's browser on the new egress next use.
        let _ = self.call_account(account, "close", json!({})).await;
        Ok(())
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

//! The browser host: the one irreplaceable piece. Castle request tokens
//! fingerprint a real browser engine, so every account gets a real Chromium
//! instance with its own persistent profile — "accounts are devices".
//!
//! Design (see reference/analysis/framer-castle-token.md):
//! - one OS-level browser profile directory per account: cookies, localStorage
//!   and the Castle device id (`__cuid`) persist across restarts automatically;
//! - before the first token mint of a run, the account's `castle_device_id` is
//!   seeded into both stores (castle::SEED_DEVICE_SNIPPET) as a backstop for
//!   wiped/copied profiles;
//! - repeated generic-403s are ENVIRONMENT-STICKY: rotate by wiping the profile
//!   and regenerating the device id (`rotate_identity`), never fast-retry.
//!
//! Transport is hand-rolled CDP over WebSocket: launch Chrome with
//! `--remote-debugging-port=0`, read `DevToolsActivePort` from the profile dir,
//! attach to the page target, `Runtime.evaluate` with awaitPromise.

use async_trait::async_trait;
use framer_core::ids::AccountId;
use framer_core::port::{BrowserBridge, CastleToken};
use futures::{SinkExt, StreamExt};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use tokio::process::{Child, Command};
use tokio::sync::Mutex;
use tokio_tungstenite::{connect_async, tungstenite::Message};

const FRAMER_HOME: &str = "https://framer.com/login";
const CASTLE_CHUNK_PINNED: &str = "https://app.framerstatic.com/chunk-5LKSDTQ5.mjs";

/// JS: seed __cuid (cookie + localStorage) — must run before the first mint.
fn seed_device_snippet(device_id: &str) -> String {
    format!(
        r#"(() => {{
  const id = {device_id:?};
  if (localStorage.getItem("__cuid") !== id) localStorage.setItem("__cuid", id);
  if (!document.cookie.includes("__cuid=" + id)) {{
    document.cookie = "__cuid=" + id + "; domain=.framer.com; path=/; max-age=31536000; SameSite=Lax";
  }}
  return id;
}})()"#
    )
}

const MINT_SNIPPET: &str = r#"import("%CHUNK%").then(m => m.a())"#;

/// Per-account persisted device record (lives in the profile dir, survives restarts).
struct DeviceRecord;

impl DeviceRecord {
    fn path(profile: &Path) -> PathBuf {
        profile.join("framerize-device.json")
    }
    async fn load_or_create(profile: &Path) -> Result<String, String> {
        let p = Self::path(profile);
        if let Ok(raw) = tokio::fs::read_to_string(&p).await {
            if let Ok(v) = serde_json::from_str::<Value>(&raw) {
                if let Some(id) = v["castle_device_id"].as_str() {
                    return Ok(id.to_string());
                }
            }
        }
        let id = uuid::Uuid::new_v4().to_string();
        tokio::fs::create_dir_all(profile).await.map_err(|e| e.to_string())?;
        tokio::fs::write(&p, json!({"castle_device_id": id}).to_string())
            .await
            .map_err(|e| e.to_string())?;
        Ok(id)
    }
}

struct CdpInstance {
    #[allow(dead_code)]
    child: Child, // kept alive; kill on drop
    ws: tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    next_id: u64,
}

impl CdpInstance {
    async fn eval(&mut self, expression: String) -> Result<Value, String> {
        self.next_id += 1;
        let id = self.next_id;
        let msg = json!({
            "id": id,
            "method": "Runtime.evaluate",
            "params": { "expression": expression, "awaitPromise": true, "returnByValue": true }
        });
        self.ws
            .send(Message::Text(msg.to_string().into()))
            .await
            .map_err(|e| e.to_string())?;
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(30);
        loop {
            let next = tokio::time::timeout_at(deadline, self.ws.next())
                .await
                .map_err(|_| "cdp eval timed out".to_string())?
                .ok_or("cdp ws closed")?
                .map_err(|e| e.to_string())?;
            if let Message::Text(txt) = next {
                let v: Value = serde_json::from_str(&txt).map_err(|e| e.to_string())?;
                if v["id"].as_u64() == Some(id) {
                    if let Some(err) = v.get("error") {
                        return Err(format!("cdp error: {err}"));
                    }
                    return Ok(v["result"]["result"]["value"].clone());
                }
            }
        }
    }
}

pub struct CdpBridge {
    chrome: PathBuf,
    profiles_dir: PathBuf,
    instances: Mutex<HashMap<AccountId, Arc<Mutex<CdpInstance>>>>,
}

impl CdpBridge {
    /// `chrome` = path to a Chrome/Chromium binary; `profiles_dir` holds one
    /// subdirectory per account (the account's whole browser identity).
    pub fn new(chrome: impl Into<PathBuf>, profiles_dir: impl Into<PathBuf>) -> Self {
        Self {
            chrome: chrome.into(),
            profiles_dir: profiles_dir.into(),
            instances: Mutex::new(HashMap::new()),
        }
    }

    fn profile_dir(&self, account: &AccountId) -> PathBuf {
        self.profiles_dir.join(account.0.to_string())
    }

    async fn instance(&self, account: &AccountId) -> Result<Arc<Mutex<CdpInstance>>, String> {
        if let Some(inst) = self.instances.lock().await.get(account) {
            return Ok(inst.clone());
        }
        let profile = self.profile_dir(account);
        tokio::fs::create_dir_all(&profile).await.map_err(|e| e.to_string())?;
        let device_id = DeviceRecord::load_or_create(&profile).await?;

        let child = Command::new(&self.chrome)
            .arg("--headless=new")
            .arg("--remote-debugging-port=0")
            .arg(format!("--user-data-dir={}", profile.display()))
            .arg("--no-first-run")
            .arg("--no-default-browser-check")
            .arg(FRAMER_HOME)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("launch chrome: {e}"))?;

        // Chrome writes DevToolsActivePort (port\nws-path) into the profile dir.
        let port_file = profile.join("DevToolsActivePort");
        let mut port = None;
        for _ in 0..100 {
            if let Ok(raw) = tokio::fs::read_to_string(&port_file).await {
                if let Some(line) = raw.lines().next() {
                    if let Ok(p) = line.trim().parse::<u16>() {
                        port = Some(p);
                        break;
                    }
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        let port = port.ok_or("chrome did not write DevToolsActivePort")?;

        // Find the page target's websocket URL.
        let list: Value = serde_json::from_slice(
            &reqwest_get(&format!("http://127.0.0.1:{port}/json/list")).await?,
        )
        .map_err(|e| e.to_string())?;
        let ws_url = list
            .as_array()
            .and_then(|a| {
                a.iter()
                    .find(|t| t["type"].as_str() == Some("page"))
                    .and_then(|t| t["webSocketDebuggerUrl"].as_str())
            })
            .ok_or("no page target")?
            .to_string();

        let (ws, _) = connect_async(&ws_url).await.map_err(|e| e.to_string())?;
        let mut inst = CdpInstance { child, ws, next_id: 0 };

        // Wait for framer.com to be ready (window.castlePublishableAPIKey set).
        for _ in 0..50 {
            let ready = inst
                .eval("typeof window.castlePublishableAPIKey !== 'undefined' && document.readyState === 'complete'".into())
                .await
                .ok()
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            if ready {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        }

        // Backstop: seed the stable Castle device id before any mint.
        inst.eval(seed_device_snippet(&device_id)).await?;

        let inst = Arc::new(Mutex::new(inst));
        self.instances
            .lock()
            .await
            .insert(*account, inst.clone());
        Ok(inst)
    }

    /// The 403 remediation: drop the instance, wipe the profile (cookies,
    /// storage, fingerprint surface), regenerate the device id. The account
    /// will need to log in again — this is the "fresh device" escape hatch.
    pub async fn rotate_identity(&self, account: &AccountId) -> Result<(), String> {
        self.instances.lock().await.remove(account);
        let profile = self.profile_dir(account);
        if profile.exists() {
            tokio::fs::remove_dir_all(&profile).await.map_err(|e| e.to_string())?;
        }
        // Next castle_token() recreates the profile with a fresh device id.
        Ok(())
    }
}

#[async_trait]
impl BrowserBridge for CdpBridge {
    async fn castle_token(&self, account: &AccountId) -> Result<CastleToken, String> {
        let inst = self.instance(account).await?;
        let mut guard = inst.lock().await;
        let expr = MINT_SNIPPET.replace("%CHUNK%", CASTLE_CHUNK_PINNED);
        let v = guard.eval(expr).await?;
        v.as_str()
            .map(|s| CastleToken(s.to_string()))
            .ok_or_else(|| format!("castle mint returned non-string: {v}"))
    }
}

/// Minimal GET helper (avoids pulling reqwest into this crate).
async fn reqwest_get(url: &str) -> Result<Vec<u8>, String> {
    let resp = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        tokio::task::spawn_blocking({
            let url = url.to_string();
            move || {
                // ureq-free: use a raw HTTP/1.1 GET over TcpStream (localhost only).
                use std::io::{Read, Write};
                let host = url
                    .trim_start_matches("http://")
                    .split('/')
                    .next()
                    .unwrap_or("")
                    .to_string();
                let path = url.split_once(&host).map(|(_, p)| p.to_string()).unwrap_or_default();
                let mut s = std::net::TcpStream::connect(&host).map_err(|e| e.to_string())?;
                s.write_all(format!("GET {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n").as_bytes())
                    .map_err(|e| e.to_string())?;
                let mut buf = Vec::new();
                s.read_to_end(&mut buf).map_err(|e| e.to_string())?;
                let text = String::from_utf8_lossy(&buf);
                let body = text.split("\r\n\r\n").nth(1).unwrap_or("").as_bytes().to_vec();
                Ok::<Vec<u8>, String>(body)
            }
        }),
    )
    .await
    .map_err(|_| "http get timed out".to_string())?
    .map_err(|e| e.to_string())??;
    Ok(resp)
}

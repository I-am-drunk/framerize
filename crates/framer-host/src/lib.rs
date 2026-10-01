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
const API_BASE: &str = "https://api.framer.com";

/// A current ordinary desktop Chrome UA (matches what real framer.com
/// visitors send; keep roughly in sync with the chrome binary's version).
const DESKTOP_UA: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/141.0.0.0 Safari/537.36";

/// Combined warm-mint + authenticated POST, evaluated inside the account's
/// page on framer.com. Sending auth requests from the page (not reqwest) is
/// what makes Origin / User-Agent / TLS / fingerprint consistent with the
/// minted Castle token — the server rejects the bare-HTTP-client variant of
/// the exact same body with a 400 (verified live).
/// Placeholders: %CHUNK%, %PATH%, %BODY% (JSON object literal WITHOUT the
/// castleToken field — it is added from the fresh mint).
const AUTH_POST_SNIPPET: &str = r#"(async () => {
  let tok = null;
  for (let i = 0; i < 3 && !tok; i++) {
    try {
      const m = await import("%CHUNK%?cb=" + Date.now() + "_" + i);
      const t = await m.a();
      if (typeof t === "string" && t.length > 4000 && t.charAt(8) === "|") tok = t;
    } catch (e) {}
    if (!tok) await new Promise(r => setTimeout(r, 1200));
  }
  if (!tok) return { status: 0, body: "castle-v3-unavailable" };
  const body = %BODY%;
  body.castleToken = tok;
  try {
    const r = await fetch("%API%%PATH%", {
      method: "POST",
      credentials: "include",
      headers: { "content-type": "application/json; charset=utf-8", "x-requested-by": "Framer" },
      body: JSON.stringify(body),
    });
    return { status: r.status, body: await r.text() };
  } catch (e) {
    return { status: -1, body: String(e) };
  }
})()"#;

fn auth_post_expr(path: &str, body: &Value) -> String {
    AUTH_POST_SNIPPET
        .replace("%CHUNK%", CASTLE_CHUNK_PINNED)
        .replace("%API%", API_BASE)
        .replace("%PATH%", path)
        .replace("%BODY%", &body.to_string())
}

/// In-page GET (cookies ride along via credentials:include).
const PAGE_GET_SNIPPET: &str = r#"(async () => {
  try {
    const r = await fetch("%API%%PATH%", { headers: { "x-requested-by": "Framer" }, credentials: "include" });
    return { status: r.status, body: await r.text() };
  } catch (e) {
    return { status: -1, body: String(e) };
  }
})()"#;

fn page_get_expr(path: &str) -> String {
    PAGE_GET_SNIPPET.replace("%API%", API_BASE).replace("%PATH%", path)
}
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

/// The wrapper's export a() is a FALLBACK CHAIN: Castle v3 first, legacy v2
/// if v3 fails — and the v3 failure flag (Rh) latches forever inside that
/// module instance. Cold headless chrome can't beat the 5s internal timeouts
/// (IX = 5e3), so the first import often locks in v2 (a ~2.1KB pipeless
/// string the API rejects with a bare 400 — observed live).
///
/// Strategy: cache-busted imports (?cb=) re-run the module scope = fresh Rh,
/// while the underlying castleSDKV3 module stays warm after first load. Only
/// accept a v3-shaped token: ~5.6KB with an 8-char device prefix + '|' at
/// index 8. Return null after a few attempts so the caller can retry.
const MINT_SNIPPET: &str = r#"(async () => {
  for (let i = 0; i < 3; i++) {
    try {
      const m = await import("%CHUNK%?cb=" + Date.now() + "_" + i); // fresh module = un-latched Rh
      const t = await m.a();
      if (typeof t === "string" && t.length > 4000 && t.charAt(8) === "|") return t;
    } catch (e) {}
    await new Promise(r => setTimeout(r, 1200));
  }
  return null;
})()"#;

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
    ws: WsStream,
    next_id: u64,
}

impl CdpInstance {
    async fn cmd(&mut self, method: &str, params: Value) -> Result<Value, String> {
        self.next_id += 1;
        let id = self.next_id;
        let msg = json!({ "id": id, "method": method, "params": params });
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
                    return Ok(v["result"].clone());
                }
            }
        }
    }

    async fn eval(&mut self, expression: String) -> Result<Value, String> {
        let r = self
            .cmd(
                "Runtime.evaluate",
                json!({ "expression": expression, "awaitPromise": true, "returnByValue": true }),
            )
            .await?;
        Ok(r["result"]["value"].clone())
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

        // The profile persists between launches, so a STALE DevToolsActivePort
        // from a previous (dead) chrome can be sitting there. Reading it wins
        // the race against the new chrome's write and sends every devtools
        // call to a dead port ("Connection refused" forever — observed live).
        let _ = tokio::fs::remove_file(profile.join("DevToolsActivePort")).await;

        let mut child = Command::new(&self.chrome)
            .arg("--headless=new")
            .arg("--remote-debugging-port=0")
            .arg(format!("--user-data-dir={}", profile.display()))
            .arg("--no-first-run")
            .arg("--no-default-browser-check")
            // Containers run as root (chrome refuses without this) and mount a
            // tiny /dev/shm (chrome crashes rendering without this).
            .arg("--no-sandbox")
            .arg("--disable-dev-shm-usage")
            .arg("--disable-gpu")
            .arg("--mute-audio")
            // headless-shell's default UA says "HeadlessChrome" — a Castle
            // bot signal, and inconsistent with any token minted as a normal
            // browser. Present as ordinary desktop Chrome.
            .arg(format!("--user-agent={DESKTOP_UA}"))
            .arg(FRAMER_HOME)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("launch chrome: {e}"))?;

        // Everything from here to a constructed CdpInstance must kill the
        // child on error — a leaked chrome is ~300MB and a few of them OOM
        // the host (this bit us repeatedly in live testing).
        let ws = match bring_up_ws(&profile).await {
            Ok(ws) => ws,
            Err(e) => {
                let _ = child.start_kill();
                return Err(e);
            }
        };
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
        if let Err(e) = inst.eval(seed_device_snippet(&device_id)).await {
            let _ = inst.child.start_kill();
            return Err(e);
        }

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
        // Up to 3 whole-eval retries: the first cold mint pays the SDK
        // download; later ones reuse the warm castleSDKV3 module.
        let mut last = String::new();
        for attempt in 1..=3u32 {
            let v = match guard.eval(expr.clone()).await {
                Ok(v) => v,
                Err(e) => {
                    eprintln!("[castle] attempt {attempt}: eval failed: {e}");
                    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                    continue;
                }
            };
            match v.as_str() {
                Some(s) => {
                    eprintln!(
                        "[castle] attempt {attempt}: v3 token minted, {} chars, device prefix {:?}",
                        s.len(),
                        &s[..8]
                    );
                    return Ok(CastleToken(s.to_string()));
                }
                _ => {
                    last = format!("{v}");
                    eprintln!("[castle] attempt {attempt}: no v3 token yet (got {last})");
                    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                }
            }
        }
        Err(format!(
            "castle v3 unavailable after retries (last: {last}) — refusing to send a v2 token the API will 400"
        ))
    }
}

type WsStream = tokio_tungstenite::WebSocketStream<
    tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
>;

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

/// Waits for DevToolsActivePort, then for the HTTP endpoint behind it.
/// The port FILE appears before the socket accepts connections — hence retry.
async fn bring_up_ws(profile: &std::path::Path) -> Result<WsStream, String> {
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
    let port = port.ok_or("chrome did not write DevToolsActivePort (chrome died? run with --no-sandbox on root containers)")?;

    let mut list_raw = None;
    let mut last_err = String::new();
    for _ in 0..60 {
        match reqwest_get(&format!("http://127.0.0.1:{port}/json/list")).await {
            Ok(b) => {
                list_raw = Some(b);
                break;
            }
            Err(e) => {
                last_err = e;
                tokio::time::sleep(std::time::Duration::from_millis(250)).await;
            }
        }
    }
    let list: Value = serde_json::from_slice(
        &list_raw.ok_or_else(|| format!("devtools http never came up: {last_err}"))?,
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
    Ok(ws)
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
                s.set_read_timeout(Some(std::time::Duration::from_secs(4))).map_err(|e| e.to_string())?;
                s.write_all(format!("GET {path} HTTP/1.1\r\nHost: {host}\r\n\r\n").as_bytes())
                    .map_err(|e| e.to_string())?;
                // Chrome's devtools HTTP server keeps the socket OPEN
                // (keep-alive) — read_to_end blocks forever. Read headers,
                // take Content-Length, read exactly that many body bytes.
                let mut buf = Vec::new();
                let mut tmp = [0u8; 4096];
                let body_start = loop {
                    let n = s.read(&mut tmp).map_err(|e| e.to_string())?;
                    if n == 0 { return Err("devtools http closed".into()); }
                    buf.extend_from_slice(&tmp[..n]);
                    if let Some(pos) = find_subslice(&buf, b"\r\n\r\n") { break pos + 4; }
                    if buf.len() > 16384 { return Err("devtools http headers too large".into()); }
                };
                let headers = String::from_utf8_lossy(&buf[..body_start]).to_lowercase();
                let len: Option<usize> = headers.lines()
                    .find_map(|l| l.strip_prefix("content-length:").and_then(|v| v.trim().parse().ok()));
                match len {
                    Some(len) => {
                        while buf.len() < body_start + len {
                            let n = s.read(&mut tmp).map_err(|e| e.to_string())?;
                            if n == 0 { break; }
                            buf.extend_from_slice(&tmp[..n]);
                        }
                        Ok::<Vec<u8>, String>(buf[body_start..(body_start + len).min(buf.len())].to_vec())
                    }
                    None => {
                        // No Content-Length (chunked/close-delimited): drain
                        // until the server goes quiet, take what arrived.
                        s.set_read_timeout(Some(std::time::Duration::from_millis(800))).ok();
                        loop {
                            match s.read(&mut tmp) {
                                Ok(0) => break,
                                Ok(n) => buf.extend_from_slice(&tmp[..n]),
                                Err(_) => break, // timeout = done
                            }
                        }
                        Ok::<Vec<u8>, String>(buf[body_start..].to_vec())
                    }
                }
            }
        }),
    )
    .await
    .map_err(|_| "http get timed out".to_string())?
    .map_err(|e| e.to_string())??;
    Ok(resp)
}

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

/// Derive the desktop UA from the actual chrome binary (so it always matches
/// the engine's real version — a UA/version mismatch is itself a bot signal).
async fn desktop_ua(chrome: &Path) -> String {
    let out = Command::new(chrome).arg("--version").output().await;
    let ver = out.ok().and_then(|o| {
        let s = String::from_utf8_lossy(&o.stdout).to_string();
        s.split_whitespace()
            .find(|w| w.chars().next().map(|c| c.is_ascii_digit()).unwrap_or(false))
            .map(|v| v.to_string())
    });
    match ver {
        Some(v) => format!(
            "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/{v} Safari/537.36"
        ),
        None => DESKTOP_UA.to_string(),
    }
}

/// Attach mode: connect to a user-launched Chrome (host:port in `addr`),
/// find its framer.com tab (or open one), return the page websocket.
async fn attach_ws(addr: &Path, account: &AccountId) -> Result<WsStream, String> {
    let addr = addr.to_string_lossy().to_string();
    let base = format!("http://{addr}");
    // find an existing framer.com page tab
    let list: Value = serde_json::from_slice(
        &reqwest_get(&format!("{base}/json/list")).await?,
    )
    .map_err(|e| e.to_string())?;
    let ws_url = list
        .as_array()
        .and_then(|a| {
            a.iter()
                .find(|t| t["type"].as_str() == Some("page")
                    && t["url"].as_str().map(|u| u.contains("framer.com")).unwrap_or(false))
                .and_then(|t| t["webSocketDebuggerUrl"].as_str())
        })
        .map(|s| s.to_string());
    let ws_url = match ws_url {
        Some(u) => u,
        None => {
            // open a framer.com tab (newer Chrome requires PUT for /json/new)
            let created = reqwest_put(&format!("{base}/json/new?{}", urlencoding(FRAMER_HOME))).await?;
            let v: Value = serde_json::from_slice(&created).map_err(|e| e.to_string())?;
            v["webSocketDebuggerUrl"]
                .as_str()
                .ok_or("attach: /json/new returned no ws url")?
                .to_string()
        }
    };
    let (ws, _) = connect_async(&ws_url).await.map_err(|e| e.to_string())?;
    eprintln!("[host] attach mode: driving user Chrome tab for account {}", &account.0.to_string()[..8]);
    Ok(ws)
}

fn urlencoding(u: &str) -> String {
    u.replace(':', "%3A").replace('/', "%2F")
}

/// Minimal PUT helper (Chrome's /json/new requires PUT in recent versions).
async fn reqwest_put(url: &str) -> Result<Vec<u8>, String> {
    let resp = tokio::task::spawn_blocking({
        let url = url.to_string();
        move || {
            use std::io::{Read, Write};
            let host = url.trim_start_matches("http://").split('/').next().unwrap_or("").to_string();
            let path = url.split_once(&host).map(|(_, p)| p.to_string()).unwrap_or_default();
            let mut s = std::net::TcpStream::connect(&host).map_err(|e| e.to_string())?;
            s.write_all(format!("PUT {path} HTTP/1.1
Host: {host}
Content-Length: 0

").as_bytes())
                .map_err(|e| e.to_string())?;
            let mut buf = Vec::new();
            s.read_to_end(&mut buf).map_err(|e| e.to_string())?;
            let text = String::from_utf8_lossy(&buf);
            Ok::<Vec<u8>, String>(text.split("

").nth(1).unwrap_or("").as_bytes().to_vec())
        }
    })
    .await
    .map_err(|e| e.to_string())??;
    Ok(resp)
}

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

/// In-page authenticated AI POST: warm-mint a Castle v3 token, attach it to
/// the envelope, and send from the account's own page with the AI JWT.
/// THE rule (verified quadrant matrix 2026-10-02): a Castle token is only
/// accepted when the request shares the minting environment — same TLS stack,
/// same UA, same Origin. The exact same envelope via reqwest gets risk-refused
/// even with a perfect token. SSE bodies are consumed to completion in-page
/// (r.text() waits out the stream) and returned whole.
/// Returns { status, usage (x-framer-ai-usage header, may be null when CORS
/// hides it), body }. Placeholders: %CHUNK%, %API%, %PATH%, %BEARER%, %BODY%.
const AI_POST_SNIPPET: &str = r#"(async () => {
  let tok = null;
  for (let i = 0; i < 3 && !tok; i++) {
    try {
      const m = await import("%CHUNK%?cb=" + Date.now() + "_" + i);
      const t = await m.a();
      if (typeof t === "string" && t.length > 4000 && t.charAt(8) === "|") tok = t;
    } catch (e) {}
    if (!tok) await new Promise(r => setTimeout(r, 1200));
  }
  if (!tok) return { status: 0, usage: null, body: "castle-v3-unavailable" };
  const body = %BODY%;
  body.castleToken = tok;
  try {
    const r = await fetch("%API%%PATH%", {
      method: "POST",
      credentials: "include",
      headers: {
        "content-type": "application/json; charset=utf-8",
        "x-requested-by": "Framer",
        "authorization": "Bearer %BEARER%",
      },
      body: JSON.stringify(body),
    });
    return { status: r.status, usage: r.headers.get("x-framer-ai-usage"), body: await r.text() };
  } catch (e) {
    return { status: -1, usage: null, body: String(e) };
  }
})()"#;

fn ai_post_expr(path: &str, bearer: &str, body: &Value) -> String {
    AI_POST_SNIPPET
        .replace("%CHUNK%", CASTLE_CHUNK_PINNED)
        .replace("%API%", API_BASE)
        .replace("%PATH%", path)
        .replace("%BEARER%", bearer)
        .replace("%BODY%", &body.to_string())
}

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
    child: Option<Child>, // kept alive; kill on drop. None in Attach mode.
    ws: WsStream,
    next_id: u64,
}

impl CdpInstance {
    async fn cmd(&mut self, method: &str, params: Value) -> Result<Value, String> {
        self.cmd_deadline(method, params, std::time::Duration::from_secs(30)).await
    }

    async fn cmd_deadline(&mut self, method: &str, params: Value, wait: std::time::Duration) -> Result<Value, String> {
        self.next_id += 1;
        let id = self.next_id;
        let msg = json!({ "id": id, "method": method, "params": params });
        self.ws
            .send(Message::Text(msg.to_string().into()))
            .await
            .map_err(|e| e.to_string())?;
        let deadline = tokio::time::Instant::now() + wait;
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

    /// Eval with a long deadline — in-page AI calls wait out a full generation
    /// (window-mapped ceiling ~109 s on the wide route; give it room).
    async fn eval_long(&mut self, expression: String) -> Result<Value, String> {
        let r = self
            .cmd_deadline(
                "Runtime.evaluate",
                json!({ "expression": expression, "awaitPromise": true, "returnByValue": true }),
                std::time::Duration::from_secs(300),
            )
            .await?;
        Ok(r["result"]["value"].clone())
    }
}

/// Which browser engine backs account sessions — a config choice, not a code
/// change. Motivation (verified live 2026-10-02): chrome-headless-shell's
/// fingerprint is Castle-scored (its minted tokens get 403 even from a clean
/// environment), while a full Chrome's pass. The engine is therefore
/// swappable per daemon, and attach mode removes the launch question
/// entirely by driving a real browser the user already trusts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EngineKind {
    /// Full desktop Chrome, headless=new + stealth flags (default).
    FullChrome,
    /// The stripped headless build — known-bad Castle fingerprint, kept only
    /// as a fallback for minimal environments.
    HeadlessShell,
    /// Drive a REAL Chrome the user launched themselves
    /// (`--remote-debugging-port=9222`): their actual profile, history, GPU
    /// and trust state. The best Castle score possible — the "working browser"
    /// mode. Daemon never launches anything in this mode.
    Attach,
}

pub struct CdpBridge {
    chrome: PathBuf,
    profiles_dir: PathBuf,
    engine: EngineKind,
    /// Per-account proxy override: account uuid → proxy URL. Populated by the
    /// daemon from account records (and reloadable); applied as
    /// `--proxy-server` at chrome launch for that account's profile.
    proxies: std::sync::Arc<tokio::sync::RwLock<std::collections::HashMap<String, String>>>,
    instances: Mutex<HashMap<AccountId, Arc<Mutex<CdpInstance>>>>,
}

impl CdpBridge {
    /// `chrome` = path to a Chrome/Chromium binary; `profiles_dir` holds one
    /// subdirectory per account (the account's whole browser identity).
    pub fn new(chrome: impl Into<PathBuf>, profiles_dir: impl Into<PathBuf>) -> Self {
        Self::with_engine(chrome, profiles_dir, EngineKind::FullChrome)
    }

    pub fn with_engine(chrome: impl Into<PathBuf>, profiles_dir: impl Into<PathBuf>, engine: EngineKind) -> Self {
        Self {
            chrome: chrome.into(),
            profiles_dir: profiles_dir.into(),
            engine,
            proxies: std::sync::Arc::new(tokio::sync::RwLock::new(HashMap::new())),
            instances: Mutex::new(HashMap::new()),
        }
    }

    /// Set/replace the proxy map (account uuid string → proxy URL). Takes
    /// effect on the NEXT chrome launch for that account; a running instance
    /// keeps its current egress until rotated/dropped.
    pub async fn set_proxies(&self, map: HashMap<String, String>) {
        *self.proxies.write().await = map;
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

        // ATTACH MODE: no launch, no profile management. The user runs their
        // own real Chrome with --remote-debugging-port=<port>; we drive a
        // framer.com tab in it. `chrome` path field is reused as "host:port".
        if self.engine == EngineKind::Attach {
            let ws = attach_ws(&self.chrome, account).await?;
            let mut inst = CdpInstance { child: None, ws, next_id: 0 };
            for _ in 0..50 {
                let ready = inst
                    .eval("typeof window.castlePublishableAPIKey !== 'undefined' && document.readyState === 'complete'".into())
                    .await
                    .ok()
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                if ready { break; }
                tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            }
            inst.eval(seed_device_snippet(&device_id)).await?;
            let inst = Arc::new(Mutex::new(inst));
            self.instances.lock().await.insert(*account, inst.clone());
            return Ok(inst);
        }

        let proxy = self.proxies.read().await.get(&account.0.to_string()).cloned();
        let mut cmd = Command::new(&self.chrome);
        if let Some(p) = &proxy {
            cmd.arg(format!("--proxy-server={p}"));
            eprintln!("[host] account {} launching with per-account proxy", &account.0.to_string()[..8]);
        }
        cmd.arg("--remote-debugging-port=0")
            .arg(format!("--user-data-dir={}", profile.display()))
            .arg("--no-first-run")
            .arg("--no-default-browser-check")
            // Containers run as root (chrome refuses without this) and mount a
            // tiny /dev/shm (chrome crashes rendering without this).
            .arg("--no-sandbox")
            .arg("--disable-dev-shm-usage")
            .arg("--mute-audio");
        let ua = desktop_ua(&self.chrome).await;
        match self.engine {
            EngineKind::FullChrome => {
                // Stealth: present as an ordinary desktop browser. The two
                // signals that most betray automation: the headless UA string
                // and the webdriver flag. headless=new shares the real
                // renderer; headless-shell is a different, detectable build.
                cmd.arg("--headless=new")
                    .arg("--disable-blink-features=AutomationControlled")
                    .arg(format!("--user-agent={ua}"))
                    .arg("--lang=en-US,en")
                    .arg("--window-size=1920,1080")
                    .arg("--disable-features=TranslateUI");
            }
            EngineKind::HeadlessShell => {
                cmd.arg("--headless=new")
                    .arg("--disable-gpu")
                    .arg(format!("--user-agent={DESKTOP_UA}"));
            }
            EngineKind::Attach => unreachable!(),
        }
        let mut child = cmd
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
        let mut inst = CdpInstance { child: Some(child), ws, next_id: 0 };

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
            if let Some(c) = inst.child.as_mut() { let _ = c.start_kill(); }
            return Err(e);
        }

        let inst = Arc::new(Mutex::new(inst));
        self.instances
            .lock()
            .await
            .insert(*account, inst.clone());
        Ok(inst)
    }

}

fn body_preview(v: &Value) -> String {
    let b = v["body"].as_str().unwrap_or("");
    b[..b.len().min(400)].to_string()
}

#[async_trait]
impl BrowserBridge for CdpBridge {
    async fn seed_session(&self, account: &AccountId, cookie_header: &str) -> Result<(), String> {
        // Imported accounts: the browser profile never saw the login flow, so
        // its cookie jar is empty. In-page fetches authenticate via that jar —
        // seed the httpOnly session cookie over CDP (document.cookie can't
        // write httpOnly). Cheap and idempotent: set it before every AI call.
        let Some((name, value)) = cookie_header.split_once('=') else {
            return Err(format!("bad cookie header: {cookie_header:?}"));
        };
        let inst = self.instance(account).await?;
        let mut g = inst.lock().await;
        g.cmd(
            "Network.setCookie",
            json!({
                "name": name.trim(),
                "value": value.trim(),
                "domain": ".framer.com",
                "path": "/",
                "secure": true,
                "httpOnly": true,
                "sameSite": "None",
            }),
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
        let inst = self.instance(account).await?;
        let mut g = inst.lock().await;
        let v = g.eval_long(ai_post_expr(path, bearer, body)).await?;
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

    async fn rotate_identity(&self, account: &AccountId) -> Result<(), String> {
        // Drop the live instance (kills chrome), wipe the profile. Next mint
        // recreates the profile; the caller pairs this with a fresh
        // castle_device_id. The stored session survives — the browser is only
        // the minting device.
        if let Some(inst) = self.instances.lock().await.remove(account) {
            let mut g = inst.lock().await;
            if let Some(c) = g.child.as_mut() {
                let _ = c.start_kill();
            }
        }
        let profile = self.profile_dir(account);
        if profile.exists() {
            tokio::fs::remove_dir_all(&profile).await.map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    async fn auth_begin(&self, account: &AccountId, email: &str) -> Result<(), String> {
        let inst = self.instance(account).await?;
        let mut guard = inst.lock().await;
        let v = guard
            .eval(auth_post_expr("/auth/v4/signin", &json!({"email": email, "origin": "web"})))
            .await?;
        let status = v["status"].as_i64().unwrap_or(-1);
        match status {
            s if (200..300).contains(&s) => {
                eprintln!("[auth] signin accepted for account {}", account.0);
                Ok(())
            }
            0 => Err("castle v3 unavailable during signin (no token minted)".into()),
            s => Err(format!("signin {s}: {}", body_preview(&v))),
        }
    }

    async fn auth_complete(
        &self,
        account: &AccountId,
        email_token: &str,
    ) -> Result<(String, Value), String> {
        let inst = self.instance(account).await?;
        let mut guard = inst.lock().await;

        let v = guard
            .eval(auth_post_expr("/auth/v4/continue", &json!({"token": email_token})))
            .await?;
        let status = v["status"].as_i64().unwrap_or(-1);
        if !(200..300).contains(&status) {
            return Err(format!("continue {status}: {}", body_preview(&v)));
        }

        // The session cookie is httpOnly — only CDP can read it.
        let cookies = guard
            .cmd(
                "Network.getCookies",
                json!({"urls": ["https://framer.com", "https://api.framer.com", "https://framer.com/login"]}),
            )
            .await?;
        let session_value = cookies["cookies"]
            .as_array()
            .and_then(|a| a.iter().find(|c| c["name"].as_str() == Some("session")))
            .and_then(|c| c["value"].as_str())
            .ok_or_else(|| format!("no session cookie in browser jar after continue: {}", body_preview(&v)))?;
        let cookie_header = format!("session={session_value}");

        // /web/users/me in-page (cookie jar is live in this context).
        let v = guard.eval(page_get_expr("/web/users/me")).await?;
        let status = v["status"].as_i64().unwrap_or(-1);
        if !(200..300).contains(&status) {
            return Err(format!("me {status}: {}", body_preview(&v)));
        }
        let me: Value = serde_json::from_str(v["body"].as_str().unwrap_or("{}"))
            .map_err(|e| format!("me json: {e}"))?;
        eprintln!("[auth] login complete for account {}", account.0);
        Ok((cookie_header, me))
    }

    async fn set_proxy(&self, account: &AccountId, proxy: Option<String>) -> Result<(), String> {
        let key = account.0.to_string();
        match proxy {
            Some(p) => self.proxies.write().await.insert(key, p),
            None => self.proxies.write().await.remove(&key),
        };
        // Drop any live instance: the next use relaunches on the new egress.
        self.instances.lock().await.remove(account);
        Ok(())
    }

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

//! The binary: CLI + daemon wiring. Keeps Notionize's shape: daemon serves the
//! REST/SSE API + web UI; CLI subcommands for account management.

use clap::{Parser, Subcommand};
use framer_host::CdpBridge;
use framer_server::AppState;
use std::io::Write;
use std::sync::Arc;

#[derive(Parser)]
#[command(name = "framerize", about = "Multi-account Framer AI service")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Run the daemon (REST + SSE + web UI + /anthropic + /openai compat).
    Daemon {
        #[arg(long, default_value = "127.0.0.1:8787")]
        listen: String,
        #[arg(long, default_value = "web/dist")]
        web_dir: String,
        #[arg(long, default_value = "framerize.db")]
        db: String,
        /// Path to a Chrome/Chromium binary (the Castle engine per account).
        #[arg(long, env = "CHROME_PATH", default_value = "chrome")]
        chrome: String,
        /// Engine for account sessions: full-chrome (default, full desktop
        /// Chrome + stealth flags), headless-shell (minimal build — known-bad
        /// Castle fingerprint, fallback only), attach (drive your own Chrome
        /// at --chrome host:port; the best possible Castle score).
        #[arg(long, env = "FRAMERIZE_ENGINE", default_value = "full-chrome")]
        engine: String,
        /// One browser profile directory per account lives under here.
        #[arg(long, default_value = "profiles")]
        profiles: String,
    },
    /// Log in a Framer account: sends the magic-link email, then prompts for
    /// the #token from the emailed link. Talks to the running daemon.
    #[command(alias = "account-add")]
    Login {
        /// Framer account email (prompted if omitted).
        email: Option<String>,
        /// Daemon base URL.
        #[arg(long, env = "FRAMERIZE_URL", default_value = "http://127.0.0.1:8787")]
        daemon: String,
    },
    /// List accounts and their state.
    Accounts {
        /// Daemon base URL.
        #[arg(long, env = "FRAMERIZE_URL", default_value = "http://127.0.0.1:8787")]
        daemon: String,
    },
}

fn build_state(db: &str, chrome: &str, profiles: &str, engine: &str) -> Result<AppState, String> {
    // Create our own dirs — a first run must not fail because
    // ~/.framerize doesn't exist yet (observed on a fresh Linux install).
    if let Some(parent) = std::path::Path::new(db).parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("create db dir: {e}"))?;
    }
    std::fs::create_dir_all(profiles).map_err(|e| format!("create profiles dir: {e}"))?;
    let state = Arc::new(framer_store::SqliteStateStore::open(db)?);
    // File-based session store: daemons run headless (no dbus/keychain).
    // Sessions live 0600 under <profiles>/.sessions/.
    let sessions = Arc::new(framer_store::FileSessionStore::new(
        std::path::Path::new(profiles).join(".sessions"),
    )?);
    let client = Arc::new(framer_api::HttpFramerClient::new());
    let kind = match engine {
        "full-chrome" => framer_host::EngineKind::FullChrome,
        "headless-shell" => framer_host::EngineKind::HeadlessShell,
        "attach" => framer_host::EngineKind::Attach,
        other => return Err(format!("unknown --engine {other} (full-chrome|headless-shell|attach)")),
    };
    let browser = Arc::new(CdpBridge::with_engine(chrome, profiles, kind));
    Ok(AppState { client, browser, sessions, state })
}

/// Per-account egress proxies from persisted account records into the bridge
/// (Castle binds tokens to egress IP — each account needs its own network).
async fn load_proxies(state: &AppState) {
    if let Ok(accounts) = state.state.list().await {
        for a in accounts.into_iter().filter(|a| a.proxy.is_some()) {
            // set_proxy drops any live browser instance so it relaunches on
            // the new egress.
            let _ = state.browser.set_proxy(&a.id, a.proxy.clone()).await;
        }
    }
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Daemon { listen, web_dir, db, chrome, engine, profiles } => {
            let state = match build_state(&db, &chrome, &profiles, &engine) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("failed to initialize: {e}");
                    std::process::exit(1);
                }
            };
            load_proxies(&state).await;
            let addr = match listen.parse() {
                Ok(a) => a,
                Err(e) => {
                    eprintln!("bad --listen {listen}: {e}");
                    std::process::exit(1);
                }
            };
            eprintln!("framerize daemon on {listen} (web: {web_dir}, db: {db})");
            if let Err(e) = framer_server::serve(state, &web_dir, addr).await {
                eprintln!("server error: {e}");
                std::process::exit(1);
            }
        }
        Cmd::Login { email, daemon } => {
            if let Err(e) = cli_login(&daemon, email).await {
                eprintln!("\x1b[31m✗\x1b[0m {e}");
                std::process::exit(1);
            }
        }
        Cmd::Accounts { daemon } => {
            if let Err(e) = cli_accounts(&daemon).await {
                eprintln!("\x1b[31m✗\x1b[0m {e}");
                std::process::exit(1);
            }
        }
    }
}

fn prompt(label: &str) -> Result<String, String> {
    eprint!("\x1b[36m?\x1b[0m \x1b[1m{label}\x1b[0m ");
    std::io::stderr().flush().ok();
    let mut s = String::new();
    std::io::stdin().read_line(&mut s).map_err(|e| e.to_string())?;
    Ok(s.trim().to_string())
}

fn http_err(where_: &str, status: reqwest::StatusCode, body: String) -> String {
    format!("{where_} → HTTP {status}: {body}")
}

/// Interactive magic-link login against the running daemon.
async fn cli_login(daemon: &str, email: Option<String>) -> Result<(), String> {
    eprintln!("\x1b[1m◆ framerize login\x1b[0m");
    let email = match email {
        Some(e) => e,
        None => prompt("Framer email:")?,
    };
    if email.is_empty() || !email.contains('@') {
        return Err("that doesn't look like an email address".into());
    }
    let account_id = uuid::Uuid::new_v4().to_string();
    let http = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(120))
        .build()
        .map_err(|e| e.to_string())?;

    eprintln!("\x1b[90m● sending magic link (the account's browser starts, mints Castle, signs in)…\x1b[0m");
    let r = http
        .post(format!("{daemon}/api/accounts/begin"))
        .json(&serde_json::json!({ "account_id": account_id, "email": email }))
        .send()
        .await
        .map_err(|e| {
            if e.is_connect() {
                format!("daemon unreachable at {daemon} — start it first: framerize daemon &  (or scripts/linux/start-daemon.sh)")
            } else {
                format!("begin request failed: {e}")
            }
        })?;
    let status = r.status();
    let body = r.text().await.unwrap_or_default();
    if !status.is_success() {
        if body.contains("Access denied by policy") || body.contains("Verification token") {
            return Err(format!(
                r#"Castle refused sign-in from this network ({body}).
  This egress IP is reputation-flagged (typical for cloud containers).
  Options: run on a residential network, or import a live session instead:
  curl -X POST {daemon}/api/accounts/import -H 'content-type: application/json' \
    -d '{{"account_id":"<uuid>","session_cookie":"session=<value>","alias":"main","project_id":"<20-char-project>"}}'"#
            ));
        }
        return Err(http_err("begin", status, body));
    }
    eprintln!("\x1b[32m✓\x1b[0m magic link sent to \x1b[1m{email}\x1b[0m — open the newest email, copy the link");
    eprintln!("  \x1b[90maccount id: {account_id}\x1b[0m");

    let mut token = prompt("paste the link or just the #token= UUID:")?;
    if let Some((_, t)) = token.rsplit_once("#token=") {
        token = t.to_string();
    }
    if token.is_empty() {
        return Err("empty token".into());
    }

    eprintln!("\x1b[90m● completing login…\x1b[0m");
    let r = http
        .post(format!("{daemon}/api/accounts/complete"))
        .json(&serde_json::json!({ "account_id": account_id, "email_token": token }))
        .send()
        .await
        .map_err(|e| format!("complete request failed: {e}"))?;
    let status = r.status();
    let body: serde_json::Value = r.json().await.map_err(|e| e.to_string())?;
    if !status.is_success() {
        return Err(format!("complete → HTTP {status}: {body}"));
    }
    eprintln!("\x1b[32m✓ signed in\x1b[0m  {}", serde_json::to_string_pretty(&body).unwrap_or_default());
    eprintln!("\x1b[90m\x1b[0m");
    eprintln!("next: \x1b[1mfc\x1b[0m  (Claude Code on the free route) — or framerize accounts");
    Ok(())
}

/// List accounts from the running daemon, human-readable.
async fn cli_accounts(daemon: &str) -> Result<(), String> {
    let r = reqwest::Client::new()
        .get(format!("{daemon}/api/accounts"))
        .send()
        .await
        .map_err(|e| {
            if e.is_connect() {
                format!("daemon unreachable at {daemon} — start it first: framerize daemon &")
            } else {
                e.to_string()
            }
        })?;
    let accounts: serde_json::Value = r.json().await.map_err(|e| e.to_string())?;
    let list = accounts.as_array().ok_or("unexpected response")?;
    if list.is_empty() {
        eprintln!("no accounts yet — \x1b[1mframerize login\x1b[0m to add one");
        return Ok(());
    }
    for a in list {
        let alias = a["alias"].as_str().unwrap_or("?");
        let id = &a["id"].as_str().unwrap_or("?")[..8.min(a["id"].as_str().unwrap_or("?").len())];
        let state = a["state"].as_str().unwrap_or("?");
        let credits = a["credit"]["available"].as_f64()
            .map(|c| format!("{c:.0} credits"))
            .unwrap_or_else(|| "credits unknown".into());
        let proxy = if a["proxy"].is_null() { "direct" } else { "proxied" };
        let color = match state { "ready" => "32", "cooling_down" => "33", _ => "31" };
        eprintln!("  \x1b[{color}m●\x1b[0m \x1b[1m{alias}\x1b[0m \x1b[90m{id}…\x1b[0m  {state} · {credits} · {proxy}");
    }
    Ok(())
}


//! The binary: CLI + daemon wiring. Keeps Notionize's shape: daemon serves the
//! REST/SSE API + web UI; CLI subcommands for account management.

use clap::{Parser, Subcommand};
use framer_core::ids::AccountId;
use framer_host::CdpBridge;
use framer_server::AppState;
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
    /// Add an account: sends the Framer magic-link email, then prompts for the
    /// #token from the emailed link.
    AccountAdd { email: String },
    /// List accounts and their state.
    Accounts,
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
        Cmd::AccountAdd { email } => {
            let account = AccountId::new();
            eprintln!("account add for {email} (id {account:?}): use POST /api/accounts/begin-login then /api/accounts/complete-login against the daemon");
        }
        Cmd::Accounts => {
            eprintln!("accounts: query GET /api/accounts on the running daemon");
        }
    }
}

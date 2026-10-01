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

fn build_state(db: &str, chrome: &str, profiles: &str) -> Result<AppState, String> {
    let state = Arc::new(framer_store::SqliteStateStore::open(db)?);
    // File-based session store: daemons run headless (no dbus/keychain).
    // Sessions live 0600 under <profiles>/.sessions/.
    let sessions = Arc::new(framer_store::FileSessionStore::new(
        std::path::Path::new(profiles).join(".sessions"),
    )?);
    let client = Arc::new(framer_api::HttpFramerClient::new());
    let browser = Arc::new(CdpBridge::new(chrome, profiles));
    // Per-account egress proxies from persisted account records (Castle binds
    // tokens to egress IP — each account needs its own clean network).
    if let Ok(accounts) = state.list().await {
        let map: std::collections::HashMap<String, String> = accounts
            .into_iter()
            .filter_map(|a| a.proxy.map(|p| (a.id.0.to_string(), p)))
            .collect();
        browser.set_proxies(map).await;
    }
    Ok(AppState { client, browser, sessions, state })
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Daemon { listen, web_dir, db, chrome, profiles } => {
            let state = match build_state(&db, &chrome, &profiles) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("failed to initialize: {e}");
                    std::process::exit(1);
                }
            };
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

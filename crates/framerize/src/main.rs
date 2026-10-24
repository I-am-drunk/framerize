//! The binary: CLI + daemon wiring. Keeps Notionize's shape: daemon serves the
//! REST/SSE API + web UI; CLI subcommands for account management.

use clap::{Parser, Subcommand};
use framer_host::CdpBridge;
use framer_server::AppState;
use std::io::Write;
use std::sync::Arc;

mod allocator_reclaim;
mod allocator_reclaim_task;
mod cli_control;
mod startup;
use startup::start_prewarm;
#[cfg(feature = "native-dispatch")]
mod observed_authorization;
#[cfg(feature = "native-dispatch")]
mod one_shot_control;
#[cfg(feature = "native-dispatch")]
mod resident_control;
#[cfg(feature = "native-dispatch")]
mod resident_socket;
#[cfg(feature = "native-dispatch")]
extern crate offline_worker_owner;
#[cfg(feature = "native-dispatch")]
mod offline_worker;
#[cfg(feature = "native-dispatch")]
mod offline_source;
#[cfg(feature = "native-dispatch")]
mod offline_device;
#[cfg(feature = "native-dispatch")]
mod offline_serving;
#[cfg(feature = "native-dispatch")]
mod offline_sessions;
#[cfg(feature = "native-dispatch")]
mod offline_routing;
#[cfg(feature = "native-dispatch")]
mod offline_request_context;
#[cfg(feature = "native-dispatch")]
mod offline_routing_config;
#[cfg(feature = "native-dispatch")]
mod offline_config;
#[cfg(feature = "native-dispatch")]
mod offline_http;
#[cfg(feature = "native-dispatch")]
mod retirable_assembly;
#[cfg(feature = "native-dispatch")]
mod retirable_state;

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
        /// Engine for account sessions: camoufox (stealth Firefox sidecar —
        /// the engine that passed Castle in production), full-chrome (default;
        /// desktop Chrome + stealth flags), headless-shell (known-bad Castle
        /// fingerprint, fallback only), attach (drive your own Chrome at
        /// --chrome host:port; the best possible Castle score).
        #[arg(long, env = "FRAMERIZE_ENGINE", default_value = "full-chrome")]
        engine: String,
        /// One browser profile directory per account lives under here.
        #[arg(long, default_value = "profiles")]
        profiles: String,
        /// Prepared host for this instance; bypasses the shared home-directory host.
        #[arg(long)]
        sidecar_script: Option<std::path::PathBuf>,
        /// Maximum resident account browsers; saved accounts remain available.
        #[arg(long, env = "FRAMERIZE_ACTIVE_POOL_SIZE")]
        active_pool_size: Option<usize>,
        /// Eligible accounts: all, none, or comma-separated full UUIDs.
        #[arg(long)]
        eligible_accounts: Option<String>,
        /// Track browser request cleanup without enabling native routing.
        #[arg(long)]
        #[cfg_attr(feature = "native-dispatch", arg(conflicts_with_all = [
            "offline_worker_config", "offline_routing_config",
            "resident_control_socket", "resident_v2_worker_config"
        ]))]
        browser_request_ownership: bool,
        /// Enable the explicit offline complete-JSON endpoint with pinned runtime inputs.
        #[cfg(feature = "native-dispatch")]
        #[arg(long)]
        offline_worker_config: Option<String>,
        /// Route compatible client requests using explicit per-request context.
        #[cfg(feature = "native-dispatch")]
        #[arg(long)]
        offline_routing_config: Option<String>,
        /// Enable private resident crossover control on a local Unix socket.
        #[cfg(feature = "native-dispatch")]
        #[arg(long, conflicts_with_all = ["offline_worker_config", "offline_routing_config"])]
        resident_control_socket: Option<std::path::PathBuf>,
        /// Use the explicit V2 worker only for the private resident control.
        #[cfg(feature = "native-dispatch")]
        #[arg(long, requires = "resident_control_socket",
            conflicts_with_all = ["offline_worker_config", "offline_routing_config"])]
        resident_v2_worker_config: Option<String>,
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
    /// Query the structured event log (docs/logging.md).
    Logs {
        /// How many matching events to show (newest last).
        #[arg(long, default_value = "50")]
        tail: usize,
        /// Filter: event name contains this (e.g. ai.chain, login, castle).
        #[arg(long)]
        event: Option<String>,
        /// Filter: account id prefix.
        #[arg(long)]
        account: Option<String>,
        /// Aggregate counts + avg durations by event/outcome.
        #[arg(long)]
        stats: bool,
        /// Credit accounting view: AI events with metered_chunks + credits fields.
        #[arg(long)]
        credits: bool,
        /// Event log directory.
        #[arg(long, env = "FRAMERIZE_LOG_DIR", default_value = "~/.framerize/profiles/.logs")]
        log_dir: String,
    },
    /// List accounts and their state.
    #[command(alias = "account")]
    Accounts(cli_control::AccountsArgs),
    /// Inspect and change the live pool without editing service files.
    Pool(cli_control::PoolArgs),
    /// Call any daemon API, streaming the response to standard output.
    Api(cli_control::ApiArgs),
}

struct BuiltState {
    state: AppState,
    #[cfg(feature = "native-dispatch")]
    resident: Option<Arc<resident_control::StartupOwner>>,
}

fn configure_browser_request_ownership(state: AppState, enabled: bool)
    -> Result<AppState, String> {
    if !enabled { return Ok(state); }
    #[cfg(feature = "native-dispatch")]
    if state.native_dispatch.is_some() {
        return Err("browser request ownership requires browser routing".into());
    }
    framer_server::operation_ownership::with_owned_operations(state, 256)
        .map_err(|error| format!("browser request ownership unavailable: {error:?}"))
}

#[cfg(test)]
#[path = "browser_request_ownership_startup_tests.rs"]
mod browser_request_ownership_startup_tests;

async fn build_state(db: &str, chrome: &str, profiles: &str, engine: &str,
    active_pool_size: Option<usize>, eligible_accounts: Option<&str>,
    sidecar_script: Option<std::path::PathBuf>,
    browser_request_ownership: bool,
    #[cfg(feature = "native-dispatch")] resident_controls: bool,
    #[cfg(feature = "native-dispatch")] resident_sdk: Option<Arc<offline_worker_owner::V2Worker>>,
) -> Result<BuiltState, String> {
    if sidecar_script.is_some() && engine != "camoufox" {
        return Err("--sidecar-script requires --engine camoufox".into());
    }
    #[cfg(feature = "native-dispatch")]
    if resident_controls && engine != "camoufox" {
        return Err("resident controls require --engine camoufox".into());
    }
    // Parse once before opening stores or constructing any browser bridge.
    let startup_policy = framer_server::ServingAccountPolicy::from_env()?;
    // Create our own dirs — a first run must not fail because
    // ~/.framerize doesn't exist yet (observed on a fresh Linux install).
    if let Some(parent) = std::path::Path::new(db).parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("create db dir: {e}"))?;
    }
    std::fs::create_dir_all(profiles).map_err(|e| format!("create profiles dir: {e}"))?;
    let state = Arc::new(framer_store::SqliteStateStore::open(db)?);
    use framer_core::StateStore;
    let saved = state.setting(framer_server::POOL_SETTING_KEY).await?;
    let pool_config = resolve_pool_config(startup_policy.config(), saved.as_deref(),
        active_pool_size, eligible_accounts)?;
    let serving_accounts = Arc::new(framer_server::ServingAccountPolicy::from_config(pool_config)?);
    // File-based session store: daemons run headless (no dbus/keychain).
    // Sessions live 0600 under <profiles>/.sessions/.
    let sessions = Arc::new(framer_store::FileSessionStore::new(
        std::path::Path::new(profiles).join(".sessions"),
    )?);
    let client = Arc::new(framer_api::HttpFramerClient::new());
    framer_core::observability::init(profiles);
    #[cfg(feature = "native-dispatch")]
    let mut resident_bridge = None;
    let browser: Arc<dyn framer_core::port::BrowserBridge> = match engine {
        "full-chrome" => Arc::new(CdpBridge::with_engine(chrome, profiles, framer_host::EngineKind::FullChrome)),
        "headless-shell" => Arc::new(CdpBridge::with_engine(chrome, profiles, framer_host::EngineKind::HeadlessShell)),
        "attach" => Arc::new(CdpBridge::with_engine(chrome, profiles, framer_host::EngineKind::Attach)),
        "camoufox" => {
            let bridge = framer_host::sidecar::SidecarBridge::new(
                framer_host::sidecar::default_sidecar_root(profiles));
            let bridge = match sidecar_script {
                Some(script) => bridge.with_script(script)?,
                None => bridge,
            };
            #[cfg(feature = "native-dispatch")]
            let bridge = {
                let mut bridge = bridge;
                if resident_controls {
                    let generation = (uuid::Uuid::new_v4().as_u128() as u64).max(1);
                    bridge.configure_resident_controls(
                        framer_host::sidecar::observer::ApplicationGeneration(generation))?;
                }
                bridge
            };
            let bridge = Arc::new(bridge);
            #[cfg(feature = "native-dispatch")]
            if resident_controls { resident_bridge = Some(bridge.clone()); }
            bridge
        },
        other => return Err(format!("unknown --engine {other} (full-chrome|headless-shell|attach|camoufox)")),
    };
    let state = AppState {
        operation_ownership: None,
        #[cfg(feature = "native-dispatch")]
        native_dispatch: None,
        client,
        browser,
        sessions,
        state,
        gate: std::sync::Arc::new(framer_core::gate::ConcurrencyGate::new()),
        wave: std::sync::Arc::new(framer_core::wave::WaveGuard::new()),
        serving_accounts,
    };
    let state = configure_browser_request_ownership(state, browser_request_ownership)?;
    #[cfg(feature = "native-dispatch")]
    let (state, resident) = match resident_bridge {
        Some(bridge) => {
            let mut state = framer_server::operation_ownership::with_owned_operations(state, 256)
                .map_err(|_| "resident application ownership unavailable".to_owned())?;
            let owner = resident_control::StartupOwner::install_with_sdk(&mut state, bridge, resident_sdk)
                .map_err(|_| "resident session ownership unavailable".to_owned())?;
            (state, Some(owner))
        }
        None => (state, None),
    };
    Ok(BuiltState { state, #[cfg(feature = "native-dispatch")] resident })
}

/// Apply explicit startup flags after the saved runtime configuration.
fn resolve_pool_config(default: framer_server::PoolConfig, saved: Option<&str>,
    capacity: Option<usize>, eligible: Option<&str>) -> Result<framer_server::PoolConfig, String> {
    let mut config = match saved {
        Some(json) => serde_json::from_str::<framer_server::PoolConfig>(json)
            .map_err(|error| format!("invalid saved pool configuration: {error}"))?,
        None => default,
    };
    if let Some(capacity) = capacity { config.active_capacity = capacity; }
    if let Some(eligible) = eligible {
        config.eligible_accounts = match eligible.trim() {
            "all" => framer_server::EligibleAccounts::All(framer_server::AllAccounts::All),
            "none" => framer_server::EligibleAccounts::Only(Vec::new()),
            ids => framer_server::ServingAccountPolicy::parse(Some(ids))?.config().eligible_accounts,
        };
    }
    config.validate()?;
    Ok(config)
}

#[cfg(test)]
mod pool_startup_tests;
#[cfg(all(test, feature = "native-dispatch"))]
mod resident_startup_tests;

/// Register saved per-account proxies before any browser can start.
async fn load_proxies(state: &AppState) -> Result<(), String> {
    let accounts = state.state.list().await?;
    for a in accounts.into_iter().filter(|a| a.proxy.is_some()) {
        state.browser.register_account_proxy(&a.id, a.proxy.clone()).await
            .map_err(|_| format!("could not configure proxy for account {}", a.id.0))?;
    }
    Ok(())
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Daemon { listen, web_dir, db, chrome, engine, profiles, sidecar_script,
            active_pool_size, eligible_accounts, browser_request_ownership,
            #[cfg(feature = "native-dispatch")] offline_worker_config,
            #[cfg(feature = "native-dispatch")] offline_routing_config,
            #[cfg(feature = "native-dispatch")] resident_control_socket,
            #[cfg(feature = "native-dispatch")] resident_v2_worker_config } => {
            #[cfg(feature = "native-dispatch")]
            if resident_control_socket.is_some()
                && (offline_worker_config.is_some() || offline_routing_config.is_some()) {
                eprintln!("resident controls require exclusive startup session ownership");
                std::process::exit(1);
            }
            #[cfg(feature = "native-dispatch")]
            let bound_resident_socket = match resident_control_socket.as_deref()
                .map(resident_socket::BoundSocket::bind).transpose() {
                Ok(socket) => socket,
                Err(_) => {
                    eprintln!("resident control socket unavailable");
                    std::process::exit(1);
                }
            };
            #[cfg(feature = "native-dispatch")]
            let resident_sdk = match resident_v2_worker_config.as_deref()
                .map(offline_config::load_v2).transpose() {
                Ok(worker) => worker.map(Arc::new),
                Err(error) => {
                    eprintln!("failed to initialize: {error}");
                    std::process::exit(1);
                }
            };
            let built = match build_state(&db, &chrome, &profiles, &engine,
                active_pool_size, eligible_accounts.as_deref(), sidecar_script,
                browser_request_ownership,
                #[cfg(feature = "native-dispatch")] resident_control_socket.is_some(),
                #[cfg(feature = "native-dispatch")] resident_sdk,
            ).await {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("failed to initialize: {e}");
                    #[cfg(feature = "native-dispatch")]
                    drop(bound_resident_socket);
                    std::process::exit(1);
                }
            };
            let state = built.state;
            #[cfg(feature = "native-dispatch")]
            let resident = built.resident;
            #[cfg(feature = "native-dispatch")]
            let (state, offline_worker) = match offline_http::configure(
                state, offline_worker_config.as_deref(), offline_routing_config.as_deref()) {
                Ok(value) => value,
                Err(error) => {
                    eprintln!("failed to initialize: {error}");
                    drop(bound_resident_socket);
                    std::process::exit(1);
                }
            };
            let _memory_task = allocator_reclaim_task::start();
            let proxy_state = state.clone();
            let proxies = framer_server::operation_ownership::run(
                state.operation_ownership.as_ref(),
                framer_core::operation_gate::OperationKind::Background,
                async move { load_proxies(&proxy_state).await },
            ).await.map_err(|error| format!("startup ownership unavailable: {error:?}"))
                .and_then(|result| result);
            if let Err(error) = proxies {
                eprintln!("failed to initialize: {error}");
                #[cfg(feature = "native-dispatch")]
                drop(bound_resident_socket);
                std::process::exit(1);
            }
            if let Err(error) = start_prewarm(&state) {
                eprintln!("{error}");
                #[cfg(feature = "native-dispatch")]
                drop(bound_resident_socket);
                std::process::exit(1);
            }
            let addr = match listen.parse() {
                Ok(a) => a,
                Err(e) => {
                    eprintln!("bad --listen {listen}: {e}");
                    #[cfg(feature = "native-dispatch")]
                    drop(bound_resident_socket);
                    std::process::exit(1);
                }
            };
            eprintln!("framerize daemon on {listen} (web: {web_dir}, db: {db})");
            framer_core::observability::Event::new("daemon.start")
                .field("listen", listen.clone())
                .field("engine", engine.clone())
                .emit();
            #[cfg(feature = "native-dispatch")]
            let result = match (resident, bound_resident_socket) {
                (Some(owner), Some(socket)) =>
                    resident_socket::serve(state, owner, socket, &web_dir, addr).await,
                (None, None) => match offline_worker {
                    Some(worker) => offline_http::serve(state, worker, &web_dir, addr).await,
                    None => framer_server::serve(state, &web_dir, addr).await,
                },
                _ => Err(std::io::Error::other("resident startup binding incomplete")),
            };
            #[cfg(not(feature = "native-dispatch"))]
            let result = framer_server::serve(state, &web_dir, addr).await;
            if let Err(e) = result {
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
        Cmd::Logs { tail, event, account, stats, credits, log_dir } => {
            cli_logs(&log_dir, tail, event.as_deref(), account.as_deref(), stats, credits);
        }
        Cmd::Accounts(args) => cli_result(cli_control::run_accounts(args).await),
        Cmd::Pool(args) => cli_result(cli_control::run_pool(args).await),
        Cmd::Api(args) => cli_result(cli_control::run_api(args).await),
    }
}

fn cli_result(result: Result<(), String>) {
    if let Err(error) = result {
        eprintln!("{error}");
        std::process::exit(1);
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
    let text = r.text().await.unwrap_or_default();
    if !status.is_success() {
        // The daemon returns plain text on failure — show it verbatim.
        return Err(format!("complete → HTTP {status}: {text}"));
    }
    let body: serde_json::Value = serde_json::from_str(&text).unwrap_or(serde_json::json!({"raw": text}));
    eprintln!("\x1b[32m✓ signed in\x1b[0m  {}", serde_json::to_string_pretty(&body).unwrap_or_default());
    eprintln!("\x1b[90m\x1b[0m");
    eprintln!("next: \x1b[1mfcd\x1b[0m  (Claude Code on the free route) — or framerize accounts");
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



// ---------- structured log query (docs/logging.md) ----------

fn cli_logs(log_dir: &str, tail: usize, event: Option<&str>, account: Option<&str>, stats: bool, credits: bool) {
    let dir = std::path::PathBuf::from(log_dir.replace('~', &std::env::var("HOME").unwrap_or_default()));
    let mut files: Vec<_> = std::fs::read_dir(&dir)
        .map(|rd| {
            rd.flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("ndjson"))
                .collect()
        })
        .unwrap_or_default();
    files.sort();
    let mut events: Vec<serde_json::Value> = Vec::new();
    for f in files.iter().rev().take(3) {
        // newest files first, but collect then re-sort by ts
        if let Ok(text) = std::fs::read_to_string(f) {
            for line in text.lines() {
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(line) {
                    events.push(v);
                }
            }
        }
    }
    if events.is_empty() {
        eprintln!("no events in {dir:?} yet — the daemon logs there as it works");
        return;
    }
    events.sort_by_key(|v| v["ts"].as_str().unwrap_or("").to_string());
    let matches = |v: &&serde_json::Value| -> bool {
        if let Some(e) = event {
            if !v["event"].as_str().unwrap_or("").contains(e) {
                return false;
            }
        }
        if let Some(a) = account {
            if !v["account"].as_str().unwrap_or("").starts_with(a) {
                return false;
            }
        }
        if credits && !v["event"].as_str().unwrap_or("").starts_with("ai.") {
            return false;
        }
        true
    };
    let filtered: Vec<&serde_json::Value> = events.iter().filter(matches).collect();

    if stats {
        use std::collections::BTreeMap;
        let mut by: BTreeMap<String, (u64, u64, u128)> = BTreeMap::new(); // (ok, err, total_ms)
        for v in &filtered {
            let key = v["event"].as_str().unwrap_or("?").to_string();
            let e = by.entry(key).or_default();
            if v["outcome"].as_str() == Some("ok") { e.0 += 1; } else { e.1 += 1; }
            e.2 += v["duration_ms"].as_u64().unwrap_or(0) as u128;
        }
        eprintln!("\x1b[1m{:24} {:>6} {:>6} {:>10}\x1b[0m", "event", "ok", "err", "avg_ms");
        for (k, (ok, err, ms)) in by {
            let n = (ok + err).max(1) as u128;
            eprintln!("{k:24} {ok:>6} {err:>6} {:>10}", ms / n);
        }
        return;
    }

    for v in filtered.iter().rev().take(tail).rev() {
        let ts = &v["ts"].as_str().unwrap_or("")[11..19.min(v["ts"].as_str().unwrap_or("").len())];
        let outcome = v["outcome"].as_str().unwrap_or("-");
        let color = match outcome { "ok" => "32", "error" => "31", _ => "33" };
        let dur = v["duration_ms"].as_u64().map(|d| format!(" {:.1}s", d as f64 / 1000.0)).unwrap_or_default();
        let mut extras = String::new();
        if let Some(obj) = v.as_object() {
            for (k, val) in obj {
                if matches!(k.as_str(), "ts" | "level" | "event" | "request_id" | "outcome" | "duration_ms" | "source") {
                    continue;
                }
                let sval = match val {
                    serde_json::Value::String(s) => s.clone(),
                    other => other.to_string(),
                };
                extras.push_str(&format!(" {k}={sval}"));
            }
        }
        eprintln!("{ts} \x1b[{color}m{outcome:>7}\x1b[0m \x1b[1m{}\x1b[0m{dur}{extras}", v["event"].as_str().unwrap_or("?"));
    }
}

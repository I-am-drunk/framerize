//! Standalone native startup. Both stores stay owned through final cleanup.
use super::{http, state::ServiceState};
use crate::{account::ledger::LedgerOwner, worker::backend::WorkerBackend};
use framer_core::{ids::AccountId, port::StateStore};
use framer_store::{FileSessionStore, SqliteStateStore};
use crate::authorization::control::BrowserCredentialControl;
use futures::FutureExt;
use std::{future::Future, net::SocketAddr, panic::AssertUnwindSafe, path::PathBuf, sync::Arc};

#[derive(Clone)]
pub struct ServeConfig {
    pub database: PathBuf,
    pub profiles: PathBuf,
    pub python: PathBuf,
    pub runtime_script: PathBuf,
    pub worker_config: String,
    pub selected_accounts: Vec<String>,
    pub address: SocketAddr,
    /// Explicit first migration; ignored once durable history exists.
    pub expire_legacy_from: Option<PathBuf>,
}

pub async fn serve(config: ServeConfig, shutdown: impl Future<Output = ()> + Send + 'static)
    -> Result<(), String> {
    serve_using(config, None, None, shutdown).await
}

pub async fn serve_with_browser(config: ServeConfig,
    browser: Arc<dyn crate::transport::browser::BrowserTransport>,
    shutdown: impl Future<Output = ()> + Send + 'static) -> Result<(), String> {
    serve_using(config, Some(browser), None, shutdown).await
}

/// Explicit single-use diagnostic producer, independent of sender selection.
/// The peer must be inert until prepare and retain its control until finish.
pub async fn serve_with_browser_control(config: ServeConfig,
    control: Arc<dyn BrowserCredentialControl>,
    browser: Option<Arc<dyn crate::transport::browser::BrowserTransport>>,
    shutdown: impl Future<Output = ()> + Send + 'static) -> Result<(), String> {
    serve_using(config, browser, Some(control), shutdown).await
}

async fn serve_using(config: ServeConfig,
    browser: Option<Arc<dyn crate::transport::browser::BrowserTransport>>,
    control: Option<Arc<dyn BrowserCredentialControl>>,
    shutdown: impl Future<Output = ()> + Send + 'static) -> Result<(), String> {
    let shutdown = shutdown.boxed().shared();
    if !config.address.ip().is_loopback() { return Err("native service requires a loopback address".into()); }
    let selected = config.selected_accounts.iter().map(|value|
        value.parse::<uuid::Uuid>().map(AccountId).map_err(|_| "invalid selected account"))
        .collect::<Result<Vec<_>, _>>()?;
    if selected.is_empty() || selected.len() > 64
        || selected.iter().collect::<std::collections::HashSet<_>>().len() != selected.len() {
        return Err("select 1 to 64 distinct native accounts".into());
    }
    if control.is_some() && selected.len() != 1 {
        return Err("browser credential control requires one selected account".into());
    }
    let profiles = config.profiles.canonicalize().map_err(|_| "existing profile required")?;
    let database = config.database.canonicalize().map_err(|_| "existing account database required")?;
    let history = profiles.join(".sidecar/camoufox");
    let sessions_path = profiles.join(".sessions");
    let needs_history = !history.join("rate-history.sqlite3").is_file();
    if !database.is_file() || !sessions_path.is_dir() || !history.is_dir()
        || (needs_history && config.expire_legacy_from.is_none()) {
        return Err("native startup requires existing sessions and migrated account history".into());
    }
    let worker = Arc::new(crate::worker::config::load(&config.worker_config)?);
    tokio::time::timeout(std::time::Duration::from_secs(30), worker.verify_inputs()).await
        .map_err(|_| "native worker verification timed out")?
        .map_err(|_| "native worker input verification failed")?;
    let worker = WorkerBackend::V3(worker);
    let sessions = Arc::new(FileSessionStore::new_bounded(
        sessions_path, crate::authorization::MAX_COOKIE_HEADER_BYTES)?);
    if shutdown.clone().now_or_never().is_some() { return Ok(()); }
    let database = database.to_str().ok_or("account database path is not UTF-8")?;
    let store = Arc::new(SqliteStateStore::open(database)?);
    validate_accounts(store.as_ref(), &selected).await?;
    framer_core::observability::init_path(&profiles);
    if needs_history {
        let previous = config.expire_legacy_from.clone().ok_or("legacy runtime is required")?;
        if !super::migration::expire(config.python.clone(), config.runtime_script.clone(),
            history.clone(), previous, sessions.clone(), shutdown.clone()).await? {
            return Ok(());
        }
        // The database may have changed during the legacy expiry interval.
        validate_accounts(store.as_ref(), &selected).await?;
    }
    if shutdown.clone().now_or_never().is_some() { return Ok(()); }
    let listener = tokio::net::TcpListener::bind(config.address).await
        .map_err(|_| "native listen address is unavailable")?;
    let ledger = Arc::new(LedgerOwner::start(&config.python, &config.runtime_script,
        &history, sessions.clone()).await.map_err(|_| "native history ownership unavailable")?);
    let result = AssertUnwindSafe(async {
        // Migration and the journal capture require the native owner to start
        // before unrelated child processes. Warm the sender before admission.
        if let Some(browser) = &browser {
            tokio::select! {
                _ = shutdown.clone() => return Ok(()),
                _ = ledger.wait_for_exit() => return Err("native history exited during browser preparation".into()),
                result = browser.prepare() => result.map_err(|_| "browser transport preparation failed".to_owned())?,
            }
        }
        if let Some(peer) = &control {
            let stop = async { tokio::select! {
                _ = shutdown.clone() => {}, _ = ledger.wait_for_exit() => {},
            }};
            if !prepare_control(peer.as_ref(), stop).await? { return Ok(()); }
        }
        let state = ServiceState::new(sessions, store, selected)?;
        let (state, runtime) = match &control {
            Some(peer) => http::configure_browser_control(state, worker, ledger.clone(), browser,
                peer.clone())?,
            None => http::configure_browser(state, worker, ledger.clone(), browser)?,
        };
        let monitor = ledger.clone();
        let stop = async move {
            tokio::select! {
                _ = shutdown => {},
                _ = monitor.wait_for_exit() => {},
            }
        };
        http::serve(state, runtime, listener, stop).await.map_err(|error| error.to_string())
    }).catch_unwind().await.unwrap_or_else(|_| Err("native service failed during startup or serving".into()));
    let control_closed = match &control {
        Some(peer) => AssertUnwindSafe(async { peer.shutdown().await })
            .catch_unwind().await.unwrap_or(false),
        None => true,
    };
    let ledger_closed = ledger.shutdown().await.is_ok();
    if !control_closed { return Err("browser credential control shutdown is unconfirmed".into()); }
    if !ledger_closed {
        return Err("native ledger shutdown is unconfirmed".into());
    }
    result
}

pub(super) async fn prepare_control(peer: &dyn BrowserCredentialControl,
    stop: impl Future<Output = ()>) -> Result<bool, String> {
    let preparing = peer.prepare();
    tokio::pin!(preparing);
    // Cancellation cannot discard an in-progress external preparation. The
    // peer owns its finite deadline; shutdown follows its actual settlement.
    let (ready, stopped) = tokio::select! {
        ready = &mut preparing => (ready, false),
        _ = stop => (preparing.await, true),
    };
    if !ready { return Err("browser credential control preparation failed".into()); }
    Ok(!stopped)
}

async fn validate_accounts(store: &dyn StateStore, selected: &[AccountId]) -> Result<(), String> {
    for account in selected {
        if store.get(*account).await?.is_none() {
            return Err("selected account is not in this profile's database".to_string());
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "startup_tests.rs"]
mod tests;

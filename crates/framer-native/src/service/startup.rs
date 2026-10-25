//! Standalone native startup. Both stores stay owned through final cleanup.
use super::{http, state::ServiceState};
use crate::{account::ledger::LedgerOwner, worker::backend::WorkerBackend};
use framer_core::{ids::AccountId, port::StateStore};
use framer_store::{FileSessionStore, SqliteStateStore};
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
}

pub async fn serve(config: ServeConfig, shutdown: impl Future<Output = ()> + Send + 'static)
    -> Result<(), String> {
    if !config.address.ip().is_loopback() { return Err("native service requires a loopback address".into()); }
    let selected = config.selected_accounts.iter().map(|value|
        value.parse::<uuid::Uuid>().map(AccountId).map_err(|_| "invalid selected account"))
        .collect::<Result<Vec<_>, _>>()?;
    if selected.is_empty() || selected.len() > 64
        || selected.iter().collect::<std::collections::HashSet<_>>().len() != selected.len() {
        return Err("select 1 to 64 distinct native accounts".into());
    }
    let profiles = config.profiles.canonicalize().map_err(|_| "existing profile required")?;
    let database = config.database.canonicalize().map_err(|_| "existing account database required")?;
    let history = profiles.join(".sidecar/camoufox");
    let sessions_path = profiles.join(".sessions");
    if !database.is_file() || !sessions_path.is_dir() || !history.join("rate-history.sqlite3").is_file() {
        return Err("native startup requires existing sessions and migrated account history".into());
    }
    let worker = Arc::new(crate::worker::config::load(&config.worker_config)?);
    tokio::time::timeout(std::time::Duration::from_secs(30), worker.verify_inputs()).await
        .map_err(|_| "native worker verification timed out")?
        .map_err(|_| "native worker input verification failed")?;
    let worker = WorkerBackend::V3(worker);
    let listener = tokio::net::TcpListener::bind(config.address).await
        .map_err(|_| "native listen address is unavailable")?;
    let sessions = Arc::new(FileSessionStore::new(sessions_path)?);
    let ledger = Arc::new(LedgerOwner::start(&config.python, &config.runtime_script,
        &history, sessions.clone()).await.map_err(|_| "native history ownership unavailable")?);
    let result = AssertUnwindSafe(async {
        let database = database.to_str().ok_or("account database path is not UTF-8")?;
        let store = Arc::new(SqliteStateStore::open(database)?);
        for account in &selected {
            if store.get(*account).await?.is_none() {
                return Err("selected account is not in this profile's database".to_string());
            }
        }
        let state = ServiceState::new(sessions, store, selected)?;
        let (state, runtime) = http::configure(state, worker, ledger.clone())?;
        http::serve(state, runtime, listener, shutdown).await.map_err(|error| error.to_string())
    }).catch_unwind().await.unwrap_or_else(|_| Err("native service failed during startup or serving".into()));
    if ledger.shutdown().await.is_err() {
        return Err("native ledger shutdown is unconfirmed".into());
    }
    result
}

#[cfg(test)]
#[path = "startup_tests.rs"]
mod tests;

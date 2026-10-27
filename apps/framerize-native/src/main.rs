//! Standalone native application. Browser operation stays in `framerize`.
use clap::{Parser, Subcommand, ValueEnum};
use std::{net::SocketAddr, path::PathBuf, process::ExitCode};
#[cfg(feature = "browser-transport")]
mod hybrid;

#[derive(Parser)]
#[command(version, about = "Native Framer runtime development tools")]
struct Cli { #[command(subcommand)] command: Command }

#[derive(Subcommand)]
enum Command {
    /// Serve ledger-backed V3 requests using an existing migrated profile.
    Serve {
        #[arg(long)] db: PathBuf,
        #[arg(long)] profiles: PathBuf,
        /// Explicit override; complete packages carry their own worker config.
        #[arg(long)] worker_config: Option<String>,
        #[arg(long, value_delimiter = ',', required = true)] accounts: Vec<String>,
        #[arg(long, default_value = "127.0.0.1:18789")] listen: SocketAddr,
        #[arg(long, default_value = "/usr/bin/python3")] python: PathBuf,
        /// Explicit development override; installed packages use their own runtime.
        #[arg(long)] runtime_script: Option<PathBuf>,
        /// Experimental supplied-token browser sender; requires browser-transport feature.
        #[arg(long)] browser_transport_script: Option<PathBuf>,
        /// Experimental browser credential source. This does not select the HTTP sender.
        #[arg(long, requires = "browser_control_sha256")]
        browser_control_descriptor: Option<PathBuf>,
        /// Exact SHA256 of the credential-free browser control descriptor.
        #[arg(long, requires = "browser_control_descriptor")]
        browser_control_sha256: Option<String>,
        /// First migration only: hold the profile idle for legacy rate expiry.
        /// The file identifies the previous invocation of this same service.
        #[arg(long)] expire_legacy_from: Option<PathBuf>,
    },
    /// Validate the structure of declared worker inputs without executing a worker or opening accounts.
    CheckConfig {
        #[arg(long)] config: String,
        #[arg(long, value_enum, default_value = "v3")] sdk: Sdk,
    },
    /// Check a pinned launch descriptor without opening accounts or starting a child.
    CheckBrowserControl {
        #[arg(long)] descriptor: PathBuf,
        #[arg(long)] sha256: String,
    },
}

#[derive(Clone, Copy, ValueEnum)]
enum Sdk { V2, V3 }

async fn run(db: PathBuf, profiles: PathBuf, worker_config: Option<String>, accounts: Vec<String>,
    listen: SocketAddr, python: PathBuf, runtime_script: Option<PathBuf>,
    expire_legacy_from: Option<PathBuf>, browser_transport_script: Option<PathBuf>,
    browser_control_descriptor: Option<PathBuf>, browser_control_sha256: Option<String>) -> Result<(), String> {
    use tokio::signal::unix::{signal, SignalKind};
    let control = match (browser_control_descriptor, browser_control_sha256) {
        (None, None) => None,
        (Some(path), Some(hash)) => Some(framer_native::authorization::control::load_browser_control(
            &path, &hash).map_err(str::to_owned)?),
        _ => return Err("browser control requires both descriptor and SHA256".into()),
    };
    let mut term = signal(SignalKind::terminate()).map_err(|e| e.to_string())?;
    let mut interrupt = signal(SignalKind::interrupt()).map_err(|e| e.to_string())?;
    let script = match runtime_script {
        Some(path) => path,
        None => {
            let executable = std::env::current_exe().map_err(|e| e.to_string())?
                .canonicalize().map_err(|e| e.to_string())?;
            executable.parent().and_then(|bin| bin.parent())
                .ok_or("native package directory unavailable")?.join("runtime/native.py")
        }
    }.canonicalize().map_err(|_| "native runtime is missing; use a complete package or --runtime-script")?;
    let worker_config = match worker_config {
        Some(path) => path,
        None => script.parent().ok_or("native runtime directory is unavailable")?
            .join("worker/worker-config.json").to_str()
            .ok_or("native worker path is not UTF-8")?.to_owned(),
    };
    let python = python.canonicalize().map_err(|_| "native Python executable is unavailable")?;
    let config = framer_native::ServeConfig { database: db, profiles, worker_config,
        selected_accounts: accounts, address: listen, python, runtime_script: script,
        expire_legacy_from };
    let stop = async move {
        tokio::select! { _ = term.recv() => {}, _ = interrupt.recv() => {} }
    };
    if let Some(script) = browser_transport_script {
        #[cfg(feature = "browser-transport")]
        return hybrid::run(config, script, control, stop).await;
        #[cfg(not(feature = "browser-transport"))]
        { let _ = script; return Err("rebuild with the browser-transport feature".into()); }
    }
    match control {
        Some(control) => framer_native::serve_with_browser_control(config, control, None, stop).await,
        None => framer_native::serve(config, stop).await,
    }
}

#[tokio::main]
async fn main() -> ExitCode {
    let result = match Cli::parse().command {
        Command::CheckConfig { config, sdk } => {
            framer_native::check_config(&config, matches!(sdk, Sdk::V2))
                .map(|()| println!("Worker configuration structure is valid. No dependencies were verified or executed."))
                .map_err(str::to_owned)
        }
        Command::CheckBrowserControl { descriptor, sha256 } => {
            framer_native::authorization::control::load_browser_control(&descriptor, &sha256)
                .map(|_| println!("Browser control descriptor structure and file hash are valid. Executables were not verified or started."))
                .map_err(str::to_owned)
        }
        Command::Serve { db, profiles, worker_config, accounts, listen, python, runtime_script,
            expire_legacy_from, browser_transport_script, browser_control_descriptor, browser_control_sha256 } => {
            run(db, profiles, worker_config, accounts, listen, python, runtime_script,
                expire_legacy_from, browser_transport_script, browser_control_descriptor,
                browser_control_sha256).await
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => { eprintln!("{error}"); ExitCode::FAILURE }
    }
}

#[cfg(test)]
mod control_cli_tests;

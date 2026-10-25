//! Standalone native application. Browser operation stays in `framerize`.
use clap::{Parser, Subcommand, ValueEnum};
use std::{net::SocketAddr, path::PathBuf, process::ExitCode};

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
        /// First migration only: hold the profile idle for legacy rate expiry.
        /// The file identifies the previous invocation of this same service.
        #[arg(long)] expire_legacy_from: Option<PathBuf>,
    },
    /// Validate the structure of declared worker inputs without executing a worker or opening accounts.
    CheckConfig {
        #[arg(long)] config: String,
        #[arg(long, value_enum, default_value = "v3")] sdk: Sdk,
    },
}

#[derive(Clone, Copy, ValueEnum)]
enum Sdk { V2, V3 }

async fn run(db: PathBuf, profiles: PathBuf, worker_config: Option<String>, accounts: Vec<String>,
    listen: SocketAddr, python: PathBuf, runtime_script: Option<PathBuf>,
    expire_legacy_from: Option<PathBuf>) -> Result<(), String> {
    use tokio::signal::unix::{signal, SignalKind};
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
    framer_native::serve(config, async move {
        tokio::select! { _ = term.recv() => {}, _ = interrupt.recv() => {} }
    }).await
}

#[tokio::main]
async fn main() -> ExitCode {
    let result = match Cli::parse().command {
        Command::CheckConfig { config, sdk } => {
            framer_native::check_config(&config, matches!(sdk, Sdk::V2))
                .map(|()| println!("Worker configuration structure is valid. No dependencies were verified or executed."))
                .map_err(str::to_owned)
        }
        Command::Serve { db, profiles, worker_config, accounts, listen, python, runtime_script,
            expire_legacy_from } => {
            run(db, profiles, worker_config, accounts, listen, python, runtime_script,
                expire_legacy_from).await
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => { eprintln!("{error}"); ExitCode::FAILURE }
    }
}

//! Developer entry point. Live account ownership is not yet integrated.
use clap::{Parser, Subcommand, ValueEnum};
use std::process::ExitCode;

#[derive(Parser)]
#[command(version, about = "Native Framer runtime development tools")]
struct Cli { #[command(subcommand)] command: Command }

#[derive(Subcommand)]
enum Command {
    /// Validate the structure of declared worker inputs without executing a worker or opening accounts.
    CheckConfig {
        #[arg(long)] config: String,
        #[arg(long, value_enum, default_value = "v3")] sdk: Sdk,
    },
}

#[derive(Clone, Copy, ValueEnum)]
enum Sdk { V2, V3 }

fn main() -> ExitCode {
    let Command::CheckConfig { config, sdk } = Cli::parse().command;
    match framer_native::check_config(&config, matches!(sdk, Sdk::V2)) {
        Ok(()) => { println!("Worker configuration structure is valid. No dependencies were verified or executed."); ExitCode::SUCCESS }
        Err(error) => { eprintln!("{error}"); ExitCode::FAILURE }
    }
}

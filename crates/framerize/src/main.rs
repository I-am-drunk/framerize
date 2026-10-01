//! The binary: CLI + daemon wiring. Keeps Notionize's shape: daemon serves the
//! REST/SSE API + web UI; CLI subcommands for account management.

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "framerize", about = "Multi-account Framer AI service")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Run the daemon (REST + SSE + web UI).
    Daemon {
        #[arg(long, default_value = "127.0.0.1:8787")]
        listen: String,
        #[arg(long, default_value = "web/dist")]
        web_dir: String,
        #[arg(long, default_value = "framerize.db")]
        db: String,
    },
    /// Add an account: sends the Framer magic-link email, then prompts for the
    /// #token from the emailed link.
    AccountAdd { email: String },
    /// List accounts and their state.
    Accounts,
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Daemon { listen, web_dir, db } => {
            // Wiring: SqliteStateStore + KeyringSessionStore + HttpFramerClient +
            // the WebKit session host (BrowserBridge). The webview host is the one
            // piece not yet implemented — see docs/adr/ADR-002-castle-bridge.md.
            eprintln!("daemon wiring pending BrowserBridge host ({listen}, {web_dir}, {db})");
        }
        Cmd::AccountAdd { email } => {
            eprintln!("account add for {email}: pending BrowserBridge host (Castle)");
        }
        Cmd::Accounts => {
            eprintln!("accounts: pending daemon store wiring");
        }
    }
}

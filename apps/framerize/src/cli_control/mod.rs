mod account;
mod api;
mod http;
mod pool;
mod presentation;

pub use account::{run_accounts, AccountsArgs};
pub use api::{run_api, ApiArgs};
pub use pool::{run_pool, PoolArgs};

#[derive(clap::Args)]
pub(super) struct Options {
    #[arg(long, global = true, env = "FRAMERIZE_URL", default_value = "http://127.0.0.1:8787")]
    daemon: String,
    /// Emit machine-readable JSON.
    #[arg(long, global = true)]
    json: bool,
    /// JSON object of supplied headers; credentials stay out of arguments.
    #[arg(long, global = true, value_name = "FILE")]
    headers_file: Option<String>,
}

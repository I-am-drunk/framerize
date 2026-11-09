use clap::{Args, Subcommand};
use reqwest::Method;
use serde_json::{json, Value};
use super::{http::{send_json, small_input}, presentation, Options};

#[derive(Args)]
pub struct AccountsArgs {
    #[command(flatten)]
    options: Options,
    #[command(subcommand)]
    command: Option<AccountCommand>,
}

#[derive(Subcommand)]
enum AccountCommand {
    /// List saved accounts. Browser residency is shown by pool status.
    List,
    /// Frame-wise live view: every account with its workspace marks
    /// (funded/dead/saturated), verdict (active/skipped/unused), up/down to
    /// select, Enter expands workspaces, d marks for removal, x removes
    /// marked (profiles kept), r refreshes, q quits.
    View,
    /// Remove a saved record. Keep its browser profile unless explicitly requested.
    Remove {
        account_id: uuid::Uuid,
        /// Also reset the removed account's browser profile/device identity.
        #[arg(long)]
        wipe_profile: bool,
    },
    /// Preview duplicate removal; --apply executes the server's plan.
    Dedupe {
        #[arg(long)]
        apply: bool,
    },
    /// Begin magic-link login for a specified account identity.
    Begin {
        #[arg(long, visible_alias = "account")]
        account_id: uuid::Uuid,
        #[arg(long)]
        email: String,
    },
    /// Complete login using a JSON file containing the token and account identity.
    Complete {
        #[arg(long, value_name = "FILE|-")]
        data_file: String,
    },
    /// Import a legitimate session from a JSON file without putting it in argv.
    Import {
        #[arg(long, value_name = "FILE|-")]
        data_file: String,
    },
}

async fn credential_body(options: &Options, path: &str, key: &str) -> Result<Value, String> {
    if path == "-" && options.headers_file.as_deref() == Some("-") {
        return Err("body and headers cannot both read standard input".into());
    }
    let bytes = small_input(path, 1024 * 1024).await?;
    let value: Value = serde_json::from_slice(&bytes).map_err(|_| "account data file must contain JSON")?;
    let id = value.get("account_id").and_then(Value::as_str)
        .ok_or("account data file requires account_id")?;
    uuid::Uuid::parse_str(id).map_err(|_| "account_id must be a full UUID")?;
    if !value.get(key).and_then(Value::as_str).is_some_and(|secret| !secret.trim().is_empty()) {
        return Err(format!("account data file requires a nonempty {key} field"));
    }
    Ok(value)
}

pub async fn run_accounts(args: AccountsArgs) -> Result<(), String> {
    let command = args.command.unwrap_or(AccountCommand::List);
    let listing = matches!(command, AccountCommand::List);
    if let AccountCommand::View = command {
        return super::view::run_view(super::view::ViewArgs {
            options: Options {
                daemon: args.options.daemon.clone(),
                json: args.options.json,
                headers_file: args.options.headers_file.clone(),
            },
        })
        .await;
    }
    let (method, path, body) = match command {
        // The live-marks endpoint wraps the same accounts with verdict +
        // funded/dead workspace counts (dead accounts show "skipped", never
        // "ready").
        AccountCommand::List => (Method::GET, "/api/accounts/live", None),
        AccountCommand::Remove { account_id, wipe_profile } => (Method::POST, "/api/accounts/remove",
            Some(json!({"account_id": account_id.to_string(), "wipe_profile": wipe_profile}))),
        AccountCommand::Dedupe { apply } => (Method::POST, "/api/accounts/dedupe", Some(json!({"apply": apply}))),
        AccountCommand::Begin { account_id, email } => (Method::POST, "/api/accounts/begin",
            Some(json!({"account_id": account_id.to_string(), "email": email}))),
        AccountCommand::Complete { data_file } => (Method::POST, "/api/accounts/complete",
            Some(credential_body(&args.options, &data_file, "email_token").await?)),
        AccountCommand::Import { data_file } => (Method::POST, "/api/accounts/import",
            Some(credential_body(&args.options, &data_file, "session_cookie").await?)),
        // View returns before this match; the arm is unreachable by
        // construction (see the early return above).
        AccountCommand::View => unreachable!("View returns early above"),
    };
    let reply = send_json(&args.options, method, path, body).await?;
    presentation::finish_accounts(reply, args.options.json, listing)
}

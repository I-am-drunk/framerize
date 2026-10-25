use clap::{Args, Subcommand};
use reqwest::Method;
use serde_json::{json, Value};
use super::{http::send_json, presentation, Options};

#[derive(Args)]
pub struct PoolArgs {
    #[command(flatten)]
    options: Options,
    #[command(subcommand)]
    command: Option<PoolCommand>,
}

#[derive(Subcommand)]
enum PoolCommand {
    /// Show current pool configuration and observed residency.
    Status,
    /// Read the persisted runtime pool configuration.
    Config,
    /// Update browser capacity, rotation eligibility, or both.
    Configure {
        #[arg(long, visible_alias = "capacity", value_parser = clap::value_parser!(u32).range(1..=64))]
        active_capacity: Option<u32>,
        /// all, none (pause admission), or comma-separated full account UUIDs.
        #[arg(long)]
        eligible: Option<String>,
    },
    /// Activate an eligible account in an available browser slot.
    Activate { account_id: uuid::Uuid },
    /// Unload an idle browser once; the account remains eligible for rotation.
    Park { account_id: uuid::Uuid },
}

fn eligibility(raw: &str) -> Result<Value, String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Err("--eligible cannot be empty; use none to pause admission".into());
    }
    if raw == "all" { return Ok(Value::String("all".into())); }
    if raw == "none" { return Ok(Value::Array(Vec::new())); }
    let mut selected = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for value in raw.split(',') {
        let id = uuid::Uuid::parse_str(value.trim())
            .map_err(|_| "--eligible must be all, none, or comma-separated full UUIDs")?;
        if !seen.insert(id) { return Err("--eligible contains a duplicate account UUID".into()); }
        selected.push(Value::String(id.to_string()));
    }
    Ok(Value::Array(selected))
}

fn config_patch(capacity: Option<u32>, eligible: Option<&str>) -> Result<Value, String> {
    let mut patch = serde_json::Map::new();
    if let Some(capacity) = capacity {
        patch.insert("active_capacity".into(), json!(capacity));
    }
    if let Some(eligible) = eligible {
        patch.insert("eligible_accounts".into(), eligibility(eligible)?);
    }
    if patch.is_empty() {
        return Err("specify --active-capacity or --eligible".into());
    }
    Ok(Value::Object(patch))
}

fn request_for(command: PoolCommand) -> Result<(Method, &'static str, Option<Value>), String> {
    Ok(match command {
        PoolCommand::Status => (Method::GET, "/api/pool", None),
        PoolCommand::Config => (Method::GET, "/api/pool/config", None),
        PoolCommand::Configure { active_capacity, eligible } => (Method::POST, "/api/pool/config",
            Some(config_patch(active_capacity, eligible.as_deref())?)),
        PoolCommand::Activate { account_id } => (Method::POST, "/api/pool/activate",
            Some(json!({"account_id": account_id.to_string()}))),
        PoolCommand::Park { account_id } => (Method::POST, "/api/pool/park",
            Some(json!({"account_id": account_id.to_string()}))),
    })
}

pub async fn run_pool(args: PoolArgs) -> Result<(), String> {
    let (method, path, body) = request_for(args.command.unwrap_or(PoolCommand::Status))?;
    let reply = send_json(&args.options, method, path, body).await?;
    presentation::finish_pool(reply, args.options.json)
}

#[cfg(test)]
#[path = "pool_tests.rs"]
mod tests;

//! Existing-device ledger process, independent of the browser host.
use super::{device::{DeviceAuthority, DeviceFailure, DeviceGrant, valid_device_id},
    device_owner::DeviceOwner};
use async_trait::async_trait;
use framer_core::{ids::AccountId, port::SessionStore};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{path::Path, sync::Arc};
use uuid::Uuid;

mod process;
#[cfg(test)]
mod tests;
use process::{Connection, Undo};

pub struct LedgerOwner { connection: Connection }

/// Keep migration and both runtime consumers on the configured rate policy.
/// Pass only these public settings through the otherwise empty child environment.
pub(crate) fn rate_policy_environment() -> Vec<(&'static str, std::ffi::OsString)> {
    ["FRAMERIZE_RESERVE_PER_DAY", "FRAMERIZE_MINT_GAP_S"].into_iter()
        .filter_map(|key| std::env::var_os(key).map(|value| (key, value))).collect()
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GrantReply { generation: String, lease: Uuid, device: String, authority: String }

impl LedgerOwner {
    pub async fn start(python: &Path, script: &Path, root: &Path,
        sessions: Arc<dyn SessionStore>) -> Result<Self, DeviceFailure> {
        let (python, script, root) = (python.to_owned(), script.to_owned(), root.to_owned());
        let runtime = tokio::runtime::Handle::try_current().map_err(|_| DeviceFailure::Rejected)?;
        let (sender, receiver) = tokio::sync::oneshot::channel();
        runtime.spawn(async move {
            let result = Connection::start(&python, &script, &root, sessions).await;
            let _ = sender.send(result);
        });
        let connection = receiver.await.map_err(|_| DeviceFailure::Unconfirmed)??;
        Ok(Self { connection })
    }

    pub async fn shutdown(&self) -> Result<(), DeviceFailure> { self.connection.shutdown().await }

    /// Observe actual helper termination without initiating shutdown.
    pub async fn wait_for_exit(&self) -> Result<(), DeviceFailure> {
        self.connection.wait_for_exit().await
    }
}

fn identity(grant: &DeviceGrant) -> Result<Value, DeviceFailure> {
    if grant.authority != DeviceAuthority::Ledger { return Err(DeviceFailure::Rejected); }
    Ok(json!({"account": grant.account.0, "lease": grant.lease, "generation": grant.host_generation}))
}

#[async_trait]
impl DeviceOwner for LedgerOwner {
    async fn close_native_device(&self, grant: &DeviceGrant, joined: bool, risk_refused: bool)
        -> Result<bool, DeviceFailure> {
        let mut args = identity(grant)?;
        args["joined"] = json!(joined);
        args["risk_refused"] = json!(risk_refused);
        let reply = self.connection.request("close", args, Undo::None).await?;
        if reply.value().as_object().map(|v| v.len()) != Some(1) {
            return Err(reply.invalid());
        }
        let closed = reply.value().get("closed").and_then(Value::as_bool)
            .ok_or_else(|| reply.invalid())?;
        reply.accept();
        Ok(closed)
    }

    async fn finish_native_mint(&self, grant: &DeviceGrant, sequence: u64, joined: bool)
        -> Result<bool, DeviceFailure> {
        let mut args = identity(grant)?;
        args["sequence"] = json!(sequence);
        args["joined"] = json!(joined);
        let reply = self.connection.request("finish", args, Undo::None).await?;
        if reply.value().as_object().map(|v| v.len()) != Some(2) {
            return Err(reply.invalid());
        }
        let confirmed = reply.value().get("confirmed").and_then(Value::as_bool)
            .ok_or_else(|| reply.invalid())?;
        let ready = reply.value().get("ready").and_then(Value::as_bool).ok_or_else(|| reply.invalid())?;
        reply.accept();
        if !confirmed { return Err(DeviceFailure::Unconfirmed); }
        Ok(ready)
    }

    async fn admit_native_mint(&self, grant: &DeviceGrant, budget_ms: u64)
        -> Result<u64, DeviceFailure> {
        if !(1..=16000).contains(&budget_ms) { return Err(DeviceFailure::Rejected); }
        let undo = identity(grant)?;
        let mut args = undo.clone();
        args["budget_ms"] = json!(budget_ms);
        let reply = self.connection.request("admit", args, Undo::Mint(undo)).await?;
        let sequence = reply.value().get("sequence").and_then(Value::as_u64);
        if reply.value().as_object().map(|v| v.len()) != Some(1) || sequence == Some(0) {
            return Err(reply.invalid());
        }
        let sequence = sequence.ok_or_else(|| reply.invalid())?;
        reply.accept();
        Ok(sequence)
    }

    async fn open_native_device(&self, account: AccountId) -> Result<DeviceGrant, DeviceFailure> {
        let lease = Uuid::new_v4();
        let args = json!({"account": account.0, "lease": lease});
        let reply = self.connection.request("open", args.clone(), Undo::Open(args)).await?;
        let grant: GrantReply = serde_json::from_value(reply.value().clone()).map_err(|_| reply.invalid())?;
        if grant.lease != lease || grant.authority != "ledger" || !valid_device_id(&grant.device)
            || grant.generation.len() != 32 || !grant.generation.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(reply.invalid());
        }
        reply.accept();
        Ok(DeviceGrant { account, lease, device: grant.device,
            host_generation: grant.generation, authority: DeviceAuthority::Ledger })
    }
}

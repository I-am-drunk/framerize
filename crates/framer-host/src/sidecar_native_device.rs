use super::*;
use framer_core::native_device::{DeviceFailure, DeviceGrant};
use std::time::Duration;

fn fields(grant: &DeviceGrant) -> Value {
    json!({"account": grant.account.0, "lease": grant.lease,
        "generation": grant.host_generation})
}

impl SidecarBridge {
    pub(super) async fn native_close(&self, grant: &DeviceGrant, joined: bool, risk_refused: bool)
        -> Result<bool, DeviceFailure> {
        let mut input = fields(grant);
        input["joined"] = json!(joined);
        input["risk_refused"] = json!(risk_refused);
        let value = self.native_rpc("native_device_close", input, Duration::from_secs(5), false).await?;
        if value.as_object().is_none_or(|o| o.len() != 1) { return Err(DeviceFailure::Unconfirmed); }
        value["confirmed"].as_bool().ok_or(DeviceFailure::Unconfirmed)
    }

    pub(super) async fn native_mint(&self, grant: &DeviceGrant, budget_ms: u64)
        -> Result<u64, DeviceFailure> {
        if !(1..=16000).contains(&budget_ms) { return Err(DeviceFailure::Rejected); }
        let mut input = fields(grant);
        input["budget_ms"] = json!(budget_ms);
        let value = self.native_rpc("native_device_mint", input,
            Duration::from_millis(budget_ms + 1000), false).await?;
        let sequence = value["sequence"].as_u64().filter(|s| *s > 0)
            .ok_or(DeviceFailure::Unconfirmed)?;
        if value.as_object().is_none_or(|o| o.len() != 3)
            || value["generation"].as_str() != Some(&grant.host_generation)
            || value["lease"].as_str() != Some(grant.lease.to_string().as_str()) {
            return Err(DeviceFailure::Unconfirmed);
        }
        Ok(sequence)
    }

    pub(super) async fn native_minted(&self, grant: &DeviceGrant, sequence: u64, joined: bool)
        -> Result<bool, DeviceFailure> {
        let mut input = fields(grant);
        input["sequence"] = json!(sequence);
        input["joined"] = json!(joined);
        let value = self.native_rpc("native_device_minted", input, Duration::from_secs(5), false).await?;
        if value.as_object().is_none_or(|o| o.len() != 2)
            || value["confirmed"].as_bool() != Some(true) { return Err(DeviceFailure::Unconfirmed); }
        value["ready"].as_bool().ok_or(DeviceFailure::Unconfirmed)
    }
}

impl SidecarBridge {
    async fn native_rpc(&self, command: &str, input: Value, timeout: Duration,
        start: bool) -> Result<Value, DeviceFailure> {
        let value = self.call_with_policy(command, input, timeout, start).await
            .map_err(|_| DeviceFailure::Unconfirmed)?;
        if value.as_object().is_some_and(|o| o.len() == 1)
            && value["rejected"].as_bool() == Some(true) {
            return Err(DeviceFailure::Rejected);
        }
        Ok(value)
    }

    pub(super) async fn native_open(&self, account: AccountId) -> Result<DeviceGrant, DeviceFailure> {
        let lease = uuid::Uuid::new_v4();
        let value = self.native_rpc("native_device_open", json!({"account": account.0,
            "lease": lease}), Duration::from_secs(5), true).await?;
        let host_generation = value["generation"].as_str().ok_or(DeviceFailure::Unconfirmed)?;
        let device = value["device"].as_str().ok_or(DeviceFailure::Unconfirmed)?;
        if value.as_object().is_none_or(|o| o.len() != 3)
            || value["lease"].as_str() != Some(lease.to_string().as_str())
            || host_generation.len() != 32 || !host_generation.bytes().all(|b| b.is_ascii_hexdigit())
            || device.is_empty() || device.len() > 256
            || !device.bytes().all(|b| b.is_ascii_alphanumeric() || b"._~-".contains(&b)) {
            return Err(DeviceFailure::Unconfirmed);
        }
        Ok(DeviceGrant { account, lease, host_generation: host_generation.into(), device: device.into() })
    }
}

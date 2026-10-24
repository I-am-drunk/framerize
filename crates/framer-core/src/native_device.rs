//! Private handles into the existing host's account/device ledger.
use crate::{ids::AccountId, port::Session};
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeviceFailure {
    Rejected,
    Unconfirmed,
}

/// An initial enrollment is bound to one host and its full cooling interval.
pub struct DeviceEnrollment {
    pub host_generation: String,
    pub enrollment: uuid::Uuid,
    pub cooldown_ms: u64,
}

pub fn valid_device_id(device: &str) -> bool {
    !device.is_empty() && device.len() <= 256
        && device.bytes().all(|b| b.is_ascii_alphanumeric() || b"._~-".contains(&b))
}

/// Contains private device state. Do not log or serialize this handle.
pub struct DeviceGrant {
    pub account: AccountId,
    pub host_generation: String,
    pub lease: uuid::Uuid,
    pub device: String,
}

/// Private host observation, bound to the original allocation and login.
/// This is neither a device default nor a persisted identity credential.
pub struct NativeSdkSeed {
    grant: Arc<DeviceGrant>,
    session: Session,
    account_incarnation: u64,
    session_id: uuid::Uuid,
    observation_id: uuid::Uuid,
    seed: String,
}

fn proof_id(value: &serde_json::Value) -> Result<uuid::Uuid, DeviceFailure> {
    let text = value.as_str().ok_or(DeviceFailure::Unconfirmed)?;
    let id = uuid::Uuid::parse_str(text).map_err(|_| DeviceFailure::Unconfirmed)?;
    if id.is_nil() || id.to_string() != text { return Err(DeviceFailure::Unconfirmed); }
    Ok(id)
}

impl NativeSdkSeed {
    pub fn from_wire(grant: Arc<DeviceGrant>, session: &Session,
        value: serde_json::Value) -> Result<Self, DeviceFailure> {
        let invalid = DeviceFailure::Unconfirmed;
        let seed = value["seed"].as_str().ok_or(invalid)?;
        let hex = |b: u8| b.is_ascii_digit() || (b'a'..=b'f').contains(&b);
        if value.as_object().is_none_or(|o| o.len() != 8)
            || grant.account.0.is_nil() || grant.lease.is_nil()
            || session.active_user.0.is_empty() || session.active_user.0.len() > 128
            || session.active_user.0.trim() != session.active_user.0
            || session.cookie_header.is_empty() || session.cookie_header.len() > 16 * 1024
            || !session.cookie_header.bytes().all(|b| (0x20..=0x7e).contains(&b))
            || grant.host_generation.len() != 32 || !grant.host_generation.bytes().all(hex)
            || value["account"].as_str() != Some(grant.account.0.to_string().as_str())
            || value["generation"].as_str() != Some(grant.host_generation.as_str())
            || value["lease"].as_str() != Some(grant.lease.to_string().as_str())
            || value["user"].as_str() != Some(session.active_user.0.as_str())
            || seed.len() != 32 || !seed.bytes().all(hex)
            || !(b'0'..=b'5').contains(&seed.as_bytes()[12])
            || !b"89ab".contains(&seed.as_bytes()[16]) {
            return Err(invalid);
        }
        Ok(Self { account_incarnation: value["account_incarnation"].as_u64()
                .filter(|n| *n > 0).ok_or(invalid)?,
            session_id: proof_id(&value["session_id"])?,
            observation_id: proof_id(&value["observation_id"])?,
            seed: seed.to_owned(), session: session.clone(), grant })
    }

    pub fn grant(&self) -> &Arc<DeviceGrant> { &self.grant }
    pub fn seed(&self) -> &str { &self.seed }
    pub fn account_incarnation(&self) -> u64 { self.account_incarnation }
    pub fn session_id(&self) -> uuid::Uuid { self.session_id }
    pub fn observation_id(&self) -> uuid::Uuid { self.observation_id }

    pub fn matches(&self, grant: &Arc<DeviceGrant>, session: &Session) -> bool {
        Arc::ptr_eq(&self.grant, grant) && self.session.active_user == session.active_user
            && self.session.cookie_header == session.cookie_header
    }

    fn same_receipt(&self, value: &serde_json::Value) -> bool {
        value["session_id"].as_str() == Some(self.session_id.to_string().as_str())
            && value["observation_id"].as_str() == Some(self.observation_id.to_string().as_str())
    }

    pub fn admission_from_wire(&self, value: &serde_json::Value) -> Result<u64, DeviceFailure> {
        if value.as_object().is_none_or(|o| o.len() != 5) || !self.same_receipt(value)
            || value["generation"].as_str() != Some(self.grant.host_generation.as_str())
            || value["lease"].as_str() != Some(self.grant.lease.to_string().as_str()) {
            return Err(DeviceFailure::Unconfirmed);
        }
        value["sequence"].as_u64().filter(|s| *s > 0).ok_or(DeviceFailure::Unconfirmed)
    }

    pub fn completion_from_wire(&self, value: &serde_json::Value) -> Result<bool, DeviceFailure> {
        if value.as_object().is_none_or(|o| o.len() != 4) || !self.same_receipt(value)
            || value["confirmed"].as_bool() != Some(true) {
            return Err(DeviceFailure::Unconfirmed);
        }
        value["ready"].as_bool().ok_or(DeviceFailure::Unconfirmed)
    }
}

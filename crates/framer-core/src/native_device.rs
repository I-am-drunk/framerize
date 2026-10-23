//! Private handles into the existing host's account/device ledger.
use crate::ids::AccountId;

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

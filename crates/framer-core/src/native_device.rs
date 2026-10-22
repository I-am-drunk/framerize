//! Private handles into the existing host's account/device ledger.
use crate::ids::AccountId;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeviceFailure {
    Rejected,
    Unconfirmed,
}

/// Contains private device state. Do not log or serialize this handle.
pub struct DeviceGrant {
    pub account: AccountId,
    pub host_generation: String,
    pub lease: uuid::Uuid,
    pub device: String,
}

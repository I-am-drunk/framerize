//! Device admission and cleanup owned independently of the browser interface.
use super::device::{DeviceEnrollment, DeviceFailure, DeviceGrant, NativeSdkSeed};
use async_trait::async_trait;
use framer_core::{ids::{AccountId, FramerUserId}, port::Session};
use std::sync::Arc;

#[async_trait]
pub trait DeviceOwner: Send + Sync {
    async fn enroll_native_device(&self, _account: AccountId, _user: &FramerUserId,
        _device: &str) -> Result<DeviceEnrollment, DeviceFailure> {
        Err(DeviceFailure::Rejected)
    }
    async fn open_native_device(&self, _account: AccountId) -> Result<DeviceGrant, DeviceFailure> {
        Err(DeviceFailure::Rejected)
    }
    async fn admit_native_mint(&self, _grant: &DeviceGrant, _budget_ms: u64)
        -> Result<u64, DeviceFailure> {
        Err(DeviceFailure::Rejected)
    }
    async fn finish_native_mint(&self, _grant: &DeviceGrant, _sequence: u64, _joined: bool)
        -> Result<bool, DeviceFailure> {
        Err(DeviceFailure::Unconfirmed)
    }
    async fn close_native_device(&self, _grant: &DeviceGrant, _joined: bool, _risk_refused: bool)
        -> Result<bool, DeviceFailure> {
        Err(DeviceFailure::Unconfirmed)
    }

    /// Transfer an already observed SDK identity after joined browser closure.
    async fn transfer_native_sdk_seed(&self, _grant: Arc<DeviceGrant>, _session: &Session)
        -> Result<NativeSdkSeed, DeviceFailure> {
        Err(DeviceFailure::Rejected)
    }
    async fn admit_native_sdk_mint(&self, _proof: &NativeSdkSeed, _budget_ms: u64)
        -> Result<u64, DeviceFailure> {
        Err(DeviceFailure::Rejected)
    }
    /// Recheck the observation after the original worker's cleanup has joined.
    async fn finish_native_sdk_mint(&self, _proof: &NativeSdkSeed, _sequence: u64, _joined: bool)
        -> Result<bool, DeviceFailure> {
        Err(DeviceFailure::Unconfirmed)
    }
}

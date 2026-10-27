use super::browser_control::BrowserCredentialControl;
use crate::account::device::{DeviceAuthority, DeviceFailure, DeviceGrant};
use crate::account::device_owner::DeviceOwner;
use async_trait::async_trait;
use framer_core::ids::AccountId;
use futures::FutureExt;
use std::{panic::AssertUnwindSafe, sync::{Arc, Mutex, atomic::{AtomicBool, Ordering}}};

#[derive(PartialEq, Eq)]
struct GrantKey {
    account: AccountId,
    generation: String,
    lease: uuid::Uuid,
    device: String,
    authority: DeviceAuthority,
}
impl From<&DeviceGrant> for GrantKey {
    fn from(g: &DeviceGrant) -> Self {
        Self { account: g.account, generation: g.host_generation.clone(), lease: g.lease,
            device: g.device.clone(), authority: g.authority }
    }
}

struct State {
    key: GrantKey,
    claimed: bool,
    issuing: bool,
    begun: bool,
    mint_settled: bool,
    finishing: bool,
}

pub(crate) struct ControlDeviceOwner {
    pub(crate) inner: Arc<dyn DeviceOwner>,
    pub(crate) peer: Arc<dyn BrowserCredentialControl>,
    opened: AtomicBool,
    state: Mutex<Option<State>>,
}
impl ControlDeviceOwner {
    pub(crate) fn new(inner: Arc<dyn DeviceOwner>, peer: Arc<dyn BrowserCredentialControl>) -> Self {
        Self { inner, peer, opened: AtomicBool::new(false), state: Mutex::new(None) }
    }

    fn update(&self, grant: &DeviceGrant,
        update: impl FnOnce(&mut State) -> Result<(), DeviceFailure>) -> Result<(), DeviceFailure> {
        let mut state = self.state.lock().map_err(|_| DeviceFailure::Unconfirmed)?;
        let state = state.as_mut().ok_or(DeviceFailure::Rejected)?;
        if state.key != GrantKey::from(grant) || state.finishing {
            return Err(DeviceFailure::Rejected);
        }
        update(state)
    }

    pub(crate) fn claim(&self, grant: &DeviceGrant) -> Result<(), DeviceFailure> {
        self.update(grant, |state| {
            if state.claimed { return Err(DeviceFailure::Rejected); }
            state.claimed = true;
            Ok(())
        })
    }

    pub(crate) fn issue(&self, grant: &DeviceGrant) -> Result<(), DeviceFailure> {
        self.update(grant, |state| {
            if !state.claimed || state.issuing { return Err(DeviceFailure::Rejected); }
            state.issuing = true;
            Ok(())
        })
    }

    pub(crate) fn begin(&self, grant: &DeviceGrant) -> Result<(), DeviceFailure> {
        self.update(grant, |state| {
            if !state.issuing || state.begun { return Err(DeviceFailure::Rejected); }
            state.begun = true;
            Ok(())
        })
    }

    pub(crate) fn minted(&self, grant: &DeviceGrant, joined: bool) -> Result<(), DeviceFailure> {
        self.update(grant, |state| {
            if !state.begun { return Err(DeviceFailure::Rejected); }
            state.mint_settled = joined;
            Ok(())
        })
    }
}

#[async_trait]
impl DeviceOwner for ControlDeviceOwner {
    async fn open_native_device(&self, account: AccountId) -> Result<DeviceGrant, DeviceFailure> {
        if self.opened.swap(true, Ordering::AcqRel) { return Err(DeviceFailure::Rejected); }
        let grant = self.inner.open_native_device(account).await?;
        if grant.account != account || grant.lease.is_nil()
            || grant.host_generation.len() != 32
            || !grant.host_generation.bytes().all(|b| b.is_ascii_hexdigit())
            || !crate::account::device::valid_device_id(&grant.device) {
            return Err(DeviceFailure::Unconfirmed);
        }
        *self.state.lock().map_err(|_| DeviceFailure::Unconfirmed)? = Some(State {
            key: GrantKey::from(&grant), claimed: false, issuing: false, begun: false,
            mint_settled: false, finishing: false,
        });
        Ok(grant)
    }

    async fn close_native_device(&self, grant: &DeviceGrant, joined: bool, risk: bool)
        -> Result<bool, DeviceFailure> {
        let (begun, mint_settled) = {
            let mut guard = self.state.lock().map_err(|_| DeviceFailure::Unconfirmed)?;
            let state = guard.as_mut().ok_or(DeviceFailure::Rejected)?;
            if state.key != GrantKey::from(grant) || state.finishing {
                return Err(DeviceFailure::Rejected);
            }
            state.finishing = true;
            (state.begun, state.mint_settled)
        };
        let known = joined && (!begun || mint_settled);
        let control_closed = if begun {
            AssertUnwindSafe(async { self.peer.finish(grant, known, risk).await })
                .catch_unwind().await.unwrap_or(false)
        } else { true };
        let closed = AssertUnwindSafe(self.inner.close_native_device(
            grant, known && control_closed, risk)).catch_unwind().await
            .unwrap_or(Err(DeviceFailure::Unconfirmed))?;
        Ok(closed && known && control_closed)
    }
}

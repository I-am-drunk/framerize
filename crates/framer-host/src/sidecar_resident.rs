//! Experimental wire client only. The host command is not implemented here.
//!
//! A daemon worker must retain the whole acquire/native/finish operation across
//! client disconnects. Dropping an RPC future removes its pending reply and
//! requests best-effort cancellation; a started frame still finishes writing.
//! Neither cancellation, timeout, a reply, nor dropping this handle proves
//! browser/native settlement or permits releasing an account fence.
//! Lease exclusivity, single use, reply validation and the retained daemon
//! worker are not implemented by this helper. They remain required integration.
use super::{observer::ApplicationGeneration, SidecarBridge};
use framer_core::ids::AccountId;
use serde::Serialize;
use serde_json::{json, Value};
use std::sync::{atomic::Ordering, Arc, Weak};
use std::time::Duration;
use uuid::Uuid;

pub const MAX_BUDGET_MS: u64 = 45_000;
const CONTROL_TIMEOUT: Duration = Duration::from_millis(MAX_BUDGET_MS);

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ResidentAdmission { Resume, Fenced }

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ResidentDiscard { NotStarted, Unknown }

/// Opaque, potentially private wire data. No reply schema is asserted yet.
/// A successful transport result is not a validated authorization or receipt.
#[must_use = "the daemon owner must validate the reply and retain settlement ownership"]
pub struct ResidentReply(Value);

impl ResidentReply {
    /// Keep this value in the private owner; do not log or expose it over HTTP.
    pub fn into_unverified_value(self) -> Value { self.0 }
}

/// Weakly references the daemon's existing Arc; an in-flight RPC retains it.
/// Binding certifies only a local transport identity, not remote lease admission.
/// There is deliberately no Drop cleanup: async settlement belongs to the worker.
pub struct ResidentRpc {
    owner: Weak<SidecarBridge>,
    sidecar_incarnation: Uuid,
    identity: ResidentIdentity,
}

impl ResidentRpc {
    /// No startup, account registration, disk read or remote command is performed.
    pub async fn bind(owner: &Arc<SidecarBridge>, identity: ResidentIdentity)
        -> Result<Self, ResidentError> {
        let _admission = owner.admit_unscoped().map_err(|_| ResidentError::Unconfirmed)?;
        let mut inner = owner.inner.lock().await;
        let sidecar = inner.as_mut().ok_or(ResidentError::Unconfirmed)?;
        if !sidecar.healthy.load(Ordering::Acquire)
            || !matches!(sidecar.child.try_wait(), Ok(None)) {
            return Err(ResidentError::Unconfirmed);
        }
        if let Some(boundary) = &owner.retirement {
            boundary.check_ready().map_err(|_| ResidentError::Unconfirmed)?;
        }
        Ok(Self { owner: Arc::downgrade(owner),
            sidecar_incarnation: sidecar.incarnation, identity })
    }

    pub async fn acquire(&self, request: ResidentAcquire)
        -> Result<ResidentReply, ResidentError> {
        let timeout = Duration::from_millis(request.budget_ms);
        let mut fields = self.identity.fields("acquire");
        fields["project"] = json!(request.project);
        fields["path"] = json!(request.path);
        fields["wrapper_url"] = json!(request.wrapper_url);
        fields["wrapper_export"] = json!(request.wrapper_export);
        fields["cookie_header"] = json!(request.cookie_header);
        fields["budget_ms"] = json!(request.budget_ms);
        self.call(fields, timeout).await
    }

    /// The owner must already have proved native settlement before calling this.
    pub async fn finish(&self, admission: ResidentAdmission)
        -> Result<ResidentReply, ResidentError> {
        let mut fields = self.identity.fields("finish");
        fields["native_state"] = json!("settled");
        fields["admission"] = json!(admission);
        self.call(fields, CONTROL_TIMEOUT).await
    }

    /// Unknown work must remain fenced until independently confirmed settled.
    pub async fn discard(&self, native_state: ResidentDiscard)
        -> Result<ResidentReply, ResidentError> {
        let mut fields = self.identity.fields("discard");
        fields["native_state"] = json!(native_state);
        self.call(fields, CONTROL_TIMEOUT).await
    }

    async fn call(&self, fields: Value, timeout: Duration)
        -> Result<ResidentReply, ResidentError> {
        let owner = self.owner.upgrade().ok_or(ResidentError::OwnerUnavailable)?;
        owner.call_with_incarnation("resident_authorization_once", fields, timeout,
            false, Some(self.sidecar_incarnation)).await
            .map(ResidentReply)
            // Remote error strings may contain private request material.
            .map_err(|_| ResidentError::Unconfirmed)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResidentError {
    InvalidIdentity,
    InvalidAcquire,
    OwnerUnavailable,
    Unconfirmed,
}

impl std::fmt::Display for ResidentError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "resident RPC: {self:?}")
    }
}
impl std::error::Error for ResidentError {}

/// Supplied by the daemon owner, including its freshly allocated lease UUID.
/// `host_generation` is Python Host.generation, not Sidecar.incarnation.
pub struct ResidentIdentity {
    account: AccountId,
    application_generation: ApplicationGeneration,
    host_generation: String,
    account_incarnation: u64,
    lease: Uuid,
}

impl ResidentIdentity {
    pub fn new(account: AccountId, application_generation: ApplicationGeneration,
        host_generation: String, account_incarnation: u64, lease: Uuid)
        -> Result<Self, ResidentError> {
        if account.0.is_nil() || lease.is_nil() || account_incarnation == 0
            || !bounded_text(&host_generation, 128) {
            return Err(ResidentError::InvalidIdentity);
        }
        Ok(Self { account, application_generation, host_generation,
            account_incarnation, lease })
    }

    fn fields(&self, action: &str) -> Value {
        json!({"version": 1, "action": action, "account": self.account.0,
            "application_generation": self.application_generation.0,
            "host_generation": self.host_generation,
            "account_incarnation": self.account_incarnation, "lease": self.lease})
    }
}

fn bounded_text(value: &str, max_bytes: usize) -> bool {
    !value.is_empty() && value.len() <= max_bytes && !value.chars().any(char::is_control)
}

/// Private request material. Byte caps bound the wire frame, not provenance.
/// The owner must independently verify the current project, page and cookies.
pub struct ResidentAcquire {
    project: String,
    path: String,
    wrapper_url: String,
    wrapper_export: String,
    cookie_header: String,
    budget_ms: u64,
}

impl ResidentAcquire {
    /// Limits: project/export 128 B, path/URL 8 KiB, cookie header 16 KiB.
    pub fn new(project: String, path: String, wrapper_url: String,
        wrapper_export: String, cookie_header: String, budget_ms: u64)
        -> Result<Self, ResidentError> {
        if !(1..=MAX_BUDGET_MS).contains(&budget_ms)
            || !bounded_text(&project, 128) || !bounded_text(&path, 8192)
            || !bounded_text(&wrapper_url, 8192) || !bounded_text(&wrapper_export, 128)
            || !bounded_text(&cookie_header, 16_384) {
            return Err(ResidentError::InvalidAcquire);
        }
        Ok(Self { project, path, wrapper_url, wrapper_export, cookie_header, budget_ms })
    }
}

#[cfg(test)]
#[path = "sidecar_resident_tests.rs"]
mod tests;

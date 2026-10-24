//! Private wire clients. Replies still require strict owner-side validation.
//!
//! A daemon worker must retain the whole acquire/native/finish operation across
//! client disconnects. Dropping an RPC future removes its pending reply and
//! requests best-effort cancellation; a started frame still finishes writing.
//! Neither cancellation, timeout, a reply, nor dropping this handle proves
//! browser/native settlement or permits releasing an account fence.
//! Version 1 remains a transport-only API. Version 2 additionally orders each
//! single-use phase and retains one absolute acquire/baseline/mint deadline.
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
    /// The bridge must have opted into this generation before its first startup.
    pub async fn bind(owner: &Arc<SidecarBridge>, identity: ResidentIdentity)
        -> Result<Self, ResidentError> {
        Self::bind_inner(owner, identity, None).await
    }

    /// Bind only the exact sidecar incarnation observed by private bootstrap.
    /// A replacement cannot supply a new lease for the captured page identity.
    pub async fn bind_existing(owner: &Arc<SidecarBridge>, identity: ResidentIdentity,
        expected_incarnation: Uuid) -> Result<Self, ResidentError> {
        if expected_incarnation.is_nil() { return Err(ResidentError::InvalidIdentity); }
        Self::bind_inner(owner, identity, Some(expected_incarnation)).await
    }

    async fn bind_inner(owner: &Arc<SidecarBridge>, identity: ResidentIdentity,
        expected_incarnation: Option<Uuid>) -> Result<Self, ResidentError> {
        let expected = owner.resident_application_generation
            .ok_or(ResidentError::Unconfirmed)?;
        if identity.application_generation != expected {
            return Err(ResidentError::InvalidIdentity);
        }
        let _admission = owner.admit_unscoped().map_err(|_| ResidentError::Unconfirmed)?;
        let mut inner = owner.inner.lock().await;
        let sidecar = inner.as_mut().ok_or(ResidentError::Unconfirmed)?;
        if expected_incarnation.is_some_and(|expected| sidecar.incarnation != expected)
            || !sidecar.healthy.load(Ordering::Acquire)
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
        self.call(request.fields(&self.identity), timeout).await
    }

    /// Consume this handle to select the ordered v2 protocol for its lease.
    pub fn into_v2(self) -> ResidentRpcV2 {
        ResidentRpcV2 { rpc: self, phase: V2Phase::New, deadline: None }
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
    InvalidBaseline,
    OutOfOrder,
    Deadline,
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
            || application_generation.0 == 0
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
    fn fields(self, identity: &ResidentIdentity) -> Value {
        let mut fields = identity.fields("acquire");
        fields["project"] = json!(self.project);
        fields["path"] = json!(self.path);
        fields["wrapper_url"] = json!(self.wrapper_url);
        fields["wrapper_export"] = json!(self.wrapper_export);
        fields["cookie_header"] = json!(self.cookie_header);
        fields["budget_ms"] = json!(self.budget_ms);
        fields
    }

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

/// No Clone or Debug: this handle owns private single-use protocol state.
/// Phase progress only records transport completion, never baseline acceptance.
/// The caller must validate every reply before advancing; Python independently
/// enforces lease ownership, successful baseline ordering and single use.
pub struct ResidentRpcV2 {
    rpc: ResidentRpc,
    phase: V2Phase,
    deadline: Option<tokio::time::Instant>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum V2Phase { New, Pending, Acquired, Baselined, SdkAdmitted, Minted, Closed }

impl ResidentRpcV2 {
    /// The original acquire deadline, including queueing and all later work.
    /// Cleanup may outlive it; work never receives a fresh budget.
    pub fn deadline(&self) -> Option<tokio::time::Instant> { self.deadline }

    /// Settlement has a separate transport timeout, never a new work budget.
    pub async fn finish(&mut self, admission: ResidentAdmission)
        -> Result<ResidentReply, ResidentError> {
        let mut fields = self.fields("finish");
        fields["native_state"] = json!("settled");
        fields["admission"] = json!(admission);
        self.close(fields).await
    }

    pub async fn discard(&mut self, native_state: ResidentDiscard)
        -> Result<ResidentReply, ResidentError> {
        let mut fields = self.fields("discard");
        fields["native_state"] = json!(native_state);
        self.close(fields).await
    }

    async fn close(&mut self, fields: Value) -> Result<ResidentReply, ResidentError> {
        if matches!(self.phase, V2Phase::New | V2Phase::Closed) {
            return Err(ResidentError::OutOfOrder);
        }
        self.phase = V2Phase::Closed;
        self.rpc.call(fields, CONTROL_TIMEOUT).await
    }

    /// Borrow the retained bearer; this operation cannot mint native credentials.
    pub async fn baseline(&mut self, bearer: &str, body: &Value)
        -> Result<ResidentReply, ResidentError> {
        if self.phase != V2Phase::Acquired { return Err(ResidentError::OutOfOrder); }
        if !bounded_text(bearer, 16 * 1024) {
            return Err(ResidentError::InvalidBaseline);
        }
        Self::validate_baseline_body(body)?;
        let mut fields = self.fields("baseline");
        fields["bearer"] = json!(bearer);
        fields["body"] = body.clone();
        self.step(V2Phase::Acquired, V2Phase::Baselined, fields).await
    }

    /// Pure preflight of the final body, before acquiring any browser lease.
    /// InvalidBaseline always precedes phase consumption or a command write.
    pub fn validate_baseline_body(body: &Value) -> Result<(), ResidentError> {
        if baseline_body(body) { Ok(()) } else { Err(ResidentError::InvalidBaseline) }
    }

    pub async fn mint_native(&mut self) -> Result<ResidentReply, ResidentError> {
        self.step(V2Phase::Baselined, V2Phase::Minted, self.fields("mint_native")).await
    }

    /// Charges the resident rate owner before starting a separate SDK worker.
    /// This consumes the same native phase as mint_native, without a browser mint.
    pub async fn begin_native_sdk(&mut self) -> Result<ResidentReply, ResidentError> {
        self.step(V2Phase::Baselined, V2Phase::SdkAdmitted,
            self.fields("begin_native_sdk")).await
    }

    /// Call only after joining the separately owned worker, then validate the
    /// returned context. This performs no mint and supplies no token.
    pub async fn complete_native_sdk(&mut self) -> Result<ResidentReply, ResidentError> {
        self.step(V2Phase::SdkAdmitted, V2Phase::Minted,
            self.fields("complete_native_sdk")).await
    }

    async fn step(&mut self, expected: V2Phase, next: V2Phase, fields: Value)
        -> Result<ResidentReply, ResidentError> {
        if self.phase != expected { return Err(ResidentError::OutOfOrder); }
        // An error or dropped future never makes a phase retryable.
        self.phase = V2Phase::Pending;
        let deadline = self.deadline.ok_or(ResidentError::OutOfOrder)?;
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() { return Err(ResidentError::Deadline); }
        let reply = self.rpc.call(fields, remaining).await?;
        if tokio::time::Instant::now() >= deadline { return Err(ResidentError::Deadline); }
        self.phase = next;
        Ok(reply)
    }

    pub async fn acquire(&mut self, request: ResidentAcquire, provenance: &str)
        -> Result<ResidentReply, ResidentError> {
        if self.phase != V2Phase::New { return Err(ResidentError::OutOfOrder); }
        if !Uuid::parse_str(provenance).is_ok_and(|id|
            !id.is_nil() && id.to_string() == provenance) {
            return Err(ResidentError::InvalidAcquire);
        }
        self.deadline = Some(tokio::time::Instant::now()
            + Duration::from_millis(request.budget_ms));
        let mut fields = request.fields(&self.rpc.identity);
        fields["version"] = json!(2);
        fields["provenance"] = json!(provenance);
        self.step(V2Phase::New, V2Phase::Acquired, fields).await
    }

    fn fields(&self, action: &str) -> Value {
        let mut fields = self.rpc.identity.fields(action);
        fields["version"] = json!(2);
        fields
    }
}

fn baseline_body(value: &Value) -> bool {
    fn plain(value: &Value, depth: usize) -> bool {
        if depth > 64 { return false; }
        match value {
            Value::Object(map) => !map.contains_key("castleToken")
                && map.values().all(|value| plain(value, depth + 1)),
            Value::Array(array) => array.iter().all(|value| plain(value, depth + 1)),
            Value::Number(number) => {
                // The browser parses this body as JavaScript Numbers. Bound
                // floats too, so decimal/exponent spelling cannot bypass parity.
                const MAX_SAFE: i64 = 9_007_199_254_740_991;
                if let Some(value) = number.as_i64() {
                    (-MAX_SAFE..=MAX_SAFE).contains(&value)
                } else if let Some(value) = number.as_u64() {
                    value <= MAX_SAFE as u64
                } else {
                    number.as_f64().is_some_and(|value|
                        value.is_finite() && value.abs() <= MAX_SAFE as f64)
                }
            }
            _ => true,
        }
    }
    struct Bounded(usize);
    impl std::io::Write for Bounded {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            if bytes.len() > (64 * 1024usize).saturating_sub(self.0) {
                return Err(std::io::Error::other("resident body exceeds limit"));
            }
            self.0 += bytes.len();
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> { Ok(()) }
    }
    value.is_object() && plain(value, 0) && serde_json::to_writer(Bounded(0), value).is_ok()
}

#[cfg(test)]
#[path = "sidecar_resident_tests.rs"]
mod tests;

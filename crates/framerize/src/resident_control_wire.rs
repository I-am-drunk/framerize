//! Strict private reply validation. Raw values never enter diagnostics.
use super::*;
use framer_api::experimental_native::control_authorization::resident_control::ResidentLeaseIdentity;
use framer_core::port::CastleToken;
use serde_json::Value;
use uuid::Uuid;

pub(super) struct Binding {
    pub account: framer_core::ids::AccountId,
    pub application: u64,
    pub host: String,
    pub incarnation: u64,
    pub lease: Uuid,
}
impl Binding {
    pub fn api(&self) -> Result<ResidentLeaseIdentity, PreparationFailure> {
        ResidentLeaseIdentity::new(self.account, self.application, self.host.clone(),
            self.incarnation, self.lease)
    }
    fn matches(&self, value: &Value) -> bool {
        value["version"].as_u64() == Some(2)
            && value["account"].as_str() == Some(self.account.0.to_string().as_str())
            && value["application_generation"].as_u64() == Some(self.application)
            && value["host_generation"].as_str() == Some(self.host.as_str())
            && value["account_incarnation"].as_u64() == Some(self.incarnation)
            && value["lease"].as_str() == Some(self.lease.to_string().as_str())
    }
}

const IDENTITY: &[&str] = &["version", "account", "application_generation",
    "host_generation", "account_incarnation", "lease", "state", "browser_settled"];
fn keys(value: &Value, extras: &[&str]) -> bool {
    value.as_object().is_some_and(|map| map.len() == IDENTITY.len() + extras.len()
        && IDENTITY.iter().chain(extras).all(|key| map.contains_key(*key)))
}

const CONTEXT: &[&str] = &["provenance", "cookie_header", "page_url", "device", "effective_proxy"];
fn context_matches(binding: &Binding, page: &CapturedPage, value: &Value) -> bool {
    binding.matches(value)
        && Uuid::parse_str(&page.provenance).is_ok_and(|id|
            !id.is_nil() && id.to_string() == page.provenance)
        && value["provenance"].as_str() == Some(page.provenance.as_str())
        && value["page_url"].as_str() == Some(page.page_url.as_str())
        && value["device"].as_str() == Some(page.device_id.as_str())
        && proxy_matches(&value["effective_proxy"], &page.effective_proxy)
}

pub(super) struct Minted {
    pub token: CastleToken,
    pub header: String,
}
pub(super) fn minted(binding: &Binding, page: &CapturedPage, header: &str,
    mut value: Value) -> Result<Minted, Failure> {
    let extras = [CONTEXT, &["token"]].concat();
    if !keys(&value, &extras) || !context_matches(binding, page, &value)
        || value["state"].as_str() != Some("minted")
        || value["browser_settled"].as_bool() != Some(true)
        || value["cookie_header"].as_str() != Some(header) {
        return Err(Failure::Context);
    }
    Ok(Minted { token: CastleToken(take_text(&mut value, "token")?),
        header: take_text(&mut value, "cookie_header")? })
}

pub(super) struct Acquired {
    pub header: String,
}
#[derive(PartialEq, Eq)]
pub(super) struct SdkDevice(String);
impl SdkDevice {
    pub(super) fn value(&self) -> &str { &self.0 }
}
pub(super) fn sdk_context(binding: &Binding, page: &CapturedPage, header: &str,
    mut value: Value, state: &str) -> Result<SdkDevice, Failure> {
    let extras = [CONTEXT, &["sdk_device"]].concat();
    if !matches!(state, "sdk_admitted" | "sdk_complete") || !keys(&value, &extras)
        || !context_matches(binding, page, &value)
        || value["state"].as_str() != Some(state)
        || value["browser_settled"].as_bool() != Some(true)
        || value["cookie_header"].as_str() != Some(header) {
        return Err(Failure::Context);
    }
    let device = take_text(&mut value, "sdk_device")?;
    let bytes = device.as_bytes();
    if bytes.len() != 32 || !bytes.iter().all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(c))
        || !(b'0'..=b'5').contains(&bytes[12]) || !b"89ab".contains(&bytes[16]) {
        return Err(Failure::Context);
    }
    Ok(SdkDevice(device))
}

pub(super) fn acquired(binding: &Binding, page: &CapturedPage, mut value: Value)
    -> Result<Acquired, Failure> {
    if !keys(&value, CONTEXT) || !context_matches(binding, page, &value)
        || value["state"].as_str() != Some("acquired")
        || value["browser_settled"].as_bool() != Some(true)
    {
        return Err(Failure::Context);
    }
    Ok(Acquired { header: take_text(&mut value, "cookie_header")? })
}
fn proxy_matches(value: &Value, proxy: &Option<String>) -> bool {
    match proxy { Some(proxy) => value.as_str() == Some(proxy.as_str()),
        None => value.is_null() }
}
fn take_text(value: &mut Value, key: &str) -> Result<String, Failure> {
    match value[key].take() {
        Value::String(text) if !text.is_empty() && text.len() <= 16 * 1024
            && !text.chars().any(char::is_control) => Ok(text),
        _ => Err(Failure::Context),
    }
}

/// This is browser settlement only. The caller independently joins native
/// work; a fenced receipt never certifies that account admission may resume.
pub(super) fn settled(binding: &Binding, value: Value) -> bool {
    keys(&value, &[]) && binding.matches(&value)
        && value["browser_settled"].as_bool() == Some(true)
        && value["state"].as_str() == Some("fenced")
}

pub(super) struct Baseline {
    pub header: String,
    pub browser_settled: bool,
    pub status: Option<u16>,
    pub body_complete: bool,
    pub json_object: bool,
    pub accepted: bool,
    pub refusal: Option<framer_api::classify::RefusalObservation>,
}
impl Baseline {
    pub fn permits_mint(&self) -> bool {
        self.browser_settled && self.status == Some(200) && self.body_complete
            && self.json_object && self.accepted
    }
}

pub(super) fn baseline(binding: &Binding, page: &CapturedPage, header: &str,
    mut value: Value) -> Result<Baseline, Failure> {
    let extras = [CONTEXT, &["status", "body_complete", "json_object", "accepted", "refusal"]].concat();
    if !keys(&value, &extras) || !context_matches(binding, page, &value)
        || value["state"].as_str() != Some("baseline_complete")
        || value["cookie_header"].as_str() != Some(header) {
        return Err(Failure::Baseline);
    }
    let status = if value["status"].is_null() { None } else {
        Some(value["status"].as_u64().filter(|status| (100..=599).contains(status))
            .ok_or(Failure::Baseline)? as u16)
    };
    let browser_settled = flag(&value, "browser_settled")?;
    let body_complete = flag(&value, "body_complete")?;
    let json_object = flag(&value, "json_object")?;
    let accepted = flag(&value, "accepted")?;
    let refusal = refusal(&value["refusal"])?;
    if refusal.is_some() && (status != Some(403) || !body_complete) {
        return Err(Failure::Baseline);
    }
    if accepted && !(browser_settled && status == Some(200) && body_complete && json_object) {
        return Err(Failure::Baseline);
    }
    Ok(Baseline { header: take_text(&mut value, "cookie_header")?, browser_settled,
        status, body_complete, json_object, accepted, refusal })
}

fn refusal(value: &Value) -> Result<Option<framer_api::classify::RefusalObservation>, Failure> {
    use framer_api::classify::{Refusal, RefusalBasis, RefusalObservation};
    if value.is_null() { return Ok(None); }
    if !value.as_object().is_some_and(|map| map.len() == 2) { return Err(Failure::Baseline); }
    let (policy, basis) = match (value["policy"].as_str(), value["basis"].as_str()) {
        (Some("upstream"), Some("provider_marker")) => (Refusal::Upstream, RefusalBasis::ProviderMarker),
        (Some("castle"), Some("framer_security_envelope")) => (Refusal::Castle, RefusalBasis::FramerSecurityEnvelope),
        (Some("castle"), Some("framer_marker")) => (Refusal::Castle, RefusalBasis::FramerMarker),
        (Some("upstream"), Some("elapsed_fallback")) => (Refusal::Upstream, RefusalBasis::ElapsedFallback),
        (Some("castle"), Some("elapsed_fallback")) => (Refusal::Castle, RefusalBasis::ElapsedFallback),
        _ => return Err(Failure::Baseline),
    };
    Ok(Some(RefusalObservation { policy, basis }))
}

fn flag(value: &Value, key: &str) -> Result<bool, Failure> {
    value[key].as_bool().ok_or(Failure::Baseline)
}

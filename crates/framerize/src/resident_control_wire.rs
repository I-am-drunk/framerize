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
        value["version"].as_u64() == Some(1)
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

pub(super) struct Acquired {
    pub token: CastleToken,
    pub header: String,
}
pub(super) fn acquired(binding: &Binding, page: &CapturedPage, mut value: Value)
    -> Result<Acquired, Failure> {
    if !keys(&value, &["token", "cookie_header", "page_url", "device", "effective_proxy"])
        || !binding.matches(&value) || value["state"].as_str() != Some("acquired")
        || value["browser_settled"].as_bool() != Some(true)
        || value["page_url"].as_str() != Some(page.page_url.as_str())
        || value["device"].as_str() != Some(page.device_id.as_str())
        || !proxy_matches(&value["effective_proxy"], &page.effective_proxy) {
        return Err(Failure::Context);
    }
    Ok(Acquired { token: CastleToken(take_text(&mut value, "token")?),
        header: take_text(&mut value, "cookie_header")? })
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

pub(super) fn baseline_matches(binding: &Binding, page: &CapturedPage,
    identity: &Identity, path: &str, header: &str, receipt: &BaselineReceipt) -> bool {
    receipt.accepted && receipt.status == Some(200)
        && receipt.identity == *identity && receipt.path == path
        && receipt.application_generation == binding.application
        && receipt.host_generation == binding.host
        && receipt.account_incarnation == binding.incarnation
        && receipt.lease == binding.lease
        && receipt.page_url == page.page_url && receipt.device_id == page.device_id
        && receipt.cookie_header == header && receipt.effective_proxy == page.effective_proxy
}

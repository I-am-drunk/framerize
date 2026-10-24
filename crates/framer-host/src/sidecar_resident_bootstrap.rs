//! Private metadata handoff from the already-running resident owner.
//! This does not mint, start a host, register a route or certify inference.
use super::{observer::ApplicationGeneration, SidecarBridge};
use framer_core::ids::AccountId;
use serde::Deserialize;
use serde_json::{json, Value};
use std::{sync::{atomic::Ordering, Arc}, time::Duration};
use uuid::Uuid;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BootstrapError { InvalidRequest, Unavailable, Unconfirmed }

/// Private metadata only. Deliberately neither Debug nor Clone.
pub struct ResidentBootstrap {
    pub application_generation: ApplicationGeneration,
    pub sidecar_incarnation: Uuid,
    pub host_generation: String,
    pub account_incarnation: u64,
    pub proxy_revision: u64,
    pub provenance: String,
    pub page_url: String,
    pub device: String,
    pub effective_proxy: Option<String>,
    pub wrapper_url: String,
    pub wrapper_export: String,
}

fn text(value: &str, limit: usize) -> bool {
    !value.is_empty() && value.len() <= limit && !value.chars().any(char::is_control)
}

fn target(project: &str, path: &str) -> bool {
    text(project, 128) && project.bytes().all(|b| b.is_ascii_alphanumeric()
        || b == b'_' || b == b'-') && matches!(path,
        "/ai/v3/responses/" | "/ai/v3/messages/" | "/ai/v3/chat/"
        | "/ai/v3/openai/responses/" | "/ai/v3/anthropic/messages/")
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Reply {
    version: u8,
    account: String,
    application_generation: u64,
    bootstrap: String,
    host_generation: String,
    account_incarnation: u64,
    project: String,
    path: String,
    proxy_revision: u64,
    provenance: String,
    page_url: String,
    device: String,
    effective_proxy: Option<String>,
    wrapper_url: String,
    wrapper_export: String,
    browser_settled: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UnavailableReply {
    version: u8,
    account: String,
    application_generation: u64,
    bootstrap: String,
    project: String,
    path: String,
    proxy_revision: u64,
    status: String,
    stage: String,
    browser_settled: bool,
}

impl SidecarBridge {
    /// Caller retains application admission across this entire operation.
    /// An error after sending has unknown cleanup; it must retain that fence.
    pub async fn resident_bootstrap(self: &Arc<Self>, account: AccountId,
        project: &str, path: &str, configured_proxy: &Option<String>)
        -> Result<ResidentBootstrap, BootstrapError> {
        if account.0.is_nil() || !target(project, path)
            || configured_proxy.as_ref().is_some_and(|p| !text(p, 16_384)) {
            return Err(BootstrapError::InvalidRequest);
        }
        let generation = self.resident_application_generation
            .ok_or(BootstrapError::Unavailable)?;
        let _admission = self.admit_unscoped().map_err(|_| BootstrapError::Unavailable)?;
        // Only in-memory route bookkeeping; no route registration or startup.
        let cell = self.route_state(&account).await;
        let route = cell.lock().await;
        if route.pending.is_some() || &route.current.proxy != configured_proxy {
            return Err(BootstrapError::Unavailable);
        }
        let incarnation = {
            let mut inner = self.inner.lock().await;
            let host = inner.as_mut().ok_or(BootstrapError::Unavailable)?;
            if !host.healthy.load(Ordering::Acquire)
                || !matches!(host.child.try_wait(), Ok(None)) {
                return Err(BootstrapError::Unavailable);
            }
            host.incarnation
        };
        let bootstrap = Uuid::new_v4();
        let revision = route.current.revision;
        let fields = json!({"version": 2, "account": account.0,
            "application_generation": generation.0, "bootstrap": bootstrap,
            "project": project, "path": path, "proxy_revision": revision});
        let value = self.call_with_incarnation("resident_authorization_bootstrap",
            fields, Duration::from_secs(3), false, Some(incarnation)).await
            .map_err(|_| BootstrapError::Unconfirmed)?;
        let result = checked(value, account, generation, incarnation, bootstrap,
            revision, project, path);
        let mut inner = self.inner.lock().await;
        let host = inner.as_mut().ok_or(BootstrapError::Unconfirmed)?;
        if host.incarnation != incarnation || !host.healthy.load(Ordering::Acquire)
            || !matches!(host.child.try_wait(), Ok(None)) {
            return Err(BootstrapError::Unconfirmed);
        }
        result
    }
}

fn uuid(value: &str) -> bool {
    Uuid::parse_str(value).is_ok_and(|id| !id.is_nil() && id.to_string() == value)
}

fn page(value: &str) -> bool {
    if !text(value, 4096) || value.chars().any(char::is_whitespace) { return false; }
    let Some(tail) = value.strip_prefix("https://") else { return false; };
    matches!(tail.split(['/', '?', '#']).next(), Some("framer.com" | "framer.com:443"))
}

fn wrapper(url: &str, export: &str) -> bool {
    let Some(tail) = url.strip_prefix("https://app.framerstatic.com/chunk-") else { return false; };
    let (file, query) = tail.split_once('?').map_or((tail, None), |(f, q)| (f, Some(q)));
    let Some(hash) = file.strip_suffix(".mjs") else { return false; };
    hash.len() == 8 && hash.bytes().all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
        && query.is_none_or(|q| q.strip_prefix("cb=").is_some_and(|v|
            !v.is_empty() && v.len() <= 20 && v.bytes().all(|b| b.is_ascii_digit())))
        && text(export, 128) && export.bytes().enumerate().all(|(i, b)|
            b == b'_' || b == b'$' || b.is_ascii_alphabetic() || (i > 0 && b.is_ascii_digit()))
}

#[allow(clippy::too_many_arguments)]
fn checked(value: Value, account: AccountId, generation: ApplicationGeneration,
    incarnation: Uuid, bootstrap: Uuid, revision: u64, project: &str, path: &str)
    -> Result<ResidentBootstrap, BootstrapError> {
    if value.get("status").is_some() {
        let r: UnavailableReply = serde_json::from_value(value)
            .map_err(|_| BootstrapError::Unconfirmed)?;
        return Err(if r.version == 2 && r.account == account.0.to_string()
            && r.application_generation == generation.0 && r.bootstrap == bootstrap.to_string()
            && r.project == project && r.path == path && r.proxy_revision == revision
            && r.status == "unavailable" && r.stage == "provenance" && r.browser_settled {
            BootstrapError::Unavailable
        } else { BootstrapError::Unconfirmed });
    }
    if !value.as_object().is_some_and(|o| o.contains_key("effective_proxy")) {
        return Err(BootstrapError::Unconfirmed);
    }
    let r: Reply = serde_json::from_value(value).map_err(|_| BootstrapError::Unconfirmed)?;
    if r.version != 2 || r.account != account.0.to_string()
        || r.application_generation != generation.0 || r.bootstrap != bootstrap.to_string()
        || r.project != project || r.path != path || r.proxy_revision != revision
        || r.account_incarnation == 0 || !text(&r.host_generation, 128)
        || !uuid(&r.provenance) || !page(&r.page_url) || !text(&r.device, 256)
        || r.effective_proxy.as_ref().is_some_and(|p| !text(p, 16_384))
        || !wrapper(&r.wrapper_url, &r.wrapper_export) || !r.browser_settled {
        return Err(BootstrapError::Unconfirmed);
    }
    Ok(ResidentBootstrap { application_generation: generation, sidecar_incarnation: incarnation,
        host_generation: r.host_generation, account_incarnation: r.account_incarnation,
        proxy_revision: revision, provenance: r.provenance, page_url: r.page_url,
        device: r.device, effective_proxy: r.effective_proxy,
        wrapper_url: r.wrapper_url, wrapper_export: r.wrapper_export })
}

#[cfg(test)]
#[path = "sidecar_resident_bootstrap_tests.rs"]
mod tests;

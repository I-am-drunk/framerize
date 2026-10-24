//! Explicit comparison commands. The caller retains control and cleanup
//! through both HTTP attempts; this module never releases that ownership.
use super::*;
use framer_core::{ids::ProjectId, port::{BrowserAuthorization, BrowserAuthorizationRequest}};

#[path = "comparison/plan.rs"]
mod plan;
#[path = "comparison/report.rs"]
mod report;
pub use plan::ComparisonPlan;
pub use report::{BaselineReport, ContentType, Refusal, RefusalBasis, RefusalPolicy};
const FAILURE: &str = "browser comparison contract failed";
const SOURCE: &str = "framer_browser_comparison_v1";
#[cfg(test)]
#[path = "comparison/tests.rs"]
mod tests;

pub struct ComparisonScope {
    pub client: ComparisonClient,
    pub control: ScopeControl,
    pub cleanup: CleanupObserver,
}
pub struct ComparisonClient {
    bridge: Arc<bridge::ScopedBridge>,
    account: AccountId,
}

impl ObserverFactory {
    pub fn comparison_scope(&self, account: AccountId, generation: ApplicationGeneration)
        -> Result<ComparisonScope, ObserverError> {
        // A reply can arrive before its writer-owning command retires.
        // The continuation joins that command while reserving its own slot.
        if self.registry.limits.commands < 2 { return Err(ObserverError::InvalidLimits); }
        let opened = self.one_shot_scope(account, generation)?;
        let client = ComparisonClient { account, bridge: Arc::new(bridge::ScopedBridge {
            registry: self.registry.clone(), state: opened.control.state.clone(),
        }) };
        Ok(ComparisonScope { client, control: opened.control, cleanup: opened.cleanup })
    }
}

pub struct ComparisonContext {
    client: ComparisonClient,
    plan: ComparisonPlan,
    handle: String,
    page_url: String,
    cookie_header: String,
    baseline_started: bool,
    baseline_accepted: bool,
    native_issued: bool,
}

impl ComparisonClient {
    pub async fn set_proxy(&self, proxy: Option<String>) -> Result<(), &'static str> {
        self.bridge.set_proxy(&self.account, proxy).await.map_err(|_| FAILURE)
    }

    pub async fn prepare(self, plan: ComparisonPlan, cookie_header: &str)
        -> Result<ComparisonContext, &'static str> {
        if cookie_header.is_empty() || cookie_header.len() > 16 * 1024 { return Err(FAILURE); }
        let mut value = self.bridge.call(&self.account, "prepare_inference_authorization", ||
            json!({"purpose": "baseline_context", "project": &plan.project.0,
                "path": &plan.path, "cookie_header": cookie_header,
                "body": &plan.baseline, "native_body": &plan.native,
                "wrapper_url": "https://app.framerstatic.com/chunk-VPFQZMNG.mjs",
                "wrapper_export": "b"})).await.map_err(|_| FAILURE)?;
        if value["source"] != SOURCE || value["purpose"] != "baseline_context"
            || value["project"] != plan.project.0 || value["path"] != plan.path {
            return Err(FAILURE);
        }
        Ok(ComparisonContext { client: self, plan,
            handle: take(&mut value, "context_handle", 256)?,
            page_url: take(&mut value, "page_url", 4096)?,
            cookie_header: take(&mut value, "cookie_header", 16 * 1024)?,
            baseline_started: false, baseline_accepted: false, native_issued: false })
    }
}

fn take(value: &mut Value, name: &str, bound: usize) -> Result<String, &'static str> {
    match value.get_mut(name).map(Value::take) {
        Some(Value::String(text)) if !text.is_empty() && text.len() <= bound => Ok(text),
        _ => Err(FAILURE),
    }
}

impl ComparisonContext {
    pub fn cookie_header(&self) -> &str { &self.cookie_header }
    pub fn page_url(&self) -> &str { &self.page_url }

    pub async fn baseline(&mut self, bearer: &str, cookie_header: &str)
        -> Result<BaselineReport, &'static str> {
        if self.baseline_started || bearer.is_empty() || bearer.len() > 16 * 1024
            || cookie_header != self.cookie_header {
            return Err(FAILURE);
        }
        self.baseline_started = true;
        let value = self.client.bridge.call(&self.client.account, "browser_baseline_once", ||
            json!({"context_handle": &self.handle, "project": &self.plan.project.0,
                "path": &self.plan.path, "body": &self.plan.baseline,
                "bearer": bearer, "cookie_header": cookie_header}))
            .await.map_err(|_| FAILURE)?;
        if value["source"] != SOURCE || !self.same_context(&value) { return Err(FAILURE); }
        let report = BaselineReport::parse(value)?;
        self.baseline_accepted = report.accepted;
        Ok(report)
    }

    fn same_context(&self, value: &Value) -> bool {
        value["context_handle"] == self.handle && value["project"] == self.plan.project.0
            && value["path"] == self.plan.path
    }

    /// Single use even if the command fails. The outer owner still must stop
    /// or seal the original scope and consume its matching cleanup receipt.
    pub async fn native_token(&mut self) -> Result<BrowserAuthorization, &'static str> {
        if !self.baseline_accepted || self.native_issued { return Err(FAILURE); }
        self.native_issued = true;
        let value = self.client.bridge.call(&self.client.account,
            "prepare_inference_authorization", || json!({
                "purpose": "native_token", "context_handle": &self.handle,
                "project": &self.plan.project.0, "path": &self.plan.path,
                "body": &self.plan.native,
            })).await.map_err(|_| FAILURE)?;
        if value["purpose"] != "native_token" || !self.same_context(&value)
            || value["page_url"] != self.page_url {
            return Err(FAILURE);
        }
        bridge::take_context_authorization(value, BrowserAuthorizationRequest {
            project: &self.plan.project, path: &self.plan.path,
            cookie_header: &self.cookie_header,
        }).map_err(|_| FAILURE)
    }
}

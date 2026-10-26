use super::{BrowserOutcome, BrowserRequest, BrowserResponse, BrowserTransport};
use crate::transport::http::NativeCancellation;
use framer_core::port::BrowserBridge;
use framer_host::sidecar::SidecarBridge;
use futures::{future::BoxFuture, FutureExt};
use serde_json::{json, Value};
use std::{path::Path, sync::Arc, time::Duration};

#[cfg(test)]
#[path = "browser_host_tests.rs"]
mod tests;

pub struct BrowserHost { bridge: Arc<SidecarBridge> }

impl BrowserHost {
    pub fn new(script: &Path, state_root: &Path) -> Result<Self, String> {
        Ok(Self { bridge: Arc::new(SidecarBridge::new(state_root).with_script(script)?) })
    }

    pub async fn prepare(&self) -> Result<(), String> {
        <Self as BrowserTransport>::prepare(self).await
    }

    pub async fn shutdown(&self) -> Result<(), String> { self.bridge.shutdown().await }
}

impl BrowserTransport for BrowserHost {
    fn prepare(&self) -> BoxFuture<'static, Result<(), String>> {
        let bridge = self.bridge.clone();
        async move {
            let result = bridge.prepare_supplied().await?;
            if result == json!({"version": 1, "ready": true}) { Ok(()) }
            else { Err("unsupported supplied-token browser host".into()) }
        }.boxed()
    }

    fn send(&self, request: BrowserRequest, cancellation: NativeCancellation)
        -> BoxFuture<'static, BrowserOutcome> {
        let bridge = self.bridge.clone();
        async move {
            let timeout = Duration::from_millis(request.timeout_ms).saturating_add(Duration::from_secs(30));
            let wire = match String::from_utf8(request.wire) {
                Ok(wire) => wire,
                Err(_) => return BrowserOutcome { response: Err("invalid browser request"), cleanup_confirmed: true },
            };
            let fields = json!({"version": 1, "account": request.account.0,
                "user": request.user, "cookie_header": request.cookie_header,
                "device": request.device, "path": request.path, "bearer": request.bearer,
                "wire": wire, "proxy": request.proxy, "expires_unix_ms": request.expires_unix_ms,
                "timeout_ms": request.timeout_ms, "response_limit": request.response_limit});
            let result = bridge.post_supplied(fields, timeout, &|| cancellation.is_cancelled()).await;
            match result {
                Ok(value) => decode(value, request.response_limit),
                Err(_) => BrowserOutcome { response: Err("browser command failed"), cleanup_confirmed: false },
            }
        }.boxed()
    }
}

fn decode(value: Value, limit: usize) -> BrowserOutcome {
    let cleanup_confirmed = value.get("cleanup_confirmed").and_then(Value::as_bool) == Some(true);
    let response = (|| {
        if value.get("version").and_then(Value::as_u64) != Some(1) {
            return Err("unsupported browser result");
        }
        let result = value.get("response").ok_or("browser request failed")?;
        let status = result["status"].as_u64().filter(|v| (100..=599).contains(v))
            .ok_or("invalid browser status")? as u16;
        let header = |name: &str| -> Result<Option<String>, &'static str> {
            match result.get(name) {
                Some(Value::Null) => Ok(None),
                Some(Value::String(v)) if v.len() <= 16 * 1024 => Ok(Some(v.clone())),
                _ => Err("invalid browser header"),
            }
        };
        let body = result["body"].as_str().filter(|v| v.len() <= limit)
            .ok_or("invalid browser body")?.as_bytes().to_vec();
        Ok(BrowserResponse { status, usage: header("usage")?,
            content_type: header("content_type")?, body })
    })();
    BrowserOutcome { response, cleanup_confirmed }
}

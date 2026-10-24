//! Completed direct-V2 operation facts, not a credential or handoff receipt.
use serde_json::Value;

const FIELDS: &[&str] = &["version", "sdk", "selection", "status", "failure_phase",
    "device_failure", "configure_calls", "token_method_calls", "device_storage_seeded",
    "device_storage_unchanged", "window_closed", "sources_unchanged", "async_errors",
    "diagnostics", "late_settlements", "method_ms", "elapsed_ms"];

/// The caller still owns source/account binding, process join and fresh private
/// token handoff. This decoder does not authorize sending an inference request.
/// A rejected report does not establish a failure cause or settled cleanup.
pub fn completed(bytes: &[u8], budget_ms: u64) -> Option<V2OperationMetadata> {
    if bytes.len() > 16384 || !(1..=16000).contains(&budget_ms) { return None; }
    let value: Value = serde_json::from_slice(bytes).ok()?;
    finish(&value, budget_ms)
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct V2OperationMetadata {
    pub method_ms: f64,
    pub elapsed_ms: f64,
    pub diagnostics: u64,
}

fn finish(value: &Value, budget_ms: u64) -> Option<V2OperationMetadata> {
    let object = value.as_object()?;
    if object.len() != FIELDS.len() || FIELDS.iter().any(|key| !object.contains_key(*key))
        || value["version"].as_u64()? != 1 || value["status"].as_str()? != "passed"
        || value["sdk"].as_str()? != "framer-embedded-v2"
        || value["selection"].as_str()? != "configured-api"
        || !value["failure_phase"].is_null() || !value["device_failure"].is_null() {
        return None;
    }
    for key in ["device_storage_seeded", "device_storage_unchanged", "window_closed",
        "sources_unchanged"] {
        if value[key].as_bool()? != true { return None; }
    }
    for key in ["configure_calls", "token_method_calls"] {
        if value[key].as_u64()? != 1 { return None; }
    }
    for key in ["async_errors", "late_settlements"] {
        if value[key].as_u64()? != 0 { return None; }
    }
    timing(value, budget_ms)
}

fn timing(value: &Value, budget_ms: u64) -> Option<V2OperationMetadata> {
    let method_ms = value["method_ms"].as_f64()?;
    let elapsed_ms = value["elapsed_ms"].as_f64()?;
    let diagnostics = value["diagnostics"].as_u64()?;
    if !method_ms.is_finite() || !(0.0..=5000.0).contains(&method_ms)
        || !elapsed_ms.is_finite() || elapsed_ms < method_ms
        || elapsed_ms > budget_ms as f64 || diagnostics > 1_000_000 { return None; }
    Some(V2OperationMetadata { method_ms, elapsed_ms, diagnostics })
}

#[cfg(test)]
#[path = "worker_v2_metadata_tests.rs"]
mod tests;

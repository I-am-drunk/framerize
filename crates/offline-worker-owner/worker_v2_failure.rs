use super::super::Failure;
use serde_json::Value;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct V2WorkerStage {
    phase: &'static str,
    operation: Option<&'static str>,
    device: Option<&'static str>,
}
impl V2WorkerStage {
    pub fn phase(self) -> &'static str { self.phase }
    pub fn operation(self) -> Option<&'static str> { self.operation }
    pub fn device(self) -> Option<&'static str> { self.device }
}
fn label(value: &Value, choices: &[&'static str]) -> Option<&'static str> {
    let text = value.as_str()?;
    choices.iter().copied().find(|candidate| *candidate == text)
}
pub(super) fn from_exit(code: Option<i32>, bytes: &[u8]) -> Failure {
    if code != Some(2) { return Failure::Worker; }
    parse(bytes).map(Failure::V2WorkerStage).unwrap_or(Failure::Worker)
}
fn parse(bytes: &[u8]) -> Option<V2WorkerStage> {
    if bytes.len() > 16384 { return None; }
    let value: Value = serde_json::from_slice(bytes).ok()?;
    let fields = ["version", "kind", "status", "failure_phase", "operation",
        "token_handoff", "mint_to_handoff_ms"];
    let object = value.as_object()?;
    if object.len() != 7 || super::members(bytes, 1) != 7
        || fields.iter().any(|key| !object.contains_key(*key))
        || value["version"].as_u64()? != 1 || value["kind"] != "framer-v2-handoff"
        || value["status"] != "failed" || value["token_handoff"].as_bool()? { return None; }
    let phase = label(&value["failure_phase"], &["descriptors", "input", "operation", "handoff"])?;
    let age = &value["mint_to_handoff_ms"];
    if !age.is_null() && !age.as_f64().is_some_and(|n| n.is_finite() && (0.0..=2000.0).contains(&n)) {
        return None;
    }
    let op = &value["operation"];
    if op.is_null() {
        if phase == "handoff" { return None; }
        return Some(V2WorkerStage { phase, operation: None, device: None });
    }
    if super::members(bytes, 2) != 17 || op.as_object()?.len() != 17 { return None; }
    if phase == "handoff" {
        super::v2_metadata::completed(&serde_json::to_vec(op).ok()?, 16000)?;
        return Some(V2WorkerStage { phase, operation: None, device: None });
    }
    if phase != "operation" || op["version"].as_u64()? != 1 || op["status"] != "failed"
        || op["sdk"] != "framer-embedded-v2" || op["selection"] != "configured-api" { return None; }
    let operation = label(&op["failure_phase"], &["input-identity", "runtime", "window",
        "device-storage", "configure", "mint", "cleanup", "deadline", "token-shape"])?;
    let device_value = op.get("device_failure")?;
    let device = if device_value.is_null() { None } else {
        Some(label(device_value, &["local_read", "local_value", "cookie_read",
            "cookie_count", "cookie_value"])?)
    };
    for key in ["device_storage_seeded", "device_storage_unchanged", "window_closed", "sources_unchanged"] {
        op.get(key)?.as_bool()?;
    }
    for key in ["configure_calls", "token_method_calls", "async_errors", "diagnostics", "late_settlements"] {
        if op.get(key)?.as_u64()? > 1_000_000 { return None; }
    }
    for key in ["method_ms", "elapsed_ms"] {
        let n = op.get(key)?.as_f64()?;
        if !n.is_finite() || !(0.0..=1_000_000.0).contains(&n) { return None; }
    }
    Some(V2WorkerStage { phase, operation: Some(operation), device })
}

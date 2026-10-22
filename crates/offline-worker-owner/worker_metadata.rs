use serde_json::Value;

#[derive(Clone, Debug)]
pub struct Metadata {
    pub method_ms: f64, pub elapsed_ms: f64,
    pub mint_to_handoff_ms: f64, pub diagnostics: u64,
}

pub fn accept(bytes: &[u8], budget_ms: u64) -> Option<Metadata> {
    accept_mode(bytes, budget_ms, false)
}

pub fn accept_mode(bytes: &[u8], budget_ms: u64, device_required: bool) -> Option<Metadata> {
    let value: Value = serde_json::from_slice(bytes).ok()?;
    let object = value.as_object()?;
    let fields = ["version", "status", "failure_phase", "target_validated",
        "account_state_loaded", "wrapper_calls", "branch", "dynamic_imports",
        "v3_evaluated", "window_closed", "inspector_closed", "sources_unchanged",
        "async_errors", "diagnostics", "late_settlements", "method_ms",
        "elapsed_ms", "mint_to_handoff_ms", "token_handoff"];
    let device_fields = ["device_storage_seeded", "device_storage_unchanged"];
    if object.len() != fields.len() + if device_required { 2 } else { 0 }
        || fields.iter().any(|key| !object.contains_key(*key)) {
        return None;
    }
    if device_required && device_fields.iter().any(|key| value[*key].as_bool() != Some(true)) {
        return None;
    }
    let expected_version = if device_required { 2 } else { 1 };
    if value["version"].as_u64()? != expected_version || value["status"].as_str()? != "passed"
        || !value["failure_phase"].is_null() || value["account_state_loaded"].as_bool()?
        || value["wrapper_calls"].as_u64()? != 1 || value["dynamic_imports"].as_u64()? != 1
        || value["async_errors"].as_u64()? != 0 || value["late_settlements"].as_u64()? != 0 {
        return None;
    }
    for key in ["target_validated", "v3_evaluated", "window_closed",
        "inspector_closed", "sources_unchanged", "token_handoff"] {
        if value[key].as_bool()? != true { return None; }
    }
    let branch = value["branch"].as_object()?;
    if branch.len() != 4 || branch.get("v3")?.as_u64()? != 1 { return None; }
    for key in ["v2", "none", "errors"] {
        if branch.get(key)?.as_u64()? != 0 { return None; }
    }
    let method_ms = value["method_ms"].as_f64()?;
    let elapsed_ms = value["elapsed_ms"].as_f64()?;
    let diagnostics = value["diagnostics"].as_u64()?;
    if !method_ms.is_finite() || method_ms < 0.0 || method_ms > 11000.0
        || !elapsed_ms.is_finite() || elapsed_ms < method_ms
        || elapsed_ms > budget_ms as f64 || diagnostics > 1000000 { return None; }
    let mint_to_handoff_ms = value["mint_to_handoff_ms"].as_f64()?;
    if !mint_to_handoff_ms.is_finite() || !(0.0..=2000.0).contains(&mint_to_handoff_ms) {
        return None;
    }
    Some(Metadata { method_ms, elapsed_ms, mint_to_handoff_ms, diagnostics })
}

pub fn token(mut bytes: Vec<u8>) -> Option<String> {
    if bytes.len() < 12 || &bytes[..8] != b"FSDK\x01\0\0\0" { bytes.fill(0); return None; }
    let length = u32::from_be_bytes(bytes[8..12].try_into().ok()?) as usize;
    let result = if length > 0 && length <= 16384 && bytes.len() == 12 + length {
        std::str::from_utf8(&bytes[12..]).ok().map(str::to_owned)
    } else { None };
    bytes.fill(0);
    result
}

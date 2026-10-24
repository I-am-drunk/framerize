use super::Failure;
use serde_json::Value;

#[cfg(test)]
#[path = "worker_failure_tests.rs"]
mod tests;

pub(super) fn from_exit(code: Option<i32>, bytes: &[u8]) -> Failure {
    if code != Some(2) { return Failure::Worker; }
    stage(bytes).map(Failure::WorkerStage).unwrap_or(Failure::Worker)
}

const FLAGS: &[&str] = &["target_validated", "account_state_loaded", "v3_evaluated",
    "window_closed", "inspector_closed", "sources_unchanged", "token_handoff"];
const COUNTS: &[&str] = &["wrapper_calls", "dynamic_imports", "async_errors",
    "diagnostics", "late_settlements"];
const TIMES: &[&str] = &["method_ms", "elapsed_ms", "mint_to_handoff_ms"];

fn stage(bytes: &[u8]) -> Option<WorkerStage> {
    if bytes.len() > 16384 { return None; }
    let value: Value = serde_json::from_slice(bytes).ok()?;
    let object = value.as_object()?;
    let version = value["version"].as_u64()?;
    if !matches!(version, 1 | 2) || value["status"].as_str()? != "failed" { return None; }
    let label = value["failure_phase"].as_str()?;
    let stage = WorkerStage(STAGES.iter().copied().find(|candidate| *candidate == label)?);
    // The entrypoint's catch emits version 1 even for a version 2 input.
    if version == 1 && object.len() == 3 { return Some(stage); }
    full_shape(&value, version).then_some(stage)
}

fn full_shape(value: &Value, version: u64) -> bool {
    let Some(object) = value.as_object() else { return false; };
    let expected = 4 + FLAGS.len() + COUNTS.len() + TIMES.len() + if version == 2 { 2 } else { 0 };
    if object.len() != expected || FLAGS.iter().any(|key| !value[*key].is_boolean())
        || value["token_handoff"].as_bool() != Some(false) { return false; }
    if COUNTS.iter().any(|key| !count(&value[*key])) || TIMES.iter().any(|key|
        !value[*key].as_f64().is_some_and(|n| n.is_finite() && n >= 0.0)) { return false; }
    if version == 2 && ["device_storage_seeded", "device_storage_unchanged"].iter()
        .any(|key| !value[*key].is_boolean()) { return false; }
    let Some(branch) = value["branch"].as_object() else { return false; };
    branch.len() == 4 && ["v3", "v2", "none", "errors"].iter()
        .all(|key| branch.get(*key).is_some_and(count))
}

fn count(value: &Value) -> bool { value.as_u64().is_some_and(|n| n <= 1_000_000) }

/// A fixed label from the failed worker report, never an exception message.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WorkerStage(&'static str);
impl WorkerStage {
    pub fn as_str(self) -> &'static str { self.0 }
}

const STAGES: &[&str] = &["descriptors", "input", "input-identity", "runtime", "import",
    "window", "graphics", "device-storage", "module-link", "inspector", "module-evaluate",
    "mint", "dedicated-worker", "shared-worker", "graphics-cleanup", "branch", "deadline",
    "token-shape", "cleanup", "handoff"];

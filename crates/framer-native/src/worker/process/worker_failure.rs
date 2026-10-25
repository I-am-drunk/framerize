use super::Failure;
use serde_json::Value;

#[cfg(test)]
#[path = "worker_failure_tests.rs"]
mod tests;

pub(super) fn from_exit(code: Option<i32>, bytes: &[u8]) -> Failure {
    if code != Some(2) { return Failure::Worker; }
    stage(bytes).map(Failure::WorkerStage).unwrap_or(Failure::Worker)
}

const FLAGS: [&str; 7] = ["target_validated", "account_state_loaded", "v3_evaluated",
    "window_closed", "inspector_closed", "sources_unchanged", "token_handoff"];
const COUNTS: [&str; 5] = ["wrapper_calls", "dynamic_imports", "async_errors",
    "diagnostics", "late_settlements"];
const TIMES: [&str; 3] = ["method_ms", "elapsed_ms", "mint_to_handoff_ms"];
const BRANCH: [&str; 4] = ["v3", "v2", "none", "errors"];
const DEVICE: [&str; 2] = ["device_storage_seeded", "device_storage_unchanged"];
const PIPE: [&str; 2] = ["token_pipe_write_attempted", "token_pipe_write_completed"];

fn stage(bytes: &[u8]) -> Option<WorkerStage> {
    if bytes.len() > 16384 { return None; }
    let value: Value = serde_json::from_slice(bytes).ok()?;
    let object = value.as_object()?;
    let version = value["version"].as_u64()?;
    if !matches!(version, 1 | 2) || value["status"].as_str()? != "failed" { return None; }
    let label = value["failure_phase"].as_str()?;
    let refined = V3_STAGES.contains(&label);
    let label = STAGES.iter().chain(V3_STAGES).copied()
        .find(|candidate| *candidate == label)?;
    let mut stage = WorkerStage { label, summary: None };
    // Early entrypoint errors emit version 1 even for a version 2 input.
    if version == 1 && object.len() == 3 { return (!refined).then_some(stage); }
    stage.summary = Some(summary(&value, version, label)?);
    if refined && value["branch"]["errors"].as_u64()? == 0 { return None; }
    Some(stage)
}

fn summary(value: &Value, version: u64, label: &str) -> Option<WorkerSummary> {
    let object = value.as_object()?;
    let extended = PIPE.iter().any(|key| object.contains_key(*key));
    let expected = 4 + FLAGS.len() + COUNTS.len() + TIMES.len()
        + (if version == 2 { 2 } else { 0 }) + (if extended { 2 } else { 0 });
    if object.len() != expected || value["token_handoff"].as_bool()? { return None; }
    let flags = fields(value, FLAGS, Value::as_bool)?;
    let counts = fields(value, COUNTS, bounded_count)?;
    let branch = fields(&value["branch"], BRANCH, bounded_count)?;
    if value["branch"].as_object()?.len() != BRANCH.len() { return None; }
    let device = if version == 2 { Some(fields(value, DEVICE, Value::as_bool)?) } else { None };
    let pipe = if extended {
        let pipe = fields(value, PIPE, Value::as_bool)?;
        if label != "handoff" || (pipe[1] && !pipe[0]) { return None; }
        Some(pipe)
    } else { None };
    let age_value = value.get("mint_to_handoff_ms")?;
    let age = if extended && age_value.is_null() { None }
        else { Some(milliseconds(age_value)?) };
    Some(WorkerSummary { version, flags, counts, branch, device, pipe,
        times: [Some(milliseconds(&value["method_ms"])?),
            Some(milliseconds(&value["elapsed_ms"])?), age] })
}

fn fields<T, const N: usize>(value: &Value, keys: [&str; N],
    parse: fn(&Value) -> Option<T>) -> Option<[T; N]> {
    keys.into_iter().map(|key| parse(value.get(key)?))
        .collect::<Option<Vec<_>>>()?.try_into().ok()
}

fn bounded_count(value: &Value) -> Option<u64> {
    value.as_u64().filter(|n| *n <= 1_000_000)
}

fn milliseconds(value: &Value) -> Option<u64> {
    let n = value.as_f64()?;
    (n.is_finite() && n >= 0.0).then_some(n.to_bits())
}

/// A fixed label from the failed worker report, never an exception message.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WorkerStage {
    label: &'static str,
    summary: Option<WorkerSummary>,
}
impl WorkerStage {
    pub fn as_str(self) -> &'static str { self.label }
    pub fn summary(&self) -> Option<&WorkerSummary> { self.summary.as_ref() }
}

/// Validated worker observations only, never parent receipt or process cleanup.
/// Finite nonnegative timings retain their bits so failure remains Copy + Eq.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WorkerSummary {
    version: u64,
    flags: [bool; 7], counts: [u64; 5], branch: [u64; 4],
    times: [Option<u64>; 3], device: Option<[bool; 2]>, pipe: Option<[bool; 2]>,
}

impl WorkerSummary {
    pub fn version(&self) -> u64 { self.version }
    pub fn flags(&self) -> impl Iterator<Item = (&'static str, bool)> + '_ {
        FLAGS.into_iter().zip(self.flags)
            .chain(self.device.into_iter().flat_map(|v| DEVICE.into_iter().zip(v)))
            .chain(self.pipe.into_iter().flat_map(|v| PIPE.into_iter().zip(v)))
    }
    pub fn counts(&self) -> impl Iterator<Item = (&'static str, u64)> + '_ {
        COUNTS.into_iter().zip(self.counts).chain(
            ["branch_v3", "branch_v2", "branch_none", "branch_errors"]
                .into_iter().zip(self.branch))
    }
    pub fn timings(&self) -> impl Iterator<Item = (&'static str, Option<f64>)> + '_ {
        TIMES.into_iter().zip(self.times.map(|v| v.map(f64::from_bits)))
    }
}

const STAGES: &[&str] = &["descriptors", "input", "input-identity", "runtime", "import",
    "window", "graphics", "device-storage", "module-link", "inspector", "module-evaluate",
    "mint", "dedicated-worker", "shared-worker", "graphics-cleanup", "branch", "deadline",
    "token-shape", "cleanup", "handoff"];

const V3_STAGES: &[&str] = &["v3-module-load-error", "v3-module-load-timeout",
    "v3-configuration-error", "v3-configuration-timeout", "v3-token-generation-error",
    "v3-token-generation-timeout", "v3-sdk-error"];

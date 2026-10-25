//! Explicit V2 handoff. The ordinary Worker retains its V3-only decoder.
use super::Failure;
use super::Run;
use super::SavedDevice;
use super::Specification;
use super::Target;
use super::Worker;
use super::v2_metadata;
use super::v2_metadata::V2OperationMetadata;
use std::time::Instant;
#[path = "worker_v2_failure.rs"] mod failure;
pub use failure::V2WorkerStage;

pub struct V2Worker(Worker);
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct V2Metadata {
    pub operation: V2OperationMetadata,
    pub mint_to_handoff_ms: f64,
}
impl V2Worker {
    pub fn new(specification: Specification) -> Result<Self, Failure> {
        Worker::new(specification).map(Self)
    }
    pub fn begin_with_device(&self, target: Target, device: &SavedDevice,
        deadline: Instant) -> Result<Run<V2Metadata>, Failure> {
        self.0.begin_protocol(target, Some(device), deadline, accept, failure::from_exit)
    }
}

// Called only after serde has established valid JSON. Count raw members so
// duplicate keys cannot disappear during Value construction, including escapes.
fn members(bytes: &[u8], wanted: usize) -> usize {
    let (mut depth, mut count, mut string, mut escape) = (0usize, 0, false, false);
    for &byte in bytes {
        if string {
            if escape { escape = false; }
            else if byte == b'\\' { escape = true; }
            else if byte == b'"' { string = false; }
        } else { match byte {
            b'"' => string = true, b'{' | b'[' => depth += 1,
            b'}' | b']' => depth -= 1,
            b':' if depth == wanted => count += 1, _ => (),
        } }
    }
    count
}

fn accept(bytes: &[u8], budget_ms: u64, device: bool) -> Option<V2Metadata> {
    if !device || bytes.len() > 16384 { return None; }
    let value: serde_json::Value = serde_json::from_slice(bytes).ok()?;
    let object = value.as_object()?;
    let fields = ["version", "kind", "status", "failure_phase", "operation",
        "token_handoff", "mint_to_handoff_ms"];
    if object.len() != fields.len() || members(bytes, 1) != fields.len()
        || members(bytes, 2) != 17
        || fields.iter().any(|key| !object.contains_key(*key))
        || value["version"].as_u64()? != 1
        || value["kind"].as_str()? != "framer-v2-handoff"
        || value["status"].as_str()? != "passed"
        || !value["failure_phase"].is_null()
        || value["token_handoff"].as_bool()? != true { return None; }
    let age = value["mint_to_handoff_ms"].as_f64()?;
    if !age.is_finite() || !(0.0..=2000.0).contains(&age) { return None; }
    let operation = serde_json::to_vec(&value["operation"]).ok()?;
    Some(V2Metadata { operation: v2_metadata::completed(&operation, budget_ms)?,
        mint_to_handoff_ms: age })
}

#[cfg(test)]
#[path = "worker_v2_tests.rs"]
mod tests;

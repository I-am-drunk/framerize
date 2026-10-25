//! Classify while the original bounded body is still privately owned.
use super::port;
use super::RefusalObservation;
use super::BYTE_LIMIT;

fn same_metadata(a: &port::AttemptMetadata, b: &port::AttemptMetadata) -> bool {
    a.status == b.status && a.step_index == b.step_index && a.elapsed == b.elapsed
        && a.usage_header == b.usage_header
        && a.original_content_type == b.original_content_type
}

pub(super) fn observe(error: &port::DispatchError) -> Option<RefusalObservation> {
    if error.kind != port::FailureKind::Upstream { return None; }
    let original = error.upstream.as_ref()?;
    if original.metadata.status != 403 || original.metadata.step_index != 0
        || original.body.len() > BYTE_LIMIT {
        return None;
    }
    // The one-shot adapter cannot attribute a later or mismatched attempt to
    // its first status. Missing original body never falls back to that status.
    if !error.attempts.is_empty() && (error.attempts.len() != 1
        || !same_metadata(&error.attempts[0], &original.metadata)) {
        return None;
    }
    let body = std::str::from_utf8(&original.body).ok()?;
    let elapsed = original.metadata.elapsed.as_millis().min(u64::MAX as u128) as u64;
    // This duration includes authorization and cleanup. Its fallback is
    // explicitly labelled and must not become positive body evidence.
    Some(framer_api::classify::observe_403(body, elapsed))
}

#[cfg(test)]
#[path = "refusal_evidence_tests.rs"]
mod tests;

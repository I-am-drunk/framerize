//! Classify while the original bounded body is still privately owned.
use super::port;
use super::RefusalObservation;
use super::BYTE_LIMIT;

pub(super) fn same_metadata(a: &port::AttemptMetadata, b: &port::AttemptMetadata) -> bool {
    a.status == b.status && a.step_index == b.step_index && a.elapsed == b.elapsed
        && a.usage_header == b.usage_header
        && a.original_content_type == b.original_content_type
}

pub(super) fn observe(error: &port::DispatchError) -> Option<RefusalObservation> {
    observe_with_limit(error, 1)
}

pub(super) fn observe_with_limit(error: &port::DispatchError, max_attempts: usize)
    -> Option<RefusalObservation> {
    if error.kind != port::FailureKind::Upstream { return None; }
    let original = error.upstream.as_ref()?;
    if original.metadata.status != 403 || original.metadata.step_index as usize >= max_attempts
        || original.body.len() > BYTE_LIMIT {
        return None;
    }
    let mut prior = error.attempts.as_slice();
    if prior.last().is_some_and(|last| same_metadata(last, &original.metadata)) {
        prior = &prior[..prior.len() - 1];
    }
    if prior.len() != original.metadata.step_index as usize
        || prior.iter().enumerate().any(|(index, value)| value.step_index as usize != index
            || value.status != 200 || super::content_type(value.original_content_type.as_deref())
                != super::ContentType::Json) {
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

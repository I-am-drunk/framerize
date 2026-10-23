//! Evidence for a body whose native completion and independent cleanup joined.
use crate::native_dispatch::http_stream::{NativeStreamCompletion, NativeStreamObservers};

pub(super) async fn observe(observers: NativeStreamObservers) -> bool {
    let (completion, cleanup) = tokio::join!(observers.completion, observers.cleanup);
    let confirmed = cleanup.is_ok();
    framer_core::observability::Event::new("native.stream.complete")
        .field("completion", fields(&completion, confirmed)).emit();
    confirmed
}

fn fields(completion: &NativeStreamCompletion, cleanup_confirmed: bool) -> serde_json::Value {
    let (kind, result, attempts, status, bytes, elapsed) = match completion {
        NativeStreamCompletion::Raw(value) => ("raw", value.result,
            usize::from(value.headers.is_some()), value.headers.as_ref().map(|h| h.status),
            value.emitted_bytes, value.elapsed),
        NativeStreamCompletion::Composed(value) => ("composed", value.result,
            value.attempts.len(), value.attempts.last().map(|a| a.headers.status),
            value.emitted_bytes, value.elapsed),
    };
    // Never serialize headers, provider IDs, usage, request context or text.
    // Observed metadata is not a count of all transport starts; yielded bytes
    // are not delivery proof. Missing events do not establish inactivity.
    serde_json::json!({
        "kind": kind,
        "body_failure": result.err().map(|failure| format!("{failure:?}")),
        "cleanup_confirmed": cleanup_confirmed,
        "observed_attempts": attempts,
        "last_status": status,
        "yielded_bytes": bytes,
        "body_elapsed_ms": elapsed.as_millis().min(u64::MAX as u128) as u64,
    })
}

#[cfg(test)]
#[path = "native_stream_evidence_tests.rs"]
mod tests;

//! Process-wide send accounting. A send invocation is not upstream acceptance.
use std::sync::atomic::{AtomicU64, Ordering};

static INVOCATIONS: AtomicU64 = AtomicU64::new(0);

pub(crate) fn total() -> u64 { INVOCATIONS.load(Ordering::Acquire) }

pub(super) fn invoked() -> Result<(), String> {
    let count = INVOCATIONS.fetch_update(Ordering::AcqRel, Ordering::Acquire,
        |value| value.checked_add(1))
        .map_err(|_| "native-http: send counter exhausted".to_owned())? + 1;
    // Count before calling send, including errors or cancellation before headers.
    // Never include account, route, authorization, request or provider data.
    framer_core::observability::Event::new("native.http.send")
        .field("invocation", count).emit();
    Ok(())
}

pub(crate) fn settled(cleanup_confirmed: bool) {
    // The caller awaits the original complete stream cleanup before this read.
    // A cumulative snapshot exposes missing send events in a fresh generation.
    framer_core::observability::Event::new("native.stream.sends")
        .field("sends", serde_json::json!({
            "send_invocations": total(),
            "cleanup_confirmed": cleanup_confirmed,
        })).emit();
}

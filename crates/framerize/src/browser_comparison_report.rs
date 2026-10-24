//! Only typed, bounded coordinator facts reach the parent. Never provider data.
use crate::one_shot_control::comparison::Report;
use serde_json::{json, Value};

pub(super) fn bounded(report: &Report) -> Value {
    let baseline = report.baseline.as_ref().map(|b| json!({
        "accepted": b.accepted, "status": b.status,
        "content_type": format!("{:?}", b.content_type),
        "response_complete": b.response_complete, "complete_json": b.complete_json,
        "nonempty_text": b.nonempty_text, "body_bytes": b.body_bytes,
        "transport_starts": b.transport_starts, "transport_finishes": b.transport_finishes,
        "fetch_settled": b.fetch_settled,
        "refusal": b.refusal.map(|r| json!({
            "policy": format!("{:?}", r.policy), "basis": format!("{:?}", r.basis)})),
    }));
    let native = report.native.as_ref().map(|n| json!({
        "complete_json": n.complete_json, "status": n.status,
        "attempts": n.attempts, "authorization_calls": n.authorization_calls,
        "transport_starts": n.transport_starts, "local_cleanup": n.local_cleanup,
        "failed": n.failure.is_some(),
        "refusal": n.refusal.map(|r| json!({
            "policy": format!("{:?}", r.policy), "basis": format!("{:?}", r.basis)})),
    }));
    json!({"baseline": baseline, "native": native,
        "native_nonempty_text": report.native_nonempty_text,
        "browser_cleanup_confirmed": report.browser_cleanup_confirmed,
        "account_closed": report.account_closed,
        "failure": report.failure.map(|f| format!("{f:?}"))})
}

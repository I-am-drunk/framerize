use super::*;
use super::super::{Refusal, RefusalBasis};
use bytes::Bytes;
use std::time::Duration;

fn error(status: u16, body: impl Into<Bytes>) -> port::DispatchError {
    port::DispatchError {
        kind: port::FailureKind::Upstream,
        upstream: Some(port::UpstreamFailure { body: body.into(),
            metadata: port::AttemptMetadata { status, step_index: 0,
                usage_header: None, original_content_type: Some("application/json".into()),
                elapsed: Duration::from_millis(250) } }),
        attempts: vec![],
    }
}

fn security_body() -> serde_json::Value {
    serde_json::json!({"error": {"status":403,
        "message":"This request was blocked by our security checks",
        "data":{"reason":"request_blocked"}, "ref":"private-reference",
        "timestamp":"private-timestamp"}})
}

#[test]
fn body_markers_preserve_policy_and_expose_its_basis() {
    for (body, policy, basis) in [
        ("Forbidden", Refusal::Castle, RefusalBasis::FramerMarker),
        ("provider refused", Refusal::Upstream, RefusalBasis::ProviderMarker),
        ("provider says Forbidden", Refusal::Upstream, RefusalBasis::ProviderMarker),
    ] {
        let observed = observe(&error(403, body)).unwrap();
        assert_eq!(observed, RefusalObservation { policy, basis });
        assert_eq!(crate::classify::classify_403(body, 250), policy);
    }
}

#[test]
fn structured_refusal_and_provider_override_remain_distinct() {
    let mut body = security_body();
    let mut e = error(403, body.to_string());
    e.upstream.as_mut().unwrap().metadata.elapsed = Duration::from_secs(30);
    assert_eq!(observe(&e), Some(RefusalObservation { policy: Refusal::Castle,
        basis: RefusalBasis::FramerSecurityEnvelope }));
    body["request_id"] = serde_json::json!("gen-private-provider-reference");
    assert_eq!(observe(&error(403, body.to_string())), Some(RefusalObservation {
        policy: Refusal::Upstream, basis: RefusalBasis::ProviderMarker }));
}

#[test]
fn empty_or_ambiguous_bodies_are_explicitly_timing_only() {
    for body in ["", "unknown private response"] {
        for (ms, policy) in [(250, Refusal::Castle), (1500, Refusal::Upstream)] {
            let mut e = error(403, body);
            e.upstream.as_mut().unwrap().metadata.elapsed = Duration::from_millis(ms);
            assert_eq!(observe(&e), Some(RefusalObservation { policy,
                basis: RefusalBasis::ElapsedFallback }));
        }
    }
}

#[test]
fn status_without_original_body_cannot_classify() {
    let mut e = error(403, "Forbidden");
    e.attempts.push(e.upstream.take().unwrap().metadata);
    assert_eq!(observe(&e), None);
}

#[test]
fn non_inference_errors_and_other_statuses_are_unclassified() {
    for status in [200, 401, 402, 429, 500, 503] {
        assert_eq!(observe(&error(status, "Forbidden")), None);
    }
    for kind in [port::FailureKind::Authorization, port::FailureKind::Body,
        port::FailureKind::BodyTooLarge, port::FailureKind::Cleanup,
        port::FailureKind::Transport, port::FailureKind::Cancelled] {
        let mut e = error(403, "Forbidden");
        e.kind = kind;
        assert_eq!(observe(&e), None);
    }
}

#[test]
fn invalid_or_oversized_bodies_do_not_gain_a_classification() {
    assert_eq!(observe(&error(403, vec![0xff, 0xfe])), None);
    assert_eq!(observe(&error(403, format!("Forbidden{}", "x".repeat(BYTE_LIMIT)))), None);
}

#[test]
fn inconsistent_or_multiple_attempts_are_not_misattributed() {
    for changed in 0..7 {
        let mut e = error(403, "Forbidden");
        e.attempts.push(e.upstream.as_ref().unwrap().metadata.clone());
        match changed {
            0 => e.attempts.push(e.attempts[0].clone()),
            1 => e.attempts[0].status = 200,
            2 => e.attempts[0].step_index = 1,
            3 => e.attempts[0].elapsed += Duration::from_millis(1),
            4 => e.attempts[0].usage_header = Some("different-private-usage".into()),
            5 => e.attempts[0].original_content_type = None,
            _ => { e.attempts.clear(); e.upstream.as_mut().unwrap().metadata.step_index = 1; }
        }
        assert_eq!(observe(&e), None);
    }
    let mut e = error(403, "Forbidden");
    e.attempts.push(e.upstream.as_ref().unwrap().metadata.clone());
    assert!(observe(&e).is_some());
}

#[test]
fn private_text_is_dropped_after_failure_projection() {
    let facts = super::super::failure(error(403, security_body().to_string()));
    let observed = facts.refusal.unwrap();
    assert_eq!(observed.basis, RefusalBasis::FramerSecurityEnvelope);
    assert_eq!(facts.failure, Some(port::FailureKind::Upstream));
    assert!(!facts.complete);
    let output = format!("{observed:?}");
    for text in ["private-reference", "private-timestamp", "request_blocked"] {
        assert!(!output.contains(text));
    }
}

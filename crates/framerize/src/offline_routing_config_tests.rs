use super::*;
use serde_json::{json, Value};

fn config() -> Value {
    json!({"timeout_ms":60000,"max_body_bytes":65536,"missing_content_type":"application/json",
        "chunk_tokens":512,"max_chunks":2,"continuation":"explicit continuation",
        "allow_paid":false,"credit_floor":1.0,"streaming":null})
}

#[test]
fn routing_config_preserves_explicit_limits_and_continuation() {
    let (policy, streaming) = decode(config().to_string().as_bytes()).unwrap();
    assert_eq!(policy.timeout, Duration::from_secs(60));
    assert_eq!(policy.chunk_tokens, 512);
    assert_eq!(policy.max_chunks, 2);
    assert_eq!(policy.continuation.as_deref(), Some("explicit continuation"));
    assert!(!policy.allow_paid && streaming.is_none());
}

#[test]
fn routing_config_rejects_invalid_limits_and_unproved_paid_credit() {
    for (field, value) in [("timeout_ms", json!(0)), ("max_chunks", json!(65)),
        ("max_body_bytes", json!(67108865)), ("paid_credit", json!(1000)),
        ("page_url", json!("https://example.invalid"))] {
        let mut c = config(); c[field] = value;
        assert!(decode(c.to_string().as_bytes()).is_err());
    }
}

#[test]
fn routing_config_allows_paid_policy_without_accepting_credit_evidence() {
    let mut c = config();
    c["allow_paid"] = json!(true);
    let (policy, _) = decode(c.to_string().as_bytes()).unwrap();
    assert!(policy.allow_paid);
    assert_eq!(policy.credit_floor, 1.0);
    c["paid_credit"] = json!({"available":1000});
    assert!(decode(c.to_string().as_bytes()).is_err());
}

#[test]
fn routing_config_checks_both_streaming_budgets() {
    let mut c = config();
    c["streaming"] = json!({"raw":{"chunk_bytes":1024,"retained_bytes":4096,
        "total_bytes":65536,"header_bytes":4096}, "composed":{"frame_bytes":4096,
        "retained_bytes":8192,"retained_frames":4,"state_bytes":65536,"json_bytes":8192,
        "source_chunk_bytes":1024,"header_bytes":4096,"total_bytes":65536,"terminal_bytes":1024},
        "heartbeat_ms":1000});
    let (_, streaming) = decode(c.to_string().as_bytes()).unwrap();
    assert_eq!(streaming.unwrap().heartbeat, Some(Duration::from_secs(1)));
    for (group, field, value) in [("raw", "retained_bytes", json!(1)),
        ("composed", "json_bytes", json!(0)), ("composed", "retained_frames", json!(0))] {
        let mut bad = c.clone(); bad["streaming"][group][field] = value;
        assert!(decode(bad.to_string().as_bytes()).is_err());
    }
}

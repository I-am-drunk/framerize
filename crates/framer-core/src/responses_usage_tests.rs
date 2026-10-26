use super::{responses_json, AssembledResponses, ResponsesComposer};
use serde_json::{json, Value};

fn result(total: Option<Value>) -> AssembledResponses {
    let mut usage = json!({"input_tokens":11, "output_tokens":7,
        "input_tokens_details":{"cached_tokens":3},
        "output_tokens_details":{"reasoning_tokens":2}, "provider_extension":"retained"});
    if let Some(total) = total {
        usage["total_tokens"] = total;
    }
    AssembledResponses { id:"upstream-response".into(), created_at:123,
        output:vec![], status:"completed".into(), incomplete_details:None,
        usage, service_tier:json!("priority"), input_tokens:11, output_tokens:14,
        chunks:2, metered_chunks:0 }
}

fn expected_usage() -> Value {
    json!({"input_tokens":11, "output_tokens":14, "total_tokens":25,
        "input_tokens_details":{"cached_tokens":3},
        "output_tokens_details":{"reasoning_tokens":2}, "provider_extension":"retained"})
}

fn completed_usage(result: &AssembledResponses) -> Value {
    let frames = ResponsesComposer::new("fixture-model", "local-response").finish(result);
    let events: Vec<Value> = frames.iter().map(|frame| {
        let data = frame.lines().find_map(|line| line.strip_prefix("data: ")).unwrap();
        serde_json::from_str::<Value>(data).unwrap()
    }).filter(|event| event["type"] == "response.completed").collect();
    assert_eq!(events.len(), 1);
    events[0]["response"]["usage"].clone()
}

#[test]
fn json_recomputes_stale_total_and_retains_metadata() {
    let actual = responses_json(&result(Some(json!(18))), "fixture-model", "local-response");
    assert_eq!(actual["usage"], expected_usage());
}

#[test]
fn json_recomputes_null_total_and_retains_metadata() {
    let actual = responses_json(&result(Some(Value::Null)), "fixture-model", "local-response");
    assert_eq!(actual["usage"], expected_usage());
}

#[test]
fn completed_recomputes_stale_total_and_retains_metadata() {
    assert_eq!(completed_usage(&result(Some(json!(18)))), expected_usage());
}

#[test]
fn completed_recomputes_null_total_and_retains_metadata() {
    assert_eq!(completed_usage(&result(Some(Value::Null))), expected_usage());
}

#[test]
fn absent_total_is_filled_in_both_serializers() {
    let result = result(None);
    assert_eq!(responses_json(&result, "fixture-model", "local-response")["usage"], expected_usage());
    assert_eq!(completed_usage(&result), expected_usage());
}

#[test]
fn matching_total_remains_consistent_in_both_serializers() {
    let result = result(Some(json!(25)));
    assert_eq!(responses_json(&result, "fixture-model", "local-response")["usage"], expected_usage());
    assert_eq!(completed_usage(&result), expected_usage());
}

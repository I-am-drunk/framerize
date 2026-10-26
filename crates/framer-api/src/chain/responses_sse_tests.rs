use super::*;

fn completed(id: &str) -> Value {
    json!({"type":"response.completed", "response":{
        "id":id, "created_at":9, "status":"completed", "service_tier":"default",
        "output":[{"type":"function_call", "name":"fixture", "call_id":"call-fixture",
            "arguments":"{\"x\":1}"}],
        "usage":{"input_tokens":11,"output_tokens":7,"input_tokens_details":{"cached_tokens":3}}
    }})
}

#[test]
fn spaced_completion_preserves_tool_output_and_usage() {
    let event = completed("fixture-response");
    let actual = parse_streamed_chunk(&format!("data: {event}\n\ndata: [DONE]\n\n")).unwrap();
    assert_eq!(actual.id, "fixture-response");
    assert_eq!(actual.created_at, 9);
    assert_eq!(actual.status, "completed");
    assert_eq!(actual.output, event["response"]["output"].as_array().unwrap().clone());
    assert_eq!(actual.usage, event["response"]["usage"]);
    assert_eq!(actual.service_tier, "default");
    assert_eq!((actual.input_tokens, actual.output_tokens), (11, 7));
    assert_eq!((actual.chunks, actual.metered_chunks), (1, 1));
}

#[test]
fn completion_without_a_space_after_colon_is_accepted() {
    let actual = parse_streamed_chunk(&format!("data:{}\n\n", completed("fixture-response"))).unwrap();
    assert_eq!(actual.id, "fixture-response");
}

#[test]
fn multiline_crlf_completion_is_accepted() {
    let pretty = serde_json::to_string_pretty(&completed("fixture-response")).unwrap();
    let lines: Vec<_> = pretty.lines().map(|line| format!("data: {line}\r\n")).collect();
    let actual = parse_streamed_chunk(&(lines.join("") + "\r\n")).unwrap();
    assert_eq!(actual.id, "fixture-response");
    assert_eq!(actual.output[0]["arguments"], "{\"x\":1}");
}

#[test]
fn comments_and_other_fields_do_not_split_data() {
    let pretty = serde_json::to_string_pretty(&completed("fixture-response")).unwrap();
    let mut lines = vec!["event: response.completed".to_string(), "id: fixture".into()];
    for line in pretty.lines() {
        lines.push(format!("data: {line}"));
        lines.push(": keepalive".into());
    }
    lines.extend([String::new(), String::new()]);
    assert_eq!(parse_streamed_chunk(&lines.join("\n")).unwrap().id, "fixture-response");
}

#[test]
fn last_completion_without_final_blank_separator_is_preserved() {
    let body = format!("data: {}\n\ndata: {}", completed("first"), completed("last"));
    assert_eq!(parse_streamed_chunk(&body).unwrap().id, "last");
}

#[test]
fn missing_or_partial_completion_remains_an_error() {
    for body in ["", ": keepalive\n\n", "data: [DONE]\n\n",
        "data: {\"type\":\"response.completed\"\n\n",
        "data: {\"type\":\"response.output_text.delta\",\"delta\":\"partial\"}\n\n"] {
        assert!(parse_streamed_chunk(body).is_err());
    }
}

fn frame(event: &Value) -> String {
    format!("data: {event}\n\n")
}

#[test]
fn terminal_response_must_be_an_object() {
    for body in [Value::Null, json!(true), json!(1), json!("fixture"), json!([])] {
        let event = json!({"type":"response.completed", "response":body});
        assert_eq!(parse_streamed_chunk(&frame(&event)).unwrap_err(),
            "streamed chunk has invalid response object");
    }
}

#[test]
fn terminal_output_must_be_an_array() {
    let mut event = completed("fixture");
    event["response"].as_object_mut().unwrap().remove("output");
    assert_eq!(parse_streamed_chunk(&frame(&event)).unwrap_err(),
        "streamed chunk has invalid output array");
    for output in [Value::Null, json!(true), json!(1), json!("fixture"), json!({})] {
        event["response"]["output"] = output;
        assert_eq!(parse_streamed_chunk(&frame(&event)).unwrap_err(),
            "streamed chunk has invalid output array");
    }
}

#[test]
fn failed_or_nonterminal_status_is_not_a_completion() {
    for status in [json!("failed"), json!("cancelled"), json!("in_progress"),
        json!("queued"), json!("fixture-private-status"), Value::Null, json!(1), json!({})] {
        let mut event = completed("fixture");
        event["response"]["status"] = status;
        assert_eq!(parse_streamed_chunk(&frame(&event)).unwrap_err(),
            "streamed chunk has invalid terminal status");
    }
}

#[test]
fn malformed_usage_is_rejected_before_assembly() {
    for usage in [json!("fixture-private-usage"), json!(1), json!(true), json!([])] {
        let mut event = completed("fixture");
        event["response"]["usage"] = usage;
        assert_eq!(parse_streamed_chunk(&frame(&event)).unwrap_err(),
            "streamed chunk has invalid usage object");
    }
}

#[test]
fn explicit_failure_before_completion_is_not_hidden() {
    for kind in ["error", "response.failed", "response.incomplete"] {
        let failure = json!({"type":kind, "message":"fixture-private-diagnostic"});
        let body = frame(&failure) + &frame(&completed("fixture"));
        assert_eq!(parse_streamed_chunk(&body).unwrap_err(), format!("streamed chunk ended with {kind}"));
    }
}

#[test]
fn explicit_failure_after_completion_is_not_hidden() {
    for kind in ["error", "response.failed", "response.incomplete"] {
        let failure = json!({"type":kind, "message":"fixture-private-diagnostic"});
        let body = frame(&completed("fixture")) + &frame(&failure);
        assert_eq!(parse_streamed_chunk(&body).unwrap_err(), format!("streamed chunk ended with {kind}"));
    }
}

#[test]
fn empty_completed_output_remains_supported() {
    let mut event = completed("fixture");
    event["response"]["output"] = json!([]);
    let result = parse_streamed_chunk(&frame(&event)).unwrap();
    assert!(result.output.is_empty());
    assert_eq!(result.status, "completed");
}

#[test]
fn completion_without_status_retains_the_event_contract() {
    let mut event = completed("fixture");
    event["response"].as_object_mut().unwrap().remove("status");
    let result = parse_streamed_chunk(&frame(&event)).unwrap();
    assert_eq!(result.status, "completed");
    assert_eq!(result.output, event["response"]["output"].as_array().unwrap().clone());
}

#[test]
fn incomplete_terminal_body_keeps_its_output_and_reason() {
    let mut event = completed("fixture");
    event["response"]["status"] = json!("incomplete");
    event["response"]["incomplete_details"] = json!({"reason":"max_output_tokens"});
    let result = parse_streamed_chunk(&frame(&event)).unwrap();
    assert_eq!(result.status, "incomplete");
    assert_eq!(result.incomplete_details, Some(json!({"reason":"max_output_tokens"})));
    assert_eq!(result.output, event["response"]["output"].as_array().unwrap().clone());
}

#[test]
fn absent_or_null_usage_remains_optional() {
    for omit in [false, true] {
        let mut event = completed("fixture");
        if omit {
            event["response"].as_object_mut().unwrap().remove("usage");
        } else {
            event["response"]["usage"] = Value::Null;
        }
        let result = parse_streamed_chunk(&frame(&event)).unwrap();
        assert_eq!((result.input_tokens, result.output_tokens), (0, 0));
        assert_eq!(result.status, "completed");
    }
}

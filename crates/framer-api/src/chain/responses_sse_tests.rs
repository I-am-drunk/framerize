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

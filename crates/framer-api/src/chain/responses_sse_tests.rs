use super::*;

#[test]
fn cr_named_failure_or_invalid_terminal_cannot_hide_beside_completion() {
    let valid = frame(&completed("fixture"));
    for (kind, expected) in [
        ("error", "streamed chunk ended with error: the upstream supplied no further cause"),
        ("response.failed", "streamed chunk ended with response.failed: the upstream supplied no further cause"),
        ("response.completed", "streamed chunk has invalid named terminal event"),
        ("response.incomplete", "streamed chunk has invalid named terminal event"),
    ] {
        for ending in ["\r", "\r\r", "\r\n", "\r\n\r\n"] {
            let invalid = format!("event: {kind}{ending}");
            for body in [format!("{valid}{invalid}"), format!("{invalid}\r\r{valid}")] {
                assert_eq!(parse_streamed_chunk(&body).unwrap_err(), expected);
            }
        }
    }
}

#[test]
fn mixed_line_endings_preserve_event_overwrite_and_reset() {
    let event = completed("last");
    let pretty = serde_json::to_string_pretty(&event).unwrap();
    let data: String = pretty.lines().enumerate().map(|(i, line)| {
        let ending = ["\r", "\n", "\r\n"][i % 3];
        format!("data:{line}{ending}")
    }).collect();
    let body = format!("{}event: error\revent: response.completed\r\n{data}\r\n\
        event: response.failed\revent\r\nevent: ping\rdata: not-json\r\r",
        frame(&completed("first")));
    assert_eq!(parse_streamed_chunk(&body).unwrap().id, "last");
}

#[test]
fn cr_line_endings_preserve_terminal_output_and_metadata() {
    let mut legacy = incomplete_event("legacy");
    legacy["type"] = json!("response.completed");
    for event in [completed("α\\rβ"), incomplete_event("partial"), legacy] {
        for ending in ["\r", "\r\n", "\n"] {
            for eof in [false, true] {
                let pretty = serde_json::to_string_pretty(&event).unwrap();
                let data: String = pretty.lines()
                    .map(|line| format!("data: {line}{ending}")).collect();
                let kind = event["type"].as_str().unwrap();
                let body = format!("event: {kind}{ending}{data}{ending}");
                let body = if eof { body.trim_end_matches(['\r', '\n']) } else { &body };
                let actual = parse_streamed_chunk(body).unwrap();
                assert_eq!(actual.id, event["response"]["id"].as_str().unwrap());
                assert_eq!(actual.output, event["response"]["output"].as_array().unwrap().clone());
                assert_eq!(actual.status, event["response"]["status"].as_str().unwrap());
                assert_eq!(actual.usage, event["response"]["usage"]);
                assert_eq!(actual.incomplete_details,
                    event["response"].get("incomplete_details").cloned());
            }
        }
    }
}

#[test]
fn explicit_nonassistant_terminal_message_roles_are_rejected() {
    for role in [json!("user"), json!("system"), json!("developer"), json!("tool"),
        json!(""), json!("Assistant"), Value::Null, json!(false), json!(7), json!([]), json!({})] {
        for (kind, status) in [("response.completed", "completed"),
            ("response.completed", "incomplete"), ("response.incomplete", "incomplete")] {
            let mut terminal = completed("fixture");
            terminal["type"] = json!(kind);
            terminal["response"]["status"] = json!(status);
            terminal["response"]["output"] = json!([{"type":"message", "role":role,
                "content":[{"type":"output_text", "text":"unpublished"}]}]);
            assert_eq!(parse_streamed_chunk(&frame(&terminal)).unwrap_err(),
                "streamed chunk has invalid message role");
        }
    }
}

#[test]
fn assistant_and_absent_roles_preserve_partial_and_opaque_output() {
    for role in [None, Some(json!("assistant"))] {
        for (kind, status) in [("response.completed", "completed"),
            ("response.completed", "incomplete"), ("response.incomplete", "incomplete")] {
            let mut message = json!({"type":"message", "content":[], "extension":{"keep":true}});
            if let Some(role) = &role { message["role"] = role.clone(); }
            let mut terminal = completed("fixture");
            terminal["type"] = json!(kind);
            terminal["response"]["status"] = json!(status);
            terminal["response"]["output"] = json!([
                message, {"type":"future_item", "role":"user", "extension":7}
            ]);
            let actual = parse_streamed_chunk(&frame(&terminal)).unwrap();
            assert_eq!(actual.status, status);
            assert_eq!(actual.output, terminal["response"]["output"].as_array().unwrap().clone());
            assert_eq!(actual.usage, terminal["response"]["usage"]);
        }
    }
}

#[test]
fn terminal_response_error_is_not_accepted_stream_output() {
    for error in [json!({"code":"server_error", "message":"fixture diagnostic"}),
        json!({}), json!("fixture diagnostic"), json!(false), json!(7), json!([])] {
        for (kind, status) in [("response.completed", "completed"),
            ("response.completed", "incomplete"), ("response.incomplete", "incomplete")] {
            let mut terminal = completed("fixture");
            terminal["type"] = json!(kind);
            terminal["response"]["status"] = json!(status);
            terminal["response"]["error"] = error.clone();
            let expected = if error.is_string() || error["message"].is_string() {
                "streamed chunk contains an upstream error: fixture diagnostic"
            } else { "streamed chunk has non-null response error" };
            assert_eq!(parse_streamed_chunk(&frame(&terminal)).unwrap_err(),
                expected);
        }
    }
}

#[test]
fn absent_or_null_response_error_preserves_both_stream_statuses() {
    for error in [None, Some(Value::Null)] {
        for (kind, status) in [("response.completed", "completed"),
            ("response.completed", "incomplete"), ("response.incomplete", "incomplete")] {
            let mut terminal = completed("fixture");
            terminal["type"] = json!(kind);
            terminal["response"]["status"] = json!(status);
            terminal["response"]["output"] = json!([
                {"type":"future_item", "error":{"opaque":true}}
            ]);
            if let Some(error) = &error { terminal["response"]["error"] = error.clone(); }
            let actual = parse_streamed_chunk(&frame(&terminal)).unwrap();
            assert_eq!(actual.status, status);
            assert_eq!(actual.output, terminal["response"]["output"].as_array().unwrap().clone());
            assert_eq!(actual.usage, terminal["response"]["usage"]);
        }
    }
}

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
fn named_error_frames_are_fatal_independently_of_data() {
    let valid = frame(&completed("fixture"));
    for error in [
        "event:error\ndata: {\"message\":\"private-fixture\"}\n\n".to_string(),
        "event: error\ndata: not-json\n\n".into(),
        "event: error\ndata: [DONE]\n\n".into(),
        "data: {}\nevent:error\n\n".into(),
        "event: error\r\ndata: {\r\ndata:}\r\n\r\n".into(),
        "event: error\n\n".into(), "event: error".into(),
        format!("event: error\n{valid}"),
        "event: ping\nevent:error\ndata: {}\n\n".into(),
    ] {
        let cause = if error.contains("private-fixture") { "private-fixture" }
            else { "the upstream supplied no further cause" };
        for input in [format!("{}\n\n{valid}", error.trim_end()),
            format!("{valid}{error}")] {
            assert_eq!(parse_streamed_chunk(&input).unwrap_err(),
                format!("streamed chunk ended with error: {cause}"));
        }
    }
}

#[test]
fn event_fields_keep_last_value_and_other_name_compatibility() {
    for prefix in ["event:error\nevent: ping\n", "event:error\nevent:\n\n",
        "event:error\nevent\n\n", "event:unknown\n", "event:  error\n",
        "event:ignored\n\n"] {
        let input = format!("{prefix}{}", frame(&completed("fixture")));
        assert_eq!(parse_streamed_chunk(&input).unwrap().id, "fixture");
    }
}

#[test]
fn named_failed_frames_cannot_be_hidden_by_completion() {
    let valid = frame(&completed("fixture"));
    for data in [String::new(), "data: not-json".into(),
        "data: [DONE]".into(), "data: {}".into(), valid.clone()] {
        let failed = format!("event: response.failed\n{data}");
        for input in [format!("{failed}\n\n{valid}"), format!("{valid}{failed}")] {
            assert_eq!(parse_streamed_chunk(&input).unwrap_err(),
                "streamed chunk ended with response.failed: the upstream supplied no further cause");
        }
    }
}

#[test]
fn named_terminals_require_matching_typed_json() {
    let valid = frame(&completed("fixture"));
    for name in ["response.completed", "response.incomplete"] {
        for data in ["", "data: not-json", "data: [DONE]", "data: null",
            "data: {}", "data: {\"type\":\"response.output_text.delta\",\"delta\":\"private-fixture\"}"] {
            let invalid = format!("event: {name}\n{data}");
            for input in [format!("{invalid}\n\n{valid}"), format!("{valid}{invalid}")] {
                assert_eq!(parse_streamed_chunk(&input).unwrap_err(),
                    "streamed chunk has invalid named terminal event");
            }
        }
        let opposite = if name == "response.completed" {
            incomplete_event("other")
        } else { completed("other") };
        let input = format!("event: {name}\n{}", frame(&opposite));
        assert_eq!(parse_streamed_chunk(&input).unwrap_err(),
            "streamed chunk has invalid named terminal event");
    }
}

#[test]
fn matching_named_terminals_preserve_partial_metadata_and_frame_reset() {
    let mut legacy = incomplete_event("legacy");
    legacy["type"] = json!("response.completed");
    for event in [completed("fixture"), incomplete_event("partial"), legacy] {
        let kind = event["type"].as_str().unwrap();
        let pretty = serde_json::to_string_pretty(&event).unwrap();
        let data: String = pretty.lines().map(|line| format!("data:{line}\r\n")).collect();
        let input = format!("event: response.failed\r\nevent: {kind}\r\n{data}\r\nevent: ping\r\ndata: not-json\r\n\r\n");
        let actual = parse_streamed_chunk(&input).unwrap();
        assert_eq!(actual.output, event["response"]["output"].as_array().unwrap().clone());
        assert_eq!(actual.status, event["response"]["status"].as_str().unwrap());
        assert_eq!(actual.usage, event["response"]["usage"]);
        assert_eq!(actual.incomplete_details, event["response"].get("incomplete_details").cloned());
    }
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
fn terminal_output_items_require_objects_and_nonempty_types() {
    for bad in [Value::Null, json!(true), json!(42), json!("private-fixture"),
        json!([]), json!({}), json!({"type":null}), json!({"type":false}),
        json!({"type":42}), json!({"type":[]}), json!({"type":{}}), json!({"type":""})] {
        for (kind, status) in [("response.completed", "completed"),
            ("response.incomplete", "incomplete")] {
            let mut event = completed("fixture");
            event["type"] = json!(kind);
            event["response"]["status"] = json!(status);
            event["response"]["output"].as_array_mut().unwrap().push(bad.clone());
            assert_eq!(parse_streamed_chunk(&frame(&event)).unwrap_err(),
                "streamed chunk has invalid output item");
        }
    }
}

#[test]
fn failed_or_nonterminal_status_is_not_a_completion() {
    for (status, cause) in [
        (json!("failed"), "upstream response failed: the provider did not supply a failure cause"),
        (json!("cancelled"), "terminal status \"cancelled\""),
        (json!("in_progress"), "terminal status \"in_progress\""),
        (json!("queued"), "terminal status \"queued\""),
        (json!("fixture-private-status"), "terminal status \"fixture-private-status\""),
        (Value::Null, "terminal status null"), (json!(1), "terminal status a number"),
        (json!({}), "terminal status an object")] {
        let mut event = completed("fixture");
        event["response"]["status"] = status;
        let error = parse_streamed_chunk(&frame(&event)).unwrap_err();
        assert!(error.contains(cause), "{error}");
    }
}

#[test]
fn unknown_output_types_preserve_all_fields() {
    let output = json!([{"type":"future_output", "id":null, "status":false,
        "extension":{"keep":[1, 2, 3]}}]);
    for (kind, status) in [("response.completed", "completed"),
        ("response.incomplete", "incomplete")] {
        let mut event = completed("fixture");
        event["type"] = json!(kind);
        event["response"]["status"] = json!(status);
        event["response"]["output"] = output.clone();
        let actual = parse_streamed_chunk(&frame(&event)).unwrap();
        assert_eq!(actual.output, output.as_array().unwrap().clone());
        assert_eq!(actual.status, status);
        assert_eq!((actual.input_tokens, actual.output_tokens), (11, 7));
    }
}

#[test]
fn malformed_incomplete_output_item_is_not_hidden_by_completion() {
    let mut invalid = completed("invalid");
    invalid["type"] = json!("response.incomplete");
    invalid["response"]["status"] = json!("incomplete");
    invalid["response"]["output"] = json!([null]);
    let body = frame(&invalid) + &frame(&completed("later"));
    assert_eq!(parse_streamed_chunk(&body).unwrap_err(),
        "streamed chunk has invalid output item");
}

#[test]
fn message_content_must_be_an_array() {
    for content in [None, Some(Value::Null), Some(json!(false)), Some(json!(7)),
        Some(json!("private-fixture")), Some(json!({}))] {
        let mut item = json!({"type":"message"});
        if let Some(content) = content { item["content"] = content; }
        let mut event = completed("fixture");
        event["response"]["output"] = json!([item]);
        assert_eq!(parse_streamed_chunk(&frame(&event)).unwrap_err(),
            "streamed chunk has invalid message content");
    }
}

#[test]
fn intermediate_message_placeholders_are_not_terminal_content() {
    let mut stream = frame(&json!({"type":"response.output_item.added",
        "item":{"type":"message", "content":null}}));
    stream.push_str(&frame(&json!({"type":"response.content_part.added",
        "part":{"type":"output_text"}})));
    let terminal = completed("fixture");
    stream.push_str(&frame(&terminal));
    let actual = parse_streamed_chunk(&stream).unwrap();
    assert_eq!(actual.output, terminal["response"]["output"].as_array().unwrap().clone());
}

#[test]
fn malformed_message_parts_are_not_completed_output() {
    for (part, reason) in [
        (Value::Null, "invalid message part"), (json!(false), "invalid message part"),
        (json!(7), "invalid message part"), (json!([]), "invalid message part"),
        (json!({}), "invalid message part"), (json!({"type":null}), "invalid message part"),
        (json!({"type":""}), "invalid message part"),
        (json!({"type":"output_text"}), "invalid output text"),
        (json!({"type":"output_text", "text":7}), "invalid output text"),
        (json!({"type":"refusal"}), "invalid refusal"),
        (json!({"type":"refusal", "refusal":{}}), "invalid refusal"),
    ] {
        let mut event = completed("fixture");
        event["response"]["output"] = json!([{"type":"message", "content":[part]}]);
        assert_eq!(parse_streamed_chunk(&frame(&event)).unwrap_err(),
            format!("streamed chunk has {reason}"));
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
fn valid_message_parts_keep_empty_values_and_opaque_metadata() {
    for content in [json!([]), json!([
        {"type":"output_text", "text":"", "annotations":[{"fixture":true}]},
        {"type":"refusal", "refusal":"", "extension":[1,2]},
        {"type":"future_part", "opaque":{"untouched":true}},
    ])] {
        let item = json!({"type":"message", "content":content});
        for (kind, status) in [("response.completed", "completed"),
            ("response.incomplete", "incomplete")] {
            let mut event = completed("fixture");
            event["type"] = json!(kind);
            event["response"]["status"] = json!(status);
            event["response"]["output"] = json!([item.clone()]);
            let actual = parse_streamed_chunk(&frame(&event)).unwrap();
            assert_eq!(actual.output, vec![item.clone()]);
            assert_eq!(actual.status, status);
        }
    }
}

#[test]
fn explicit_failure_before_completion_is_not_hidden() {
    for kind in ["error", "response.failed", "response.incomplete"] {
        let failure = json!({"type":kind, "message":"fixture-private-diagnostic"});
        let body = frame(&failure) + &frame(&completed("fixture"));
        assert_eq!(parse_streamed_chunk(&body).unwrap_err(), format!("streamed chunk ended with {kind}: fixture-private-diagnostic"));
    }
}

#[test]
fn explicit_failure_after_completion_is_not_hidden() {
    for kind in ["error", "response.failed", "response.incomplete"] {
        let failure = json!({"type":kind, "message":"fixture-private-diagnostic"});
        let body = frame(&completed("fixture")) + &frame(&failure);
        assert_eq!(parse_streamed_chunk(&body).unwrap_err(), format!("streamed chunk ended with {kind}: fixture-private-diagnostic"));
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

fn incomplete_event(id: &str) -> Value {
    let mut event = completed(id);
    event["type"] = json!("response.incomplete");
    event["response"]["status"] = json!("incomplete");
    event["response"]["incomplete_details"] = json!({"reason":"max_output_tokens"});
    event
}

#[test]
fn explicit_incomplete_event_preserves_output_and_metadata() {
    let event = incomplete_event("partial");
    let actual = parse_streamed_chunk(&frame(&event)).unwrap();
    assert_eq!(actual.status, "incomplete");
    assert_eq!(actual.id, "partial");
    assert_eq!(actual.output, event["response"]["output"].as_array().unwrap().clone());
    assert_eq!(actual.incomplete_details, Some(json!({"reason":"max_output_tokens"})));
    assert_eq!(actual.usage, event["response"]["usage"]);
    assert_eq!(actual.service_tier, "default");
}

#[test]
fn explicit_incomplete_event_supports_multiline_crlf_and_eof() {
    let pretty = serde_json::to_string_pretty(&incomplete_event("partial")).unwrap();
    let data = pretty.lines().map(|line| format!("data: {line}\r\n")).collect::<String>();
    let result = parse_streamed_chunk(&data).unwrap();
    assert_eq!(result.status, "incomplete");
    assert_eq!(result.id, "partial");
}

#[test]
fn explicit_incomplete_event_requires_matching_status() {
    for status in [None, Some(Value::Null), Some(json!("completed")),
        Some(json!("failed")), Some(json!("in_progress")), Some(json!(false))] {
        let mut event = incomplete_event("private-fixture");
        if let Some(status) = status { event["response"]["status"] = status; }
        else { event["response"].as_object_mut().unwrap().remove("status"); }
        for data in [frame(&event), frame(&event) + &frame(&completed("later")),
            frame(&completed("earlier")) + &frame(&event)] {
            assert_eq!(parse_streamed_chunk(&data).unwrap_err(),
                "streamed chunk ended with response.incomplete: max_output_tokens");
        }
    }
}

#[test]
fn malformed_incomplete_body_cannot_be_hidden_by_later_completion() {
    for body in [Value::Null, json!([]), json!({"status":"incomplete"}),
        json!({"status":"incomplete", "output":null}),
        json!({"status":"incomplete", "output":[], "usage":"private-fixture"})] {
        let event = json!({"type":"response.incomplete", "response":body});
        let data = frame(&event) + &frame(&completed("later"));
        let error = parse_streamed_chunk(&data).unwrap_err();
        assert!(!error.contains("private-fixture"));
    }
}

#[test]
fn explicit_incomplete_events_retain_terminal_frame_order() {
    let incomplete = incomplete_event("partial");
    let complete = completed("finished");
    let last_partial = parse_streamed_chunk(&(frame(&complete) + &frame(&incomplete))).unwrap();
    assert_eq!((last_partial.id.as_str(), last_partial.status.as_str()), ("partial", "incomplete"));
    let last_complete = parse_streamed_chunk(&(frame(&incomplete) + &frame(&complete))).unwrap();
    assert_eq!((last_complete.id.as_str(), last_complete.status.as_str()), ("finished", "completed"));
}

#[test]
fn failures_beside_valid_incomplete_events_remain_errors() {
    for kind in ["error", "response.failed"] {
        let failure = json!({"type":kind, "message":"private-fixture"});
        let partial = incomplete_event("partial");
        for data in [frame(&failure) + &frame(&partial), frame(&partial) + &frame(&failure)] {
            assert_eq!(parse_streamed_chunk(&data).unwrap_err(),
                format!("streamed chunk ended with {kind}: private-fixture"));
        }
    }
}

#[test]
fn terminal_usage_rejects_invalid_supplied_counts() {
    for kind in ["response.completed", "response.incomplete"] {
        for field in ["input_tokens", "output_tokens"] {
            for bad in [Value::Null, json!("7"), json!(-1), json!(1.5),
                json!(true), json!({}), json!([]), json!(1e100)] {
                let mut event = if kind == "response.incomplete" {
                    incomplete_event("fixture")
                } else { completed("fixture") };
                event["response"]["usage"][field] = bad;
                assert_eq!(parse_streamed_chunk(&frame(&event)).unwrap_err(),
                    "streamed chunk has invalid usage count");
            }
        }
    }
}

#[test]
fn valid_partial_usage_preserves_counts_and_opaque_metadata() {
    for (usage, input, output) in [
        (Value::Null, 0, 0), (json!({}), 0, 0),
        (json!({"input_tokens":7, "total_tokens":null, "extension":{"keep":true}}), 7, 0),
        (json!({"output_tokens":9, "total_tokens":1, "input_tokens_details":null}), 0, 9),
        (json!({"input_tokens":u64::MAX, "output_tokens":0}), u64::MAX, 0),
        (json!({"input_tokens":0, "output_tokens":u64::MAX}), 0, u64::MAX),
    ] {
        for mut event in [completed("fixture"), incomplete_event("fixture")] {
            event["response"]["usage"] = usage.clone();
            let result = parse_streamed_chunk(&frame(&event)).unwrap();
            assert_eq!((result.input_tokens, result.output_tokens), (input, output));
            assert_eq!(result.usage, usage);
        }
    }
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

#[test]
fn malformed_completed_body_cannot_be_hidden_by_later_terminal() {
    for (body, reason) in [
        (None, "invalid response object"),
        (Some(Value::Null), "invalid response object"),
        (Some(json!([])), "invalid response object"),
        (Some(json!({"output":null})), "invalid output array"),
        (Some(json!({"output":[null]})), "invalid output item"),
        (Some(json!({"output":[], "status":"failed"})), "invalid terminal status"),
        (Some(json!({"output":[], "usage":"private-fixture"})), "invalid usage object"),
        (Some(json!({"output":[], "usage":{"output_tokens":-1}})), "invalid usage count"),
    ] {
        let mut invalid = json!({"type":"response.completed"});
        if let Some(body) = body { invalid["response"] = body; }
        let expected = if reason == "invalid terminal status" {
            "upstream response failed: the provider did not supply a failure cause".to_owned()
        } else { format!("streamed chunk has {reason}") };
        for later in [completed("later"), incomplete_event("later")] {
            for prefix in ["", "event: response.completed\n"] {
                let input = format!("{prefix}{}{}", frame(&invalid), frame(&later));
                assert_eq!(parse_streamed_chunk(&input).unwrap_err(),
                    expected);
            }
        }
    }
}

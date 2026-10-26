use super::*;

fn framed(values: &[Value], endings: &[&str]) -> String {
    let mut lines = Vec::new();
    for value in values {
        lines.push(": fixture comment".to_string());
        lines.push(format!("event: {}", value["type"].as_str().unwrap()));
        lines.extend(serde_json::to_string_pretty(value).unwrap().lines()
            .map(|line| format!("data:{line}")));
        lines.push(String::new());
    }
    lines.iter().enumerate().map(|(index, line)|
        format!("{line}{}", endings[index % endings.len()])).collect()
}

#[test]
fn cr_delimited_errors_cannot_hide_after_a_complete_message() {
    let valid = encode(&text_events("end_turn"));
    for error in [
        "\revent:error\rdata:{\"type\":\"ping\"}\r\r",
        "\revent:error\r\r", "\revent:error",
        "\rdata:{\"type\":\"error\"}\r\r",
        "\revent:error\rdata:not-json\r\r",
    ] {
        assert!(parse(&format!("{valid}{error}")).unwrap_err().contains("upstream error"));
    }
}

#[test]
fn cr_frames_keep_last_event_field_and_data_only_compatibility() {
    let valid = encode(&text_events("end_turn")).replace('\n', "\r");
    for prefix in [
        "event:error\revent:ping\rdata:{\"type\":\"ping\"}\r\r",
        "event:error\revent:\r\r", "event:error\revent\r\r",
        "event:future\r\r",
    ] {
        let response = parse(&format!("{prefix}{valid}")).unwrap();
        assert_eq!(response.content, vec![json!({"type":"text","text":"prefix suffix"})]);
        assert_eq!(response.stop_reason, "end_turn");
    }
}

#[test]
fn line_endings_preserve_complete_content_and_metadata() {
    for (block, reason) in [
        (json!({"type":"text","text":"first\r\n雪","citations":[]}), "stop_sequence"),
        (json!({"type":"thinking","thinking":"thought\r","signature":"signed"}), "end_turn"),
        (json!({"type":"tool_use","id":"call-fixture","name":"fixture",
            "input":{"value":"line\r\nbreak"},"extension":true}), "tool_use"),
    ] {
        let mut values = events(block.clone(), vec![], reason);
        let sequence = (reason == "stop_sequence").then_some("END\r\n");
        values[3]["delta"]["stop_sequence"] = json!(sequence);
        for endings in [&["\n"][..], &["\r\n"], &["\r"], &["\n", "\r", "\r\n"]] {
            let wire = framed(&values, endings);
            for data in [wire.as_str(), wire.trim_end_matches(['\r', '\n'])] {
                let response = parse(data).unwrap();
                assert_eq!(response.id, "msg-fixture");
                assert_eq!(response.content, vec![block.clone()]);
                assert_eq!(response.stop_reason, reason);
                assert_eq!(response.stop_sequence.as_deref(), sequence);
                assert_eq!((response.input_tokens, response.output_tokens), (17, 9));
                assert_eq!(response.usage, json!({"input_tokens":17,"output_tokens":9}));
            }
        }
    }
}

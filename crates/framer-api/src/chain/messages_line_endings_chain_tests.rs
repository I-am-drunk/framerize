use super::*;

fn fallback(wire: String, tool_repair: bool) -> Observed {
    let mut replies = if tool_repair {
        vec![buffered(tool(), "max_tokens"), buffered(tool(), "max_tokens"),
            buffered(tool(), "max_tokens")]
    } else { vec![failed_request(), failed_request()] };
    replies.push(streamed(wire));
    run(replies, if tool_repair { 3 } else { 1 })
}

#[test]
fn both_fallbacks_reject_cr_delimited_errors_after_completion() {
    for error in ["\revent:error\rdata:{\"type\":\"ping\"}\r\r",
        "\revent:error\r\r", "\revent:error", "\rdata:{\"type\":\"error\"}\r\r"] {
        for tool_repair in [false, true] {
            let observed = fallback(stream(tool(), "tool_use") + error, tool_repair);
            assert!(observed.result.unwrap_err().contains("upstream error"));
            assert!(observed.emitted.is_empty());
            assert_eq!(observed.requests.len(), if tool_repair { 4 } else { 3 });
            assert_eq!(observed.refunds, 1);
        }
    }
}

#[test]
fn both_fallbacks_accept_cr_and_mixed_line_endings() {
    let mut block = tool();
    block["input"]["value"] = json!("first\r\n雪");
    block["extension"] = json!({"preserve":true});
    let valid = stream(block.clone(), "tool_use");
    for endings in [&["\n"][..], &["\r\n"], &["\r"], &["\n", "\r", "\r\n"]] {
        let wire: String = valid.lines().enumerate().map(|(index, line)|
            format!("{line}{}", endings[index % endings.len()])).collect();
        for tool_repair in [false, true] {
            let observed = fallback(wire.clone(), tool_repair);
            let response = observed.result.unwrap();
            assert_eq!(response.content, vec![block.clone()]);
            assert_eq!(response.stop_reason, "tool_use");
            assert_eq!(response.output_tokens, 80);
            assert_eq!(observed.emitted, vec![block.clone()]);
            assert_eq!(observed.requests.len(), if tool_repair { 4 } else { 3 });
            assert_eq!(observed.refunds, 1);
        }
    }
}

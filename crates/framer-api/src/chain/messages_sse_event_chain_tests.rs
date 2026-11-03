use super::*;

#[test]
fn streaming_paths_reject_explicit_named_error_frames() {
    for error in ["event: error\ndata: {\"type\":\"ping\"}\n\n", "event: error\n\n"] {
        for tool_repair in [false, true] {
            let mut replies = if tool_repair {
                vec![buffered(tool(), "max_tokens"), buffered(tool(), "max_tokens")]
            } else { Vec::new() };
            replies.push(streamed(stream(tool(), "tool_use") + error));
            let observed = if tool_repair { run(replies, 3) } else { run_opted_stream(replies, 1) };
            assert!(observed.result.unwrap_err().contains("upstream error"));
            assert!(observed.emitted.is_empty());
            assert_eq!(observed.requests.len(), if tool_repair { 3 } else { 1 });
            assert_eq!(observed.refunds, 0);
        }
    }
}

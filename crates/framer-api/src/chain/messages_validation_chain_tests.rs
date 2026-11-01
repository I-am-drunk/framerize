use super::*;

#[test]
fn transport_fallback_rejects_completed_tools_without_identity() {
    for kind in ["tool_use", "server_tool_use"] {
        for field in ["id", "name"] {
            let mut block = tool();
            block["type"] = json!(kind);
            block.as_object_mut().unwrap().remove(field);
            let observed = run(vec![failed_request(), failed_request(),
                streamed(stream(block, "tool_use"))], 1);
            assert!(observed.result.unwrap_err().contains("tool identity"));
            assert!(observed.emitted.is_empty());
            assert_eq!(observed.requests.len(), 3);
            assert_eq!(observed.refunds, 0);
        }
    }
}

#[test]
fn tool_fallback_rejects_completed_text_without_text_data() {
    for block in [json!({"type":"text"}), json!({"type":"text", "text":42})] {
        let observed = run(vec![buffered(tool(), "max_tokens"),
            buffered(tool(), "max_tokens"), buffered(tool(), "max_tokens"),
            streamed(stream(block, "end_turn"))], 3);
        assert!(observed.result.unwrap_err().contains("text content"));
        assert!(observed.emitted.is_empty());
        assert_eq!(observed.requests.len(), 4);
        assert_eq!(observed.refunds, 0);
    }
}

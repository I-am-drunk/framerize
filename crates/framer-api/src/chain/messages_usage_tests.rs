use super::*;

fn config(max_chunks: u32, cap: Option<u32>) -> ChainConfig {
    ChainConfig { max_chunks, max_total_tokens:cap, metered_fallback:true,
        ..ChainConfig::default() }
}

fn text(value: &str) -> Value { json!({"type":"text", "text":value}) }

fn usage_reply(block: Value, stop: &str, output_tokens: u64) -> Reply {
    let mut reply = buffered(block, stop);
    let mut body: Value = serde_json::from_str(reply.result.as_ref().unwrap()).unwrap();
    body["usage"]["output_tokens"] = json!(output_tokens);
    reply.result = Ok(body.to_string());
    reply
}

fn usage_stream(block: Value, stop: &str, output_tokens: u64) -> Reply {
    let raw = stream(block, stop);
    let mut values: Vec<Value> = raw.lines().filter_map(|line| line.strip_prefix("data: "))
        .map(|data| serde_json::from_str(data).unwrap()).collect();
    for event in &mut values {
        if event["type"] == "message_delta" {
            event["usage"]["output_tokens"] = json!(output_tokens);
        }
    }
    streamed(values.iter().map(|value| format!("data: {value}\n\n")).collect())
}

#[test]
fn buffered_usage_above_u32_exhausts_the_client_budget() {
    let count = u64::from(u32::MAX) + 2;
    let observed = run_with_config(vec![usage_reply(text("answer"), "max_tokens", count)],
        config(2, Some(256)));
    let response = observed.result.unwrap();
    assert_eq!(response.output_tokens, count);
    assert_eq!(response.usage["output_tokens"], count);
    assert_eq!(response.stop_reason, "max_tokens");
    assert_eq!(observed.emitted, vec![text("answer")]);
    assert_eq!(observed.requests.len(), 1);
}

#[test]
fn transport_fallback_usage_above_u32_exhausts_the_client_budget() {
    let count = u64::from(u32::MAX) + 2;
    let observed = run_with_config(vec![failed_request(), failed_request(),
        usage_stream(text("answer"), "max_tokens", count)], config(2, Some(256)));
    let response = observed.result.unwrap();
    assert_eq!(response.output_tokens, count);
    assert_eq!(response.stop_reason, "max_tokens");
    assert_eq!(observed.emitted, vec![text("answer")]);
    assert_eq!(observed.requests.len(), 3);
    assert_eq!(observed.refunds, 0);
}

#[test]
fn tool_fallback_usage_above_u32_exhausts_the_client_budget() {
    let count = u64::from(u32::MAX) + 2;
    let observed = run_with_config(vec![buffered(tool(), "max_tokens"),
        buffered(tool(), "max_tokens"), buffered(tool(), "max_tokens"),
        usage_stream(text("answer"), "max_tokens", count)], config(4, Some(256)));
    let response = observed.result.unwrap();
    assert_eq!(response.output_tokens, count);
    assert_eq!(response.stop_reason, "max_tokens");
    assert_eq!(observed.emitted, vec![text("answer")]);
    assert_eq!(observed.requests.len(), 4);
    assert_eq!(observed.refunds, 0);
}

#[test]
fn buffered_usage_overflow_fails_before_new_content_is_published() {
    let observed = run_with_config(vec![usage_reply(text("prefix"), "max_tokens", u64::MAX - 1),
        usage_reply(text("suffix"), "end_turn", 2)], config(2, None));
    assert!(observed.result.unwrap_err().contains("usage overflow"));
    assert_eq!(observed.emitted, vec![text("prefix")]);
    assert_eq!(observed.requests.len(), 2);
    assert_eq!(observed.refunds, 0);
}

#[test]
fn transport_fallback_usage_overflow_keeps_only_the_published_prefix() {
    let observed = run_with_config(vec![usage_reply(text("prefix"), "max_tokens", u64::MAX - 1),
        failed_request(), failed_request(), usage_stream(text("suffix"), "end_turn", 2)],
        config(2, None));
    assert!(observed.result.unwrap_err().contains("usage overflow"));
    assert_eq!(observed.emitted, vec![text("prefix")]);
    assert_eq!(observed.requests.len(), 4);
    assert_eq!(observed.refunds, 0);
}

#[test]
fn tool_fallback_usage_overflow_keeps_only_the_published_prefix() {
    let observed = run_with_config(vec![usage_reply(text("prefix"), "max_tokens", u64::MAX - 1),
        buffered(tool(), "max_tokens"), buffered(tool(), "max_tokens"),
        buffered(tool(), "max_tokens"), usage_stream(tool(), "tool_use", 2)], config(4, None));
    assert!(observed.result.unwrap_err().contains("usage overflow"));
    assert_eq!(observed.emitted, vec![text("prefix")]);
    assert_eq!(observed.requests.len(), 5);
    assert_eq!(observed.refunds, 0);
}

#[test]
fn discarded_tool_usage_cannot_overflow_delivered_output() {
    let observed = run_with_config(vec![usage_reply(text("prefix"), "max_tokens", u64::MAX - 3),
        usage_reply(tool(), "max_tokens", u64::MAX),
        usage_reply(tool(), "max_tokens", u64::MAX),
        usage_reply(tool(), "max_tokens", u64::MAX),
        usage_stream(tool(), "tool_use", 2)], config(4, None));
    let response = observed.result.unwrap();
    assert_eq!(response.output_tokens, u64::MAX - 1);
    assert_eq!(response.content, vec![text("prefix"), tool()]);
    assert_eq!(observed.emitted, response.content);
    assert_eq!(response.stop_reason, "tool_use");
    assert_eq!((response.chunks, response.metered_chunks), (4, 1));
    assert_eq!(observed.requests.len(), 5);
    assert_eq!(observed.refunds, 0);
}

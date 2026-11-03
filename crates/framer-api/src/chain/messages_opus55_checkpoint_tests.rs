use super::*;

fn run(replies: Vec<Reply>, payload: Value) -> Observed {
    run_with_payload(replies, ChainConfig { max_total_tokens:Some(64_000),
        metered_fallback:false, ..ChainConfig::default() }, Vec::new(), payload)
}

#[test]
fn opus55_continuations_keep_the_original_conversation_prefix() {
    for model in ["claude-opus-5-5", "claude-opus-5.5", "anthropic/claude-opus-5.5"] {
        let payload = request(model);
        let observed = run(vec![buffered(thought(), "max_tokens"),
            buffered(json!({"type":"text","text":"complete"}), "end_turn")], payload.clone());
        assert_eq!(observed.result.as_ref().unwrap().chunks, 2);
        assert_eq!(observed.result.as_ref().unwrap().content[0], thought());
        assert_eq!(observed.requests[0]["request"]["max_tokens"], 600);
        check_prefix(&observed, &payload);
    }
}

fn thought() -> Value {
    json!({"type":"thinking","thinking":"fixture reasoning","signature":"opus55-fixture"})
}

fn request(model: &str) -> Value {
    json!({"model":model, "thinking":{"type":"adaptive"},
        "output_config":{"effort":"xhigh"}, "speed":"fast",
        "system":[{"type":"text","text":"Preserve the original context."}],
        "tools":[{"name":"fixture_tool","input_schema":{"type":"object"}}],
        "messages":[{"role":"user","content":"original question"},
            {"role":"assistant","content":[{"type":"text","text":"original answer"}]},
            {"role":"user","content":"follow-up question"}]})
}

fn check_prefix(observed: &Observed, payload: &Value) {
    for sent in &observed.requests {
        let sent = &sent["request"];
        for key in ["model", "system", "tools", "thinking", "output_config", "speed"] {
            assert_eq!(sent[key], payload[key], "{key}");
        }
        assert_eq!(&sent["messages"].as_array().unwrap()[..3],
            payload["messages"].as_array().unwrap().as_slice());
        assert!(sent["max_tokens"].as_u64().unwrap() <= 1200);
        assert_eq!(sent["stream"], false);
    }
    assert_eq!(observed.requests[1]["request"]["messages"][3]["content"], json!([thought()]));
    assert_eq!(observed.requests[1]["request"]["messages"].as_array().unwrap().len(), 5);
    assert_eq!(observed.requests[1]["request"]["messages"][4]["content"],
        super::super::checkpoint::CONTINUE);
    assert_eq!(observed.refunds, 0);
}

#[test]
fn opus55_unknown_completion_is_terminal_after_a_signed_checkpoint() {
    let failure = Reply { streaming:false, status:504,
        result:Ok("upstream completion unknown".into()) };
    let observed = run(vec![buffered(thought(), "max_tokens"), failure],
        request("anthropic/claude-opus-5.5"));
    assert!(observed.result.unwrap_err().contains("completion is unknown"));
    assert_eq!(observed.requests.len(), 2);
    assert_eq!(observed.emitted, vec![thought()]);
    assert_eq!(observed.refunds, 0);
}

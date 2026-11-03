use super::*;

fn thought() -> Value {
    json!({"type":"thinking","thinking":"fixture reasoning","signature":"fixture-signature"})
}

fn run_for(model: &str, replies: Vec<Reply>) -> Observed {
    run_with_payload(replies, ChainConfig {
        max_total_tokens:Some(64_000), metered_fallback:false,
        ..ChainConfig::default()
    }, Vec::new(), json!({"model":model, "thinking":{"type":"adaptive"},
        "output_config":{"effort":"xhigh"}, "speed":"fast",
        "tools":[{"name":"fixture_tool","input_schema":{"type":"object"}}],
        "messages":[{"role":"user","content":"fixture request"}]}))
}

#[test]
fn opus48_checkpoints_keep_model_effort_tools_and_signed_blocks() {
    for model in ["claude-opus-4-8", "claude-opus-4.8", "anthropic/claude-opus-4.8"] {
        let observed = run_for(model, vec![buffered(thought(), "max_tokens"),
            buffered(json!({"type":"text","text":"complete answer"}), "end_turn")]);
        let response = observed.result.unwrap();
        assert_eq!(response.stop_reason, "end_turn");
        assert_eq!(response.chunks, 2);
        assert_eq!(response.content[0], thought());
        assert_eq!(observed.requests[0]["request"]["max_tokens"], 600);
        assert_eq!(observed.requests[1]["request"]["messages"][1]["content"], json!([thought()]));
        for request in &observed.requests {
            let sent = &request["request"];
            assert_eq!(sent["model"], model);
            assert_eq!(sent["thinking"]["type"], "adaptive");
            assert_eq!(sent["output_config"]["effort"], "xhigh");
            assert_eq!(sent["speed"], "fast");
            assert_eq!(sent["tools"][0]["name"], "fixture_tool");
            assert_eq!(sent["stream"], false);
            assert!(sent["max_tokens"].as_u64().unwrap() <= 1200);
        }
        assert_eq!(observed.refunds, 0);
    }
}

#[test]
fn opus48_unknown_completion_never_replays_or_publishes_a_partial_reply() {
    for has_checkpoint in [false, true] {
        let mut replies = Vec::new();
        if has_checkpoint { replies.push(buffered(thought(), "max_tokens")); }
        replies.push(Reply { streaming:false, status:504,
            result:Ok("upstream execution status unknown".into()) });
        let observed = run_for("anthropic/claude-opus-4.8", replies);
        let error = observed.result.unwrap_err();
        assert!(error.contains("completion is unknown"));
        assert!(error.contains("504"));
        assert_eq!(observed.requests.len(), 1 + usize::from(has_checkpoint));
        assert_eq!(observed.emitted, if has_checkpoint { vec![thought()] } else { vec![] });
        assert_eq!(observed.refunds, 0);
    }
}

#[test]
fn opus48_unsigned_checkpoint_stops_before_publication() {
    let observed = run_for("anthropic/claude-opus-4.8", vec![buffered(
        json!({"type":"thinking","thinking":"unfinished"}), "max_tokens")]);
    assert!(observed.result.unwrap_err().contains("complete signed thinking block"));
    assert!(observed.emitted.is_empty());
    assert_eq!(observed.requests.len(), 1);
}

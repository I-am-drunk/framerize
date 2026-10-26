use super::*;

fn assert_rejected(config: ChainConfig, field: &str) {
    for metered_fallback in [false, true] {
        let observed = run_with_config(Vec::new(), ChainConfig {
            metered_fallback, ..config.clone()
        });
        assert!(observed.result.unwrap_err().contains(field));
        assert!(observed.requests.is_empty());
        assert!(observed.emitted.is_empty());
        assert_eq!(observed.refunds, 0);
    }
}

#[test]
fn zero_total_budget_is_rejected_before_any_attempt() {
    assert_rejected(ChainConfig { max_total_tokens: Some(0),
        ..ChainConfig::default() }, "max_total_tokens");
}

#[test]
fn zero_chunk_limit_cannot_return_an_empty_success() {
    assert_rejected(ChainConfig { max_chunks: 0,
        ..ChainConfig::default() }, "max_chunks");
}

#[test]
fn zero_chunk_budget_is_rejected_before_any_attempt() {
    assert_rejected(ChainConfig { chunk_tokens: 0,
        ..ChainConfig::default() }, "chunk_tokens");
}

fn one_token_reply(reason: &str) -> Reply {
    Reply { streaming: false, status: 200, result: Ok(json!({
        "id":"one-token", "content":[{"type":"text","text":"a"}],
        "stop_reason":reason, "usage":{"input_tokens":2,"output_tokens":1}
    }).to_string()) }
}

#[test]
fn smallest_positive_configuration_preserves_exhausted_budget_status() {
    let observed = run_with_config(vec![one_token_reply("max_tokens")], ChainConfig {
        chunk_tokens: 1, max_chunks: 1, max_total_tokens: Some(1),
        ..ChainConfig::default()
    });
    let response = observed.result.unwrap();
    assert_eq!(response.stop_reason, "max_tokens");
    assert_eq!(response.content, vec![json!({"type":"text","text":"a"})]);
    assert_eq!(observed.emitted, response.content);
    assert_eq!((response.input_tokens, response.output_tokens), (2, 1));
    assert_eq!((response.chunks, response.metered_chunks), (1, 0));
    assert_eq!(observed.requests.len(), 1);
    assert_eq!(observed.requests[0]["request"]["max_tokens"], 1);
    assert_eq!(observed.refunds, 0);
}

#[test]
fn absent_total_budget_still_allows_a_one_token_chunk() {
    let observed = run_with_config(vec![one_token_reply("end_turn")], ChainConfig {
        chunk_tokens: 1, max_chunks: 1, max_total_tokens: None,
        ..ChainConfig::default()
    });
    let response = observed.result.unwrap();
    assert_eq!(response.stop_reason, "end_turn");
    assert_eq!(response.output_tokens, 1);
    assert_eq!(response.chunks, 1);
    assert_eq!(response.content, vec![json!({"type":"text","text":"a"})]);
    assert_eq!(observed.emitted, response.content);
    assert_eq!(observed.requests.len(), 1);
    assert_eq!(observed.requests[0]["request"]["max_tokens"], 1);
    assert_eq!(observed.refunds, 0);
}

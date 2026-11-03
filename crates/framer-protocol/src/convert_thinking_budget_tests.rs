use super::*;

#[test]
fn explicit_thinking_budget_survives_conversion_without_a_cap() {
    for budget in [1024_u64, 4096, 4294967294] {
        let body = json!({"model":"claude-opus-5-5", "speed":"fast",
            "thinking":{"type":"enabled","budget_tokens":budget},
            "output_config":{"effort":"high"}, "max_tokens":budget + 1,
            "messages":[{"role":"user","content":"fixture"}]});
        let converted = to_anthropic_request(ClientDialect::AnthropicMessages, &body).unwrap();
        assert_eq!(converted["thinking"], body["thinking"]);
        assert_eq!(converted["output_config"], body["output_config"]);
        assert_eq!(converted["speed"], "fast");
        assert_eq!(anthropic_output_floor(&converted).unwrap() as u64, budget + 1);
    }
}

#[test]
fn invalid_enabled_thinking_budgets_are_not_rewritten() {
    for budget in [Value::Null, json!(1023), json!(-1), json!(1024.5),
        json!("4096"), json!(true), json!(4294967295_u64)] {
        let body = json!({"model":"claude-fixture",
            "thinking":{"type":"enabled","budget_tokens":budget},
            "messages":[{"role":"user","content":"fixture"}]});
        assert!(to_anthropic_request(ClientDialect::AnthropicMessages, &body)
            .unwrap_err().starts_with("invalid enabled thinking budget"));
    }
}

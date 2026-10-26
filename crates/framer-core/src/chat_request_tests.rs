use super::*;

#[test]
fn function_tool_strictness_preserves_true_false_null_and_omission() {
    let schema = json!({"type":"object","properties":{"value":{"type":"integer"}},
        "required":["value"],"additionalProperties":false});
    for strict in [Some(json!(true)), Some(json!(false)), Some(Value::Null), None] {
        let mut function = json!({"name":"fixture","description":"fixture tool","parameters":schema});
        if let Some(value) = &strict { function["strict"] = value.clone(); }
        let converted = chat_to_responses_request(&json!({"model":"fixture-model",
            "messages":[{"role":"user","content":"prompt"}],
            "tools":[{"type":"function","function":function}],
            "tool_choice":{"type":"function","function":{"name":"fixture"}},
            "parallel_tool_calls":false})).unwrap();
        let tool = &converted["tools"][0];
        assert_eq!(tool.get("strict"), strict.as_ref());
        assert_eq!(tool["type"], "function");
        assert_eq!(tool["name"], "fixture");
        assert_eq!(tool["description"], "fixture tool");
        assert_eq!(tool["parameters"], schema);
        assert_eq!(converted["tool_choice"], json!({"type":"function","name":"fixture"}));
        assert_eq!(converted["parallel_tool_calls"], false);
    }
}

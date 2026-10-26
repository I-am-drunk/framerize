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

#[test]
fn json_schema_response_format_preserves_constraints_and_optional_fields() {
    let schema = json!({"type":"object", "properties":{"value":{"type":"integer","minimum":2}},
        "required":["value"], "additionalProperties":false});
    for strict in [Some(json!(true)), Some(json!(false)), Some(Value::Null), None] {
        let mut format = json!({"name":"result", "description":"fixture schema", "schema":schema});
        if let Some(value) = strict { format["strict"] = value; }
        let converted = chat_to_responses_request(&json!({"model":"fixture-model",
            "messages":[{"role":"user","content":"structured answer"}],
            "response_format":{"type":"json_schema","json_schema":format}})).unwrap();
        format["type"] = json!("json_schema");
        assert_eq!(converted["text"]["format"], format);
        assert!(converted.get("response_format").is_none());
    }
}

#[test]
fn malformed_json_schema_envelopes_are_not_silently_dropped() {
    for schema in [Value::Null, json!([]), json!(42), json!("not a schema")] {
        let result = chat_to_responses_request(&json!({"model":"fixture-model",
            "messages":[{"role":"user","content":"structured answer"}],
            "response_format":{"type":"json_schema","json_schema":schema}}));
        assert!(result.unwrap_err().contains("response_format.json_schema"));
    }
    assert!(chat_to_responses_request(&json!({"model":"fixture-model",
        "messages":[{"role":"user","content":"structured answer"}],
        "response_format":{"type":"json_schema"}})).is_err());
}

#[test]
fn json_object_and_default_response_formats_keep_existing_behavior() {
    for format in [None, Some(json!({"type":"text"})), Some(json!({"type":"json_object"}))] {
        let mut body = json!({"model":"fixture-model", "messages":[{"role":"user","content":"answer"}]});
        if let Some(format) = &format { body["response_format"] = format.clone(); }
        let converted = chat_to_responses_request(&body).unwrap();
        if format.as_ref().is_some_and(|format| format["type"] == "json_object") {
            assert_eq!(converted["text"]["format"], json!({"type":"json_object"}));
        } else {
            assert!(converted.get("text").is_none());
        }
    }
}

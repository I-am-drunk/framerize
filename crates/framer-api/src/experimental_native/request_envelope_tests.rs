use super::*;
use serde_json::json;

fn context(dialect: Dialect, payload: Value) -> RequestContext {
    RequestContext {
        dialect, project: ProjectId("fixture-project".into()),
        feature_category: "fixture-category".into(), feature_name: "fixture-feature".into(),
        step: "fixture-step".into(), chat_session: "fixture-session".into(),
        step_index: 0, turn_id: "fixture-turn".into(), user_prompt: "fixture-prompt".into(),
        payload,
    }
}

#[test]
fn responses_keep_model_inside_request_only() {
    for (dialect, path) in [(Dialect::OpenAiResponses, "/ai/v3/openai/responses/"),
                            (Dialect::OpenRouterResponses, "/ai/v3/responses/")] {
        let payload = json!({"model": "fixture-model", "input": "fixture-input", "stream": false});
        let (actual_path, body) = context(dialect, payload.clone()).prepare().unwrap();
        assert_eq!(actual_path, path);
        assert!(body.get("model").is_none());
        assert_eq!(body["request"]["model"], payload["model"]);
        assert_eq!(body["request"]["input"], payload["input"]);
        assert_eq!(body["request"]["stream"], false);
        assert_eq!(body["turnId"], "fixture-turn");
        assert_eq!(body["userPrompt"], "fixture-prompt");
    }
}

#[test]
fn messages_keep_their_existing_outer_model() {
    for dialect in [Dialect::AnthropicMessages, Dialect::OpenRouterMessages] {
        let (_, body) = context(dialect, json!({"model": "fixture-model"})).prepare().unwrap();
        assert_eq!(body["model"], "fixture-model");
        assert_eq!(body["request"]["model"], "fixture-model");
    }
}

#[test]
fn responses_still_require_a_nonempty_nested_model() {
    for dialect in [Dialect::OpenAiResponses, Dialect::OpenRouterResponses] {
        for payload in [json!({}), json!({"model": ""}), json!({"model": 1})] {
            assert!(matches!(context(dialect, payload).prepare(), Err(ClientError::InvalidContext)));
        }
    }
}

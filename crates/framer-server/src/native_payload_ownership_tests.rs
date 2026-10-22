use super::*;
use serde_json::json;

fn location(value: &Value) -> (*const u8, usize) {
    let text = value.as_str().expect("fixture text");
    (text.as_ptr(), text.len())
}

#[test]
fn responses_preparation_retains_owned_history_allocation() {
    let payload = json!({
        "model": "gpt-6-astra", "max_output_tokens": 64,
        "input": [{"type":"message", "role":"user", "content":[{
            "type":"input_text", "text":"fixture-history-".repeat(8192)
        }]}],
        "reasoning":{"effort":"max"},
        "fixture_extension":{"identity":"kept"}
    });
    let expected = payload.clone();
    let before = location(&payload["input"][0]["content"][0]["text"]);
    let prepared = responses(payload).unwrap();
    assert_eq!(prepared["input"], expected["input"]);
    assert_eq!(prepared["reasoning"], expected["reasoning"]);
    assert_eq!(prepared["fixture_extension"], expected["fixture_extension"]);
    assert_eq!(location(&prepared["input"][0]["content"][0]["text"]), before);
}

#[test]
fn messages_preparation_moves_original_history_and_extensions() {
    let payload = json!({
        "model":"claude-sonnet-5-5", "max_tokens":64,
        "messages":[{"role":"user", "content":"fixture-messages-".repeat(8192)}],
        "system":"fixture instructions",
        "thinking":{"type":"enabled", "budget_tokens":4096},
        "fixture_extension":{"opaque":"fixture-extension-".repeat(8192)}
    });
    let expected = payload.clone();
    let history = location(&payload["messages"][0]["content"]);
    let extension = location(&payload["fixture_extension"]["opaque"]);
    let prepared = messages(payload).unwrap();
    assert_eq!(prepared["messages"], expected["messages"]);
    assert_eq!(prepared["thinking"], expected["thinking"]);
    assert_eq!(prepared["fixture_extension"], expected["fixture_extension"]);
    assert!(prepared["system"].as_str().unwrap().contains("fixture instructions"));
    assert_eq!(location(&prepared["messages"][0]["content"]), history);
    assert_eq!(location(&prepared["fixture_extension"]["opaque"]), extension);
}

#[test]
fn chat_field_merge_moves_converted_history_without_recloning() {
    let original = json!({
        "model":"gpt-6-astra", "messages":[{"role":"user", "content":"old"}],
        "reasoning_effort":"max", "fixture_extension":{"kept":true}
    });
    let mapped = json!({"model":"gpt-6-astra", "input":[{
        "role":"user", "content":[{"type":"input_text",
            "text":"fixture-converted-".repeat(8192)}]
    }], "max_output_tokens":64});
    let expected = mapped["input"].clone();
    let history = location(&mapped["input"][0]["content"][0]["text"]);
    let prepared = preserve_fields(original, mapped).unwrap();
    assert!(prepared.get("messages").is_none());
    assert!(prepared.get("reasoning_effort").is_none());
    assert_eq!(prepared["reasoning"]["effort"], "max");
    assert_eq!(prepared["fixture_extension"]["kept"], true);
    assert_eq!(prepared["input"], expected);
    assert_eq!(location(&prepared["input"][0]["content"][0]["text"]), history);
}

use super::{sse, AssembledResponse};
use serde_json::{json, Value};

pub(super) fn explanation(response: &AssembledResponse) -> Option<String> {
    if response.stop_reason != "refusal" {
        return None;
    }
    let details = response.stop_details.as_ref();
    if let Some(text) = details.and_then(|value| value["explanation"].as_str())
        .filter(|text| !text.trim().is_empty()) {
        return Some(text.to_owned());
    }
    // Accumulated text may belong to earlier accepted chunks, not this refusal.
    Some(match details.and_then(|value| value["category"].as_str()) {
        Some(category) => format!("Upstream refused the request with category {category}; no explanation was provided."),
        None => "Upstream refused the request without structured category or explanation details.".into(),
    })
}

pub(super) fn chat_delta(response: &AssembledResponse) -> Value {
    match explanation(response) {
        Some(text) => json!({"refusal": text}),
        None => json!({}),
    }
}

pub(super) fn response_item(response: &AssembledResponse) -> Option<Value> {
    explanation(response).map(|text| json!({
        "id": format!("msg_{}_refusal", response.id.trim_start_matches("msg_")),
        "type": "message", "status": "completed", "role": "assistant",
        "content": [{"type": "refusal", "refusal": text}],
    }))
}

pub(super) fn response_frames(index: usize, item: &Value) -> Vec<String> {
    let part = &item["content"][0];
    let mut initial = item.clone();
    initial["status"] = json!("in_progress");
    initial["content"] = json!([]);
    let event = |kind: &str, extra: Value| {
        let mut value = json!({"type": kind, "output_index": index});
        super::merge(&mut value, extra);
        sse(kind, value)
    };
    vec![
        event("response.output_item.added", json!({"item": initial})),
        event("response.content_part.added", json!({"item_id": item["id"], "content_index": 0,
            "part": {"type": "refusal", "refusal": ""}})),
        event("response.refusal.delta", json!({"item_id": item["id"], "content_index": 0,
            "delta": part["refusal"]})),
        event("response.refusal.done", json!({"item_id": item["id"], "content_index": 0,
            "refusal": part["refusal"]})),
        event("response.content_part.done", json!({"item_id": item["id"], "content_index": 0,
            "part": part})),
        event("response.output_item.done", json!({"item": item})),
    ]
}

use super::sse;
use serde_json::{json, Value};

pub(super) fn plain_text(block: &Value) -> bool {
    block["type"] == "text" && block["text"].is_string()
        && block.as_object().is_some_and(|object|
            object.keys().all(|key| key == "type" || key == "text"))
}

/// Keep metadata attached to its original block. Only plain text can join.
pub fn merge_message_blocks(acc: &mut Vec<Value>, new: Vec<Value>) {
    for block in new {
        if plain_text(&block) {
            if let Some(last) = acc.last_mut().filter(|last| plain_text(last)) {
                last["text"] = json!(format!("{}{}",
                    last["text"].as_str().expect("plain text"),
                    block["text"].as_str().expect("plain text")));
                continue;
            }
        }
        acc.push(block);
    }
}

pub(super) fn block_frames(index: usize, block: &Value) -> Vec<String> {
    let mut start = block.clone();
    match block["type"].as_str() {
        Some("text") => {
            start["text"] = json!("");
            if block["citations"].is_array() { start["citations"] = json!([]); }
        }
        Some("tool_use") => start["input"] = json!({}),
        _ => {}
    }
    let mut frames = vec![sse("content_block_start", json!({
        "type":"content_block_start", "index":index, "content_block":start,
    }))];
    append_deltas(&mut frames, index, block);
    frames.push(sse("content_block_stop", json!({
        "type":"content_block_stop", "index":index,
    })));
    frames
}

fn append_deltas(frames: &mut Vec<String>, index: usize, block: &Value) {
    let mut push = |delta: Value| frames.push(sse("content_block_delta", json!({
        "type":"content_block_delta", "index":index, "delta":delta,
    })));
    match block["type"].as_str() {
        Some("text") => {
            push(json!({"type":"text_delta", "text":block["text"]}));
            if let Some(citations) = block["citations"].as_array() {
                for citation in citations {
                    push(json!({"type":"citations_delta", "citation":citation}));
                }
            }
        }
        Some("tool_use") => push(json!({
            "type":"input_json_delta", "partial_json":block["input"].to_string(),
        })),
        _ => {}
    }
}

use serde_json::{json, Value};
use crate::proxy::Dialect;

pub fn merge_blocks(acc: &mut Vec<Value>, new: Vec<Value>) {
    framer_core::convert::merge_message_blocks(acc, new);
}

pub fn push_continuation(messages: &mut Value, blocks: &[Value], prompt: &str) {
    let turns = messages.as_array_mut().expect("messages array");
    turns.push(json!({"role":"assistant", "content":blocks}));
    turns.push(json!({"role":"user", "content":prompt}));
}

pub fn prepare_payload(anthro_payload: &Value, messages: &Value, chunk: u32, dialect: Dialect) -> Value {
    let base = match dialect {
        Dialect::OpenRouterMessages => anthro_payload.clone(),
        _ => framer_core::convert::anthro_to_chat(&anthro_payload),
    };
    let mut p = base;
    p["messages"] = match dialect {
        Dialect::OpenRouterMessages => messages.clone(),
        _ => framer_core::convert::anthro_to_chat(&json!({"messages": messages}))["messages"].clone(),
    };
    p["stream"] = json!(false);
    p["max_tokens"] = json!(chunk);
    p
}

use serde_json::{json, Value};
use framer_core::port::ChainConfig;
use crate::proxy::Dialect;

pub fn merge_blocks(acc: &mut Vec<Value>, new: Vec<Value>) {
    for b in new {
        let is_text = b["type"].as_str() == Some("text");
        if is_text {
            if let Some(last) = acc.last_mut() {
                if last["type"].as_str() == Some("text") {
                    let joined = format!(
                        "{}{}",
                        last["text"].as_str().unwrap_or_default(),
                        b["text"].as_str().unwrap_or_default()
                    );
                    last["text"] = json!(joined);
                    continue;
                }
            }
        }
        acc.push(b);
    }
}

pub fn push_continuation(messages: &mut Value, chunk_blocks: &[Value], cfg: &ChainConfig) {
    // Text only: thinking blocks in prior turns must pass signature
    // validation or be dropped (API rule); a chained continuation can do
    // neither cleanly, and the model does not need them to continue.
    let content: Vec<Value> = chunk_blocks
        .iter()
        .filter(|b| b["type"].as_str() == Some("text"))
        .cloned()
        .collect();
    let arr = messages.as_array_mut().expect("messages array");
    arr.push(json!({"role": "assistant", "content": content}));
    arr.push(json!({"role": "user", "content": cfg.continue_prompt}));
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

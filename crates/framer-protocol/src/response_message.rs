use serde_json::{json, Value};

fn plain_text(part: &Value) -> bool {
    part["type"] == "output_text" && part["text"].is_string()
        && part.as_object().is_some_and(|object| object.keys().all(|key|
            matches!(key.as_str(), "type" | "text" | "annotations" | "logprobs")))
        && ["annotations", "logprobs"].iter().all(|key| part.get(*key)
            .map_or(true, |value| value.as_array().is_some_and(Vec::is_empty)))
}

fn joins_text(parts: &[Value], next: &[Value]) -> bool {
    parts.last().is_some_and(plain_text) && next.first().is_some_and(plain_text)
}

/// Join consecutive message fragments without moving or discarding rich parts.
/// The first message owns identity; the latest supplied status owns completion.
pub fn merge_response_message(previous: &mut Value, next: &Value) {
    let mut parts = previous["content"].as_array().cloned().unwrap_or_default();
    let mut incoming = next["content"].as_array().cloned().unwrap_or_default();
    if joins_text(&parts, &incoming) {
        let first = incoming.remove(0);
        let last = parts.last_mut().unwrap();
        last["text"] = json!(format!("{}{}", last["text"].as_str().unwrap(),
            first["text"].as_str().unwrap()));
    }
    parts.extend(incoming);
    previous["content"] = json!(parts);
    if let Some(status) = next.get("status") { previous["status"] = status.clone(); }
}

pub(super) struct MessageStream {
    item: Value,
    output_index: usize,
}

impl MessageStream {
    pub(super) fn new(source: &Value, id: String, output_index: usize) -> Self {
        let mut item = source.clone();
        item["id"] = json!(id);
        item["content"] = json!([]);
        Self { item, output_index }
    }

    pub(super) fn start(&self) -> Value {
        let mut item = self.item.clone();
        item["status"] = json!("in_progress");
        json!({"type":"response.output_item.added",
            "output_index":self.output_index, "item":item})
    }

    fn event(&self, kind: &str, content_index: usize, extra: Value) -> Value {
        let mut value = json!({"type":kind, "output_index":self.output_index,
            "item_id":self.item["id"], "content_index":content_index});
        super::merge(&mut value, extra);
        value
    }

    pub(super) fn append(&mut self, source: &Value) -> Vec<Value> {
        let incoming = source["content"].as_array().map(Vec::as_slice).unwrap_or(&[]);
        let previous = self.item["content"].as_array().unwrap();
        let offset = previous.len();
        let joined = joins_text(previous, incoming);
        merge_response_message(&mut self.item, source);
        let mut frames = Vec::new();
        for (i, part) in incoming.iter().enumerate() {
            let index = offset + i - usize::from(joined);
            if i != 0 || !joined {
                frames.push(self.event("response.content_part.added", index,
                    json!({"part":initial_part(part)})));
            }
            frames.extend(self.deltas(index, part));
        }
        frames
    }

    fn deltas(&self, index: usize, part: &Value) -> Vec<Value> {
        let mut frames = Vec::new();
        match part["type"].as_str() {
            Some("output_text") => {
                let text = part["text"].as_str().unwrap_or_default();
                if !text.is_empty() {
                    frames.push(self.event("response.output_text.delta", index,
                        json!({"delta":text,"logprobs":part.get("logprobs")
                            .cloned().unwrap_or_else(|| json!([]))})));
                }
                for (i, annotation) in part["annotations"].as_array().into_iter().flatten().enumerate() {
                    frames.push(self.event("response.output_text.annotation.added", index,
                        json!({"annotation_index":i,"annotation":annotation})));
                }
            }
            Some("refusal") => {
                let text = part["refusal"].as_str().unwrap_or_default();
                if !text.is_empty() {
                    frames.push(self.event("response.refusal.delta", index, json!({"delta":text})));
                }
            }
            _ => {}
        }
        frames
    }

    pub(super) fn finish(self) -> Vec<Value> {
        let mut frames = Vec::new();
        for (i, part) in self.item["content"].as_array().unwrap().iter().enumerate() {
            match part["type"].as_str() {
                Some("output_text") => frames.push(self.event("response.output_text.done", i,
                    json!({"text":part["text"],"logprobs":part.get("logprobs")
                        .cloned().unwrap_or_else(|| json!([]))}))),
                Some("refusal") => frames.push(self.event("response.refusal.done", i,
                    json!({"refusal":part["refusal"]}))),
                _ => {}
            }
            frames.push(self.event("response.content_part.done", i, json!({"part":part})));
        }
        frames.push(json!({"type":"response.output_item.done",
            "output_index":self.output_index, "item":self.item}));
        frames
    }
}

fn initial_part(part: &Value) -> Value {
    let mut initial = part.clone();
    match part["type"].as_str() {
        Some("output_text") => {
            initial["text"] = json!("");
            initial["annotations"] = json!([]);
            if initial.get("logprobs").is_some() { initial["logprobs"] = json!([]); }
        }
        Some("refusal") => initial["refusal"] = json!(""),
        _ => {}
    }
    initial
}

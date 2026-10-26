use serde_json::{json, Value};
use crate::response_stream::{Events, ResponseWriter, View};
use std::io;

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

pub(crate) struct MessageStream {
    item: Value,
    output_index: usize,
}

impl MessageStream {
    pub(crate) fn new(source: &Value, id: String, output_index: usize) -> Self {
        let mut item = Value::Object(source.as_object().into_iter().flatten()
            .filter(|(key, _)| key.as_str() != "content")
            .map(|(key, value)| (key.clone(), value.clone())).collect());
        item["id"] = json!(id);
        item["content"] = json!([]);
        Self { item, output_index }
    }
}

impl MessageStream {
    pub(crate) fn write_start<W: ResponseWriter>(&self, events: &mut Events<'_, W>) -> io::Result<()> {
        events.emit("response.output_item.added", vec![
            ("output_index", View::Number(self.output_index as u64)),
            ("item", View::Overlay(&self.item, vec![("status", View::Text("in_progress"))]))])
    }
    fn write_event<'a, W: ResponseWriter>(&'a self, events: &mut Events<'_, W>,
        kind: &'static str, index: usize, mut fields: Vec<(&'static str, View<'a>)>) -> io::Result<()> {
        fields.extend([("output_index", View::Number(self.output_index as u64)),
            ("item_id", View::Value(&self.item["id"])), ("content_index", View::Number(index as u64))]);
        events.emit(kind, fields)
    }
    pub(crate) fn write_append<W: ResponseWriter>(&mut self, source: &Value,
        events: &mut Events<'_, W>) -> io::Result<()> {
        let incoming = source["content"].as_array().map(Vec::as_slice).unwrap_or(&[]);
        let previous = self.item["content"].as_array().unwrap();
        let offset = previous.len();
        let joined = joins_text(previous, incoming);
        merge_response_message(&mut self.item, source);
        for (i, part) in incoming.iter().enumerate() {
            let index = offset + i - usize::from(joined);
            if i != 0 || !joined {
                self.write_event(events, "response.content_part.added", index,
                    vec![("part", initial_view(part))])?;
            }
            self.write_deltas(events, index, part)?;
        }
        Ok(())
    }
    fn write_deltas<W: ResponseWriter>(&self, events: &mut Events<'_, W>,
        index: usize, part: &Value) -> io::Result<()> {
        match part["type"].as_str() {
            Some("output_text") => {
                let text = part["text"].as_str().unwrap_or_default();
                if !text.is_empty() {
                    self.write_event(events, "response.output_text.delta", index, vec![
                        ("delta", View::Text(text)), ("logprobs", array_field(part, "logprobs"))])?;
                }
                for (i, annotation) in part["annotations"].as_array().into_iter().flatten().enumerate() {
                    self.write_event(events, "response.output_text.annotation.added", index,
                        vec![("annotation_index", View::Number(i as u64)),
                            ("annotation", View::Value(annotation))])?;
                }
            }
            Some("refusal") => {
                let text = part["refusal"].as_str().unwrap_or_default();
                if !text.is_empty() {
                    self.write_event(events, "response.refusal.delta", index,
                        vec![("delta", View::Text(text))])?;
                }
            }
            _ => {}
        }
        Ok(())
    }
    pub(crate) fn write_finish<W: ResponseWriter>(self, events: &mut Events<'_, W>) -> io::Result<()> {
        for (index, part) in self.item["content"].as_array().unwrap().iter().enumerate() {
            match part["type"].as_str() {
                Some("output_text") => self.write_event(events, "response.output_text.done", index,
                    vec![("text", View::Value(&part["text"])), ("logprobs", array_field(part, "logprobs"))])?,
                Some("refusal") => self.write_event(events, "response.refusal.done", index,
                    vec![("refusal", View::Value(&part["refusal"]))])?,
                _ => {}
            }
            self.write_event(events, "response.content_part.done", index,
                vec![("part", View::Value(part))])?;
        }
        events.emit("response.output_item.done", vec![
            ("output_index", View::Number(self.output_index as u64)), ("item", View::Value(&self.item))])
    }
}
fn array_field<'a>(part: &'a Value, field: &str) -> View<'a> {
    part.get(field).map(View::Value).unwrap_or(View::EmptyArray)
}
fn initial_view(part: &Value) -> View<'_> {
    let mut fields = Vec::new();
    match part["type"].as_str() {
        Some("output_text") => {
            fields.extend([("text", View::Text("")), ("annotations", View::EmptyArray)]);
            if part.get("logprobs").is_some() { fields.push(("logprobs", View::EmptyArray)); }
        }
        Some("refusal") => fields.push(("refusal", View::Text(""))),
        _ => {}
    }
    View::Overlay(part, fields)
}

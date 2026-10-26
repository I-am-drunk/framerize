//! Incremental event views. Payload fields borrow their owner during encoding.
use crate::convert::{AssembledResponses, ClientDialect, error_json};
use crate::convert::response_message::MessageStream;
use serde::{Serialize, Serializer, ser::{SerializeMap, SerializeSeq}};
use serde_json::Value;
use std::io::{self, Write};

pub(crate) enum View<'a> {
    Text(&'a str), Number(u64), Value(&'a Value), EmptyArray,
    Object(Vec<(&'static str, View<'a>)>),
    Overlay(&'a Value, Vec<(&'static str, View<'a>)>),
    Output(&'a [Value], &'a [String], &'a [String]),
}
use View::{Text, Number, Object, Overlay};
impl Serialize for View<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Text(v) => serializer.serialize_str(v),
            Self::Number(v) => serializer.serialize_u64(*v),
            Self::Value(v) => v.serialize(serializer),
            Self::EmptyArray => serializer.serialize_seq(Some(0))?.end(),
            Self::Object(fields) => serialize_fields(serializer, None, fields),
            Self::Overlay(base, fields) => serialize_fields(serializer, Some(base), fields),
            Self::Output(items, messages, tools) => serialize_output(serializer, items, messages, tools),
        }
    }
}

fn serialize_fields<S: Serializer>(serializer: S, base: Option<&Value>,
    fields: &[(&str, View<'_>)]) -> Result<S::Ok, S::Error> {
    let mut map = serializer.serialize_map(None)?;
    let mut sorted: Vec<_> = fields.iter().collect();
    sorted.sort_unstable_by_key(|(key, _)| *key);
    let mut sorted = sorted.into_iter().peekable();
    if let Some(base) = base.and_then(Value::as_object) {
        for (key, value) in base {
            while sorted.peek().is_some_and(|(name, _)| *name < key.as_str()) {
                let (name, value) = sorted.next().unwrap(); map.serialize_entry(name, value)?;
            }
            if sorted.peek().is_some_and(|(name, _)| *name == key.as_str()) {
                let (name, value) = sorted.next().unwrap(); map.serialize_entry(name, value)?;
            } else { map.serialize_entry(key, value)?; }
        }
    }
    for (key, value) in sorted { map.serialize_entry(key, value)?; }
    map.end()
}
fn serialize_output<S: Serializer>(serializer: S, items: &[Value],
    messages: &[String], tools: &[String]) -> Result<S::Ok, S::Error> {
    let mut sequence = serializer.serialize_seq(Some(items.len()))?;
    let (mut messages, mut tools) = (messages.iter(), tools.iter());
    for item in items {
        let id = match item["type"].as_str() {
            Some("message") => messages.next(),
            Some("function_call" | "custom_tool_call") => tools.next(),
            _ => None,
        };
        match id {
            Some(id) => sequence.serialize_element(&Overlay(item, vec![("id", Text(id))]))?,
            None => sequence.serialize_element(item)?,
        }
    }
    sequence.end()
}

/// Begin/end delimit semantic events; end is called only after the SSE suffix.
/// Implementations must retain ownership of partial writes when an error occurs.
/// Even an empty payload writes the event header and JSON before the suffix.
/// Calling begin/end without writing an SSE event is outside this contract.
pub trait ResponseWriter: Write {
    fn begin_event(&mut self, sequence: u64, content: bool) -> io::Result<()>;
    fn end_event(&mut self) -> io::Result<()>;
}
pub(crate) struct Events<'a, W> {
    writer: &'a mut W, sequence: u64,
}
impl<W: ResponseWriter> Events<'_, W> {
    pub(crate) fn emit(&mut self, kind: &'static str,
        mut fields: Vec<(&'static str, View<'_>)>) -> io::Result<()> {
        let content = !matches!(kind, "response.created" | "response.in_progress"
            | "response.failed" | "response.completed" | "response.incomplete");
        self.writer.begin_event(self.sequence, content)?;
        write!(self.writer, "event: {kind}\ndata: ")?;
        fields.push(("type", Text(kind)));
        fields.push(("sequence_number", Number(self.sequence)));
        serde_json::to_writer(&mut self.writer, &Object(fields))?;
        self.writer.write_all(b"\n\n")?;
        self.writer.end_event()?;
        self.sequence += 1;
        Ok(())
    }
}

pub struct ResponseEncoder {
    client_model: String, logical_id: String, item_idx: usize,
    message: Option<MessageStream>, message_ids: Vec<String>, tool_ids: Vec<String>,
    next_sequence: u64,
}
impl ResponseEncoder {
    pub fn new(model: &str, id: &str) -> Self {
        Self { client_model: model.into(), logical_id: id.into(), item_idx: 0,
            message: None, message_ids: Vec::new(), tool_ids: Vec::new(), next_sequence: 0 }
    }
    fn response_id(&self) -> String {
        if self.logical_id.starts_with("resp_") { self.logical_id.clone() }
        else { format!("resp_{}", self.logical_id) }
    }
    fn write<W: ResponseWriter>(&mut self, writer: &mut W, _content: bool,
        run: impl FnOnce(&mut Self, &mut Events<'_, W>) -> io::Result<()>) -> io::Result<()> {
        let mut events = Events { writer, sequence: self.next_sequence };
        let result = run(self, &mut events);
        self.next_sequence = events.sequence;
        result
    }
    pub fn open(&mut self, writer: &mut impl ResponseWriter) -> io::Result<()> {
        self.write(writer, false, |this, events| {
            events.emit("response.created", vec![("response", Object(vec![
                ("id", Text(&this.response_id())), ("object", Text("response")),
                ("created_at", Number(crate::convert::now_unix())),
                ("status", Text("in_progress")), ("model", Text(&this.client_model)),
            ]))])?;
            this.pulse(events)
        })
    }
    fn pulse<W: ResponseWriter>(&self, events: &mut Events<'_, W>) -> io::Result<()> {
        events.emit("response.in_progress", vec![("response", Object(vec![
            ("id", Text(&self.response_id())), ("status", Text("in_progress")),
        ]))])
    }
    pub fn heartbeat(&mut self, writer: &mut impl ResponseWriter) -> io::Result<()> {
        self.write(writer, false, |this, events| this.pulse(events))
    }
    pub fn fail(&mut self, writer: &mut impl ResponseWriter, error: &str) -> io::Result<()> {
        self.write(writer, false, |_, events| {
            let (_, shaped) = error_json(ClientDialect::OpenAiResponses, error);
            events.emit("response.failed", vec![("response", Object(vec![
                ("status", Text("failed")), ("error", View::Value(&shaped["error"])),
            ]))])
        })
    }
    fn close_text<W: ResponseWriter>(&mut self, events: &mut Events<'_, W>) -> io::Result<()> {
        if let Some(message) = self.message.take() {
            message.write_finish(events)?;
            self.item_idx += 1;
        }
        Ok(())
    }
    pub fn push_items(&mut self, writer: &mut impl ResponseWriter,
        items: &[Value], continues_text: bool) -> io::Result<()> {
        self.write(writer, !items.is_empty(), |this, events| {
            for item in items { this.push_item(events, item)?; }
            if !continues_text { this.close_text(events)?; }
            Ok(())
        })
    }
    fn push_item<W: ResponseWriter>(&mut self, events: &mut Events<'_, W>,
        item: &Value) -> io::Result<()> {
        match item["type"].as_str() {
            Some("message") => {
                if self.message.is_none() {
                    let id = format!("msg_{}_{}", self.logical_id.trim_start_matches("resp_"),
                        self.message_ids.len() + 1);
                    self.message_ids.push(id.clone());
                    let message = MessageStream::new(item, id, self.item_idx);
                    message.write_start(events)?;
                    self.message = Some(message);
                }
                self.message.as_mut().unwrap().write_append(item, events)
            }
            Some("function_call" | "custom_tool_call") => {
                self.close_text(events)?;
                self.write_tool(events, item)
            }
            Some(_) => {
                self.close_text(events)?;
                for kind in ["response.output_item.added", "response.output_item.done"] {
                    events.emit(kind, vec![("output_index", Number(self.item_idx as u64)),
                        ("item", View::Value(item))])?;
                }
                self.item_idx += 1;
                Ok(())
            }
            None => Ok(()),
        }
    }
    fn write_tool<W: ResponseWriter>(&mut self, events: &mut Events<'_, W>,
        item: &Value) -> io::Result<()> {
        let function = item["type"] == "function_call";
        let (field, prefix, delta, done) = if function {
            ("arguments", "fc", "response.function_call_arguments.delta", "response.function_call_arguments.done")
        } else {
            ("input", "ctc", "response.custom_tool_call_input.delta", "response.custom_tool_call_input.done")
        };
        let id = item["id"].as_str().map(str::to_owned).unwrap_or_else(|| format!(
            "{prefix}_{}_{}", self.logical_id.trim_start_matches("resp_"), self.tool_ids.len() + 1));
        self.tool_ids.push(id.clone());
        let index = self.item_idx as u64;
        let mut added = vec![("id", Text(&id)), (field, Text(""))];
        if function { added.push(("status", Text("in_progress"))); }
        events.emit("response.output_item.added", vec![
            ("output_index", Number(index)), ("item", Overlay(item, added))])?;
        let value = item[field].as_str().unwrap_or_default();
        events.emit(delta, vec![("output_index", Number(index)),
            ("item_id", Text(&id)), ("delta", Text(value))])?;
        events.emit(done, vec![("output_index", Number(index)),
            ("item_id", Text(&id)), (field, Text(value)),
            ("name", View::Value(&item["name"])), ("call_id", View::Value(&item["call_id"]))])?;
        events.emit("response.output_item.done", vec![("output_index", Number(index)),
            ("item", Overlay(item, vec![("id", Text(&id))]))])?;
        self.item_idx += 1;
        Ok(())
    }
    pub fn finish(&mut self, writer: &mut impl ResponseWriter,
        result: &AssembledResponses) -> io::Result<()> {
        self.write(writer, false, |this, events| {
            this.close_text(events)?;
            let id = this.response_id();
            let mut response = vec![("id", Text(&id)), ("object", Text("response")),
                ("created_at", Number(if result.created_at > 0 { result.created_at }
                    else { crate::convert::now_unix() })),
                ("status", Text(&result.status)), ("model", Text(&this.client_model)),
                ("output", View::Output(&result.output, &this.message_ids, &this.tool_ids)),
                ("service_tier", View::Value(&result.service_tier)),
                ("usage", Overlay(&result.usage, vec![
                    ("input_tokens", Number(result.input_tokens)),
                    ("output_tokens", Number(result.output_tokens)),
                    ("total_tokens", Number(result.input_tokens + result.output_tokens)),
                ]))];
            let kind = if result.status == "incomplete" {
                response.push(("incomplete_details", result.incomplete_details.as_ref()
                    .map(View::Value).unwrap_or_else(|| Object(vec![
                        ("reason", Text("max_output_tokens"))]))));
                "response.incomplete"
            } else { "response.completed" };
            events.emit(kind, vec![("response", Object(response))])
        })
    }
}

pub(crate) struct Collector { frames: Vec<String>, current: Vec<u8> }
impl Write for Collector {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.current.extend_from_slice(bytes); Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> { Ok(()) }
}
impl ResponseWriter for Collector {
    fn begin_event(&mut self, _: u64, _: bool) -> io::Result<()> { Ok(()) }
    fn end_event(&mut self) -> io::Result<()> {
        self.current.truncate(self.current.len() - 2);
        self.frames.push(String::from_utf8(std::mem::take(&mut self.current))
            .expect("JSON and SSE framing are UTF-8"));
        Ok(())
    }
}
pub(crate) fn collect(run: impl FnOnce(&mut Collector) -> io::Result<()>) -> Vec<String> {
    let mut writer = Collector { frames: Vec::new(), current: Vec::new() };
    run(&mut writer).expect("in-memory JSON encoding cannot fail");
    writer.frames
}

impl ResponseEncoder {
    /// A publisher may fail at its accepted frontier after speculative encoding.
    pub fn fail_at(writer: &mut impl ResponseWriter, sequence: u64,
        message: &str) -> io::Result<()> {
        let mut events = Events { writer, sequence };
        let (_, shaped) = error_json(ClientDialect::OpenAiResponses, message);
        events.emit("response.failed", vec![("response", Object(vec![
            ("status", Text("failed")), ("error", View::Value(&shaped["error"])),
        ]))])
    }
}

#[cfg(test)]
#[path = "response_stream_tests.rs"]
mod tests;

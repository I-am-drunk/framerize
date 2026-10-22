use super::delivery::Control;
use framer_core::{convert::ClientDialect, native_dispatch::{ComposedStreamOptions,
    FailureKind, stream_frames::Field as F, stream_state::StateBudget}};
use serde_json::Value;
use std::sync::Arc;

fn fields(value: &Value, names: &[&str]) -> bool {
    value.as_object().is_some_and(|m| m.keys().all(|k| names.contains(&k.as_str())))
}

pub(super) struct Renderer {
    options: ComposedStreamOptions, control: Arc<Control>,
    id: String, model: String, created: u64, index: u64, tool_index: u64,
    service_tier: Option<Value>, fingerprint: Option<Value>,
}
impl Renderer {
    pub fn new(options: ComposedStreamOptions, payload: &Value, first: &Value,
        client_model: Option<&str>, budget: &mut StateBudget, control: Arc<Control>)
        -> Result<Self, FailureKind> {
        let id = first["id"].as_str().ok_or(FailureKind::UnsupportedResponse)?;
        let model = client_model.or_else(|| first["model"].as_str())
            .or_else(|| payload["model"].as_str())
            .ok_or(FailureKind::InvalidRequest)?;
        budget.charge(id.len().checked_add(model.len()).and_then(|n| n.checked_add(32))
            .ok_or(FailureKind::BodyTooLarge)?)?;
        if options.dialect == ClientDialect::OpenAiChat { Self::chat_response_supported(first)?; }
        let service_tier = first.get("service_tier").map(|v| budget.copy(v)).transpose()?;
        let fingerprint = first.get("system_fingerprint").map(|v| budget.copy(v)).transpose()?;
        Ok(Self { options, control, id: id.to_owned(), model: model.to_owned(),
            created: first["created_at"].as_u64().unwrap_or(0), index: 0, tool_index: 0,
            service_tier, fingerprint })
    }
    async fn ev(&self, ty: &str, fields: &[(&str, F<'_>)]) -> Result<(), FailureKind> {
        self.control.send(Some(ty), fields).await
    }
    async fn chat(&self, delta: F<'_>, finish: F<'_>) -> Result<(), FailureKind> {
        let choice = [("index", F::Int(0)), ("delta", delta), ("finish_reason", finish)];
        self.chat_frame(F::Array(&[F::Object(&choice)]), F::Null).await
    }
    fn chat_fields<'a>(&'a self, choices: F<'a>, usage: F<'a>)
        -> ([(&'a str, F<'a>); 8], usize) {
        let mut fields = [
            ("id", F::Str(&self.id)), ("object", F::Str("chat.completion.chunk")),
            ("created", F::Int(self.created)), ("model", F::Str(&self.model)),
            ("choices", choices), ("", F::Null), ("", F::Null), ("", F::Null),
        ];
        let mut len = 5;
        if self.options.include_usage { fields[len] = ("usage", usage); len += 1; }
        if let Some(value) = &self.service_tier {
            fields[len] = ("service_tier", F::Value(value)); len += 1;
        }
        if let Some(value) = &self.fingerprint {
            fields[len] = ("system_fingerprint", F::Value(value)); len += 1;
        }
        (fields, len)
    }
    async fn chat_frame(&self, choices: F<'_>, usage: F<'_>) -> Result<(), FailureKind> {
        let (fields, len) = self.chat_fields(choices, usage);
        self.control.send(None, &fields[..len]).await
    }

    pub async fn open(&self, first: &Value) -> Result<(), FailureKind> {
        match self.options.dialect {
            ClientDialect::AnthropicMessages => {
                let usage = [("output_tokens", F::Int(0))];
                let message = [
                    ("id", F::Str(&self.id)), ("type", F::Str("message")),
                    ("role", F::Str("assistant")), ("model", F::Str(&self.model)),
                    ("content", F::Array(&[])), ("stop_reason", F::Null),
                    ("stop_sequence", F::Null),
                    ("usage", F::Overlay(&first["usage"], &usage)),
                ];
                self.ev("message_start", &[("message", F::Overlay(first, &message))]).await
            }
            ClientDialect::OpenAiChat => {
                self.chat(F::Object(&[("role", F::Str("assistant"))]), F::Null).await
            }
            ClientDialect::OpenAiResponses => {
                let response = [("id", F::Str(&self.id)), ("model", F::Str(&self.model)),
                    ("status", F::Str("in_progress")), ("output", F::Array(&[]))];
                self.ev("response.created", &[("response", F::Overlay(first, &response))]).await?;
                self.ev("response.in_progress", &[("response", F::Overlay(first, &response))]).await
            }
        }
    }
    pub async fn chunk(&mut self, items: &[Value]) -> Result<(), FailureKind> {
        if self.options.dialect == ClientDialect::OpenAiChat {
            for item in items { Self::chat_supported(item)?; }
        }
        for item in items {
            match self.options.dialect {
                ClientDialect::AnthropicMessages => self.message_block(item).await?,
                ClientDialect::OpenAiResponses => self.response_item(item).await?,
                ClientDialect::OpenAiChat => self.chat_item(item).await?,
            }
            self.index = self.index.checked_add(1).ok_or(FailureKind::BodyTooLarge)?;
        }
        Ok(())
    }

    async fn message_block(&self, block: &Value) -> Result<(), FailureKind> {
        let kind = block["type"].as_str().ok_or(FailureKind::UnsupportedResponse)?;
        if kind == "text" {
            let text = block["text"].as_str().ok_or(FailureKind::UnsupportedResponse)?;
            self.ev("content_block_start", &[("index", F::Int(self.index)),
                ("content_block", F::Overlay(block, &[("text", F::Str(""))]))]).await?;
            self.ev("content_block_delta", &[("index", F::Int(self.index)),
                ("delta", F::Object(&[("type", F::Str("text_delta")), ("text", F::Str(text))]))]).await?;
        } else if kind == "tool_use" {
            if !block["id"].is_string() || !block["name"].is_string() || !block["input"].is_object() {
                return Err(FailureKind::UnsupportedResponse);
            }
            self.ev("content_block_start", &[("index", F::Int(self.index)),
                ("content_block", F::Overlay(block, &[("input", F::Object(&[]))]))]).await?;
            self.ev("content_block_delta", &[("index", F::Int(self.index)),
                ("delta", F::Object(&[("type", F::Str("input_json_delta")),
                    ("partial_json", F::Encoded(&block["input"]))]))]).await?;
        } else {
            self.ev("content_block_start", &[("index", F::Int(self.index)),
                ("content_block", F::Value(block))]).await?;
        }
        self.ev("content_block_stop", &[("index", F::Int(self.index))]).await
    }
    async fn response_item(&self, item: &Value) -> Result<(), FailureKind> {
        if !item["id"].is_string() { return Err(FailureKind::UnsupportedResponse); }
        match item["type"].as_str().ok_or(FailureKind::UnsupportedResponse)? {
            "message" => {
                let parts = item["content"].as_array().ok_or(FailureKind::UnsupportedResponse)?;
                self.ev("response.output_item.added", &[("output_index", F::Int(self.index)),
                    ("item", F::Overlay(item, &[("content", F::Array(&[])),
                        ("status", F::Str("in_progress"))]))]).await?;
                for (index, part) in parts.iter().enumerate() {
                    self.response_part(item, part, index as u64).await?;
                }
            }
            "function_call" => self.call_item(item, false).await?,
            "custom_tool_call" => self.call_item(item, true).await?,
            _ => {
                // Reasoning, encrypted data, tool results and future typed
                // items remain whole. No field or ordering is discarded.
                self.ev("response.output_item.added", &[("output_index", F::Int(self.index)),
                    ("item", F::Value(item))]).await?;
            }
        }
        self.ev("response.output_item.done", &[("output_index", F::Int(self.index)),
            ("item", F::Value(item))]).await
    }

    async fn call_item(&self, item: &Value, custom: bool) -> Result<(), FailureKind> {
        let (field, delta, done) = if custom {
            ("input", "response.custom_tool_call_input.delta", "response.custom_tool_call_input.done")
        } else {
            ("arguments", "response.function_call_arguments.delta", "response.function_call_arguments.done")
        };
        let text = item[field].as_str().ok_or(FailureKind::UnsupportedResponse)?;
        if !item["name"].is_string() || !item["call_id"].is_string() {
            return Err(FailureKind::UnsupportedResponse);
        }
        self.ev("response.output_item.added", &[("output_index", F::Int(self.index)),
            ("item", F::Overlay(item, &[(field, F::Str("")), ("status", F::Str("in_progress"))]))]).await?;
        self.ev(delta, &[("output_index", F::Int(self.index)),
            ("item_id", F::Value(&item["id"])), ("delta", F::Str(text))]).await?;
        self.ev(done, &[("output_index", F::Int(self.index)),
            ("item_id", F::Value(&item["id"])), (field, F::Str(text))]).await
    }
    async fn response_part(&self, item: &Value, part: &Value, index: u64)
        -> Result<(), FailureKind> {
        let ty = part["type"].as_str().ok_or(FailureKind::UnsupportedResponse)?;
        let text = match ty {
            "output_text" => Some(("text", "response.output_text.delta", "response.output_text.done")),
            "refusal" => Some(("refusal", "response.refusal.delta", "response.refusal.done")),
            _ => None,
        };
        if let Some((field, delta, done)) = text {
            let text = part[field].as_str().ok_or(FailureKind::UnsupportedResponse)?;
            self.ev("response.content_part.added", &[("output_index", F::Int(self.index)),
                ("item_id", F::Value(&item["id"])), ("content_index", F::Int(index)),
                ("part", F::Overlay(part, &[(field, F::Str(""))]))]).await?;
            let logprobs = part.get("logprobs");
            if logprobs.is_some_and(|v| !v.is_array()) { return Err(FailureKind::UnsupportedResponse); }
            let mut fields = [("output_index", F::Int(self.index)),
                ("item_id", F::Value(&item["id"])), ("content_index", F::Int(index)),
                ("delta", F::Str(text)), ("logprobs", F::Array(&[]))];
            if let Some(value) = logprobs { fields[4].1 = F::Value(value); }
            let len = if ty == "output_text" { 5 } else { 4 };
            self.ev(delta, &fields[..len]).await?;
            fields[3].0 = field;
            self.ev(done, &fields[..len]).await?;
        } else {
            self.ev("response.content_part.added", &[("output_index", F::Int(self.index)),
                ("item_id", F::Value(&item["id"])), ("content_index", F::Int(index)),
                ("part", F::Value(part))]).await?;
        }
        self.ev("response.content_part.done", &[("output_index", F::Int(self.index)),
            ("item_id", F::Value(&item["id"])), ("content_index", F::Int(index)),
            ("part", F::Value(part))]).await
    }
    fn chat_supported(item: &Value) -> Result<(), FailureKind> {
        let supported = match item["type"].as_str() {
            Some("message") => {
                fields(item, &["type", "status", "role", "content"])
                    && item.get("status").is_none_or(|v| v.as_str() == Some("completed"))
                    && item["role"].as_str() == Some("assistant")
                    && item["content"].as_array().is_some_and(|parts| parts.len() == 1 && parts.iter().all(|part| {
                        part["type"].as_str() == Some("output_text") && part["text"].is_string()
                            && fields(part, &["type", "text", "annotations"])
                            && part.get("annotations").is_none_or(|a| a.as_array().is_some_and(Vec::is_empty))
                    }))
            }
            Some("function_call") => {
                fields(item, &["type", "status", "call_id", "name", "arguments"])
                    && item["call_id"].is_string() && item["name"].is_string()
                    && item["arguments"].is_string()
                    && item.get("status").is_none_or(|v| v.as_str() == Some("completed"))
            }
            _ => false,
        };
        if supported { Ok(()) } else { Err(FailureKind::UnsupportedResponse) }
    }

    fn chat_usage_supported(usage: &Value) -> Result<(), FailureKind> {
        if !fields(usage, &["input_tokens", "output_tokens", "total_tokens",
            "input_tokens_details", "output_tokens_details"])
            || !usage["input_tokens"].is_u64() || !usage["output_tokens"].is_u64() {
            return Err(FailureKind::UnsupportedResponse);
        }
        if let Some(total) = usage.get("total_tokens") {
            if total.as_u64() != usage["input_tokens"].as_u64()
                .and_then(|input| input.checked_add(usage["output_tokens"].as_u64().unwrap())) {
                return Err(FailureKind::UnsupportedResponse);
            }
        }
        for (key, allowed) in [
            ("input_tokens_details", &["cached_tokens", "audio_tokens"][..]),
            ("output_tokens_details", &["reasoning_tokens", "audio_tokens",
                "accepted_prediction_tokens", "rejected_prediction_tokens"][..]),
        ] {
            if let Some(value) = usage.get(key) {
                if !fields(value, allowed) || value.as_object().unwrap().values().any(|v| !v.is_u64()) {
                    return Err(FailureKind::UnsupportedResponse);
                }
            }
        }
        Ok(())
    }

    fn chat_response_supported(body: &Value) -> Result<(), FailureKind> {
        // An output item ID or arbitrary response field has no Chat carrier.
        if !fields(body, &["id", "object", "created_at", "model", "status", "output",
            "usage", "error", "incomplete_details", "service_tier", "system_fingerprint"])
            || body.get("object").is_some_and(|v| v.as_str() != Some("response"))
            || body.get("created_at").is_some_and(|v| !v.is_u64())
            || body.get("model").is_some_and(|v| !v.is_string())
            || body.get("system_fingerprint").is_some_and(|v| !v.is_string()) {
            return Err(FailureKind::UnsupportedResponse);
        }
        if body["status"] == "incomplete" {
            if !fields(&body["incomplete_details"], &["reason"])
                || !matches!(body["incomplete_details"]["reason"].as_str(),
                    Some("max_output_tokens") | Some("content_filter")) {
                return Err(FailureKind::UnsupportedResponse);
            }
        } else if body.get("incomplete_details").is_some_and(|v| !v.is_null()) {
            return Err(FailureKind::UnsupportedResponse);
        }
        let output = body["output"].as_array().ok_or(FailureKind::UnsupportedResponse)?;
        if output.iter().filter(|item| item["type"] == "message").count() > 1 {
            return Err(FailureKind::UnsupportedResponse);
        }
        for item in output { Self::chat_supported(item)?; }
        Self::chat_usage_supported(&body["usage"])
    }

    async fn chat_item(&mut self, item: &Value) -> Result<(), FailureKind> {
        if item["type"].as_str() == Some("message") {
            for part in item["content"].as_array().ok_or(FailureKind::UnsupportedResponse)? {
                self.chat(F::Object(&[("content", F::Value(&part["text"]))]), F::Null).await?;
            }
        } else {
            let function = [("name", F::Value(&item["name"])), ("arguments", F::Value(&item["arguments"]))];
            let call = [("index", F::Int(self.tool_index)), ("id", F::Value(&item["call_id"])),
                ("type", F::Str("function")), ("function", F::Object(&function))];
            self.chat(F::Object(&[("tool_calls", F::Array(&[F::Object(&call)]))]), F::Null).await?;
            self.tool_index = self.tool_index.checked_add(1).ok_or(FailureKind::BodyTooLarge)?;
        }
        Ok(())
    }
    pub async fn finish(self, last: &Value, output: &[Value], input: u64, tokens: u64)
        -> Result<(), FailureKind> {
        let total = input.checked_add(tokens).ok_or(FailureKind::UnsupportedResponse)?;
        match self.options.dialect {
            ClientDialect::AnthropicMessages => {
                let delta = [("stop_reason", F::Value(&last["stop_reason"])),
                    ("stop_sequence", F::Value(&last["stop_sequence"]))];
                let usage = [("input_tokens", F::Int(input)), ("output_tokens", F::Int(tokens))];
                self.ev("message_delta", &[("delta", F::Object(&delta)),
                    ("usage", F::Overlay(&last["usage"], &usage))]).await?;
                self.ev("message_stop", &[]).await
            }
            ClientDialect::OpenAiResponses => {
                let usage = [("input_tokens", F::Int(input)), ("output_tokens", F::Int(tokens)),
                    ("total_tokens", F::Int(total))];
                let response = [("id", F::Str(&self.id)), ("model", F::Str(&self.model)),
                    ("created_at", F::Int(self.created)),
                    ("output", F::Values(output)), ("usage", F::Overlay(&last["usage"], &usage))];
                let event = if last["status"].as_str() == Some("incomplete") {
                    "response.incomplete"
                } else { "response.completed" };
                self.ev(event, &[("response", F::Overlay(last, &response))]).await
            }
            ClientDialect::OpenAiChat => {
                let reason = if last["status"].as_str() == Some("incomplete") {
                    if last["incomplete_details"]["reason"] == "content_filter" { "content_filter" }
                    else { "length" }
                }
                    else if self.tool_index != 0 { "tool_calls" } else { "stop" };
                let choice = [("index", F::Int(0)), ("delta", F::Object(&[])),
                    ("finish_reason", F::Str(reason))];
                let choices = [F::Object(&choice)];
                let (finish, finish_len) = self.chat_fields(F::Array(&choices), F::Null);
                let mut usage = [("prompt_tokens", F::Int(input)), ("completion_tokens", F::Int(tokens)),
                    ("total_tokens", F::Int(total)), ("", F::Null), ("", F::Null)];
                let mut len = 3;
                for (from, to) in [("input_tokens_details", "prompt_tokens_details"),
                    ("output_tokens_details", "completion_tokens_details")] {
                    if let Some(value) = last["usage"].get(from) {
                        usage[len] = (to, F::Value(value)); len += 1;
                    }
                }
                let (last, last_len) = self.chat_fields(F::Array(&[]), F::Object(&usage[..len]));
                self.control.chat_terminal(&finish[..finish_len],
                    self.options.include_usage.then_some(&last[..last_len])).await
            }
        }
    }
}

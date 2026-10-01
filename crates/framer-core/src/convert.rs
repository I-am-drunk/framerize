//! Dialect conversion via a canonical intermediate representation (IR).
//!
//! Why this exists: the only UNMETERED upstream route is `/ai/v3/chat/`
//! (OpenRouter chat/completions) with `stream:false` — verified live
//! 2026-10-01 (verified-captures §6-8: chat stream:false never debits;
//! anthropic + responses routes debit in BOTH modes). So a client speaking
//! Anthropic Messages (Claude Code) or OpenAI Responses (Codex) gets free
//! inference by converting its dialect to chat/completions, running the
//! freechat chain, and converting the answer back.
//!
//! Extensibility rule: every dialect implements exactly three functions —
//! `parse_request`, `render_response`, `render_stream` — and registers them in
//! the match arms below. Adding a dialect = one module + one enum arm. The
//! upstream side is always chat/completions, so converters are written once
//! per CLIENT dialect, never per pair.

use serde_json::{json, Map, Value};

/// Client-facing dialects (the shapes our compat endpoints accept).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClientDialect {
    /// Anthropic /v1/messages (Claude Code).
    AnthropicMessages,
    /// OpenAI /v1/chat/completions.
    OpenAiChat,
    /// OpenAI /v1/responses (Codex).
    OpenAiResponses,
}

/// Normalized request. Content is flattened to plain text (tool use, images,
/// and reasoning blocks are out of scope for the free path — use the metered
/// passthrough for those).
#[derive(Debug, Clone)]
pub struct CanonicalRequest {
    /// Model id AS THE CLIENT SENT IT. Normalize with `to_openrouter_id` when
    /// building the upstream chat payload.
    pub model: String,
    pub instructions: Option<String>,
    pub messages: Vec<CanonicalMessage>,
    pub max_tokens: u32,
    pub temperature: Option<f64>,
    pub stop: Vec<String>,
    /// Whether the client asked for a stream (decides render_stream vs JSON).
    pub stream: bool,
}

#[derive(Debug, Clone)]
pub struct CanonicalMessage {
    pub role: String,
    pub content: String,
}

/// Normalized response.
#[derive(Debug, Clone)]
pub struct CanonicalResponse {
    pub text: String,
    /// "stop" | "length" (dialects map these to their own vocabularies).
    pub finish_reason: String,
    pub model: String,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
}

// ---------------------------------------------------------------------------
// Model id normalization: every upstream free call goes to the OpenRouter chat
// route, which takes PREFIXED, DOTTED catalog ids ("anthropic/claude-opus-5.5").
// Verified live: unprefixed ids 404 on /ai/v3/chat/; the anthropic route takes
// dashed unprefixed ("claude-opus-5-5"); responses route takes unprefixed too.
// ---------------------------------------------------------------------------

pub fn to_openrouter_id(model: &str) -> String {
    let m = model.strip_suffix(":batch").unwrap_or(model);
    if m.contains('/') {
        return m.to_string();
    }
    if let Some(rest) = m.strip_prefix("claude-") {
        return format!("anthropic/claude-{}", join_version(rest));
    }
    if m.starts_with("gpt") || m.starts_with("o1") || m.starts_with("o3") || m.starts_with("o4") {
        return format!("openai/{m}");
    }
    if m.starts_with("gemini") {
        return format!("google/{m}");
    }
    if m.starts_with("grok") {
        return format!("x-ai/{m}");
    }
    if m.starts_with("deepseek") {
        return format!("deepseek/{m}");
    }
    m.to_string()
}

/// "opus-5-5" → "opus-5.5"; "sonnet-4-5" → "sonnet-4.5"; "fable-5-1" → "fable-5.1".
/// Trailing all-numeric dash segments fold into a dotted version suffix.
fn join_version(rest: &str) -> String {
    let parts: Vec<&str> = rest.split('-').collect();
    let mut split_at = parts.len();
    while split_at > 0 && !parts[split_at - 1].is_empty() && parts[split_at - 1].chars().all(|c| c.is_ascii_digit() || c == '.') {
        split_at -= 1;
    }
    if split_at == parts.len() {
        return rest.to_string();
    }
    let name = parts[..split_at].join("-");
    let version = parts[split_at..].join(".");
    if name.is_empty() { version } else { format!("{name}-{version}") }
}

// ---------------------------------------------------------------------------
// Registry: parse client request → IR
// ---------------------------------------------------------------------------

pub fn parse_request(d: ClientDialect, body: &Value) -> Result<CanonicalRequest, String> {
    match d {
        ClientDialect::AnthropicMessages => anthropic::parse_request(body),
        ClientDialect::OpenAiChat => openai_chat::parse_request(body),
        ClientDialect::OpenAiResponses => openai_responses::parse_request(body),
    }
}

/// Render IR → client dialect JSON (non-streaming).
pub fn render_response(d: ClientDialect, r: &CanonicalResponse) -> Value {
    match d {
        ClientDialect::AnthropicMessages => anthropic::render_response(r),
        ClientDialect::OpenAiChat => openai_chat::render_response(r),
        ClientDialect::OpenAiResponses => openai_responses::render_response(r),
    }
}

/// Render IR → client dialect SSE frames (each WITHOUT the trailing blank line;
/// the transport adds "\n\n"). Used when the client asked for a stream but the
/// free path produced a complete answer: we synthesize a well-formed stream so
/// streaming-only clients (Claude Code) work unchanged.
pub fn render_stream(d: ClientDialect, r: &CanonicalResponse) -> Vec<String> {
    match d {
        ClientDialect::AnthropicMessages => anthropic::render_stream(r),
        ClientDialect::OpenAiChat => openai_chat::render_stream(r),
        ClientDialect::OpenAiResponses => openai_responses::render_stream(r),
    }
}

// ---------------------------------------------------------------------------
// Upstream chat/completions side (the one upstream dialect everything shares)
// ---------------------------------------------------------------------------

/// IR → upstream chat payload for the free route (always stream:false).
pub fn to_chat_payload(req: &CanonicalRequest, chunk_tokens: u32) -> Value {
    let mut messages = Vec::new();
    if let Some(sys) = &req.instructions {
        messages.push(json!({"role": "system", "content": sys}));
    }
    for m in &req.messages {
        messages.push(json!({"role": m.role, "content": m.content}));
    }
    let mut payload = Map::new();
    payload.insert("model".into(), json!(to_openrouter_id(&req.model)));
    payload.insert("messages".into(), Value::Array(messages));
    payload.insert("max_tokens".into(), json!(chunk_tokens));
    payload.insert("stream".into(), json!(false));
    if let Some(t) = req.temperature {
        payload.insert("temperature".into(), json!(t));
    }
    if !req.stop.is_empty() {
        payload.insert("stop".into(), json!(req.stop));
    }
    Value::Object(payload)
}

/// Upstream chat completion JSON → IR.
pub fn from_chat_response(body: &Value) -> Result<CanonicalResponse, String> {
    let choice = body
        .get("choices").and_then(|c| c.get(0))
        .ok_or_else(|| format!("chat response has no choices: {}", &body.to_string()[..200.min(body.to_string().len())]))?;
    let text = choice.pointer("/message/content").and_then(Value::as_str).unwrap_or_default().to_string();
    let finish = choice.get("finish_reason").and_then(Value::as_str).unwrap_or("stop").to_string();
    let u = body.get("usage").cloned().unwrap_or(Value::Null);
    Ok(CanonicalResponse {
        text,
        finish_reason: finish,
        model: body.get("model").and_then(Value::as_str).unwrap_or_default().to_string(),
        prompt_tokens: u.get("prompt_tokens").and_then(Value::as_u64).unwrap_or(0),
        completion_tokens: u.get("completion_tokens").and_then(Value::as_u64).unwrap_or(0),
    })
}

fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn rid(prefix: &str) -> String {
    format!("{prefix}{:016x}", rand_u64())
}

fn rand_u64() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let mut x = nanos as u64 ^ (std::process::id() as u64) << 32;
    // splitmix64
    x = x.wrapping_add(0x9E3779B97F4A7C15);
    x = (x ^ (x >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94D049BB133111EB);
    x ^ (x >> 31)
}

// ---------------------------------------------------------------------------
pub mod anthropic {
    use super::*;

    pub fn parse_request(body: &Value) -> Result<CanonicalRequest, String> {
        let model = body.get("model").and_then(Value::as_str)
            .ok_or("anthropic request: model is required")?.to_string();
        let instructions = match body.get("system") {
            Some(Value::String(s)) => Some(s.clone()),
            Some(Value::Array(parts)) => Some(parts.iter()
                .filter_map(|p| p.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>().join("\n")),
            _ => None,
        };
        let raw = body.get("messages").and_then(Value::as_array)
            .ok_or("anthropic request: messages array is required")?;
        let mut messages = Vec::new();
        for m in raw {
            let role = m.get("role").and_then(Value::as_str).unwrap_or("user").to_string();
            let content = match m.get("content") {
                Some(Value::String(s)) => s.clone(),
                Some(Value::Array(parts)) => parts.iter()
                    .filter(|p| p.get("type").and_then(Value::as_str) == Some("text"))
                    .filter_map(|p| p.get("text").and_then(Value::as_str))
                    .collect::<Vec<_>>().join("\n"),
                _ => String::new(),
            };
            messages.push(CanonicalMessage { role, content });
        }
        let stop = body.get("stop_sequences").and_then(Value::as_array)
            .map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect())
            .unwrap_or_default();
        Ok(CanonicalRequest {
            model,
            instructions,
            messages,
            max_tokens: body.get("max_tokens").and_then(Value::as_u64).unwrap_or(1024) as u32,
            temperature: body.get("temperature").and_then(Value::as_f64),
            stop,
            stream: body.get("stream").and_then(Value::as_bool).unwrap_or(false),
        })
    }

    fn stop_reason(finish: &str) -> &str {
        match finish {
            "length" => "max_tokens",
            "stop" => "end_turn",
            _ => "end_turn",
        }
    }

    pub fn render_response(r: &CanonicalResponse) -> Value {
        json!({
            "id": rid("msg_"),
            "type": "message",
            "role": "assistant",
            "model": r.model,
            "content": [{ "type": "text", "text": r.text }],
            "stop_reason": stop_reason(&r.finish_reason),
            "stop_sequence": null,
            "usage": { "input_tokens": r.prompt_tokens, "output_tokens": r.completion_tokens },
        })
    }

    pub fn render_stream(r: &CanonicalResponse) -> Vec<String> {
        let id = rid("msg_");
        vec![
            format!("event: message_start\ndata: {}", json!({"type":"message_start","message":{
                "id": id, "type":"message", "role":"assistant", "model": r.model,
                "content": [], "stop_reason": null, "stop_sequence": null,
                "usage": {"input_tokens": r.prompt_tokens, "output_tokens": 0}
            }})),
            format!("event: content_block_start\ndata: {}", json!({"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}})),
            format!("event: content_block_delta\ndata: {}", json!({"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text": r.text}})),
            format!("event: content_block_stop\ndata: {}", json!({"type":"content_block_stop","index":0})),
            format!("event: message_delta\ndata: {}", json!({"type":"message_delta","delta":{"stop_reason": stop_reason(&r.finish_reason),"stop_sequence":null},"usage":{"output_tokens": r.completion_tokens}})),
            "event: message_stop\ndata: {\"type\":\"message_stop\"}".to_string(),
        ]
    }
}

// ---------------------------------------------------------------------------
pub mod openai_chat {
    use super::*;

    pub fn parse_request(body: &Value) -> Result<CanonicalRequest, String> {
        let model = body.get("model").and_then(Value::as_str)
            .ok_or("chat request: model is required")?.to_string();
        let raw = body.get("messages").and_then(Value::as_array)
            .ok_or("chat request: messages array is required")?;
        let mut instructions = None;
        let mut messages = Vec::new();
        for m in raw {
            let role = m.get("role").and_then(Value::as_str).unwrap_or("user");
            let content = match m.get("content") {
                Some(Value::String(s)) => s.clone(),
                Some(Value::Array(parts)) => parts.iter()
                    .filter(|p| p.get("type").and_then(Value::as_str) == Some("text"))
                    .filter_map(|p| p.get("text").and_then(Value::as_str))
                    .collect::<Vec<_>>().join("\n"),
                _ => String::new(),
            };
            if matches!(role, "system" | "developer") && instructions.is_none() {
                instructions = Some(content);
            } else {
                messages.push(CanonicalMessage { role: role.to_string(), content });
            }
        }
        let stop = body.get("stop").and_then(Value::as_array)
            .map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect())
            .unwrap_or_default();
        Ok(CanonicalRequest {
            model,
            instructions,
            messages,
            max_tokens: body.get("max_completion_tokens").or_else(|| body.get("max_tokens"))
                .and_then(Value::as_u64).unwrap_or(1024) as u32,
            temperature: body.get("temperature").and_then(Value::as_f64),
            stop,
            stream: body.get("stream").and_then(Value::as_bool).unwrap_or(false),
        })
    }

    pub fn render_response(r: &CanonicalResponse) -> Value {
        json!({
            "id": rid("chatcmpl-"),
            "object": "chat.completion",
            "created": now_unix(),
            "model": r.model,
            "choices": [{
                "index": 0,
                "message": { "role": "assistant", "content": r.text },
                "finish_reason": r.finish_reason,
            }],
            "usage": {
                "prompt_tokens": r.prompt_tokens,
                "completion_tokens": r.completion_tokens,
                "total_tokens": r.prompt_tokens + r.completion_tokens,
            },
        })
    }

    pub fn render_stream(r: &CanonicalResponse) -> Vec<String> {
        let id = rid("chatcmpl-");
        let mk = |delta: Value, finish: Value| format!("data: {}", json!({
            "id": id, "object": "chat.completion.chunk", "created": now_unix(), "model": r.model,
            "choices": [{ "index": 0, "delta": delta, "finish_reason": finish }]
        }));
        vec![
            mk(json!({"role":"assistant"}), Value::Null),
            mk(json!({"content": r.text}), Value::Null),
            mk(json!({}), json!(r.finish_reason)),
            "data: [DONE]".to_string(),
        ]
    }
}

// ---------------------------------------------------------------------------
pub mod openai_responses {
    use super::*;

    pub fn parse_request(body: &Value) -> Result<CanonicalRequest, String> {
        let model = body.get("model").and_then(Value::as_str)
            .ok_or("responses request: model is required")?.to_string();
        let instructions = body.get("instructions").and_then(Value::as_str).map(str::to_string);
        let mut messages = Vec::new();
        match body.get("input") {
            Some(Value::String(s)) => messages.push(CanonicalMessage { role: "user".into(), content: s.clone() }),
            Some(Value::Array(items)) => {
                for item in items {
                    let role = item.get("role").and_then(Value::as_str).unwrap_or("user");
                    let content = match item.get("content") {
                        Some(Value::String(s)) => s.clone(),
                        Some(Value::Array(parts)) => parts.iter()
                            .filter_map(|p| p.get("text").and_then(Value::as_str))
                            .collect::<Vec<_>>().join("\n"),
                        _ => String::new(),
                    };
                    messages.push(CanonicalMessage { role: role.into(), content });
                }
            }
            _ => return Err("responses request: input is required".into()),
        }
        Ok(CanonicalRequest {
            model,
            instructions,
            messages,
            max_tokens: body.get("max_output_tokens").and_then(Value::as_u64).unwrap_or(1024) as u32,
            temperature: body.get("temperature").and_then(Value::as_f64),
            stop: Vec::new(),
            stream: body.get("stream").and_then(Value::as_bool).unwrap_or(false),
        })
    }

    fn output_item(r: &CanonicalResponse) -> Value {
        json!({
            "type": "message", "id": rid("msg_"), "status": "completed", "role": "assistant",
            "content": [{ "type": "output_text", "text": r.text, "annotations": [] }]
        })
    }

    pub fn render_response(r: &CanonicalResponse) -> Value {
        json!({
            "id": rid("resp_"),
            "object": "response",
            "created_at": now_unix(),
            "status": "completed",
            "model": r.model,
            "output": [output_item(r)],
            "usage": {
                "input_tokens": r.prompt_tokens,
                "output_tokens": r.completion_tokens,
                "total_tokens": r.prompt_tokens + r.completion_tokens,
            },
        })
    }

    pub fn render_stream(r: &CanonicalResponse) -> Vec<String> {
        let resp = render_response(r);
        let id = resp["id"].clone();
        vec![
            format!("event: response.created\ndata: {}", json!({"type":"response.created","response":{"id":id,"object":"response","created_at":now_unix(),"status":"in_progress","model":r.model,"output":[]}})),
            format!("event: response.output_item.added\ndata: {}", json!({"type":"response.output_item.added","output_index":0,"item":{"type":"message","id":rid("msg_"),"status":"in_progress","role":"assistant","content":[]}})),
            format!("event: response.output_text.delta\ndata: {}", json!({"type":"response.output_text.delta","item_id":rid("msg_"),"output_index":0,"content_index":0,"delta": r.text})),
            format!("event: response.output_item.done\ndata: {}", json!({"type":"response.output_item.done","output_index":0,"item": output_item(r)})),
            format!("event: response.completed\ndata: {}", json!({"type":"response.completed","response": resp})),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn openrouter_id_normalization() {
        assert_eq!(to_openrouter_id("claude-opus-5-5"), "anthropic/claude-opus-5.5");
        assert_eq!(to_openrouter_id("claude-sonnet-4-5"), "anthropic/claude-sonnet-4.5");
        assert_eq!(to_openrouter_id("claude-fable-5-1"), "anthropic/claude-fable-5.1");
        assert_eq!(to_openrouter_id("gpt-4.1"), "openai/gpt-4.1");
        assert_eq!(to_openrouter_id("gpt-6-sol"), "openai/gpt-6-sol");
        assert_eq!(to_openrouter_id("gemini-2.5-flash"), "google/gemini-2.5-flash");
        assert_eq!(to_openrouter_id("anthropic/claude-opus-5.5"), "anthropic/claude-opus-5.5");
    }

    #[test]
    fn anthropic_roundtrip() {
        let req = json!({"model":"claude-opus-5-5","max_tokens":100,"system":"Be terse.","messages":[{"role":"user","content":[{"type":"text","text":"hi"}]}]});
        let ir = parse_request(ClientDialect::AnthropicMessages, &req).unwrap();
        assert_eq!(ir.instructions.as_deref(), Some("Be terse."));
        assert_eq!(ir.messages[0].content, "hi");
        let payload = to_chat_payload(&ir, 600);
        assert_eq!(payload["model"], "anthropic/claude-opus-5.5");
        assert_eq!(payload["stream"], false);
        assert_eq!(payload["messages"][0]["role"], "system");
        let up = json!({"model":"anthropic/claude-opus-5.5","choices":[{"message":{"role":"assistant","content":"hello"},"finish_reason":"stop"}],"usage":{"prompt_tokens":5,"completion_tokens":2}});
        let resp = from_chat_response(&up).unwrap();
        let out = render_response(ClientDialect::AnthropicMessages, &resp);
        assert_eq!(out["type"], "message");
        assert_eq!(out["content"][0]["text"], "hello");
        assert_eq!(out["stop_reason"], "end_turn");
        assert_eq!(out["usage"]["output_tokens"], 2);
    }

    #[test]
    fn responses_roundtrip() {
        let req = json!({"model":"gpt-6-sol","instructions":"Be terse.","input":[{"role":"user","content":[{"type":"input_text","text":"hi"}]}]});
        let ir = parse_request(ClientDialect::OpenAiResponses, &req).unwrap();
        let payload = to_chat_payload(&ir, 600);
        assert_eq!(payload["model"], "openai/gpt-6-sol");
        let up = json!({"choices":[{"message":{"content":"ok"},"finish_reason":"length"}],"usage":{"prompt_tokens":1,"completion_tokens":1}});
        let resp = from_chat_response(&up).unwrap();
        let out = render_response(ClientDialect::OpenAiResponses, &resp);
        assert_eq!(out["object"], "response");
        assert_eq!(out["output"][0]["content"][0]["text"], "ok");
    }

    #[test]
    fn streams_are_wellformed() {
        let r = CanonicalResponse { text: "hi".into(), finish_reason: "stop".into(), model: "m".into(), prompt_tokens: 1, completion_tokens: 1 };
        for d in [ClientDialect::AnthropicMessages, ClientDialect::OpenAiChat, ClientDialect::OpenAiResponses] {
            let frames = render_stream(d, &r);
            assert!(frames.len() >= 4);
            for f in &frames { assert!(f.starts_with("data:") || f.starts_with("event:"), "{f}"); }
        }
    }
}

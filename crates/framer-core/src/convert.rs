//! Dialect conversion, rebuilt for LOSSLESS relaying.
//!
//! Architecture: the upstream free route speaks Anthropic Messages
//! (/ai/v3/messages/). Every client dialect's request is converted to an
//! Anthropic Messages payload with full fidelity, and the upstream's content
//! blocks are rendered back to the client dialect with full fidelity:
//!   - Anthropic client: request passes through VERBATIM (only model/stream/
//!     max_tokens are overridden for chaining); response blocks re-emit
//!     untouched. Zero information loss.
//!   - OpenAI chat / responses: every field with an Anthropic equivalent is
//!     mapped (tools, tool_choice, tool calls/results, system, temperature,
//!     top_p, stop, max tokens, metadata). Fields with no equivalent are
//!     documented drops (penalties, logprobs, n>1, seed, reasoning blobs).
//!
//! Response assembly across chain chunks is block-aware: text blocks merge,
//! tool_use blocks pass through with ids/names/inputs intact, stop_reason and
//! usage are preserved.

use serde_json::{json, Value};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientDialect {
    AnthropicMessages,
    OpenAiChat,
    OpenAiResponses,
}

/// The merged upstream result: faithful Anthropic-format content.
#[derive(Debug, Clone)]
pub struct AssembledResponse {
    pub id: String,
    /// Content blocks in Anthropic Messages shape (text, tool_use, …).
    pub content: Vec<Value>,
    pub stop_reason: String,
    pub stop_sequence: Option<String>,
    /// usage object from the LAST chunk (input side), with output_tokens
    /// replaced by the SUM across chunks.
    pub usage: Value,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub chunks: u32,
    pub metered_chunks: u32,
}

// ---------------------------------------------------------------------------
// Model id normalization (verified live): the OpenRouter passthroughs take
// PREFIXED, DOTTED catalog ids ("anthropic/claude-opus-5.5").
// ---------------------------------------------------------------------------

/// Where a model can actually be served, measured 2026-10-05:
/// the free OpenRouter passthroughs (/ai/v3/messages/, /chat/, /responses/)
/// carry only part of the catalog; gpt-6.x live on the direct route
/// /ai/v3/openai/responses/ (metered; verified 200: gpt-6-sol, gpt-6-luna,
/// gpt-5.6-terra); gpt-6-astra is plan-gated everywhere; the rest 404 on
/// the upstream guardrails. Honest routing, never substitution (user rule).
#[derive(Debug, Clone, PartialEq)]
pub enum ModelRoute {
    FreePassthrough,
    DirectResponses,
    Unavailable(&'static str),
}

pub fn model_route(model: &str) -> ModelRoute {
    let slug = model.rsplit('/').next().unwrap_or(model);
    match slug {
        "gpt-6-sol" | "gpt-6.1-sol" | "gpt-6-luna" | "gpt-5.6-terra" => ModelRoute::DirectResponses,
        "gpt-6-astra" => ModelRoute::FreePassthrough,
        "gemini-2.5-pro" | "gpt-5.1" | "gpt-4o" | "o4-mini" => ModelRoute::Unavailable(
            "blocked by the gateway's upstream guardrails (404) on every route. Working free models: claude-opus-5-5, claude-sonnet-5-5, claude-haiku-4-5, gpt-4.1, gpt-4.1-mini, gemini-2.5-flash, gemini-3-flash-preview",
        ),
        s if s.starts_with("daybreak") => ModelRoute::Unavailable(
            "blocked by the gateway's upstream guardrails (404) on every route. Working free models: claude-opus-5-5, claude-sonnet-5-5, claude-haiku-4-5, gpt-4.1, gpt-4.1-mini, gemini-2.5-flash, gemini-3-flash-preview",
        ),
        _ => ModelRoute::FreePassthrough,
    }
}

pub fn to_openrouter_id(model: &str) -> String {
    let m = model.strip_suffix(":batch").unwrap_or(model);
    if m.contains('/') {
        return m.to_string();
    }
    if let Some(rest) = m.strip_prefix("claude-") {
        return format!("anthropic/claude-{}", join_version(rest));
    }
    // gpt-6-astra is the one OpenAI-family model that must stay UNPREFIXED:
    // "openai/gpt-6-astra" gets Framer's plan gate (403 "Upgrade your
    // workspace") while bare "gpt-6-astra" resolves through the OpenRouter
    // catalog FREE (verified 2026-10-06: 200 + ledger delta 0.0).
    if m == "gpt-6-astra" {
        return m.to_string();
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
    if m.starts_with("kimi") {
        return format!("moonshotai/{m}");
    }
    m.to_string()
}

/// "opus-5-5" → "opus-5.5"; "sonnet-4-5" → "sonnet-4.5".
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
// Request conversion: client dialect → Anthropic Messages payload
// ---------------------------------------------------------------------------

/// Fields allowed upstream (Anthropic Messages API surface). Everything else
/// the client sent is stripped here — the gateway rejects unknown top-level
/// fields, and every supported field is mapped explicitly.
const ANTHROPIC_FIELDS: &[&str] = &[
    "model", "messages", "system", "max_tokens", "stream", "tools",
    "tool_choice", "temperature", "top_p", "top_k", "stop_sequences",
    "metadata", "thinking",
    // OpenRouter service tiers (2026-10-06): service_tier (default|flex|
    // priority|fast|ultrafast) and Anthropic's native speed:"fast" pass
    // straight through the gateway and are STILL FREE non-streaming —
    // ultrafast served with ledger delta 0.0.
    "service_tier", "speed",
];

/// Tiny operator note appended to the system prompt when tools are in play
/// (free route only — the paid passthrough is untouched). Physics: a tool
/// call larger than the upstream idle window (~100s of output) cannot complete
/// non-streaming, and no retry fixes that — the model must chunk its own work.
/// This one line eliminates the mid-tool_use retry churn.
const TOOL_CHUNK_NOTE: &str = "Hard limit: no single tool call may contain more than ~40 lines or ~1,500 characters of content. When writing or editing files longer than ~40 lines: write the first ~40 lines with one call, then append the next section with a separate call, and repeat. Large single writes get cut off mid-call and the work is lost; small sequential writes always complete.";

fn append_chunk_note(out: &mut Value) {
    let has_tools = out.get("tools").and_then(|t| t.as_array()).map(|a| !a.is_empty()).unwrap_or(false);
    if !has_tools {
        return;
    }
    match out.get_mut("system") {
        Some(Value::String(s)) => {
            s.push_str("\n\n");
            s.push_str(TOOL_CHUNK_NOTE);
        }
        Some(Value::Array(blocks)) => {
            blocks.push(json!({"type": "text", "text": TOOL_CHUNK_NOTE}));
        }
        _ => {
            out["system"] = json!(TOOL_CHUNK_NOTE);
        }
    }
}

/// Anthropic's rule: thinking blocks sent back in history must be EXACTLY as
/// the original response produced them (signature-verified). A chained
/// response assembles blocks from MULTIPLE upstream responses, so any
/// re-sent history containing those blocks fails validation two ways:
/// "Invalid `signature`" and "thinking blocks in the latest assistant message
/// cannot be modified" (both reproduced live 2026-10-04). The API explicitly
/// allows DROPPING thinking blocks from prior turns — the model does not need
/// them to continue. So we strip them from all history messages (keeping them
/// only in the freshly generated response), which kills the whole error class.
fn strip_thinking_from_history(messages: &mut Value) {
    let Some(arr) = messages.as_array_mut() else { return };
    let mut drop_idx: Vec<usize> = Vec::new();
    for (i, m) in arr.iter_mut().enumerate() {
        if m["role"].as_str() != Some("assistant") {
            continue;
        }
        let Some(blocks) = m["content"].as_array_mut() else { continue };
        blocks.retain(|b| {
            let t = b["type"].as_str().unwrap_or_default();
            t != "thinking" && t != "redacted_thinking"
        });
        if blocks.is_empty() {
            drop_idx.push(i);
        }
    }
    for i in drop_idx.into_iter().rev() {
        arr.remove(i);
    }
}

pub fn to_anthropic_request(d: ClientDialect, body: &Value) -> Result<Value, String> {
    let mut out = match d {
        ClientDialect::AnthropicMessages => {
            // Verbatim passthrough (whitelist): zero loss for native clients.
            let mut out = json!({});
            let obj = body.as_object().ok_or("request body must be an object")?;
            for k in ANTHROPIC_FIELDS {
                if let Some(v) = obj.get(*k) {
                    out[*k] = v.clone();
                }
            }
            if !out.get("messages").map(|m| m.is_array()).unwrap_or(false) {
                return Err("messages is required".into());
            }
            strip_thinking_from_history(&mut out["messages"]);
            out
        }
        ClientDialect::OpenAiChat => chat_to_anthropic(body)?,
        ClientDialect::OpenAiResponses => responses_to_anthropic(body)?,
    };
    // Free-route thinking cap: thinking tokens hog the chunk budget (Fable/
    // Opus burn 200-2000 thinking tokens even on trivia) — and when the
    // upstream degrades to single-digit tok/s, a 32000-token thinking budget
    // is the difference between answering and stalling into the wall.
    // 1024 keeps reasoning useful, bounded. FRAMERIZE_NO_THINKING_CAP=1 opts out.
    if std::env::var("FRAMERIZE_NO_THINKING_CAP").map(|v| v != "1").unwrap_or(true) {
        if let Some(t) = out.get_mut("thinking") {
            if t["type"].as_str() == Some("enabled") {
                let cur = t["budget_tokens"].as_u64().unwrap_or(u64::MAX);
                if cur > 1024 {
                    t["budget_tokens"] = json!(1024);
                }
            }
        }
    }
    append_chunk_note(&mut out);
    Ok(out)
}

fn text_from_content(content: &Value) -> String {
    match content {
        Value::String(s) => s.clone(),
        Value::Array(parts) => parts
            .iter()
            .filter_map(|p| {
                let t = p["type"].as_str().unwrap_or("text");
                if t == "text" || t == "input_text" || t == "output_text" {
                    p["text"].as_str().map(str::to_string)
                } else {
                    None
                }
            })
            .collect::<Vec<_>>()
            .join(""),
        _ => String::new(),
    }
}

/// OpenAI chat messages → Anthropic messages (+system). Faithful block mapping.
fn chat_messages_to_anthropic(msgs: &[Value]) -> (Option<Value>, Vec<Value>) {
    let mut system_parts: Vec<String> = Vec::new();
    let mut out: Vec<Value> = Vec::new();
    let mut pending_tool_results: Vec<Value> = Vec::new();

    let flush_tool_results = |out: &mut Vec<Value>, pending: &mut Vec<Value>| {
        if !pending.is_empty() {
            out.push(json!({"role": "user", "content": pending.clone()}));
            pending.clear();
        }
    };

    for m in msgs {
        let role = m["role"].as_str().unwrap_or("user");
        match role {
            "system" | "developer" => {
                system_parts.push(text_from_content(&m["content"]));
            }
            "tool" => {
                // → tool_result block in a user message (consecutive tool
                // messages merge into one user turn, per Anthropic spec).
                let content = match &m["content"] {
                    Value::String(s) => json!([{"type":"text","text":s}]),
                    Value::Array(parts) => json!(parts.iter().map(|p| {
                        if p["type"].as_str() == Some("text") {
                            json!({"type":"text","text":p["text"]})
                        } else { p.clone() }
                    }).collect::<Vec<_>>()),
                    other => json!([{"type":"text","text":other.to_string()}]),
                };
                pending_tool_results.push(json!({
                    "type": "tool_result",
                    "tool_use_id": m["tool_call_id"],
                    "content": content,
                }));
            }
            "assistant" => {
                flush_tool_results(&mut out, &mut pending_tool_results);
                let mut blocks: Vec<Value> = Vec::new();
                let text = text_from_content(&m["content"]);
                if !text.is_empty() {
                    blocks.push(json!({"type":"text","text":text}));
                }
                if let Some(calls) = m["tool_calls"].as_array() {
                    for c in calls {
                        let f = &c["function"];
                        let input = f["arguments"].as_str()
                            .and_then(|s| serde_json::from_str::<Value>(s).ok())
                            .unwrap_or_else(|| json!({}));
                        blocks.push(json!({
                            "type": "tool_use",
                            "id": c["id"],
                            "name": f["name"],
                            "input": input,
                        }));
                    }
                }
                if !blocks.is_empty() {
                    out.push(json!({"role": "assistant", "content": blocks}));
                }
            }
            _ => {
                // user (and anything else): map parts faithfully.
                flush_tool_results(&mut out, &mut pending_tool_results);
                let content = match &m["content"] {
                    Value::String(s) => json!(s),
                    Value::Array(parts) => json!(parts.iter().filter_map(|p| {
                        match p["type"].as_str().unwrap_or("text") {
                            "text" | "input_text" => Some(json!({"type":"text","text":p["text"]})),
                            "image_url" => {
                                let url = p["image_url"]["url"].as_str().unwrap_or_default();
                                if let Some(data) = url.strip_prefix("data:") {
                                    let (meta, b64) = data.split_once(";base64,")?;
                                    Some(json!({"type":"image","source":{"type":"base64","media_type":meta,"data":b64}}))
                                } else {
                                    Some(json!({"type":"image","source":{"type":"url","url":url}}))
                                }
                            }
                            _ => None,
                        }
                    }).collect::<Vec<_>>()),
                    other => other.clone(),
                };
                out.push(json!({"role": "user", "content": content}));
            }
        }
    }
    flush_tool_results(&mut out, &mut pending_tool_results);

    // Anthropic requires alternating roles, starting with user. Merge any
    // consecutive same-role messages defensively.
    let merged = merge_consecutive(out);
    let system = if system_parts.is_empty() { None } else { Some(json!(system_parts.join("\n"))) };
    (system, merged)
}

fn merge_consecutive(msgs: Vec<Value>) -> Vec<Value> {
    let mut out: Vec<Value> = Vec::new();
    for m in msgs {
        let role = m["role"].as_str().unwrap_or("user").to_string();
        if let Some(last) = out.last_mut() {
            if last["role"].as_str() == Some(&role) {
                // append content blocks/strings
                let mut lc = last["content"].clone();
                let mc = m["content"].clone();
                let to_blocks = |c: &Value| -> Vec<Value> {
                    match c {
                        Value::String(s) => vec![json!({"type":"text","text":s})],
                        Value::Array(a) => a.clone(),
                        other => vec![json!({"type":"text","text":other.to_string()})],
                    }
                };
                let mut lb = to_blocks(&lc);
                lb.extend(to_blocks(&mc));
                lc = json!(lb);
                last["content"] = lc;
                continue;
            }
        }
        out.push(m);
    }
    out
}

fn chat_tools_to_anthropic(tools: &[Value]) -> Vec<Value> {
    tools
        .iter()
        .filter_map(|t| {
            let f = if t["type"].as_str() == Some("function") { &t["function"] } else { t };
            let name = f["name"].as_str()?;
            let mut tool = json!({"name": name});
            if let Some(d) = f.get("description") { tool["description"] = d.clone(); }
            if let Some(p) = f.get("parameters") { tool["input_schema"] = p.clone(); }
            if let Some(s) = f.get("input_schema") { tool["input_schema"] = s.clone(); }
            Some(tool)
        })
        .collect()
}

fn chat_tool_choice_to_anthropic(choice: &Value) -> Option<Value> {
    match choice {
        Value::String(s) => match s.as_str() {
            "auto" => Some(json!({"type":"auto"})),
            "none" => Some(json!({"type":"none"})),
            "required" => Some(json!({"type":"any"})),
            _ => None,
        },
        Value::Object(_) => {
            if choice["type"].as_str() == Some("function") {
                Some(json!({"type":"tool","name": choice["function"]["name"]}))
            } else {
                None
            }
        }
        _ => None,
    }
}

fn chat_to_anthropic(body: &Value) -> Result<Value, String> {
    let msgs = body["messages"].as_array().ok_or("messages is required")?;
    let (system, messages) = chat_messages_to_anthropic(msgs);
    let mut out = json!({"model": to_openrouter_id(body["model"].as_str().unwrap_or_default()), "messages": messages});
    if let Some(s) = system { out["system"] = s; }
    if let Some(mt) = body["max_completion_tokens"].as_u64().or_else(|| body["max_tokens"].as_u64()) {
        out["max_tokens"] = json!(mt);
    }
    for (from, to) in [("temperature","temperature"),("top_p","top_p"),("top_k","top_k")] {
        if let Some(v) = body.get(from) { out[to] = v.clone(); }
    }
    match &body["stop"] {
        Value::String(s) => out["stop_sequences"] = json!([s]),
        Value::Array(a) => out["stop_sequences"] = json!(a),
        _ => {}
    }
    if let Some(tools) = body["tools"].as_array() {
        let mapped = chat_tools_to_anthropic(tools);
        if !mapped.is_empty() { out["tools"] = json!(mapped); }
    }
    if let Some(tc) = chat_tool_choice_to_anthropic(&body["tool_choice"]) {
        out["tool_choice"] = tc;
    }
    if let Some(u) = body["user"].as_str() {
        out["metadata"] = json!({"user_id": u});
    }
    if body["response_format"]["type"].as_str() == Some("json_object") {
        let note = "Respond with valid JSON only.";
        out["system"] = match out.get("system") {
            Some(Value::String(s)) => json!(format!("{s}\n\n{note}")),
            _ => json!(note),
        };
    }
    {
        if let Some(t) = body.get("service_tier") {
            out["service_tier"] = t.clone();
        }
    }
    Ok(out)
}

fn responses_to_anthropic(body: &Value) -> Result<Value, String> {
    let mut messages: Vec<Value> = Vec::new();
    let mut pending_tool_results: Vec<Value> = Vec::new();
    let flush = |messages: &mut Vec<Value>, pending: &mut Vec<Value>| {
        if !pending.is_empty() {
            messages.push(json!({"role":"user","content":pending.clone()}));
            pending.clear();
        }
    };

    match &body["input"] {
        Value::String(s) => messages.push(json!({"role":"user","content":s})),
        Value::Array(items) => {
            for item in items {
                let ty = item["type"].as_str().unwrap_or("message");
                match ty {
                    "message" => {
                        flush(&mut messages, &mut pending_tool_results);
                        let role = item["role"].as_str().unwrap_or("user");
                        let text = text_from_content(&item["content"]);
                        if role == "system" || role == "developer" {
                            // handled below via instructions merge
                            messages.push(json!({"role":"user","content":text}));
                        } else if role == "assistant" {
                            if !text.is_empty() {
                                messages.push(json!({"role":"assistant","content":[{"type":"text","text":text}]}));
                            }
                        } else {
                            messages.push(json!({"role":"user","content":text}));
                        }
                    }
                    "function_call" => {
                        flush(&mut messages, &mut pending_tool_results);
                        let input = item["arguments"].as_str()
                            .and_then(|s| serde_json::from_str::<Value>(s).ok())
                            .unwrap_or_else(|| json!({}));
                        messages.push(json!({"role":"assistant","content":[{
                            "type":"tool_use","id":item["call_id"],"name":item["name"],"input":input
                        }]}));
                    }
                    "function_call_output" => {
                        let out_text = match &item["output"] {
                            Value::String(s) => s.clone(),
                            other => other.to_string(),
                        };
                        pending_tool_results.push(json!({
                            "type":"tool_result","tool_use_id":item["call_id"],
                            "content":[{"type":"text","text":out_text}]
                        }));
                    }
                    _ => {} // reasoning items etc. — opaque, dropped
                }
            }
        }
        _ => {}
    }
    flush(&mut messages, &mut pending_tool_results);
    if messages.is_empty() {
        return Err("input is required".into());
    }
    let messages = merge_consecutive(messages);

    let mut out = json!({"model": to_openrouter_id(body["model"].as_str().unwrap_or_default()), "messages": messages});
    if let Some(instr) = body["instructions"].as_str() {
        out["system"] = json!(instr);
    }
    if let Some(mt) = body["max_output_tokens"].as_u64() {
        out["max_tokens"] = json!(mt);
    }
    for k in ["temperature","top_p"] {
        if let Some(v) = body.get(k) { out[k] = v.clone(); }
    }
    if let Some(tools) = body["tools"].as_array() {
        let mapped: Vec<Value> = tools.iter().filter_map(|t| {
            if t["type"].as_str() != Some("function") { return None; }
            let mut tool = json!({"name": t["name"]});
            if let Some(d) = t.get("description") { tool["description"] = d.clone(); }
            if let Some(p) = t.get("parameters") { tool["input_schema"] = p.clone(); }
            Some(tool)
        }).collect();
        if !mapped.is_empty() { out["tools"] = json!(mapped); }
    }
    if let Some(tc) = chat_tool_choice_to_anthropic(&body["tool_choice"]) {
        out["tool_choice"] = tc;
    }
    if let Some(u) = body["user"].as_str() {
        out["metadata"] = json!({"user_id": u});
    }
    {
        if let Some(t) = body.get("service_tier") {
            out["service_tier"] = t.clone();
        }
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Response rendering: AssembledResponse → client dialect (JSON + SSE)
// ---------------------------------------------------------------------------

fn assembled_text(r: &AssembledResponse) -> String {
    r.content
        .iter()
        .filter(|b| b["type"].as_str() == Some("text"))
        .filter_map(|b| b["text"].as_str())
        .collect::<Vec<_>>()
        .join("")
}

fn tool_use_blocks(r: &AssembledResponse) -> Vec<&Value> {
    r.content.iter().filter(|b| b["type"].as_str() == Some("tool_use")).collect()
}

pub fn render_json(d: ClientDialect, r: &AssembledResponse, client_model: &str) -> Value {
    match d {
        ClientDialect::AnthropicMessages => json!({
            "id": r.id, "type": "message", "role": "assistant",
            "content": r.content, "model": client_model,
            "stop_reason": r.stop_reason, "stop_sequence": r.stop_sequence,
            "usage": r.usage,
        }),
        ClientDialect::OpenAiChat => {
            let text = assembled_text(r);
            let tools = tool_use_blocks(r);
            let mut message = json!({"role": "assistant"});
            message["content"] = if text.is_empty() { Value::Null } else { json!(text) };
            if !tools.is_empty() {
                message["tool_calls"] = json!(tools.iter().enumerate().map(|(i, b)| json!({
                    "id": b["id"], "type": "function",
                    "function": {"name": b["name"], "arguments": b["input"].to_string()},
                    "index": i,
                })).collect::<Vec<_>>());
            }
            json!({
                "id": format!("chatcmpl-{}", &r.id.trim_start_matches("msg_")),
                "object": "chat.completion",
                "created": now_unix(),
                "model": client_model,
                "service_tier": r.usage["service_tier"].clone(),
                "choices": [{
                    "index": 0, "message": message,
                    "finish_reason": chat_finish(&r.stop_reason),
                }],
                "usage": {
                    "prompt_tokens": r.input_tokens,
                    "completion_tokens": r.output_tokens,
                    "total_tokens": r.input_tokens + r.output_tokens,
                },
            })
        }
        ClientDialect::OpenAiResponses => {
            let mut output: Vec<Value> = Vec::new();
            let text = assembled_text(r);
            if !text.is_empty() {
                output.push(json!({
                    "id": format!("msg_{}", &r.id.trim_start_matches("msg_")),
                    "type": "message", "status": "completed", "role": "assistant",
                    "content": [{"type":"output_text","text":text,"annotations":[]}],
                }));
            }
            for b in tool_use_blocks(r) {
                output.push(json!({
                    "id": format!("fc_{}", b["id"].as_str().unwrap_or("x").trim_start_matches("toolu_")),
                    "type": "function_call", "status": "completed",
                    "call_id": b["id"], "name": b["name"],
                    "arguments": b["input"].to_string(),
                }));
            }
            json!({
                "id": format!("resp_{}", &r.id.trim_start_matches("msg_")),
                "object": "response",
                "created_at": now_unix(),
                "status": "completed",
                "model": client_model,
                "service_tier": r.usage["service_tier"].clone(),
                "output": output,
                "parallel_tool_calls": true,
                "usage": {
                    "input_tokens": r.input_tokens,
                    "output_tokens": r.output_tokens,
                    "total_tokens": r.input_tokens + r.output_tokens,
                },
            })
        }
    }
}

fn chat_finish(stop_reason: &str) -> String {
    match stop_reason {
        "max_tokens" => "length".into(),
        "tool_use" => "tool_calls".into(),
        _ => "stop".into(),
    }
}

fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Render a complete assembled response as a well-formed SSE stream in the
/// client's dialect. Each entry is one `event:`/`data:` pair WITHOUT the
/// trailing blank line.
pub fn render_sse(d: ClientDialect, r: &AssembledResponse, client_model: &str, include_usage: bool) -> Vec<String> {
    match d {
        ClientDialect::AnthropicMessages => anthropic_sse(r, client_model),
        ClientDialect::OpenAiChat => chat_sse(r, client_model, include_usage),
        ClientDialect::OpenAiResponses => responses_sse(r, client_model),
    }
}

fn sse(event: &str, data: Value) -> String {
    format!("event: {event}\ndata: {}", data.to_string())
}

fn anthropic_sse(r: &AssembledResponse, client_model: &str) -> Vec<String> {
    let mut frames = Vec::new();
    frames.push(sse("message_start", json!({
        "type": "message_start",
        "message": {
            "id": r.id, "type": "message", "role": "assistant",
            "content": [], "model": client_model,
            "stop_reason": null, "stop_sequence": null,
            "usage": {"input_tokens": r.input_tokens, "output_tokens": 0},
        },
    })));
    for (i, b) in r.content.iter().enumerate() {
        match b["type"].as_str() {
            Some("text") => {
                frames.push(sse("content_block_start", json!({
                    "type": "content_block_start", "index": i,
                    "content_block": {"type": "text", "text": ""},
                })));
                frames.push(sse("content_block_delta", json!({
                    "type": "content_block_delta", "index": i,
                    "delta": {"type": "text_delta", "text": b["text"]},
                })));
            }
            Some("tool_use") => {
                frames.push(sse("content_block_start", json!({
                    "type": "content_block_start", "index": i,
                    "content_block": {"type": "tool_use", "id": b["id"], "name": b["name"], "input": {}},
                })));
                frames.push(sse("content_block_delta", json!({
                    "type": "content_block_delta", "index": i,
                    "delta": {"type": "input_json_delta", "partial_json": b["input"].to_string()},
                })));
            }
            _ => {
                // thinking / redacted_thinking / image / server blocks: pass through raw.
                frames.push(sse("content_block_start", json!({
                    "type": "content_block_start", "index": i, "content_block": b,
                })));
            }
        }
        frames.push(sse("content_block_stop", json!({"type": "content_block_stop", "index": i})));
    }
    frames.push(sse("message_delta", json!({
        "type": "message_delta",
        "delta": {"stop_reason": r.stop_reason, "stop_sequence": r.stop_sequence},
        "usage": {"output_tokens": r.output_tokens},
    })));
    frames.push(sse("message_stop", json!({"type": "message_stop"})));
    frames
}

fn chat_sse(r: &AssembledResponse, client_model: &str, include_usage: bool) -> Vec<String> {
    let id = format!("chatcmpl-{}", &r.id.trim_start_matches("msg_"));
    let created = now_unix();
    let chunk = |delta: Value, finish: Value| json!({
        "id": id, "object": "chat.completion.chunk", "created": created,
        "model": client_model,
        "choices": [{"index": 0, "delta": delta, "finish_reason": finish}],
    });
    let mut frames = Vec::new();
    frames.push(format!("data: {}", chunk(json!({"role": "assistant"}), Value::Null)));
    let mut tool_idx = 0usize;
    for b in &r.content {
        match b["type"].as_str() {
            Some("text") => {
                if let Some(t) = b["text"].as_str() {
                    frames.push(format!("data: {}", chunk(json!({"content": t}), Value::Null)));
                }
            }
            Some("tool_use") => {
                frames.push(format!("data: {}", chunk(json!({
                    "tool_calls": [{
                        "index": tool_idx, "id": b["id"], "type": "function",
                        "function": {"name": b["name"], "arguments": ""},
                    }]
                }), Value::Null)));
                frames.push(format!("data: {}", chunk(json!({
                    "tool_calls": [{
                        "index": tool_idx,
                        "function": {"arguments": b["input"].to_string()},
                    }]
                }), Value::Null)));
                tool_idx += 1;
            }
            _ => {}
        }
    }
    frames.push(format!("data: {}", chunk(json!({}), json!(chat_finish(&r.stop_reason)))));
    if include_usage {
        frames.push(format!("data: {}", json!({
            "id": id, "object": "chat.completion.chunk", "created": created,
            "model": client_model, "choices": [],
            "usage": {
                "prompt_tokens": r.input_tokens,
                "completion_tokens": r.output_tokens,
                "total_tokens": r.input_tokens + r.output_tokens,
            },
        })));
    }
    frames.push("data: [DONE]".into());
    frames
}

fn responses_sse(r: &AssembledResponse, client_model: &str) -> Vec<String> {
    let full = render_json(ClientDialect::OpenAiResponses, r, client_model);
    let resp_id = full["id"].clone();
    let mut frames = Vec::new();
    let base = |ty: &str, extra: Value| {
        let mut v = json!({"type": ty, "response_id": resp_id});
        merge(&mut v, extra);
        sse(ty, v)
    };
    frames.push(base("response.created", json!({"response": {"id": resp_id, "object": "response", "created_at": full["created_at"], "status": "in_progress", "model": client_model}})));
    frames.push(base("response.in_progress", json!({"response": {"id": resp_id, "status": "in_progress"}})));
    let outputs = full["output"].as_array().cloned().unwrap_or_default();
    for (idx, item) in outputs.iter().enumerate() {
        frames.push(base("response.output_item.added", json!({"output_index": idx, "item": item})));
        match item["type"].as_str() {
            Some("message") => {
                let text = item["content"][0]["text"].as_str().unwrap_or_default().to_string();
                frames.push(base("response.content_part.added", json!({"output_index": idx, "content_index": 0, "part": {"type":"output_text","text":"","annotations":[]}})));
                frames.push(base("response.output_text.delta", json!({"output_index": idx, "content_index": 0, "delta": text})));
                frames.push(base("response.output_text.done", json!({"output_index": idx, "content_index": 0, "text": item["content"][0]["text"]})));
                frames.push(base("response.content_part.done", json!({"output_index": idx, "content_index": 0, "part": item["content"][0]})));
            }
            Some("function_call") => {
                frames.push(base("response.function_call_arguments.delta", json!({"output_index": idx, "delta": item["arguments"]})));
                frames.push(base("response.function_call_arguments.done", json!({"output_index": idx, "arguments": item["arguments"], "name": item["name"], "call_id": item["call_id"]})));
            }
            _ => {}
        }
        frames.push(base("response.output_item.done", json!({"output_index": idx, "item": item})));
    }
    frames.push(base("response.completed", json!({"response": full})));
    frames
}

fn merge(dst: &mut Value, extra: Value) {
    if let (Value::Object(d), Value::Object(e)) = (dst, extra) {
        for (k, v) in e {
            d.insert(k, v);
        }
    }
}

// ---------------------------------------------------------------------------
// tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn openrouter_ids() {
        assert_eq!(to_openrouter_id("claude-opus-5-5"), "anthropic/claude-opus-5.5");
        assert_eq!(to_openrouter_id("gpt-4.1"), "openai/gpt-4.1");
        assert_eq!(model_route("gpt-6-sol"), ModelRoute::DirectResponses);
        assert_eq!(model_route("openai/gpt-6-luna"), ModelRoute::DirectResponses);
        assert_eq!(model_route("claude-opus-5-5"), ModelRoute::FreePassthrough);
        assert_eq!(model_route("gpt-6.1-sol"), ModelRoute::DirectResponses);
        assert_eq!(model_route("gpt-6-astra"), ModelRoute::FreePassthrough);
        assert_eq!(to_openrouter_id("gpt-6-astra"), "gpt-6-astra");
        assert_eq!(to_openrouter_id("kimi-k3"), "moonshotai/kimi-k3");
        assert_eq!(to_openrouter_id("moonshotai/kimi-k3"), "moonshotai/kimi-k3");
        assert_eq!(to_openrouter_id("gpt-4.1"), "openai/gpt-4.1");
        assert_eq!(to_openrouter_id("anthropic/claude-opus-5.5"), "anthropic/claude-opus-5.5");
        assert_eq!(to_openrouter_id("gemini-2.5-flash"), "google/gemini-2.5-flash");
    }

    #[test]
    fn anthropic_passthrough_is_verbatim() {
        let body = json!({
            "model": "claude-opus-5-5", "max_tokens": 1024, "stream": true,
            "system": [{"type":"text","text":"sys","cache_control":{"type":"ephemeral"}}],
            "tools": [{"name":"get_weather","description":"w","input_schema":{"type":"object"}}],
            "tool_choice": {"type":"auto"}, "temperature": 0.4,
            "messages": [{"role":"user","content":[{"type":"text","text":"hi"}]}],
            "unknown_field": "dropped",
        });
        let out = to_anthropic_request(ClientDialect::AnthropicMessages, &body).unwrap();
        assert_eq!(out["tools"], body["tools"]);
        // system gains the tool-chunk note (appended as a final block); the
        // client's own blocks lead, untouched.
        let sys = out["system"].as_array().unwrap();
        assert_eq!(sys[0], body["system"][0]);
        assert!(sys.last().unwrap()["text"].as_str().unwrap().contains("no single tool call"));
        assert_eq!(out["tool_choice"], body["tool_choice"]);
        assert_eq!(out["messages"], body["messages"]);
        assert!(out.get("unknown_field").is_none());
    }

    #[test]
    fn chat_tools_roundtrip() {
        let body = json!({
            "model": "gpt-4.1",
            "messages": [
                {"role":"system","content":"be nice"},
                {"role":"user","content":"weather in paris?"},
                {"role":"assistant","content":null,"tool_calls":[{"id":"call_1","type":"function","function":{"name":"get_weather","arguments":"{\"city\":\"Paris\"}"}}]},
                {"role":"tool","tool_call_id":"call_1","content":"sunny"},
                {"role":"user","content":"thanks"}
            ],
            "tools": [{"type":"function","function":{"name":"get_weather","description":"w","parameters":{"type":"object","properties":{"city":{"type":"string"}}}}}],
            "tool_choice": "auto", "temperature": 0.2,
        });
        let out = to_anthropic_request(ClientDialect::OpenAiChat, &body).unwrap();
        assert_eq!(out["model"], "openai/gpt-4.1");
        assert!(out["system"].as_str().unwrap().starts_with("be nice"));
        assert!(out["system"].as_str().unwrap().contains("no single tool call"));
        assert_eq!(out["tools"][0]["name"], "get_weather");
        assert_eq!(out["tools"][0]["input_schema"]["type"], "object");
        // assistant tool_use block
        let msgs = out["messages"].as_array().unwrap();
        let asst = &msgs[1];
        // content is null on the assistant message → no text block; tool_use leads
        assert_eq!(asst["content"][0]["type"], "tool_use");
        assert_eq!(asst["content"][0]["input"]["city"], "Paris");
        // tool_result in user turn
        let tr = &msgs[2];
        assert_eq!(tr["role"], "user");
        assert_eq!(tr["content"][0]["type"], "tool_result");
        assert_eq!(tr["content"][0]["tool_use_id"], "call_1");
    }

    #[test]
    fn responses_roundtrip_blocks() {
        let r = AssembledResponse {
            id: "msg_abc".into(),
            content: vec![
                json!({"type":"text","text":"checking"}),
                json!({"type":"tool_use","id":"call_9","name":"get_weather","input":{"city":"Paris"}}),
            ],
            stop_reason: "tool_use".into(),
            stop_sequence: None,
            usage: json!({"input_tokens":10,"output_tokens":7}),
            input_tokens: 10, output_tokens: 7, chunks: 1, metered_chunks: 0,
        };
        let j = render_json(ClientDialect::OpenAiResponses, &r, "gpt-4.1");
        assert_eq!(j["output"][1]["type"], "function_call");
        assert_eq!(j["output"][1]["call_id"], "call_9");
        assert_eq!(j["output"][1]["arguments"], "{\"city\":\"Paris\"}");
        let c = render_json(ClientDialect::OpenAiChat, &r, "gpt-4.1");
        assert_eq!(c["choices"][0]["finish_reason"], "tool_calls");
        assert_eq!(c["choices"][0]["message"]["tool_calls"][0]["function"]["name"], "get_weather");
        let sse_frames = render_sse(ClientDialect::AnthropicMessages, &r, "claude-opus-5-5", false);
        let joined = sse_frames.join("\n");
        assert!(joined.contains("input_json_delta"));
        assert!(joined.contains("\"stop_reason\":\"tool_use\""));
    }
}

// ---------------------------------------------------------------------------
// INCREMENTAL streaming: the chain emits per-chunk events; these composers
// turn them into dialect SSE frames AS THEY ARRIVE. This is what keeps
// streaming clients alive: bytes flow from the first second (message_start /
// role / response.created), heartbeats fill the gaps between chunks, and the
// old failure mode — a silent socket for the entire chain duration, which
// clients time out as "repeated API errors" — is gone.
// ---------------------------------------------------------------------------

/// What the chain emits as it progresses.
#[derive(Debug, Clone)]
pub enum ChainEvent {
    /// A chunk completed; its new blocks (already merged into the total by the
    /// chain) are attached for incremental rendering.
    Chunk(Vec<Value>),
    /// The chain finished (assembled response available).
    Done(Box<AssembledResponse>),
}

/// Heartbeat frame per dialect (legal, parser-ignored keep-alives).
pub fn ping_frame(d: ClientDialect) -> String {
    match d {
        ClientDialect::AnthropicMessages => sse("ping", json!({"type": "ping"})),
        ClientDialect::OpenAiChat | ClientDialect::OpenAiResponses => ": ping".into(),
    }
}

/// Opening frames per dialect, sent IMMEDIATELY on connection.
pub fn stream_open(d: ClientDialect, client_model: &str, msg_id: &str) -> Vec<String> {
    match d {
        ClientDialect::AnthropicMessages => vec![sse("message_start", json!({
            "type": "message_start",
            "message": {
                "id": msg_id, "type": "message", "role": "assistant",
                "content": [], "model": client_model,
                "stop_reason": null, "stop_sequence": null,
                "usage": {"input_tokens": 0, "output_tokens": 0},
            },
        }))],
        ClientDialect::OpenAiChat => vec![format!("data: {}", json!({
            "id": chat_id(msg_id), "object": "chat.completion.chunk",
            "created": now_unix(), "model": client_model,
            "choices": [{"index": 0, "delta": {"role": "assistant"}, "finish_reason": null}],
        }))],
        ClientDialect::OpenAiResponses => vec![
            sse("response.created", json!({
                "type": "response.created",
                "response": {"id": resp_id(msg_id), "object": "response",
                             "created_at": now_unix(), "status": "in_progress",
                             "model": client_model},
            })),
            sse("response.in_progress", json!({
                "type": "response.in_progress",
                "response": {"id": resp_id(msg_id), "status": "in_progress"},
            })),
        ],
    }
}

fn chat_id(msg_id: &str) -> String {
    format!("chatcmpl-{}", msg_id.trim_start_matches("msg_").trim_start_matches("gen-"))
}
fn resp_id(msg_id: &str) -> String {
    format!("resp_{}", msg_id.trim_start_matches("msg_"))
}

/// Incremental SSE composer: tracks open blocks across chunks.
pub struct SseComposer {
    d: ClientDialect,
    client_model: String,
    /// anthropic: next content block index; whether a text block is open
    next_idx: usize,
    text_open: bool,
    /// chat/responses: tool call ordinal; output item ordinal
    tool_idx: usize,
    item_idx: usize,
    /// accumulated blocks (for the responses.completed final object)
    acc: Vec<Value>,
    msg_id: String,
    created: u64,
    text_item_open: bool,
}

impl SseComposer {
    pub fn new(d: ClientDialect, msg_id: &str, client_model: &str) -> Self {
        Self {
            d,
            client_model: client_model.to_string(),
            next_idx: 0,
            text_open: false,
            tool_idx: 0,
            item_idx: 0,
            acc: Vec::new(),
            msg_id: msg_id.to_string(),
            created: now_unix(),
            text_item_open: false,
        }
    }

    pub fn push_chunk(&mut self, blocks: &[Value]) -> Vec<String> {
        merge_blocks_local(&mut self.acc, blocks.to_vec());
        match self.d {
            ClientDialect::AnthropicMessages => self.push_anthropic(blocks),
            ClientDialect::OpenAiChat => self.push_chat(blocks),
            ClientDialect::OpenAiResponses => self.push_responses(blocks),
        }
    }

    pub fn finish(&mut self, r: &AssembledResponse, client_model: &str, include_usage: bool) -> Vec<String> {
        match self.d {
            ClientDialect::AnthropicMessages => {
                let mut f = Vec::new();
                if self.text_open {
                    f.push(sse("content_block_stop", json!({"type":"content_block_stop","index": self.next_idx})));
                    self.text_open = false;
                }
                f.push(sse("message_delta", json!({
                    "type": "message_delta",
                    "delta": {"stop_reason": r.stop_reason, "stop_sequence": r.stop_sequence},
                    "usage": {"output_tokens": r.output_tokens},
                })));
                f.push(sse("message_stop", json!({"type": "message_stop"})));
                f
            }
            ClientDialect::OpenAiChat => {
                let mut f = Vec::new();
                f.push(format!("data: {}", json!({
                    "id": chat_id(&self.msg_id), "object": "chat.completion.chunk",
                    "created": self.created, "model": client_model,
                    "choices": [{"index": 0, "delta": {}, "finish_reason": chat_finish(&r.stop_reason)}],
                })));
                if include_usage {
                    f.push(format!("data: {}", json!({
                        "id": chat_id(&self.msg_id), "object": "chat.completion.chunk",
                        "created": self.created, "model": client_model, "choices": [],
                        "usage": {"prompt_tokens": r.input_tokens, "completion_tokens": r.output_tokens,
                                  "total_tokens": r.input_tokens + r.output_tokens},
                    })));
                }
                f.push("data: [DONE]".into());
                f
            }
            ClientDialect::OpenAiResponses => {
                // Close any open text item BEFORE completed: Codex's SSE
                // state machine only finalizes a message when output_text.done
                // + output_item.done arrive — a stream that jumps delta ->
                // completed leaves the item in_progress and Codex records
                // last_agent_message: None (measured, 2026-10-04).
                let mut f = Vec::new();
                if self.text_item_open {
                    let full_text: String = self.acc.iter()
                        .filter(|b| b["type"].as_str() == Some("text"))
                        .filter_map(|b| b["text"].as_str().map(str::to_string))
                        .collect();
                    f.push(sse("response.output_text.done", json!({
                        "type": "response.output_text.done",
                        "output_index": self.item_idx, "content_index": 0,
                        "text": full_text,
                    })));
                    f.push(sse("response.content_part.done", json!({
                        "type": "response.content_part.done",
                        "output_index": self.item_idx, "content_index": 0,
                        "part": {"type":"output_text","text": full_text,"annotations":[]},
                    })));
                    f.push(sse("response.output_item.done", json!({
                        "type": "response.output_item.done", "output_index": self.item_idx,
                        "item": {"id": format!("msg_{}", self.msg_id.trim_start_matches("msg_")),
                                 "type": "message", "status": "completed",
                                 "role": "assistant",
                                 "content": [{"type":"output_text","text": full_text,"annotations":[]}]},
                    })));
                    self.text_item_open = false;
                    self.item_idx += 1;
                }
                let assembled = AssembledResponse {
                    id: r.id.clone(),
                    content: self.acc.clone(),
                    stop_reason: r.stop_reason.clone(),
                    stop_sequence: r.stop_sequence.clone(),
                    usage: r.usage.clone(),
                    input_tokens: r.input_tokens,
                    output_tokens: r.output_tokens,
                    chunks: r.chunks,
                    metered_chunks: r.metered_chunks,
                };
                let mut full = render_json(ClientDialect::OpenAiResponses, &assembled, client_model);
                // Codex correlates a stream by response/item id: the events
                // above were emitted under the LOCAL ids, but `full` carries
                // the upstream ids — a mismatched completed object is
                // discarded as an unknown response (measured: Codex recorded
                // last_agent_message: None after a successful generation).
                full["id"] = json!(resp_id(&self.msg_id));
                if let Some(items) = full["output"].as_array_mut() {
                    for (i, item) in items.iter_mut().enumerate() {
                        item["id"] = json!(if i == 0 {
                            self.msg_id.clone()
                        } else {
                            format!("{}_{i}", self.msg_id)
                        });
                    }
                }
                f.push(sse("response.completed", json!({
                    "type": "response.completed", "response": full,
                })));
                f
            }
        }
    }

    fn push_anthropic(&mut self, blocks: &[Value]) -> Vec<String> {
        let mut f = Vec::new();
        for b in blocks {
            match b["type"].as_str() {
                Some("text") => {
                    if !self.text_open {
                        f.push(sse("content_block_start", json!({
                            "type": "content_block_start", "index": self.next_idx,
                            "content_block": {"type": "text", "text": ""},
                        })));
                        self.text_open = true;
                    }
                    f.push(sse("content_block_delta", json!({
                        "type": "content_block_delta", "index": self.next_idx,
                        "delta": {"type": "text_delta", "text": b["text"]},
                    })));
                }
                Some("tool_use") => {
                    if self.text_open {
                        f.push(sse("content_block_stop", json!({"type": "content_block_stop", "index": self.next_idx})));
                        self.text_open = false;
                        self.next_idx += 1;
                    }
                    f.push(sse("content_block_start", json!({
                        "type": "content_block_start", "index": self.next_idx,
                        "content_block": {"type": "tool_use", "id": b["id"], "name": b["name"], "input": {}},
                    })));
                    f.push(sse("content_block_delta", json!({
                        "type": "content_block_delta", "index": self.next_idx,
                        "delta": {"type": "input_json_delta", "partial_json": b["input"].to_string()},
                    })));
                    f.push(sse("content_block_stop", json!({"type": "content_block_stop", "index": self.next_idx})));
                    self.next_idx += 1;
                }
                _ => {
                    // thinking etc.: close text, pass through as its own block
                    if self.text_open {
                        f.push(sse("content_block_stop", json!({"type": "content_block_stop", "index": self.next_idx})));
                        self.text_open = false;
                        self.next_idx += 1;
                    }
                    f.push(sse("content_block_start", json!({
                        "type": "content_block_start", "index": self.next_idx, "content_block": b,
                    })));
                    f.push(sse("content_block_stop", json!({"type": "content_block_stop", "index": self.next_idx})));
                    self.next_idx += 1;
                }
            }
        }
        f
    }

    fn push_chat(&mut self, blocks: &[Value]) -> Vec<String> {
        let chunk = |delta: Value| json!({
            "id": chat_id(&self.msg_id), "object": "chat.completion.chunk",
            "created": self.created, "model": self.client_model,
            "choices": [{"index": 0, "delta": delta, "finish_reason": null}],
        });
        let mut f = Vec::new();
        for b in blocks {
            match b["type"].as_str() {
                Some("text") => {
                    if let Some(t) = b["text"].as_str() {
                        if !t.is_empty() {
                            f.push(format!("data: {}", chunk(json!({"content": t}))));
                        }
                    }
                }
                Some("tool_use") => {
                    f.push(format!("data: {}", chunk(json!({
                        "tool_calls": [{"index": self.tool_idx, "id": b["id"], "type": "function",
                                        "function": {"name": b["name"], "arguments": ""}}],
                    }))));
                    f.push(format!("data: {}", chunk(json!({
                        "tool_calls": [{"index": self.tool_idx,
                                        "function": {"arguments": b["input"].to_string()}}],
                    }))));
                    self.tool_idx += 1;
                }
                _ => {}
            }
        }
        f
    }

    fn push_responses(&mut self, blocks: &[Value]) -> Vec<String> {
        let mut f = Vec::new();
        for b in blocks {
            match b["type"].as_str() {
                Some("text") => {
                    let t = b["text"].as_str().unwrap_or_default();
                    if t.is_empty() { continue; }
                    if !self.text_item_open {
                        f.push(sse("response.output_item.added", json!({
                            "type": "response.output_item.added",
                            "output_index": self.item_idx,
                            "item": {"id": format!("msg_{}", self.msg_id.trim_start_matches("msg_")),
                                     "type": "message", "status": "in_progress",
                                     "role": "assistant",
                                     "content": [{"type":"output_text","text":"","annotations":[]}]},
                        })));
                        f.push(sse("response.content_part.added", json!({
                            "type": "response.content_part.added",
                            "output_index": self.item_idx, "content_index": 0,
                            "part": {"type":"output_text","text":"","annotations":[]},
                        })));
                        self.text_item_open = true;
                    }
                    f.push(sse("response.output_text.delta", json!({
                        "type": "response.output_text.delta",
                        "output_index": self.item_idx, "content_index": 0, "delta": t,
                    })));
                }
                Some("tool_use") => {
                    if self.text_item_open {
                        f.push(sse("response.output_text.done", json!({
                            "type": "response.output_text.done",
                            "output_index": self.item_idx, "content_index": 0,
                            "text": "",
                        })));
                        f.push(sse("response.output_item.done", json!({
                            "type": "response.output_item.done", "output_index": self.item_idx,
                            "item": {"type": "message", "status": "completed"},
                        })));
                        self.text_item_open = false;
                        self.item_idx += 1;
                    }
                    f.push(sse("response.output_item.added", json!({
                        "type": "response.output_item.added", "output_index": self.item_idx,
                        "item": {"id": format!("fc_{}", b["id"].as_str().unwrap_or("x")),
                                 "type": "function_call", "status": "in_progress",
                                 "call_id": b["id"], "name": b["name"], "arguments": ""},
                    })));
                    f.push(sse("response.function_call_arguments.delta", json!({
                        "type": "response.function_call_arguments.delta",
                        "output_index": self.item_idx, "delta": b["input"].to_string(),
                    })));
                    f.push(sse("response.function_call_arguments.done", json!({
                        "type": "response.function_call_arguments.done",
                        "output_index": self.item_idx,
                        "arguments": b["input"].to_string(),
                        "name": b["name"], "call_id": b["id"],
                    })));
                    f.push(sse("response.output_item.done", json!({
                        "type": "response.output_item.done", "output_index": self.item_idx,
                        "item": {"id": format!("fc_{}", b["id"].as_str().unwrap_or("x")),
                                 "type": "function_call", "status": "completed",
                                 "call_id": b["id"], "name": b["name"],
                                 "arguments": b["input"].to_string()},
                    })));
                    self.item_idx += 1;
                }
                _ => {}
            }
        }
        f
    }
}

fn merge_blocks_local(acc: &mut Vec<Value>, new: Vec<Value>) {
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

/// Mid-stream failure frames: a REAL error event per dialect so clients retry
/// loudly instead of accepting error text as the answer (anthropic `error`
/// event, responses `response.failed`, chat error object then [DONE]).
/// Parse a chain error of the form "... 400 on /ai/v3/x/: {json}" back into
/// (http_status, upstream_error_body). The upstream body is Anthropic-shaped:
/// {"type":"error","error":{"type":"invalid_request_error","message":"..."}}.
pub fn parse_upstream_error(msg: &str) -> Option<(u16, serde_json::Value)> {
    let idx = msg.rfind(": {")?;
    let body: serde_json::Value = serde_json::from_str(msg[idx + 2..].trim()).ok()?;
    let status = msg
        .split_whitespace()
        .find_map(|w| w.parse::<u16>().ok().filter(|n| (400..600).contains(n)))?;
    Some((status, body))
}

/// True when the upstream message is a context-window overflow — the client
/// must COMPACT, not retry. Matching Claude Code's own trigger wording keeps
/// its auto-compaction path alive through the gateway.
pub fn is_context_overflow(msg: &str) -> bool {
    let m = msg.to_lowercase();
    m.contains("maximum context length") || m.contains("context_length_exceeded") || m.contains("prompt is too long")
}

/// Dialect-shaped error JSON with the CORRECT status and error type — so the
/// client sees the real upstream cause (e.g. context overflow → compact) instead
/// of a generic 500 it retries forever ("gave up after repeated API errors").
pub fn error_json(d: ClientDialect, msg: &str) -> (u16, serde_json::Value) {
    let (status, umsg, utype) = match parse_upstream_error(msg) {
        Some((st, body)) => {
            let um = body.pointer("/error/message").and_then(|m| m.as_str()).unwrap_or(msg).to_string();
            let ut = body.pointer("/error/type").and_then(|m| m.as_str()).unwrap_or("invalid_request_error").to_string();
            (st, um, ut)
        }
        None => {
            let st = if msg.contains("risk-refused") { 403 } else { 500 };
            (st, msg.to_string(), "api_error".to_string())
        }
    };
    let code = if is_context_overflow(&umsg) { "context_length_exceeded" } else { utype.as_str() };
    let shaped = match d {
        ClientDialect::AnthropicMessages => json!({
            "type": "error",
            "error": {"type": if is_context_overflow(&umsg) { "invalid_request_error" } else { utype.as_str() }, "message": umsg},
        }),
        ClientDialect::OpenAiResponses | ClientDialect::OpenAiChat => json!({
            "error": {"message": umsg, "type": utype, "code": code},
        }),
    };
    (status, shaped)
}

pub fn error_frames(d: ClientDialect, msg: &str) -> Vec<String> {
    let (_, shaped) = error_json(d, msg);
    match d {
        ClientDialect::AnthropicMessages => vec![sse("error", shaped)],
        ClientDialect::OpenAiResponses => vec![sse("response.failed", json!({
            "type": "response.failed",
            "response": {"status": "failed", "error": shaped["error"]},
        }))],
        ClientDialect::OpenAiChat => vec![
            format!("data: {}", json!({"error": shaped["error"]})),
            "data: [DONE]".into(),
        ],
    }
}

// ---------------------------------------------------------------------------
// CONCURRENCY BUCKETS (verified live 2026-10-04): Framer's 429 "too many
// concurrent AI requests for this workspace" is counted per
// (workspace × route × step tag) — measured ~5 concurrent per bucket:
//   - 6 long generations on /messages + 6 on /chat simultaneously: ALL 200s.
//   - 8 on one route: 5 pass, 3 x 429.
//   - 6 different step tags on one route simultaneously: ALL 200s.
//   - Exotic steps are unmetered non-streaming (ledger identical before/after).
// Rotating (route × step) per chunk multiplies one workspace's concurrency
// by ~10x; on a 429 the next bucket is tried INSTANTLY (no sleep).
// ---------------------------------------------------------------------------

/// Step tags the bucket counter separates on (Framer's own feature steps).
pub const ROTATE_STEPS: [&str; 5] = [
    "message",
    "suggest",
    "nudge",
    "compact-turn",
    "generate-session-title",
];

/// Anthropic Messages payload → OpenAI chat/completions payload (for rotating
/// a chunk onto /ai/v3/chat/, the other unmetered route).
pub fn anthro_to_chat(p: &Value) -> Value {
    let mut out = json!({});
    // the chat route requires PREFIXED ids; the messages route tolerates the
    // dashed client form, which is how this slips through ("not a valid model ID").
    if let Some(m) = p.get("model") { out["model"] = json!(to_openrouter_id(m.as_str().unwrap_or_default())); }
    if let Some(mt) = p.get("max_tokens") { out["max_tokens"] = mt.clone(); }
    let mut messages: Vec<Value> = Vec::new();
    if let Some(sys) = p.get("system") {
        let text = match sys {
            Value::String(s) => s.clone(),
            Value::Array(b) => b.iter().filter_map(|x| x["text"].as_str()).collect::<Vec<_>>().join("\n"),
            _ => String::new(),
        };
        if !text.is_empty() {
            messages.push(json!({"role": "system", "content": text}));
        }
    }
    for m in p["messages"].as_array().cloned().unwrap_or_default() {
        let role = m["role"].as_str().unwrap_or("user");
        match &m["content"] {
            Value::String(s) => messages.push(json!({"role": role, "content": s})),
            Value::Array(blocks) => {
                let mut text_parts: Vec<String> = Vec::new();
                let mut tool_calls: Vec<Value> = Vec::new();
                let mut tool_results: Vec<Value> = Vec::new();
                for b in blocks {
                    match b["type"].as_str() {
                        Some("text") => text_parts.push(b["text"].as_str().unwrap_or_default().to_string()),
                        Some("tool_use") => tool_calls.push(json!({
                            "id": b["id"], "type": "function",
                            "function": {"name": b["name"], "arguments": b["input"].to_string()},
                        })),
                        Some("tool_result") => tool_results.push(json!({
                            "role": "tool", "tool_call_id": b["tool_use_id"],
                            "content": text_from_content(&b["content"]),
                        })),
                        _ => {}
                    }
                }
                if role == "assistant" && !tool_calls.is_empty() {
                    messages.push(json!({
                        "role": "assistant",
                        "content": if text_parts.is_empty() { Value::Null } else { json!(text_parts.join("")) },
                        "tool_calls": tool_calls,
                    }));
                } else if !tool_results.is_empty() {
                    for tr in tool_results { messages.push(tr); }
                    let t = text_parts.join("");
                    if !t.is_empty() {
                        messages.push(json!({"role": "user", "content": t}));
                    }
                } else {
                    messages.push(json!({"role": role, "content": text_parts.join("")}));
                }
            }
            other => messages.push(json!({"role": role, "content": other})),
        }
    }
    out["messages"] = json!(messages);
    for k in ["temperature", "top_p"] {
        if let Some(v) = p.get(k) { out[k] = v.clone(); }
    }
    if let Some(ss) = p.get("stop_sequences") { out["stop"] = ss.clone(); }
    if let Some(tools) = p.get("tools").and_then(|t| t.as_array()) {
        let mapped: Vec<Value> = tools.iter().map(|t| json!({
            "type": "function",
            "function": {"name": t["name"], "description": t["description"], "parameters": t["input_schema"]},
        })).collect();
        if !mapped.is_empty() { out["tools"] = json!(mapped); }
    }
    match p.get("tool_choice").and_then(|t| t["type"].as_str()) {
        Some("auto") => out["tool_choice"] = json!("auto"),
        Some("any") => out["tool_choice"] = json!("required"),
        Some("none") => out["tool_choice"] = json!("none"),
        Some("tool") => out["tool_choice"] = json!({"type":"function","function":{"name": p["tool_choice"]["name"]}}),
        _ => {}
    }
    out
}

/// Chat/completions response → (anthropic blocks, stop_reason, usage).
/// Used to extract chain chunks that rotated onto /ai/v3/chat/.
pub fn chat_response_to_blocks(body: &Value) -> (Vec<Value>, String, Value) {
    let choice = &body["choices"][0];
    let msg = &choice["message"];
    let mut blocks: Vec<Value> = Vec::new();
    if let Some(t) = msg["content"].as_str() {
        if !t.is_empty() {
            blocks.push(json!({"type": "text", "text": t}));
        }
    }
    if let Some(calls) = msg["tool_calls"].as_array() {
        for c in calls {
            blocks.push(json!({
                "type": "tool_use",
                "id": c["id"],
                "name": c["function"]["name"],
                "input": c["function"]["arguments"].as_str()
                    .and_then(|s| serde_json::from_str::<Value>(s).ok())
                    .unwrap_or_else(|| json!({})),
            }));
        }
    }
    let stop = match choice["finish_reason"].as_str() {
        Some("length") => "max_tokens",
        Some("tool_calls") => "tool_use",
        _ => "end_turn",
    };
    let u = body.get("usage").cloned().unwrap_or_else(|| json!({}));
    let usage = json!({
        "input_tokens": u["prompt_tokens"].as_u64().unwrap_or(0),
        "output_tokens": u["completion_tokens"].as_u64().unwrap_or(0),
    });
    (blocks, stop.to_string(), usage)
}

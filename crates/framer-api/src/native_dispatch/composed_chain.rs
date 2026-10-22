use super::{delivery::{Control, lock}, render::Renderer};
use super::super::{NativeCall, Parts, raw_stream};
use crate::proxy::Dialect;
use framer_core::{convert::ClientDialect, native_dispatch::{self as port,
    ComposedStreamHead, ComposedStreamOptions, DispatchError, FailureKind, Preparation,
    RawStreamLimits, Route, UpstreamFailure, stream_state::{StateBudget, value_bytes}}};
use futures::StreamExt;
use serde_json::{json, Value};
use std::sync::Arc;
use tokio::sync::{oneshot, Semaphore};
use tokio::time::Instant;

type HeadSender = Option<oneshot::Sender<Result<ComposedStreamHead, DispatchError>>>;

fn reserve(budget: &mut StateBudget, values: &[&Value], factor: usize)
    -> Result<(), FailureKind> {
    let mut bytes = 8192usize;
    for value in values {
        bytes = value_bytes(value)?.checked_mul(factor)
            .and_then(|n| bytes.checked_add(n)).ok_or(FailureKind::BodyTooLarge)?;
    }
    budget.charge(bytes)
}

pub(super) async fn produce(parts: Parts, payload: Value, options: ComposedStreamOptions,
    client_model: Option<String>, mut budget: StateBudget, control: Arc<Control>,
    head: &mut HeadSender, mut hooks: super::Hooks)
    -> Result<(), DispatchError> {
    let messages = parts.request.route == Route::Messages;
    if !payload["model"].is_string()
        || (messages && !payload["messages"].is_array())
        || (!messages && !(payload["input"].is_array() || payload["input"].is_string()))
        || (!messages && (parts.policy.chunk_tokens < 16 || parts.request.max_total_tokens < 16)) {
        return Err(control.error(FailureKind::InvalidRequest));
    }
    reserve(&mut budget, &[&payload], 4).map_err(|k| control.error(k))?;
    let mut history = if messages { payload["messages"].clone() }
        else { Value::Array(crate::nativechain::prepare_input(&payload)) };
    let mut accumulated = Vec::<Value>::new();
    let mut renderer = None;
    let (mut input_tokens, mut output_tokens) = (0u64, 0u64);
    let fast = !messages && matches!(payload["service_tier"].as_str(),
        Some("ultrafast") | Some("priority") | Some("fast"));
    let grow_cap = if fast { 12000 } else { 3000 };
    let chunk_cap = parts.policy.chunk_tokens.max(if fast { 4000 } else { 2500 });
    let mut chunk_tokens = if fast { parts.policy.chunk_tokens.clamp(1500, grow_cap) }
        else { parts.policy.chunk_tokens.min(600) };
    let mut final_body = Value::Null;
    for step in 0..parts.policy.max_chunks {
        control.check().map_err(|k| control.error(k))?;
        let remaining = (parts.request.max_total_tokens as u64).saturating_sub(output_tokens);
        let mut chunk = remaining.min(chunk_tokens as u64) as u32;
        if !messages { chunk = chunk.min(chunk_cap); }
        reserve(&mut budget, &[&payload, &history], 8).map_err(|k| control.error(k))?;
        let prepared = if messages {
            crate::freechat::prepare_payload(&payload, &history, chunk, Dialect::OpenRouterMessages)
        } else {
            crate::nativechain::prepare_payload(&payload, history.as_array().unwrap(), chunk)
        };
        let body = attempt(&parts, prepared, step, &control, head, &mut budget, &mut hooks).await?;
        let field = if messages { "content" } else { "output" };
        if !body.is_object() || !body["id"].is_string() || !body[field].is_array()
            || body.get("usage").is_some_and(|u| !u.is_object())
            || (messages && !body["stop_reason"].is_string())
            || (!messages && !matches!(body["status"].as_str(), Some("completed") | Some("incomplete"))) {
            return Err(control.error(FailureKind::UnsupportedResponse));
        }
        let items = body[field].as_array().unwrap();
        if items.iter().any(|item| !item.is_object()) {
            return Err(control.error(FailureKind::UnsupportedResponse));
        }
        let response_id = body["id"].as_str().unwrap();
        budget.charge(response_id.len()).map_err(|kind| control.error(kind))?;
        if let Some(last) = lock(&control.state).attempts.last_mut() {
            last.response_id = Some(Arc::from(response_id));
        }
        let tokens = |key: &str| -> Result<u64, DispatchError> {
            match body["usage"].get(key) {
                None => Ok(0),
                Some(v) => v.as_u64().ok_or_else(|| control.error(FailureKind::UnsupportedResponse)),
            }
        };
        input_tokens = input_tokens.max(tokens("input_tokens")?);
        let produced_tokens = tokens("output_tokens")?;
        output_tokens = output_tokens.checked_add(produced_tokens)
            .ok_or_else(|| control.error(FailureKind::UnsupportedResponse))?;
        if let Some(usage) = body.get("usage") {
            let usage = budget.copy(usage).map_err(|k| control.error(k))?;
            if let Some(last) = lock(&control.state).attempts.last_mut() {
                last.usage = Some(Arc::new(usage));
            }
        }
        // Upstream remains complete JSON. Do not turn a possibly truncated
        // tool input into executable downstream deltas or a success terminal.
        if !messages && crate::nativechain::responses_tool_output_incomplete(&body, chunk.max(16)) {
            return Err(control.error(FailureKind::UnsupportedResponse));
        }
        if renderer.is_none() {
            renderer = Some(Renderer::new(options, &payload, &body, client_model.as_deref(),
                &mut budget, control.clone())
                .map_err(|k| control.error(k))?);
            renderer.as_mut().unwrap().open(&body).await.map_err(|k| control.error(k))?;
        }
        renderer.as_mut().unwrap().chunk(items).await.map_err(|k| control.error(k))?;
        if options.dialect == ClientDialect::OpenAiResponses {
            let slots = accumulated.len().checked_add(items.len())
                .and_then(|n| n.checked_mul(2 * std::mem::size_of::<Value>()))
                .ok_or_else(|| control.error(FailureKind::BodyTooLarge))?;
            budget.charge(slots).map_err(|k| control.error(k))?;
            accumulated.reserve_exact(items.len());
            for item in items {
                accumulated.push(budget.copy(item).map_err(|k| control.error(k))?);
            }
        }
        let continuation = parts.policy.continuation.as_deref().unwrap_or("");
        let remaining = (parts.request.max_total_tokens as u64).saturating_sub(output_tokens);
        let continues = !continuation.is_empty() && step + 1 < parts.policy.max_chunks && if messages {
            body["stop_reason"].as_str() == Some("max_tokens") && remaining > 0
                && !items.is_empty() && items.iter().all(|v| v["type"].as_str() == Some("text"))
        } else {
            body["status"].as_str() == Some("incomplete") && remaining >= 16
                && body["incomplete_details"]["reason"].as_str() == Some("max_output_tokens")
                && items.last().is_some_and(|v| v["type"].as_str() == Some("message"))
                && items.iter().all(|v| matches!(v["type"].as_str(), Some("message") | Some("reasoning")))
        };
        if !continues { final_body = body; break; }
        if options.dialect == ClientDialect::OpenAiChat {
            // A second Responses turn has a distinct wire identity and item
            // boundary that the documented Chat chunk schema cannot carry.
            return Err(control.error(FailureKind::UnsupportedResponse));
        }
        reserve(&mut budget, &[&history, &body[field]], 4).map_err(|k| control.error(k))?;
        budget.charge(continuation.len().checked_mul(4).and_then(|n| n.checked_add(16384))
            .ok_or_else(|| control.error(FailureKind::BodyTooLarge))?).map_err(|k| control.error(k))?;
        let array = history.as_array_mut().unwrap();
        if messages {
            array.push(json!({"role":"assistant", "content":body[field]}));
            array.push(json!({"role":"user", "content":continuation}));
        } else {
            array.extend(items.iter().cloned());
            array.push(json!({"type":"message", "role":"user",
                "content":[{"type":"input_text", "text":continuation}]}));
        }
        let elapsed = lock(&control.state).attempts.last().map_or(0.5, |a| a.elapsed.as_secs_f32().max(0.5));
        let tps = produced_tokens as f32 / elapsed;
        if tps > 1.0 { chunk_tokens = ((tps * 45.0) as u32).clamp(400, grow_cap); }
    }
    let renderer = renderer.ok_or_else(|| control.error(FailureKind::Internal))?;
    renderer.finish(&final_body, &accumulated, input_tokens, output_tokens)
        .await.map_err(|k| control.error(k))?;
    Ok(())
}

async fn attempt(parts: &Parts, payload: Value, step: u32,
    control: &Arc<Control>, head: &mut HeadSender, budget: &mut StateBudget, _hooks: &mut super::Hooks)
    -> Result<Value, DispatchError> {
    let started = Instant::now();
    let dialect = if parts.request.route == Route::Messages {
        Dialect::OpenRouterMessages
    } else { Dialect::OpenRouterResponses };
    let internal = Arc::new(Semaphore::new(1));
    let permit = internal.try_acquire_owned().map_err(|_| control.error(FailureKind::Internal))?;
    let call = Box::new(NativeCall {
        api: parts.api.clone(), native: parts.native.clone(), scope: parts.scope.clone(),
        authorization: parts.authorization.clone(), policy: parts.policy.clone(),
        account_lease: super::super::shared_lease::AccountLease::Inherited,
        request: Preparation { identity: parts.request.identity.clone(),
            route: Route::Raw(ClientDialect::OpenAiResponses),
            deadline: parts.request.deadline, max_total_tokens: parts.request.max_total_tokens }, permit,
    });
    let limits = control.limits;
    let raw_limits = RawStreamLimits {
        chunk_bytes: limits.source_chunk_bytes, retained_bytes: limits.source_chunk_bytes,
        total_bytes: limits.json_bytes,
        header_bytes: control.remaining_headers().map_err(|kind| control.error(kind))?,
    };
    #[cfg(test)]
    let raw = match _hooks.start.take() {
        Some(start) => raw_stream::launch_prepared_for_test(call, payload, raw_limits, dialect, step, start),
        None => raw_stream::launch_prepared(call, payload, raw_limits, dialect, step),
    };
    #[cfg(not(test))]
    let raw = raw_stream::launch_prepared(call, payload, raw_limits, dialect, step);
    control.register(raw.response.cleanup.clone(), raw.response.cancellation.clone())
        .map_err(|kind| control.error(kind))?;
    let response = raw.response.result.await?;
    control.check().map_err(|kind| control.error(kind))?;
    control.record(response.headers.clone(), step).map_err(|kind| control.error(kind))?;
    if (200..300).contains(&response.headers.status) {
        if let Some(sender) = head.take() {
            sender.send(Ok(control.head(response.headers.clone())))
                .map_err(|_| control.error(FailureKind::Cancelled))?;
        }
    }
    let mut stream = response.body;
    let mut bytes = control.json_allocation().await.map_err(|kind| control.error(kind))?;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|kind| control.error(kind))?;
        let start = bytes.len;
        let end = start.checked_add(chunk.len()).filter(|n| *n <= bytes.data.len())
            .ok_or_else(|| control.error(FailureKind::BodyTooLarge))?;
        bytes.data[start..end].copy_from_slice(&chunk);
        bytes.len = end;
    }
    raw.response.cleanup.await.map_err(|kind| control.error(kind))?;
    let elapsed = started.elapsed();
    if let Some(last) = lock(&control.state).attempts.last_mut() { last.elapsed = elapsed; }
    // Match the existing native chain's completed-attempt observation. The
    // transferred string copies were reserved by launch's header allowance.
    parts.authorization.observe_finish(&crate::experimental_native::application::AttemptMetadata {
        status: response.headers.status,
        usage_header: response.headers.usage_header.as_ref().map(|s| s.to_string()),
        original_content_type: response.headers.original_content_type.as_ref().map(|s| s.to_string()),
        step_index: step, elapsed,
    });
    if !(200..300).contains(&response.headers.status) {
        let mut error = control.error(FailureKind::Upstream);
        let metadata = error.attempts.last().cloned().ok_or_else(|| control.error(FailureKind::Internal))?;
        error.upstream = Some(UpstreamFailure { body: bytes::Bytes::from_owner(bytes), metadata });
        return Err(error);
    }
    let value = budget.parse(bytes.as_ref()).map_err(|kind| control.error(kind))?;
    if matches!(value["status"].as_str(), Some("failed") | Some("cancelled"))
        || value.get("error").is_some_and(|e| !e.is_null()) {
        return Err(control.error(FailureKind::TerminalFailure));
    }
    Ok(value)
}

//! Messages SSE with one request owner across admission and generation.
use super::*;
use axum::body::{Body, Bytes};
use futures::StreamExt;
use std::{sync::atomic::{AtomicBool, Ordering}, time::Duration};

struct Attempt {
    account: ManagedAccount,
    session: framer_core::port::Session,
    scope: framer_core::account::WorkspaceScope,
    permit: tokio::sync::OwnedSemaphorePermit,
    wait_budget: Duration,
}

pub(super) async fn serve(state: AppState, payload: Value, model: String, max_tokens: u32,
    requested: Option<String>, explicit_session: Option<String>)
    -> Result<axum::response::Response, ApiError> {
    preflight(&state, requested.as_deref()).await
        .map_err(operation_ownership::completed_preflight)?;
    let output_lease = state.response_output.reserve_request()
        .map_err(operation_ownership::completed_preflight)?;
    let producer_lease = output_lease.clone();
    let residency = RequestResidency::empty();
    let body_residency = residency.clone();
    let (tx, rx) = tokio::sync::mpsc::channel(32);
    let overflow = Arc::new(AtomicBool::new(false));
    let failed = overflow.clone();
    let disconnected = tx.clone();
    let cancelled: Arc<dyn Fn() -> bool + Send + Sync> =
        Arc::new(move || disconnected.is_closed() || failed.load(Ordering::SeqCst));
    let session_header = explicit_session.clone();
    let scope = state.operation_ownership.clone();
    let (handle, completion) = operation_ownership::spawn_observed(scope.as_ref(),
        framer_core::operation_gate::OperationKind::Inference, async move {
            let _output_lease = producer_lease;
            let started = std::time::Instant::now();
            let log_model = payload["model"].as_str().unwrap_or("unknown").to_owned();
            let (admitted, cleanup) = admit(&state, requested.as_deref(),
                &log_model, &residency, cancelled.as_ref()).await;
            let attempt = match admitted {
                Ok(value) => value,
                Err(error) => {
                    operation_ownership::correlate_chain(framer_core::observability::Event::new("ai.chain"))
                        .field("model", log_model.as_str()).field("dialect", "AnthropicMessages")
                        .duration_ms(started.elapsed().as_millis()).error(&error).emit();
                    return (Err(error), cleanup);
                }
            };
            let chat = explicit_session.unwrap_or_else(|| stable_session(&attempt.account.id));
            let rotation = state.serving_accounts.rotation().clone();
            let team = attempt.scope.team.0.clone();
            let config = framer_core::port::ChainConfig {
                max_total_tokens: Some(max_tokens), metered_fallback: false,
                cancelled: Some(cancelled),
                admission: Some(Arc::new(account_wait::MessagesAdmission::new(
                    state.clone(), attempt.account.clone(), attempt.wait_budget))),
                on_429: Some(Arc::new(move || rotation.mark_saturated(&team, 45_000))),
                ..Default::default()
            };
            let callback = |event| {
                if matches!(tx.try_send(event), Err(tokio::sync::mpsc::error::TrySendError::Full(_))) {
                    overflow.store(true, Ordering::SeqCst);
                }
            };
            let result = state.client.freechat_long(&attempt.session, state.browser.as_ref(),
                &attempt.account.id, &attempt.scope.project, payload, &config, &chat, Some(&callback)).await;
            drop(attempt.permit);
            let result = residency.observe_result(attempt.account.id, result);
            let cleanup = cleanup && !matches!(&result,
                Err(error) if pool_control::cleanup_unconfirmed(error));
            config.admission.as_ref().unwrap().finished(result.as_ref().err().map(String::as_str)).await;
            let result = if overflow.load(Ordering::SeqCst) && cleanup {
                Err(format!("{}Messages output queue filled before the client consumed its content. Automatic replay is disabled because delivery is incomplete.",
                    framer_core::error::ADAPTIVE_INCOMPLETE_PREFIX))
            } else { result };
            let event = operation_ownership::correlate_chain(framer_core::observability::Event::new("ai.chain"))
                .account(&attempt.account.id).field("model", log_model.as_str()).field("dialect", "AnthropicMessages")
                .duration_ms(started.elapsed().as_millis());
            match &result {
                Ok(value) => event.field("chunks", value.chunks).field("metered_chunks", value.metered_chunks)
                    .field("prompt_tokens", value.input_tokens).field("completion_tokens", value.output_tokens)
                    .field("finish_reason", value.stop_reason.as_str()).outcome("ok").emit(),
                Err(error) => event.error(error).emit(),
            }
            (result, cleanup)
        }).map_err(ApiError::operation_admission)
        .map_err(operation_ownership::completed_preflight)?;
    let buffer_lease = output_lease.clone();
    let stream = render(rx, handle, model)
        .map(move |result| result.map(|bytes| buffer_lease.retain_bytes(bytes)));
    let mut response = axum::response::Response::new(Body::from_stream(
        retain_owner(stream, (body_residency, output_lease))));
    response.extensions_mut().insert(completion);
    response.headers_mut().insert("content-type", "text/event-stream".parse().unwrap());
    response.headers_mut().insert("cache-control", "no-cache".parse().unwrap());
    if let Some(session) = session_header {
        if let Ok(value) = session.parse() { response.headers_mut().insert("x-framerize-session", value); }
    }
    Ok(response)
}

async fn preflight(state: &AppState, requested: Option<&str>) -> Result<(), ApiError> {
    let pool = state.state.list().await.map_err(ApiError::Internal)?;
    if requested.is_none() && pool.iter().any(|account|
        state.serving_accounts.allows(&account.id) && account.is_eligible()) { return Ok(()); }
    match state.serving_accounts.select(pool, requested) {
        Ok(_) | Err(ApiError::Cooling { .. } | ApiError::AccountCooling { .. }) => Ok(()),
        Err(ApiError::Internal(message)) if message == "unknown account id" =>
            Err(ApiError::InvalidRequest(message)),
        Err(error) => Err(error),
    }
}

fn render(rx: tokio::sync::mpsc::Receiver<convert::ChainEvent>,
    handle: operation_ownership::ObservedJoin<convert::AssembledResponse>, model: String)
    -> impl futures::Stream<Item = Result<Bytes, std::convert::Infallible>> {
    let dialect = ClientDialect::AnthropicMessages;
    let id = format!("msg_{}", uuid::Uuid::new_v4().simple());
    let pending = convert::stream_open(dialect, &model, &id);
    let composer = convert::SseComposer::new(dialect, &id, &model);
    futures::stream::unfold((rx, handle, composer, model, pending, false),
        move |(mut rx, mut handle, mut composer, model, mut pending, mut done)| async move {
            loop {
                if !pending.is_empty() {
                    let bytes = Bytes::from(pending.join("\n\n") + "\n\n");
                    pending.clear();
                    return Some((Ok(bytes), (rx, handle, composer, model, pending, done)));
                }
                if done { return None; }
                pending = tokio::select! {
                    biased;
                    result = &mut handle => {
                        done = true;
                        let mut frames = Vec::new();
                        while let Ok(event) = rx.try_recv() {
                            if let convert::ChainEvent::Chunk(blocks) = event {
                                frames.extend(composer.push_chunk(&blocks));
                            }
                        }
                        frames.extend(match result {
                            Ok(Ok(value)) => composer.finish(&value, &model, false),
                            Ok(Err(error)) => convert::error_frames_with_replay(dialect, &error,
                                !composer.has_content()),
                            Err(_) => convert::error_frames_with_replay(dialect,
                                "request task failed before completion", false),
                        });
                        frames
                    }

                    event = rx.recv(), if !rx.is_closed() || !rx.is_empty() => match event {
                        Some(convert::ChainEvent::Chunk(blocks)) => composer.push_chunk(&blocks),
                        _ => vec![convert::ping_frame(dialect)],
                    },
                    _ = tokio::time::sleep(Duration::from_secs(8)) => vec![convert::ping_frame(dialect)],
                };
            }
        })
}

#[cfg(test)]
#[path = "messages_output_tests.rs"]
mod tests;

async fn admit(state: &AppState, requested: Option<&str>, model: &str,
    residency: &RequestResidency, cancelled: &(dyn Fn() -> bool + Send + Sync))
    -> (Result<Attempt, String>, bool) {
    let mut budget = account_wait::MAX_WAIT;
    let mut cleanup = true;
    loop {
        if cancelled() { return (Err("client cancelled".into()), cleanup); }
        let result = async {
            let pool = state.state.list().await.map_err(ApiError::Internal)?;
            let acquired = pool_control::select_account_observed(state, pool, requested, &[]).await;
            cleanup &= acquired.cleanup_confirmed;
            let lease = acquired.result?;
            let id = lease.account();
            residency.replace(lease);
            let account = current_account(state, id).await?;
            let session = state.sessions.load(&account.framer_user_id).await
                .map_err(ApiError::Internal)?
                .ok_or_else(|| ApiError::Internal("account has no session".into()))?;
            let scope = pick_scope(state, &account)?;
            let key = format!("{}:{model}", scope.team.0);
            let acquire = state.gate.acquire(&key);
            tokio::pin!(acquire);
            let permit = loop {
                if cancelled() { return Err(ApiError::Internal("client cancelled".into())); }
                tokio::select! {
                    permit = &mut acquire => break permit,
                    _ = tokio::time::sleep(Duration::from_millis(100)) => {},
                }
            };
            current_account(state, account.id).await?;
            Ok(Attempt { account, session, scope, permit, wait_budget: budget })
        }.await;
        let error = match result { Ok(attempt) => return (Ok(attempt), cleanup), Err(error) => error };
        drop(residency.take());
        let waitable = matches!(error, ApiError::Cooling { .. } | ApiError::AccountCooling { .. });
        if !cleanup || !waitable || budget.is_zero() {
            return (Err(chain_admission_error(error)), cleanup);
        }
        let started = tokio::time::Instant::now();
        tokio::time::sleep(budget.min(Duration::from_secs(1))).await;
        budget = budget.saturating_sub(started.elapsed());
    }
}

//! Fixture-injected native dispatch, selected before all legacy forwarding.
use super::AppState;
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::Response;
use framer_core::convert::{self, ClientDialect, ModelRoute};
use framer_core::native_dispatch::{AttemptMetadata, DispatchError, FailureKind,
    ComposedStreamOptions, Identity, InvocationRun, Policy, Preparation, Route, StreamingPolicy};
use serde_json::Value;
use std::sync::Arc;
use tokio::time::Instant;

#[cfg(test)]
#[path = "native_pool_fixture.rs"]
pub(crate) mod pool_fixture;

#[path = "native_payload.rs"]
mod payload_preparation;

#[path = "conversation_context.rs"]
mod conversation_context;

#[path = "native_http_stream.rs"]
pub(crate) mod http_stream;

pub(super) async fn raw(s: &AppState, dialect: &str, headers: &HeaderMap,
    payload: Value) -> Response {
    let dialect = match dialect {
        "anthropic-messages" => ClientDialect::AnthropicMessages,
        "openai-responses" => ClientDialect::OpenAiResponses,
        "openrouter-chat" => ClientDialect::OpenAiChat,
        _ => return failure(DispatchError::new(FailureKind::InvalidRequest)),
    };
    let session = match conversation(headers) { Ok(value) => value, Err(error) => return failure(error) };
    finish(dispatch(s, dialect, true, super::hdr(headers, "x-framerize-account"),
        Some(session), payload).await)
}

pub(super) async fn chain(s: &AppState, headers: &HeaderMap,
    dialect: ClientDialect, payload: Value) -> Response {
    let session = match conversation(headers) { Ok(value) => value, Err(error) => return failure(error) };
    finish(dispatch(s, dialect, false, super::hdr(headers, "x-framerize-account"),
        Some(session), payload).await)
}

fn conversation(headers: &HeaderMap) -> Result<String, DispatchError> {
    conversation_context::session(headers)
        .map_err(|_| DispatchError::new(FailureKind::InvalidRequest))?
        .map(str::to_owned)
        .ok_or_else(|| DispatchError::new(FailureKind::MissingContext))
}

fn finish(result: Result<Response, DispatchError>) -> Response {
    result.unwrap_or_else(failure)
}

struct Prepared {
    route: Route, payload: Value, client_model: String,
    custom_tools: std::collections::HashSet<String>, max_tokens: u32,
    streaming: Option<StreamingPolicy>, include_usage: bool,
}

fn prepare(dialect: ClientDialect, paid: bool, mut payload: Value, policy: &Policy,
    streaming: Option<StreamingPolicy>)
    -> Result<Prepared, DispatchError> {
    if !payload.is_object() || payload.get("stream").is_some_and(|v| !v.is_boolean()) {
        return Err(DispatchError::new(FailureKind::InvalidRequest));
    }
    let streaming = if payload["stream"].as_bool() == Some(true) {
        let limits = streaming.ok_or_else(|| DispatchError::new(FailureKind::UnsupportedStreaming))?;
        limits.validate()?;
        Some(limits)
    } else { None };
    let model = payload["model"].as_str().filter(|m| !m.is_empty())
        .ok_or_else(|| DispatchError::new(FailureKind::InvalidRequest))?.to_owned();
    let custom_tools = convert::custom_tool_names(&payload);
    if model.ends_with(":batch") {
        return Err(DispatchError::new(FailureKind::InvalidRequest));
    }
    if let Some(base) = model.strip_suffix("-ultrafast") {
        if base.is_empty() || payload.get("service_tier").is_some_and(|v| v.as_str() != Some("ultrafast")) {
            return Err(DispatchError::new(FailureKind::InvalidRequest));
        }
        payload["model"] = Value::String(base.to_owned());
        payload["service_tier"] = Value::String("ultrafast".into());
    }
    let model_route = convert::model_route(payload["model"].as_str().unwrap_or_default());
    if matches!(model_route, ModelRoute::Unavailable(_)) {
        return Err(DispatchError::new(FailureKind::InvalidRequest));
    }
    let paid = paid || model_route == ModelRoute::DirectResponses;
    if paid && !policy.allow_paid { return Err(DispatchError::new(FailureKind::PaidDisabled)); }
    if model_route == ModelRoute::DirectResponses
        && (dialect != ClientDialect::OpenAiResponses
            || payload["service_tier"].as_str() == Some("ultrafast")) {
        return Err(DispatchError::new(FailureKind::InvalidRequest));
    }
    let mut token_limit = None;
    for key in ["max_tokens", "max_completion_tokens", "max_output_tokens"] {
        if let Some(value) = payload.get(key) {
            let limit = value.as_u64().and_then(|n| u32::try_from(n).ok()).filter(|n| *n > 0)
                .ok_or_else(|| DispatchError::new(FailureKind::InvalidRequest))?;
            if token_limit.is_some_and(|previous| previous != limit) {
                return Err(DispatchError::new(FailureKind::InvalidRequest));
            }
            token_limit = Some(limit);
        }
    }
    let max_tokens = token_limit.unwrap_or(4096);
    let route = if paid { Route::Raw(dialect) }
        else if dialect == ClientDialect::AnthropicMessages { Route::Messages }
        else { Route::Responses };
    let mut include_usage = false;
    if streaming.is_some() && !matches!(route, Route::Raw(_)) {
        if let Some(options) = payload.get("stream_options") {
            let object = options.as_object()
                .ok_or_else(|| DispatchError::new(FailureKind::InvalidRequest))?;
            if dialect != ClientDialect::OpenAiChat
                || object.keys().any(|key| key != "include_usage")
                || object.get("include_usage").is_some_and(|value| !value.is_boolean()) {
                return Err(DispatchError::new(FailureKind::InvalidRequest));
            }
            include_usage = options["include_usage"].as_bool().unwrap_or(false);
        }
    }
    let payload = match route {
        Route::Raw(_) => payload,
        Route::Messages => payload_preparation::messages(payload)?,
        Route::Responses => {
            let native = if dialect == ClientDialect::OpenAiChat {
                payload_preparation::chat(payload)?
            } else { payload };
            payload_preparation::responses(native)?
        }
    };
    Ok(Prepared { route, payload, client_model: model, custom_tools, max_tokens,
        streaming, include_usage })
}

async fn dispatch(s: &AppState, dialect: ClientDialect, paid: bool,
    account_hint: Option<String>, session_hint: Option<String>, payload: Value)
    -> Result<Response, DispatchError> {
    let started = Instant::now();
    let provider = s.native_dispatch.as_ref()
        .ok_or_else(|| DispatchError::new(FailureKind::Internal))?;
    if !provider.supports_pool_admission() {
        return Err(DispatchError::new(FailureKind::Ineligible));
    }
    let policy = provider.policy();
    policy.validate()?;
    let deadline = started.checked_add(policy.timeout)
        .ok_or_else(|| DispatchError::new(FailureKind::InvalidRequest))?;
    let prepared = prepare(dialect, paid, payload, &policy, provider.streaming_policy())?;
    let session = session_hint.filter(|s| !s.is_empty())
        .ok_or_else(|| DispatchError::new(FailureKind::MissingContext))?;
    if HeaderValue::from_str(&session).is_err() {
        return Err(DispatchError::new(FailureKind::MissingContext));
    }
    if s.wave.check().is_some() { return Err(DispatchError::new(FailureKind::Overloaded)); }
    let pool = tokio::time::timeout_at(deadline, s.state.list()).await
        .map_err(|_| DispatchError::new(FailureKind::Deadline))?
        .map_err(|_| DispatchError::new(FailureKind::Internal))?;
    let now = framer_core::rotation::now_ms();
    let eligible: Vec<_> = pool.into_iter().filter(|a| s.serving_accounts.allows(&a.id)
        && a.is_available_at(now) && provider.contains_account(a.id)).collect();
    let account = s.serving_accounts.select(eligible, account_hint.as_deref())
        .map_err(|_| DispatchError::new(FailureKind::Ineligible))?;
    let identity = Identity { account: account.id, user: account.framer_user_id.clone(),
        team: account.current_team.clone().ok_or_else(|| DispatchError::new(FailureKind::InvalidScope))?,
        project: account.current_project.clone().ok_or_else(|| DispatchError::new(FailureKind::InvalidScope))?,
        chat_session: session.clone() };
    let resident = tokio::time::timeout_at(deadline, super::lease_account(s, identity.account))
        .await.map_err(|_| DispatchError::new(FailureKind::Deadline))?
        .map_err(|_| DispatchError::new(FailureKind::Ineligible))?;
    let owner = s.operation_ownership.as_ref().map(|scope| scope.native_cleanup())
        .transpose().map_err(|_| DispatchError::new(FailureKind::Ineligible))?;
    let admission = framer_core::native_dispatch::PoolAdmission::new(resident, owner);
    let model = convert::to_openrouter_id(prepared.payload["model"].as_str().unwrap_or_default());
    let key = format!("{}:{model}", identity.team.0);
    let permit = tokio::time::timeout_at(deadline, s.gate.acquire(&key)).await
        .map_err(|_| DispatchError::new(FailureKind::Deadline))?;
    tokio::time::timeout_at(deadline, revalidate(s, &identity)).await
        .map_err(|_| DispatchError::new(FailureKind::Deadline))??;
    let call = tokio::time::timeout_at(deadline, provider.prepare_in_pool(Preparation {
        identity, route: prepared.route, deadline, max_total_tokens: prepared.max_tokens,
    }, permit, admission)).await.map_err(|_| DispatchError::new(FailureKind::Deadline))??;
    if let Some(limits) = prepared.streaming {
        return match prepared.route {
            Route::Raw(_) => http_stream::raw(
                call.raw_stream(prepared.payload, limits.raw), deadline, &session, limits.heartbeat).await,
            Route::Messages | Route::Responses => http_stream::composed(
                call.composed_stream_for_client(prepared.payload, ComposedStreamOptions {
                    dialect, include_usage: prepared.include_usage, limits: limits.composed,
                }, prepared.client_model), deadline, &session, limits.heartbeat).await,
        };
    }
    match prepared.route {
        Route::Raw(_) => {
            let result = await_run(call.raw(prepared.payload), deadline).await?;
            let content_type = result.metadata.original_content_type.as_deref()
                .unwrap_or(&policy.missing_content_type).to_owned();
            response(result.body, result.metadata.status, &content_type,
                &session, vec![result.metadata])
        }
        Route::Messages => {
            let result = await_run(call.messages(prepared.payload), deadline).await?;
            let body = convert::render_json_with(dialect, &result.value,
                &prepared.client_model, &prepared.custom_tools);
            json_response(body, &session, result.attempts)
        }
        Route::Responses => {
            let result = await_run(call.responses(prepared.payload), deadline).await?;
            let body = if dialect == ClientDialect::OpenAiChat {
                convert::responses_to_chat_completion(&result.value, &prepared.client_model)
            } else {
                convert::responses_json(&result.value, &prepared.client_model, &result.value.id)
            };
            json_response(body, &session, result.attempts)
        }
    }
}

async fn revalidate(s: &AppState, identity: &Identity) -> Result<(), DispatchError> {
    let account = s.state.get(identity.account).await
        .map_err(|_| DispatchError::new(FailureKind::Internal))?
        .ok_or_else(|| DispatchError::new(FailureKind::Ineligible))?;
    if s.wave.check().is_some() { return Err(DispatchError::new(FailureKind::Overloaded)); }
    if !s.serving_accounts.allows(&account.id)
        || !account.is_available_at(framer_core::rotation::now_ms())
        || !s.native_dispatch.as_ref().is_some_and(|p| p.contains_account(account.id)) {
        return Err(DispatchError::new(FailureKind::Ineligible));
    }
    if account.framer_user_id != identity.user
        || account.current_team.as_ref() != Some(&identity.team)
        || account.current_project.as_ref() != Some(&identity.project) {
        return Err(DispatchError::new(FailureKind::InvalidScope));
    }
    // Another request can fence our resident slot while the model gate waits.
    let snapshot = s.serving_accounts.residents.snapshot()
        .map_err(|_| DispatchError::new(FailureKind::Ineligible))?;
    if !snapshot.residents.iter().any(|slot| slot.account == identity.account
        && slot.users > 0 && !slot.uncertain
        && slot.status == framer_core::resident_pool::ResidentStatus::Active) {
        return Err(DispatchError::new(FailureKind::Ineligible));
    }
    Ok(())
}

async fn await_run<T>(run: InvocationRun<T>, deadline: Instant) -> Result<T, DispatchError> {
    let InvocationRun { result, cleanup, cancellation, completed_attempts } = run;
    let completed = tokio::time::timeout_at(deadline, async {
        // A failure is deliverable while the independent guardian cleans up.
        let value = result.await?;
        cleanup.await.map_err(DispatchError::new)?;
        Ok(value)
    }).await;
    match completed {
        Ok(result) => result,
        Err(_) => {
            cancellation.cancel();
            let mut error = DispatchError::new(FailureKind::Deadline);
            if let Some(snapshot) = completed_attempts { error.attempts = snapshot(); }
            Err(error)
        }
    }
}

fn json_response(body: Value, session: &str, attempts: Vec<AttemptMetadata>)
    -> Result<Response, DispatchError> {
    let status = attempts.last().map(|m| m.status).unwrap_or(200);
    let bytes = serde_json::to_vec(&body).map_err(|_| DispatchError::new(FailureKind::Internal))?;
    response(bytes, status, "application/json", session, attempts)
}

fn response(body: Vec<u8>, status: u16, content_type: &str, session: &str,
    attempts: Vec<AttemptMetadata>) -> Result<Response, DispatchError> {
    let mut response = Response::new(axum::body::Body::from(body));
    *response.status_mut() = StatusCode::from_u16(status)
        .map_err(|_| DispatchError::new(FailureKind::UnsupportedResponse))?;
    response.headers_mut().insert("content-type", HeaderValue::from_str(content_type)
        .map_err(|_| DispatchError::new(FailureKind::UnsupportedResponse))?);
    response.headers_mut().insert("x-framerize-session", HeaderValue::from_str(session)
        .map_err(|_| DispatchError::new(FailureKind::MissingContext))?);
    response.extensions_mut().insert(Arc::new(attempts));
    Ok(response)
}

fn failure(mut error: DispatchError) -> Response {
    if let Some(upstream) = error.upstream.take() {
        let status = StatusCode::from_u16(upstream.metadata.status)
            .unwrap_or(StatusCode::BAD_GATEWAY);
        let content_type = upstream.metadata.original_content_type.as_deref()
            .and_then(|ct| HeaderValue::from_str(ct).ok())
            .unwrap_or_else(|| HeaderValue::from_static("application/octet-stream"));
        let mut response = Response::new(axum::body::Body::from(upstream.body));
        *response.status_mut() = status;
        response.headers_mut().insert("content-type", content_type);
        error.attempts.push(upstream.metadata);
        response.extensions_mut().insert(Arc::new(error.attempts));
        response.extensions_mut().insert(error.kind);
        return response;
    }
    let status = match error.kind {
        FailureKind::UnsupportedStreaming => StatusCode::NOT_IMPLEMENTED,
        FailureKind::InvalidRequest | FailureKind::MissingContext => StatusCode::BAD_REQUEST,
        FailureKind::PaidDisabled => StatusCode::FORBIDDEN,
        FailureKind::CreditFloor => StatusCode::PAYMENT_REQUIRED,
        FailureKind::InvalidScope | FailureKind::ScopeClosed => StatusCode::CONFLICT,
        FailureKind::Deadline => StatusCode::GATEWAY_TIMEOUT,
        FailureKind::Cancelled => StatusCode::REQUEST_TIMEOUT,
        FailureKind::Overloaded => StatusCode::TOO_MANY_REQUESTS,
        FailureKind::MissingAuthorization | FailureKind::Ineligible => StatusCode::SERVICE_UNAVAILABLE,
        _ => StatusCode::BAD_GATEWAY,
    };
    let body = serde_json::json!({"error": {
        "type": "native_dispatch_error", "code": format!("{:?}", error.kind)
    }});
    let mut response = Response::new(axum::body::Body::from(body.to_string()));
    *response.status_mut() = status;
    response.headers_mut().insert("content-type", HeaderValue::from_static("application/json"));
    response.extensions_mut().insert(Arc::new(error.attempts));
    response.extensions_mut().insert(error.kind);
    response
}

#[cfg(test)]
#[path = "native_dispatch_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "native_serving_policy_tests.rs"]
mod serving_policy_tests;

#[cfg(test)]
#[path = "native_context_tests.rs"]
mod context_tests;

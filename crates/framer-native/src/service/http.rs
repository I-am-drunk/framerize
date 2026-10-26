//! Explicit native endpoints. Every request supplies its actual context.
use crate::service::serving as offline_serving;
use crate::service::serving::Request;
use crate::service::sessions::Sessions;
use axum::extract::DefaultBodyLimit;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::response::Response;
use axum::routing::post;
use axum::Json;
use axum::Router;
use crate::request::invocation::AttemptContext;
use crate::request::complete_json::FreeRoute;
use crate::request::complete_json::Specification;
use framer_core::ids::AccountId;
use framer_core::ids::FramerUserId;
use framer_core::ids::ProjectId;
use framer_core::ids::TeamId;
use crate::request::types::FailureKind;
use crate::request::types::Identity;
use super::state::ServiceState;
use serde::Deserialize;
use std::sync::Arc;
use std::time::Duration;
use crate::worker::backend::WorkerBackend;
use crate::account::device_owner::DeviceOwner;
use crate::request::complete_json::CompletedJson;
use framer_core::convert;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Input {
    account_id: uuid::Uuid, user_id: String, team_id: String, project_id: String,
    conversation_id: String, page_url: String, route: String,
    feature_category: String, feature_name: String, step: String,
    turn_id: String, user_prompt: String, timeout_ms: u64, token_limit: u32,
    request: serde_json::Value,
    #[serde(default)]
    continuation: Option<crate::request::continuation::Continuation>,
}

#[derive(Clone)]
pub(crate) struct Runtime {
    worker: WorkerBackend,
    sessions: Sessions,
    device_owner: Arc<dyn DeviceOwner>,
}
#[derive(Clone)]
struct Endpoint { state: ServiceState, runtime: Runtime }

#[cfg(test)]
#[path = "offline_http_tests.rs"]
mod tests;

pub(crate) fn configure(state: ServiceState, worker: WorkerBackend, device_owner: Arc<dyn DeviceOwner>)
    -> Result<(ServiceState, Runtime), &'static str> {
    configure_browser(state, worker, device_owner, None)
}

pub(crate) fn configure_browser(mut state: ServiceState, worker: WorkerBackend,
    device_owner: Arc<dyn DeviceOwner>, browser: Option<Arc<dyn crate::transport::browser::BrowserTransport>>)
    -> Result<(ServiceState, Runtime), &'static str> {
    let sessions = Sessions::with_browser(state.sessions.clone(),
        state.operation_ownership.clone().ok_or("missing serving owner")?, browser,
        state.session_authorization.clone())?;
    state.sessions = sessions.store();
    Ok((state, Runtime { worker, sessions, device_owner }))
}

async fn complete(State(endpoint): State<Endpoint>, Json(input): Json<Input>) -> Response {
    dispatch(endpoint, input, false).await
}

async fn stream(State(endpoint): State<Endpoint>, Json(input): Json<Input>) -> Response {
    dispatch(endpoint, input, true).await
}

async fn dispatch(endpoint: Endpoint, input: Input, streaming: bool) -> Response {
    if !streaming && input.continuation.is_some() { return failure(FailureKind::InvalidRequest); }
    let capacity = input.continuation.as_ref().map_or(1, |v| v.max_attempts());
    let model = input.request["model"].as_str().unwrap_or_default().to_owned();
    let route = if input.route == "/ai/v3/messages/" { FreeRoute::Messages }
        else { FreeRoute::Responses };
    let request = match request(input) {
        Ok(value) => value, Err(kind) => return failure(kind),
    };
    let (sender, receiver) = tokio::sync::mpsc::channel(capacity);
    let run = match offline_serving::launch(endpoint.state, request,
        endpoint.runtime.worker, endpoint.runtime.sessions, endpoint.runtime.device_owner,
        streaming.then_some(sender)) {
        Ok(value) => value, Err(error) => return failure(error.kind),
    };
    if streaming { return super::stream::response(run.result, receiver, route, model, run.diagnostic_id); }
    let outcome = run.result.await;
    record_completion(&outcome, run.diagnostic_id);
    match (outcome.response, outcome.failure, outcome.cleanup_confirmed) {
        (Some(value), None, true) => Json(match value {
            CompletedJson::Responses(value) => convert::responses_json(&value, &model, &value.id),
            CompletedJson::Messages(value) => convert::render_json(
                convert::ClientDialect::AnthropicMessages, &value, &model),
        }).into_response(),
        (_, kind, _) => failure(kind.unwrap_or(FailureKind::Cleanup)),
    }
}

pub(crate) fn record_completion(outcome: &offline_serving::Outcome, diagnostic_id: uuid::Uuid) {
    record_outcome("offline.complete", outcome, diagnostic_id);
}

pub(super) fn record_owned_completion(outcome: &offline_serving::Outcome, diagnostic_id: uuid::Uuid) {
    record_outcome("offline.request.finished", outcome, diagnostic_id);
}

fn record_outcome(event: &'static str, outcome: &offline_serving::Outcome, diagnostic_id: uuid::Uuid) {
    // Diagnostics cannot change the request outcome or interrupt owned cleanup.
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        framer_core::observability::Event::new(event)
            .request_id(&diagnostic_id.to_string())
            .field("completion", completion_fields(outcome)).emit();
    }));
}

fn completion_fields(outcome: &offline_serving::Outcome) -> serde_json::Value {
    // Select facts explicitly. Never serialize the response or request context.
    let attempt = outcome.evidence.as_ref().map(|e| serde_json::json!({
        "status": e.status,
        "content_type": format!("{:?}", e.content_type),
        "attempts": e.attempts,
        "authorization_calls": e.authorization_calls,
        "transport_starts": e.transport_starts,
        "transport_finishes": e.transport_finishes,
        "complete_json": e.complete_json,
        "cleanup_confirmed": e.cleanup_confirmed,
        "refusal_policy": e.refusal.map(|r| format!("{:?}", r.policy)),
        "refusal_basis": e.refusal.map(|r| format!("{:?}", r.basis)),
    }));
    serde_json::json!({
        "cleanup_confirmed": outcome.cleanup_confirmed,
        "failure": outcome.failure.map(|kind| format!("{kind:?}")),
        "attempt": attempt,
    })
}

fn request(input: Input) -> Result<Request, FailureKind> {
    if !(1..=60_000).contains(&input.timeout_ms) { return Err(FailureKind::InvalidRequest); }
    let route = match input.route.as_str() {
        "/ai/v3/responses/" => FreeRoute::Responses,
        "/ai/v3/messages/" => FreeRoute::Messages,
        _ => return Err(FailureKind::InvalidRequest),
    };
    Request::new(Specification {
        identity: Identity { account: AccountId(input.account_id),
            user: FramerUserId(input.user_id), team: TeamId(input.team_id),
            project: ProjectId(input.project_id), chat_session: input.conversation_id },
        context: AttemptContext { feature_category: input.feature_category,
            feature_name: input.feature_name, step: input.step,
            turn_id: input.turn_id, user_prompt: input.user_prompt },
        route, payload: input.request, token_limit: input.token_limit,
        deadline: tokio::time::Instant::now() + Duration::from_millis(input.timeout_ms),
    }, input.page_url).and_then(|request| request.with_continuation(input.continuation))
        .map_err(|error| error.kind)
}

fn failure(kind: FailureKind) -> Response {
    let status = match kind {
        FailureKind::InvalidRequest | FailureKind::InvalidScope | FailureKind::MissingContext =>
            StatusCode::BAD_REQUEST,
        FailureKind::Ineligible | FailureKind::Overloaded | FailureKind::Cleanup =>
            StatusCode::SERVICE_UNAVAILABLE,
        FailureKind::Deadline | FailureKind::Cancelled => StatusCode::REQUEST_TIMEOUT,
        _ => StatusCode::BAD_GATEWAY,
    };
    (status, Json(serde_json::json!({"error": {"code": format!("{kind:?}")}}))).into_response()
}

async fn health(State(endpoint): State<Endpoint>) -> Response {
    health_response(endpoint.state.operation_ownership.as_deref())
}

fn health_response(owner: Option<&framer_server::operation_ownership::ServingOwnership>) -> Response {
    let ready = owner.is_some_and(|owner| owner.accepting_requests());
    (if ready { StatusCode::OK } else { StatusCode::SERVICE_UNAVAILABLE },
        [("x-framerize-pid", std::process::id().to_string())],
        if ready { "ok" } else { "operation admission closed" }).into_response()
}

pub(crate) async fn serve(state: ServiceState, runtime: Runtime,
    listener: tokio::net::TcpListener,
    shutdown: impl std::future::Future<Output = ()> + Send + 'static) -> std::io::Result<()> {
    let sessions = runtime.sessions.clone();
    let router = Router::new().route("/api/offline/complete", post(complete))
        .route("/api/offline/stream", post(stream))
        .route("/api/health", axum::routing::get(health))
        .layer(DefaultBodyLimit::max(96 * 1024))
        .with_state(Endpoint { state: state.clone(), runtime });
    let result = axum::serve(listener, router).with_graceful_shutdown(shutdown).await;
    sessions.shutdown().await.map_err(std::io::Error::other)?;
    result
}

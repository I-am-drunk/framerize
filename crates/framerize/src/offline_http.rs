//! Explicit complete-JSON endpoint. Every request supplies its actual context.
use super::{offline_serving::{self, Request}, offline_sessions::Sessions};
use axum::{extract::{DefaultBodyLimit, State}, http::StatusCode,
    response::{IntoResponse, Response}, routing::post, Json, Router};
use framer_api::{experimental_native::application::AttemptContext,
    native_dispatch::complete_json::{FreeRoute, Specification}};
use framer_core::{ids::{AccountId, FramerUserId, ProjectId, TeamId},
    native_dispatch::{FailureKind, Identity}};
use framer_server::AppState;
use serde::Deserialize;
use std::{net::SocketAddr, sync::Arc, time::Duration};
use super::offline_worker::WorkerBackend;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Input {
    account_id: uuid::Uuid, user_id: String, team_id: String, project_id: String,
    conversation_id: String, page_url: String, route: String,
    feature_category: String, feature_name: String, step: String,
    turn_id: String, user_prompt: String, timeout_ms: u64, token_limit: u32,
    request: serde_json::Value,
}

#[derive(Clone)]
pub(crate) struct Runtime { worker: WorkerBackend, sessions: Sessions, routing: bool }
#[derive(Clone)]
struct Endpoint { state: AppState, runtime: Runtime }

#[cfg(test)]
#[path = "offline_http_tests.rs"]
mod tests;

pub(crate) fn configure(state: AppState, path: Option<&str>, routing_path: Option<&str>)
    -> Result<(AppState, Option<Runtime>), &'static str> {
    configure_with(state, path, routing_path, false)
}

pub(crate) fn configure_v2(state: AppState, path: Option<&str>, routing_path: Option<&str>)
    -> Result<(AppState, Option<Runtime>), &'static str> {
    if path.is_none() { return Err("V2 serving requires its worker config"); }
    configure_with(state, path, routing_path, true)
}

fn configure_with(state: AppState, path: Option<&str>, routing_path: Option<&str>, v2: bool)
    -> Result<(AppState, Option<Runtime>), &'static str> {
    if path.is_none() && routing_path.is_some() { return Err("offline routing requires its worker"); }
    let Some(path) = path else { return Ok((state, None)); };
    let routing = routing_path.map(super::offline_routing_config::load).transpose()?;
    if routing.is_some() && state.native_dispatch.is_some() {
        return Err("offline routing provider already installed");
    }
    let worker = if v2 { WorkerBackend::V2(Arc::new(super::offline_config::load_v2(path)?)) }
        else { WorkerBackend::V3(Arc::new(super::offline_config::load(path)?)) };
    let mut state = framer_server::operation_ownership::with_owned_operations(state, 256)
        .map_err(|_| "offline serving ownership unavailable")?;
    let sessions = Sessions::install(&mut state)?;
    if let Some((policy, streaming)) = routing {
        let provider = super::offline_routing::Provider::new(state.clone(), sessions.clone(),
            worker.clone(), Arc::new(super::offline_request_context::Current), policy, streaming)
            .map_err(|_| "offline routing initialization failed")?;
        state.native_dispatch = Some(Arc::new(provider));
    }
    Ok((state, Some(Runtime { worker, sessions, routing: routing_path.is_some() })))
}

async fn complete(State(endpoint): State<Endpoint>, Json(input): Json<Input>) -> Response {
    let request = match request(input) {
        Ok(value) => value, Err(kind) => return failure(kind),
    };
    let run = match offline_serving::launch(endpoint.state, request,
        endpoint.runtime.worker, endpoint.runtime.sessions) {
        Ok(value) => value, Err(error) => return failure(error.kind),
    };
    let outcome = run.result.await;
    framer_core::observability::Event::new("offline.complete")
        .field("completion", completion_fields(&outcome))
        .emit();
    match (outcome.response, outcome.failure, outcome.cleanup_confirmed) {
        (Some(value), None, true) => Json(value).into_response(),
        (_, kind, _) => failure(kind.unwrap_or(FailureKind::Cleanup)),
    }
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
    }, input.page_url).map_err(|error| error.kind)
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

pub(crate) async fn serve(state: AppState, runtime: Runtime, web_dir: &str,
    address: SocketAddr) -> std::io::Result<()> {
    let sessions = runtime.sessions.clone();
    let routing = runtime.routing;
    let extra = Router::new().route("/api/offline/complete", post(complete))
        .layer(DefaultBodyLimit::max(96 * 1024))
        .with_state(Endpoint { state: state.clone(), runtime });
    let router = framer_server::router_with_request_layer(state.clone(), web_dir, |router| {
        if routing {
            router.layer(axum::middleware::from_fn(super::offline_request_context::middleware))
        } else { router }
    }).merge(extra);
    let result = match tokio::net::TcpListener::bind(address).await {
        Ok(listener) => axum::serve(listener, router).await,
        Err(error) => Err(error),
    };
    sessions.shutdown(&state).await.map_err(std::io::Error::other)?;
    result
}

//! Explicit HTTP metadata, captured before entering an owned request task.
use super::offline_routing::{Contexts, RequestContext};
use framer_api::experimental_native::application::AttemptContext;
use framer_core::{ids::AccountId, native_dispatch::{DispatchError, FailureKind, Preparation}};
use serde::Deserialize;
use serde_json::Value;
use std::sync::Arc;

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct Metadata {
    page_url: String,
    proxy: Option<String>,
    attempts: Vec<Attempt>,
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct Attempt {
    feature_category: String,
    feature_name: String,
    step: String,
    turn_id: String,
    user_prompt: String,
}

tokio::task_local! { static CURRENT: Arc<Metadata>; }
pub(crate) struct Current;

#[cfg(test)]
#[path = "offline_request_context_tests.rs"]
mod tests;

fn missing() -> DispatchError { DispatchError::new(FailureKind::MissingContext) }

fn label(value: &str) -> bool {
    !value.is_empty() && value.len() <= 256 && !value.chars().any(char::is_control)
}

impl Contexts for Current {
    fn contains_account(&self, _: AccountId) -> bool { CURRENT.try_with(|_| ()).is_ok() }
    fn capture(&self) -> Result<Option<Arc<dyn Contexts>>, DispatchError> {
        CURRENT.try_with(|value| Some(value.clone() as Arc<dyn Contexts>)).map_err(|_| missing())
    }
    fn resolve(&self, _: &Preparation, _: &Value) -> Result<RequestContext, DispatchError> {
        Err(missing())
    }
}

impl Contexts for Metadata {
    fn contains_account(&self, _: AccountId) -> bool { true }
    fn resolve(&self, request: &Preparation, _: &Value) -> Result<RequestContext, DispatchError> {
        Ok(RequestContext {
            identity: request.identity.clone(), proxy: self.proxy.clone(),
            page_url: self.page_url.clone(), paid_credit: None,
            attempts: self.attempts.iter().map(|value| AttemptContext {
                feature_category: value.feature_category.clone(),
                feature_name: value.feature_name.clone(), step: value.step.clone(),
                turn_id: value.turn_id.clone(), user_prompt: value.user_prompt.clone(),
            }).collect(),
        })
    }
}

fn decode(headers: &axum::http::HeaderMap) -> Result<Arc<Metadata>, DispatchError> {
    let mut values = headers.get_all("x-framerize-context").iter();
    let bytes = values.next().ok_or_else(missing)?.as_bytes();
    if values.next().is_some() || bytes.len() > 64 * 1024 { return Err(missing()); }
    let value: Metadata = serde_json::from_slice(bytes).map_err(|_| missing())?;
    if value.page_url.is_empty() || value.page_url.len() > 4096
        || value.page_url.chars().any(char::is_control)
        || value.proxy.as_ref().is_some_and(|p| p.len() > 4096 || p.chars().any(char::is_control))
        || !(1..=64).contains(&value.attempts.len())
        || value.attempts.iter().any(|a| [&a.feature_category, &a.feature_name, &a.step, &a.turn_id]
            .iter().any(|s| !label(s)) || a.user_prompt.len() > 4096) {
        return Err(missing());
    }
    Ok(Arc::new(value))
}

pub(crate) async fn middleware(mut request: axum::extract::Request,
    next: axum::middleware::Next) -> axum::response::Response {
    let path = request.uri().path().strip_prefix("/paid").unwrap_or(request.uri().path());
    let inference = request.method() == axum::http::Method::POST && matches!(path,
        "/anthropic/v1/messages" | "/openai/v1/responses" | "/openai/v1/chat/completions");
    if !inference { return next.run(request).await; }
    let context = match decode(request.headers()) {
        Ok(context) => context,
        Err(_) => {
            use axum::response::IntoResponse;
            return (axum::http::StatusCode::BAD_REQUEST,
                axum::Json(serde_json::json!({"error":{"code":"MissingContext"}}))).into_response();
        }
    };
    request.headers_mut().remove("x-framerize-context");
    CURRENT.scope(context, next.run(request)).await
}

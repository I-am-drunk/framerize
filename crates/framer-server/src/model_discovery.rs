use crate::{AppState, operation_ownership::completed_response};
use axum::{extract::State, response::{IntoResponse, Response}, Json};
use serde_json::json;

#[cfg(test)]
#[path = "model_discovery_tests.rs"]
mod tests;

pub(crate) async fn openai(State(state): State<AppState>) -> Response {
    let data: Vec<_> = state.client.model_catalog().iter().map(|id|
        json!({"id":id, "object":"model", "created":0, "owned_by":"framerize"})).collect();
    completed_response(Json(json!({"object":"list", "data":data})).into_response(), true)
}

pub(crate) async fn anthropic(State(state): State<AppState>) -> Response {
    let data: Vec<_> = state.client.model_catalog().iter()
        .filter(|id| id.starts_with("claude-"))
        .map(|id| json!({"id":id, "type":"model", "display_name":id,
            "created_at":"1970-01-01T00:00:00Z"})).collect();
    // Epoch timestamps mean unknown; the registry does not supply release dates.
    completed_response(Json(json!({"first_id":data.first().map(|v| &v["id"]),
        "last_id":data.last().map(|v| &v["id"]), "has_more":false, "data":data}))
        .into_response(), true)
}

use axum::{http::StatusCode, response::{IntoResponse, Response}, Json};

pub(super) fn disabled() -> Response {
    (
        StatusCode::FORBIDDEN,
        [("x-should-retry", "false")],
        Json(serde_json::json!({
            "type": "error",
            "error": {
                "type": "permission_error",
                "code": "metered_access_disabled",
                "message": "This request selects a metered upstream route, but metered access is disabled on this gateway. Use a supported model on the custom route. No upstream request was sent.",
                "retryable": false
            }
        })),
    ).into_response()
}

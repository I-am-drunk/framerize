use crate::ApiError;
use serde_json::Value;

pub(super) fn validate(payload: &Value) -> Result<(), ApiError> {
    if payload.is_object() {
        Ok(())
    } else {
        Err(ApiError::InvalidRequest("request body must be a JSON object".into()))
    }
}

#[cfg(test)]
#[path = "request_body_object_tests.rs"]
mod tests;

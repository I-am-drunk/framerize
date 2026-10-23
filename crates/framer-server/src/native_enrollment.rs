use super::*;
use axum::response::IntoResponse;
use framer_core::native_device::{DeviceFailure, valid_device_id};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct EnrollmentBody {
    account_id: AccountId,
    expected_framer_user_id: String,
    observed_device_id: String,
}

type Failure = (ApiError, bool);

fn refused(message: &str) -> Failure {
    (ApiError::InvalidRequest(message.into()), true)
}

pub(super) async fn enroll(
    State(state): State<AppState>,
    Json(body): Json<EnrollmentBody>,
) -> axum::response::Response {
    let (response, confirmed) = match initialize(&state, &body).await {
        Ok(value) => (Json(value).into_response(), true),
        Err((error, confirmed)) => (error.into_response(), confirmed),
    };
    operation_ownership::completed_response(response, confirmed)
}

async fn initialize(state: &AppState, body: &EnrollmentBody)
    -> Result<serde_json::Value, Failure> {
    if state.native_dispatch.is_none() {
        return Err(refused("initial native enrollment requires native dispatch"));
    }
    let user = &body.expected_framer_user_id;
    if user.is_empty() || user.len() > 128 || user.trim() != user
        || !valid_device_id(&body.observed_device_id) {
        return Err(refused("invalid native enrollment identity"));
    }
    let _management = pool_control::management(state, body.account_id, false)
        .await.map_err(|error| (error, true))?;
    let mut account = state.state.get(body.account_id).await
        .map_err(|_| (ApiError::Internal("account lookup failed".into()), true))?
        .ok_or_else(|| refused("native enrollment account is missing"))?;
    if account.framer_user_id.0 != *user {
        return Err(refused("native enrollment login differs from the account"));
    }
    let accounts = state.state.list().await
        .map_err(|_| (ApiError::Internal("account inventory failed".into()), true))?;
    if accounts.iter().any(|other| other.id != account.id
        && (other.framer_user_id == account.framer_user_id
            || other.castle_device_id == body.observed_device_id)) {
        return Err(refused("native enrollment identity is already assigned"));
    }
    let session = state.sessions.load(&account.framer_user_id).await
        .map_err(|_| (ApiError::Internal("session lookup failed".into()), true))?
        .ok_or_else(|| refused("native enrollment session is missing"))?;
    if session.active_user != account.framer_user_id {
        return Err(refused("native enrollment session identity differs"));
    }
    let proof = match state.browser.enroll_native_device(
        account.id, &account.framer_user_id, &body.observed_device_id).await {
        Ok(proof) => proof,
        Err(DeviceFailure::Rejected) => return Err(refused("native enrollment refused")),
        Err(DeviceFailure::Unconfirmed) => return Err((
            ApiError::Internal("native enrollment completion is unconfirmed".into()), false)),
    };
    account.castle_device_id = body.observed_device_id.clone();
    state.state.upsert(&account).await.map_err(|_| (
        ApiError::Internal("native enrollment account publication failed".into()), false))?;
    Ok(serde_json::json!({
        "account_id": account.id,
        "host_generation": proof.host_generation,
        "enrollment": proof.enrollment,
        "cooldown_ms": proof.cooldown_ms,
    }))
}

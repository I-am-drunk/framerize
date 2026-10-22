//! Bounded credit evidence read inside an existing owned native preparation.
use super::*;
use crate::transport::Auth;
use framer_core::ids::TeamId;

impl BoundControlClient {
    pub async fn credit_balance(&self, scope: &AccountScope, session: &Session,
        team: &TeamId, control: &PreparationControl) -> PreparationResult<f64> {
        if !self.expected.same_generation(scope) || scope.active_user() != &session.active_user
            || !CurrentRequestCookies::valid_session(session) || team.0.is_empty()
            || team.0.len() > 256
            || !team.0.bytes().all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_')) {
            return PreparationResult::finished(Err(PreparationFailure::Rejected));
        }
        if control.is_cancelled() {
            return PreparationResult::finished(Err(PreparationFailure::Cancelled));
        }
        let token = match self.transport.control_json("/auth/web/access-token", Auth::Cookie(session)).await {
            Ok(value) => value,
            Err(_) => return PreparationResult::unconfirmed(),
        };
        let bearer = token["accessToken"].as_str().filter(|v| !v.is_empty() && v.len() <= 16 * 1024);
        let expiry = token["expiresAt"].as_str()
            .and_then(|v| chrono::DateTime::parse_from_rfc3339(v).ok());
        let Some(bearer) = bearer.filter(|_| expiry.is_some_and(|v|
            v.timestamp_millis() > chrono::Utc::now().timestamp_millis() + 30_000)) else {
            return PreparationResult::finished(Err(PreparationFailure::Rejected));
        };
        self.credit_ledger(team, bearer, control).await
    }

    async fn credit_ledger(&self, team: &TeamId, bearer: &str,
        control: &PreparationControl) -> PreparationResult<f64> {
        if control.is_cancelled() {
            return PreparationResult::finished(Err(PreparationFailure::Cancelled));
        }
        let path = format!("/web/teams/{}/ai-credits", team.0);
        let value = match self.transport.control_json(&path, Auth::Bearer(bearer)).await {
            Ok(value) => value,
            Err(_) => return PreparationResult::unconfirmed(),
        };
        if control.is_cancelled() {
            return PreparationResult::finished(Err(PreparationFailure::Cancelled));
        }
        PreparationResult::finished(value["availableCredits"].as_f64()
            .filter(|n| n.is_finite() && *n >= 0.0).ok_or(PreparationFailure::Rejected))
    }
}

#[cfg(test)]
#[path = "credit_control_tests.rs"]
mod tests;

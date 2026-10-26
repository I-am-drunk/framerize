//! Publish prepared scopes after rechecking their persisted owner and peers.
use crate::{ApiError, AppState};
use framer_core::{account::ManagedAccount, error::CoreError};
use std::collections::BTreeSet;

pub(super) async fn publish(
    state: &AppState,
    expected: &ManagedAccount,
    prepared: &mut ManagedAccount,
) -> Result<(), ApiError> {
    let _update = state.serving_accounts.health_update.lock().await;
    let mut current = state.state.get(expected.id).await.map_err(ApiError::Internal)?
        .ok_or_else(|| not_eligible("account removed during update"))?;
    if current.framer_user_id != expected.framer_user_id
        || current.castle_device_id != expected.castle_device_id
        || current.current_team != expected.current_team
        || current.current_project != expected.current_project
        || current.proxy != expected.proxy
    {
        return Err(not_eligible("account scope changed during update"));
    }
    let peers = state.state.list().await.map_err(ApiError::Internal)?;
    let claimed: BTreeSet<_> = peers.iter().filter(|peer| peer.id != expected.id)
        .flat_map(|peer| peer.current_team.iter()
            .chain(peer.extra_workspaces.iter().map(|scope| &scope.team)))
        .map(|team| team.0.clone()).collect();
    current.current_project = prepared.current_project.clone();
    current.extra_workspaces = prepared.extra_workspaces.iter()
        .filter(|scope| !claimed.contains(&scope.team.0)).cloned().collect();
    state.state.upsert(&current).await.map_err(ApiError::Internal)?;
    *prepared = current;
    Ok(())
}

fn not_eligible(message: &str) -> ApiError {
    ApiError::Internal(CoreError::NotEligible(message.into()).to_string())
}

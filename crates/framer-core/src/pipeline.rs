//! The core pipeline: run one Framer agent request, and transparently rotate to a
//! fresh workspace when the current one is out of credits (HTTP 402).

use crate::account::{AccountState, ManagedAccount};
use crate::credits;
use crate::error::CoreError;
use crate::port::{BrowserBridge, FramerClient, SessionStore, StateStore, WorkspaceSpec};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InferenceRequest {
    pub prompt: String,
    /// Framer model id (e.g. "openai/gpt-6-sol"). None = the session's pinned model.
    /// NOTE: model is server-side session state, not a per-request field — see
    /// reference/analysis/framer-ai-endpoints.md.
    pub model: Option<String>,
    /// Continue an existing agent chat session (server-side history).
    #[serde(default)]
    pub chat_session: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InferenceOutcome {
    pub text: String,
    pub usage: Usage,
    pub chat_session: String,
    pub credits_remaining: Option<f64>,
}

/// The result of one upstream run. `Exhausted` encodes the fail-closed rule at the
/// type level: an exhausted run is NEVER an empty `Completed`.
#[derive(Debug, Clone)]
pub enum InferenceResult {
    Completed(InferenceOutcome),
    Exhausted { reason: String },
}

/// Ports the pipeline needs. Passed as trait objects to keep wiring simple.
pub struct Deps<'a> {
    pub client: &'a dyn FramerClient,
    pub browser: &'a dyn BrowserBridge,
    pub sessions: &'a dyn SessionStore,
    pub state: &'a dyn StateStore,
    /// Shared with every serving account-health writer.
    pub account_updates: &'a tokio::sync::Mutex<()>,
}

/// Apply only the completed operation's fields to the current account record.
pub async fn update_account(state: &dyn StateStore, updates: &tokio::sync::Mutex<()>,
    expected: &ManagedAccount, update: impl FnOnce(&mut ManagedAccount) + Send)
    -> Result<ManagedAccount, CoreError> {
    let _update = updates.lock().await;
    let mut current = state.get(expected.id).await.map_err(CoreError::Store)?
        .ok_or_else(|| CoreError::NotEligible("account removed during update".into()))?;
    if current.framer_user_id != expected.framer_user_id
        || current.castle_device_id != expected.castle_device_id
        || current.current_team != expected.current_team
        || current.current_project != expected.current_project || current.proxy != expected.proxy {
        return Err(CoreError::NotEligible("account scope changed during update".into()));
    }
    update(&mut current);
    state.upsert(&current).await.map_err(CoreError::Store)?;
    Ok(current)
}

impl Deps<'_> {
    pub(crate) async fn update(&self, account: &mut ManagedAccount,
        update: impl FnOnce(&mut ManagedAccount) + Send) -> Result<(), CoreError> {
        *account = update_account(self.state, self.account_updates, account, update).await?;
        Ok(())
    }
}

/// Run a request for `account`, rotating workspace and retrying once on exhaustion.
pub async fn run(
    deps: &Deps<'_>,
    account: &mut ManagedAccount,
    req: &InferenceRequest,
) -> Result<InferenceOutcome, CoreError> {
    let session = deps
        .sessions
        .load(&account.framer_user_id)
        .await
        .map_err(CoreError::Store)?
        .ok_or(CoreError::NoSession)?;

    credits::ensure_workspace(deps, account, &session).await?;
    let project = account
        .current_project
        .clone()
        .ok_or_else(|| CoreError::Framer("no current project — create one in the workspace".into()))?;

    match deps
        .client
        .run_inference(&session, deps.browser, &account.id, &project, req)
        .await
    {
        Ok(InferenceResult::Completed(outcome)) => {
            account.consecutive_failures = 0;
            Ok(outcome)
        }
        Ok(InferenceResult::Exhausted { .. }) => {
            // Fail closed: rotate to another credited workspace, retry exactly once.
            credits::rotate_workspace(deps, account, &session).await?;
            // The project lives in the old workspace; a fresh one is required in
            // the new workspace (project-creation endpoint not yet captured).
            let project = account
                .current_project
                .clone()
                .ok_or_else(|| CoreError::Framer("no project after rotation".into()))?;
            match deps
                .client
                .run_inference(&session, deps.browser, &account.id, &project, req)
                .await
            {
                Ok(InferenceResult::Completed(outcome)) => Ok(outcome),
                Ok(InferenceResult::Exhausted { reason }) => {
                    deps.update(account, |current| current.state = AccountState::Exhausted).await?;
                    Err(CoreError::NotEligible(format!(
                        "exhausted again after rotation: {reason}"
                    )))
                }
                Err(e) => Err(CoreError::Framer(e)),
            }
        }
        Err(e) if e == "risk-refused" => {
            // Castle refusal is transient. Cool down; never quarantine.
            deps.update(account, |current| {
                current.state = AccountState::CoolingDown;
                current.cooldown_step = current.cooldown_step.saturating_add(1);
            }).await?;
            Err(CoreError::RiskRefused(e))
        }
        Err(e) => Err(CoreError::Framer(e)),
    }
}

/// A fresh workspace name. Kept trivial on purpose.
pub(crate) fn default_workspace_spec() -> WorkspaceSpec {
    WorkspaceSpec {
        name: "Framerize".to_string(),
    }
}

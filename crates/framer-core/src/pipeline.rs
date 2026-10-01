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
                    account.state = AccountState::Exhausted;
                    deps.state.upsert(account).await.map_err(CoreError::Store)?;
                    Err(CoreError::NotEligible(format!(
                        "exhausted again after rotation: {reason}"
                    )))
                }
                Err(e) => Err(CoreError::Framer(e)),
            }
        }
        Err(e) if e == "risk-refused" => {
            // Castle refusal is transient. Cool down; never quarantine.
            account.state = AccountState::CoolingDown;
            account.cooldown_step += 1;
            deps.state.upsert(account).await.map_err(CoreError::Store)?;
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

use framer_api::{experimental_native::application::AttemptContext,
    native_dispatch::complete_json::{FreeRoute, Plan, Specification}};
use framer_core::{ids::{AccountId, FramerUserId, TeamId, ProjectId}, native_dispatch::Identity};
use serde::Deserialize;
use std::{path::PathBuf, time::Duration};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct WirePlan {
    pub version: u32,
    pub nonce: String,
    pub account: AccountId,
    pub user: FramerUserId,
    pub team: TeamId,
    pub project: ProjectId,
    pub chat_session: String,
    pub route: String,
    pub payload: serde_json::Value,
    pub context: Context,
    pub baseline_turn: String,
    pub token_limit: u32,
    pub timeout_ms: u64,
    pub proxy: Option<String>,
    pub state_parent: PathBuf,
    pub host_script: PathBuf,
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Context {
    feature_category: String, feature_name: String, step: String,
    turn_id: String, user_prompt: String,
}

impl Context {
    pub(super) fn into_attempt(self) -> AttemptContext {
        AttemptContext { feature_category: self.feature_category,
            feature_name: self.feature_name, step: self.step,
            turn_id: self.turn_id, user_prompt: self.user_prompt }
    }
}
impl WirePlan {
    pub(super) fn specification(&self) -> Result<Specification, ()> {
        if self.version != 1 || self.nonce.len() != 32
            || !self.nonce.bytes().all(|b| b.is_ascii_hexdigit())
            || !(1..=60_000).contains(&self.timeout_ms)
            || self.baseline_turn.is_empty() || self.baseline_turn.len() > 256
            || self.baseline_turn.chars().any(char::is_control)
            || self.baseline_turn == self.context.turn_id
            || self.proxy.as_ref().is_some_and(|p| p.is_empty() || p.len() > 4096
                || p.chars().any(char::is_control)) { return Err(()); }
        let identity = Identity { account: self.account, user: self.user.clone(),
            team: self.team.clone(), project: self.project.clone(),
            chat_session: self.chat_session.clone() };
        let route = match self.route.as_str() {
            "responses" => FreeRoute::Responses, "messages" => FreeRoute::Messages,
            _ => return Err(()),
        };
        let spec = Specification { identity, route, payload: self.payload.clone(),
            context: self.context.clone().into_attempt(), token_limit: self.token_limit,
            deadline: tokio::time::Instant::now() + Duration::from_millis(self.timeout_ms) };
        Plan::validate(&spec).map_err(|_| ())?;
        Ok(spec)
    }
}

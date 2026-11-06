//! Runtime-owned authorization. The invalidation handle never exposes tokens.
use crate::auth::AiToken;
use framer_core::{AccountId, port::Session};
use std::{collections::HashMap, sync::{Arc, Mutex, Weak}};

#[derive(Default)]
pub(crate) struct AuthorizationState {
    pub(crate) accounts: Mutex<Vec<(AccountId, AiToken)>>,
    pub(crate) sessions: Mutex<HashMap<(String, String), AiToken>>,
    pub(crate) mint_locks: Mutex<HashMap<(String, String), Weak<tokio::sync::Mutex<()>>>>,
}

/// Cookie-cache invalidation after the caller's session-generation barrier.
/// Default constructs empty state for a standalone native owner, without HTTP.
#[derive(Clone, Default)]
pub struct SessionAuthorization(pub(crate) Arc<AuthorizationState>);

impl SessionAuthorization {
    pub fn invalidate(&self, session: &Session) {
        self.0.sessions.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
            .retain(|(cookie, _), _| cookie != &session.cookie_header);
    }
}

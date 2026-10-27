//! Runtime-owned authorization. The invalidation handle never exposes tokens.
use crate::auth::AiToken;
use framer_core::{AccountId, port::Session};
use std::{collections::HashMap, sync::{Arc, Mutex, Weak}};

#[derive(Default)]
pub(crate) struct AuthorizationState {
    pub(crate) accounts: Mutex<Vec<(AccountId, AiToken)>>,
    pub(crate) sessions: Mutex<HashMap<(String, String), AiToken>>,
    mint_locks: Mutex<HashMap<(String, String), Weak<tokio::sync::Mutex<()>>>>,
}

impl AuthorizationState {
    pub(crate) async fn mint_guard(&self, key: &(String, String))
        -> tokio::sync::OwnedMutexGuard<()> {
        let lock = {
            let mut locks = self.mint_locks.lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            // Token invalidation must not replace a lock still owned or awaited.
            locks.retain(|_, lock| lock.strong_count() > 0);
            if let Some(lock) = locks.get(key).and_then(Weak::upgrade) {
                lock
            } else {
                let lock = Arc::new(tokio::sync::Mutex::new(()));
                locks.insert(key.clone(), Arc::downgrade(&lock));
                lock
            }
        };
        lock.lock_owned().await
    }
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

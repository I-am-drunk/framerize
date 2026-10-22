//! Ordinary client adapter. Preparation is inert; the existing owners follow
//! the invocation through session loading, transport and observed scope close.
use framer_api::{HttpFramerClient, native_dispatch::{Binding, CreditEvidence,
    NativeDispatchProvider}, experimental_native::application::AttemptContext};
use framer_core::{ids::AccountId, native_dispatch::{self as port, DispatchError,
    FailureKind, Identity, InvocationProvider, OwnedInvocation, Policy, Preparation}};
use framer_server::AppState;
use futures::{future::BoxFuture, FutureExt};
use serde_json::Value;
use std::sync::Arc;
use super::{offline_sessions::Sessions, one_shot_control::SourceBuilder};

#[path = "offline_routing_context.rs"]
mod context;
#[path = "offline_routing_run.rs"]
mod running;
#[path = "offline_routing_owned.rs"]
mod owned;
#[path = "offline_routing_methods.rs"]
mod methods;

#[path = "offline_routing_credit.rs"]
pub(crate) mod credit;

pub(crate) struct RequestContext {
    pub identity: Identity,
    pub proxy: Option<String>,
    pub page_url: String,
    pub attempts: Vec<AttemptContext>,
    pub paid_credit: Option<CreditEvidence>,
}

/// Supplies actual application metadata, never an inferred feature or turn ID.
pub(crate) trait Contexts: Send + Sync {
    fn contains_account(&self, account: AccountId) -> bool;
    /// Capture request-local metadata before entering an owned background task.
    fn capture(&self) -> Result<Option<Arc<dyn Contexts>>, DispatchError> { Ok(None) }
    fn resolve(&self, request: &Preparation, payload: &Value)
        -> Result<RequestContext, DispatchError>;
}

type SourceFactory = Arc<dyn Fn(framer_api::experimental_native::control_authorization::OfflineMintTarget)
    -> Arc<dyn SourceBuilder> + Send + Sync>;

#[derive(Clone)]
struct Shared {
    state: AppState, sessions: Sessions, contexts: Arc<dyn Contexts>,
    policy: Policy, streaming: Option<port::StreamingPolicy>, sources: SourceFactory,
    credits: Arc<dyn credit::Reader>,
}

pub(crate) struct Provider(Arc<Shared>);
impl Provider {
    pub(crate) fn new(state: AppState, sessions: Sessions,
        worker: Arc<super::offline_worker_owner::Worker>, contexts: Arc<dyn Contexts>,
        policy: Policy, streaming: Option<port::StreamingPolicy>) -> Result<Self, DispatchError> {
        Self::with_sources(state, sessions, contexts, policy, streaming,
            Arc::new(move |target| Arc::new(super::offline_source::OfflineSource::new(
                worker.clone(), target))))
    }

    pub(crate) fn with_sources(state: AppState, sessions: Sessions, contexts: Arc<dyn Contexts>,
        policy: Policy, streaming: Option<port::StreamingPolicy>, sources: SourceFactory)
        -> Result<Self, DispatchError> {
        policy.validate()?;
        if let Some(streaming) = streaming { streaming.validate()?; }
        if !sessions.matches(&state) || state.operation_ownership.is_none()
            || state.native_dispatch.is_some() {
            return Err(DispatchError::new(FailureKind::InvalidScope));
        }
        Ok(Self(Arc::new(Shared { state, sessions, contexts, policy, streaming, sources,
            credits: Arc::new(credit::Remote) })))
    }

    #[cfg(test)]
    pub(crate) fn with_credits(mut self, credits: Arc<dyn credit::Reader>) -> Self {
        Arc::get_mut(&mut self.0).unwrap().credits = credits;
        self
    }
}

impl InvocationProvider for Provider {
    fn policy(&self) -> Policy { self.0.policy.clone() }
    fn streaming_policy(&self) -> Option<port::StreamingPolicy> { self.0.streaming }
    fn contains_account(&self, account: AccountId) -> bool {
        self.0.contexts.contains_account(account)
    }
    fn supports_pool_admission(&self) -> bool { true }
    fn prepare(&self, request: Preparation, permit: tokio::sync::OwnedSemaphorePermit)
        -> BoxFuture<'_, Result<Box<dyn OwnedInvocation>, DispatchError>> {
        drop((request, permit));
        async { Err(DispatchError::new(FailureKind::Ineligible)) }.boxed()
    }
    fn prepare_in_pool(&self, request: Preparation, permit: tokio::sync::OwnedSemaphorePermit,
        pool: port::PoolAdmission) -> BoxFuture<'_, Result<Box<dyn OwnedInvocation>, DispatchError>> {
        let shared = self.0.clone();
        async move {
            let shared = match shared.contexts.capture()? {
                Some(contexts) => Arc::new(Shared { contexts, ..(*shared).clone() }),
                None => shared,
            };
            if pool.account() != request.identity.account
                || !shared.contexts.contains_account(request.identity.account) {
                return Err(DispatchError::new(FailureKind::InvalidScope));
            }
            if request.max_total_tokens == 0 || request.identity.chat_session.is_empty() {
                return Err(DispatchError::new(FailureKind::InvalidRequest));
            }
            if request.deadline <= tokio::time::Instant::now() {
                return Err(DispatchError::new(FailureKind::Deadline));
            }
            Ok(Box::new(Prepared { shared, request, permit, pool }) as Box<dyn OwnedInvocation>)
        }.boxed()
    }
}

struct Prepared {
    shared: Arc<Shared>, request: Preparation,
    permit: tokio::sync::OwnedSemaphorePermit, pool: port::PoolAdmission,
}

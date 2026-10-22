//! One-shot control authorization with independently observed source cleanup.
//! This does not provide a browserless credential source or enable a provider.
use super::authorization::{CredentialMaterial, CredentialSource, CredentialTarget, PreparedCredentials};
use super::native_account::{AccountScope, PreparationControl, PreparationFailure, PreparationResult};
use framer_core::{ids::AccountId, port::Session};
use futures::{future::BoxFuture, FutureExt};
use std::sync::{Arc, atomic::{AtomicBool, Ordering}};

#[path = "observed_authorization/issue.rs"]
mod issue;
#[cfg(test)]
#[path = "observed_authorization/tests.rs"]
mod tests;
#[cfg(test)]
#[path = "observed_authorization/panic_tests.rs"]
mod panic_tests;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SourceIdentity {
    pub account: AccountId,
    pub generation: u64,
}
impl SourceIdentity {
    pub fn from_scope(scope: &AccountScope) -> Self {
        Self { account: scope.account(), generation: scope.generation() }
    }
}

/// Closed requires the concrete adapter to verify its full incarnation,
/// scope and seal receipt, not just these two identity fields.
pub enum SourceCleanup {
    NotStarted,
    Closed(SourceIdentity),
    Unconfirmed,
}

/// Both operations are sticky and idempotent. Neither is a cleanup receipt.
pub trait SourceControl: Send + Sync {
    fn request_stop(&self);
    fn seal(&self);
}

/// The factory must register its independent supervisor before returning.
/// Dropping a waiter must not cancel that supervisor or release its fence.
pub struct OpenedSource {
    pub scope: AccountScope,
    pub source: Arc<dyn CredentialSource>,
    pub control: Arc<dyn SourceControl>,
    pub cleanup: BoxFuture<'static, SourceCleanup>,
}

pub enum OpenFailure { NotStarted, Unconfirmed }

pub trait ObservedSourceFactory: Send + Sync {
    fn open(&self, scope: &AccountScope) -> Result<OpenedSource, OpenFailure>;
}

/// One preparation for one exact native generation. The existing native
/// provider must acquire the shared account lease before invoking this source.
pub struct ObservedCredentialSource {
    expected: AccountScope,
    factory: Arc<dyn ObservedSourceFactory>,
    used: AtomicBool,
}
impl ObservedCredentialSource {
    pub fn new(expected: AccountScope, factory: Arc<dyn ObservedSourceFactory>) -> Self {
        Self { expected, factory, used: AtomicBool::new(false) }
    }
}
impl CredentialSource for ObservedCredentialSource {
    fn issue<'a>(&'a self, scope: &'a AccountScope, session: &'a Session,
        control: &'a PreparationControl) -> BoxFuture<'a, PreparationResult<CredentialMaterial>> {
        async move {
            let result = self.issue_observed(scope, session, control, None).await;
            PreparationResult { value: result.value.map(|issued| issued.material),
                cleanup_confirmed: result.cleanup_confirmed }
        }.boxed()
    }
    fn issue_for_target<'a>(&'a self, scope: &'a AccountScope, session: &'a Session,
        target: &'a CredentialTarget, control: &'a PreparationControl)
        -> BoxFuture<'a, PreparationResult<PreparedCredentials>> {
        self.issue_observed(scope, session, control, Some(target)).boxed()
    }
}

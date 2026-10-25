//! Keep the supplied pool owner until actual native cleanup completes.
use framer_core::admission::PoolAdmission;
use futures::FutureExt;
use crate::account::client::AccountScope;
use crate::request::types::FailureKind;

pub(super) struct RunningAccountLease { pool: Option<PoolAdmission> }

impl RunningAccountLease {
    pub(super) fn start(mut pool: PoolAdmission) -> Self {
        pool.start();
        Self { pool: Some(pool) }
    }

    pub(super) fn scope_failed(&self, scope: &AccountScope) -> bool {
        matches!(scope.closed().now_or_never(), Some(Err(_)))
    }

    pub(super) fn complete(mut self, scope: &AccountScope) -> Result<(), FailureKind> {
        if self.scope_failed(scope) { return Err(FailureKind::Cleanup); }
        self.pool.take().expect("owned pool admission").complete()
            .map_err(|_| FailureKind::Cleanup)
    }
}

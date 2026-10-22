//! Adapt shared account ownership to preparation and the existing guardians.
//! No external work has started while a prepared lease can be dropped cleanly.
use framer_core::{account_lease::{AccountLeaseCoordinator, LeaseError, OperationLease},
    ids::AccountId, native_dispatch::{DispatchError, FailureKind, PoolAdmission}};
use futures::FutureExt;
use crate::experimental_native::native_account::AccountScope;

pub(super) enum AccountLease {
    Unmanaged,
    // Only a composed child may use this: its outer guardian owns the lease.
    Inherited,
    Prepared(PreparedAccountLease),
}

pub(super) struct PreparedAccountLease {
    lease: Option<OperationLease>, pool: Option<PoolAdmission>,
}
impl Drop for PreparedAccountLease {
    fn drop(&mut self) {
        // Admission, synchronous validation and unused calls have no external
        // side effects. Their ownership must not poison a healthy account.
        if let Some(lease) = self.lease.take() { let _ = lease.complete(); }
    }
}

// The contained OperationLease fails closed on Drop, including guardian loss.
// Never wrap a running lease in PreparedAccountLease again.
pub(super) struct RunningAccountLease {
    lease: Option<OperationLease>, pool: Option<PoolAdmission>,
}
impl RunningAccountLease {
    #[cfg(test)]
    pub(super) fn unmanaged() -> Self { Self { lease: None, pool: None } }

    pub(super) fn scope_failed(&self, scope: &AccountScope) -> bool {
        // Unattached calls keep their existing cleanup contract. Inherited
        // children leave the extra shared-account check to their outer owner.
        (self.lease.is_some() || self.pool.is_some())
            && matches!(scope.closed().now_or_never(), Some(Err(_)))
    }

    pub(super) fn complete(mut self, scope: &AccountScope) -> Result<(), FailureKind> {
        // A close can fail while a streaming guardian waits for returned
        // credits. Recheck at the actual release point; dropping self poisons.
        if self.scope_failed(scope) {
            return Err(FailureKind::Cleanup);
        }
        if let Some(lease) = self.lease.take() {
            lease.complete().map_err(|_| FailureKind::Cleanup)?;
        }
        if let Some(pool) = self.pool.take() { pool.complete()?; }
        Ok(())
    }
}

fn admission_error(error: LeaseError) -> DispatchError {
    use LeaseError::*;
    DispatchError::new(match error {
        InvalidUser | UnknownAccount | BindingMismatch => FailureKind::InvalidScope,
        Closed | Poisoned => FailureKind::Ineligible,
        AccountLimit | UserLimit | UserBytesLimit | QueueFull => FailureKind::Overloaded,
        InvalidLimits | NotOwned | NotDrained | IdentifierExhausted => FailureKind::Internal,
    })
}

impl AccountLease {
    pub(super) fn admit(coordinator: Option<&AccountLeaseCoordinator>,
        account: AccountId, user: &str, pool: Option<PoolAdmission>) -> Result<Self, DispatchError> {
        let Some(coordinator) = coordinator else {
            return Ok(match pool {
                Some(pool) => Self::Prepared(PreparedAccountLease { lease: None, pool: Some(pool) }),
                None => Self::Unmanaged,
            });
        };
        let waiting = coordinator.operation(account, user).map_err(admission_error)?;
        // prepare already holds workspace capacity. Waiting here could deadlock
        // a legacy continuation which owns the account and needs that capacity.
        // now_or_never drops a pending waiter, releasing its bounded queue slot.
        let lease = waiting.now_or_never()
            .ok_or_else(|| DispatchError::new(FailureKind::Overloaded))?
            .map_err(admission_error)?;
        Ok(Self::Prepared(PreparedAccountLease { lease: Some(lease), pool }))
    }

    pub(super) fn into_running(self) -> RunningAccountLease {
        let (lease, mut pool) = match self {
            Self::Prepared(mut prepared) => (prepared.lease.take(), prepared.pool.take()),
            Self::Unmanaged | Self::Inherited => (None, None),
        };
        if let Some(pool) = &mut pool { pool.start(); }
        RunningAccountLease { lease, pool }
    }
}

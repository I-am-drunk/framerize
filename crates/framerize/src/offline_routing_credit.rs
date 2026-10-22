//! Credit evidence belongs to the current session generation and admitted work.
use super::*;
use framer_api::experimental_native::{control_authorization::BoundControlClient,
    native_account::{AccountScope, PreparationControl, PreparationFailure, PreparationResult},
    native_http::NativeEgress};
use framer_core::{ids::TeamId, port::Session};
use tokio::time::Instant;

pub(crate) trait Reader: Send + Sync {
    fn read<'a>(&'a self, scope: &'a AccountScope, session: &'a Session, team: &'a TeamId,
        proxy: Option<&'a str>, control: &'a PreparationControl)
        -> BoxFuture<'a, PreparationResult<CreditEvidence>>;
}

pub(super) struct Remote;
impl Reader for Remote {
    fn read<'a>(&'a self, scope: &'a AccountScope, session: &'a Session, team: &'a TeamId,
        proxy: Option<&'a str>, control: &'a PreparationControl)
        -> BoxFuture<'a, PreparationResult<CreditEvidence>> {
        async move {
            let egress = proxy.map(NativeEgress::Proxy).unwrap_or(NativeEgress::Direct);
            let client = match BoundControlClient::new(scope.clone(), egress) {
                Ok(client) => client,
                Err(_) => return PreparationResult::finished(Err(PreparationFailure::Rejected)),
            };
            let read = client.credit_balance(scope, session, team, control).await;
            PreparationResult { value: read.value.map(|available| CreditEvidence {
                available, valid_until: Instant::now() + std::time::Duration::from_secs(5),
            }), cleanup_confirmed: read.cleanup_confirmed }
        }.boxed()
    }
}

pub(super) async fn read(shared: &Shared, scope: &AccountScope, binding: &RequestContext,
    deadline: Instant, control: &running::Control) -> Result<CreditEvidence, DispatchError> {
    control.stopped(deadline)?;
    let reader = shared.credits.clone();
    let expected = scope.clone();
    let team = binding.identity.team.clone();
    let proxy = binding.proxy.clone();
    let pending = shared.sessions.native().prepare_owned(scope, deadline,
        move |session, work| async move {
            reader.read(&expected, &session, &team, proxy.as_deref(), &work).await
        }).map_err(|_| DispatchError::new(FailureKind::Ineligible))?;
    control.attach_preparation(pending.cancellation());
    let value = pending.await;
    if matches!(value, Err(PreparationFailure::CleanupUnconfirmed | PreparationFailure::WorkerLost)) {
        return Err(DispatchError::new(FailureKind::Cleanup));
    }
    control.stopped(deadline)?;
    let credit = value.map_err(|_| DispatchError::new(FailureKind::Authorization))?;
    if !credit.available.is_finite() || credit.available < shared.policy.credit_floor
        || credit.valid_until <= Instant::now() {
        return Err(DispatchError::new(FailureKind::CreditFloor));
    }
    Ok(credit)
}

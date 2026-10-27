//! Point observation tied to the existing account generation and transport.
use super::*;
use crate::transport::http::{EgressProbeError, EgressProbeRequest, NativeCancellation};
use std::{net::IpAddr, time::Duration};

/// This identifies the sampled route, not every destination or future route.
/// The weak scope does not keep an account or its HTTP pool open.
pub struct NativeEgressObservation {
    scope: AccountScope,
    url: String,
    address: IpAddr,
}

impl AccountScope {
    /// Explicit credential-free HTTPS probe, limited to 30 seconds and 1 KiB.
    /// The returned preparation retains cancellation and cleanup ownership.
    pub fn observe_egress(&self, url: &str, budget: Duration)
        -> Result<PendingPreparation<NativeEgressObservation>, ClientError> {
        let request = EgressProbeRequest::new(url, budget).map_err(|_| ClientError::InvalidContext)?;
        self.begin_egress(request, budget)
    }

    fn begin_egress(&self, request: EgressProbeRequest, budget: Duration)
        -> Result<PendingPreparation<NativeEgressObservation>, ClientError> {
        let inner = self.owner.upgrade().ok_or(ClientError::Unavailable)?;
        let scope = self.clone();
        NativeAccountClient { inner }.prepare_transport_owned(self, Instant::now() + budget,
            move |client, control| async move { observe(client, scope, request, control).await })
    }
}

impl NativeEgressObservation {
    pub fn url(&self) -> &str { &self.url }
    pub fn address(&self) -> IpAddr { self.address }
    pub fn matches_scope(&self, scope: &AccountScope) -> bool { self.scope.same_generation(scope) }
    pub fn owner_is_current(&self) -> bool {
        let Some(owner) = self.scope.owner.upgrade() else { return false; };
        let state = lock(&owner.state);
        state.healthy && state.accounts.iter().any(|account|
            Arc::ptr_eq(&account.identity, &self.scope.identity) && !account.opening
                && !account.closing && account.failure.is_none() && account.resources.is_some())
    }
}

async fn observe(client: Arc<NativeHttp>, scope: AccountScope, request: EgressProbeRequest,
    control: PreparationControl) -> PreparationResult<NativeEgressObservation> {
    let cancellation = NativeCancellation::new();
    let response = client.observe_egress(request, cancellation.clone());
    tokio::pin!(response);
    let result = tokio::select! {
        biased;
        _ = stopped(&control) => { cancellation.cancel(); response.await },
        result = &mut response => result,
    };
    let result = if control.is_cancelled() {
        Err(if Instant::now() >= control.deadline() { PreparationFailure::Deadline }
            else { PreparationFailure::Cancelled })
    } else {
        result.map(|reply| NativeEgressObservation { scope, url: reply.url, address: reply.address })
            .map_err(|error| match error {
                EgressProbeError::Rejected => PreparationFailure::Rejected,
                EgressProbeError::Cancelled => PreparationFailure::Cancelled,
                EgressProbeError::Deadline => PreparationFailure::Deadline,
            })
    };
    PreparationResult::finished(result)
}

async fn stopped(control: &PreparationControl) {
    while !control.is_cancelled() {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

#[cfg(test)]
#[path = "native_egress_tests.rs"]
mod tests;

//! Private account-bound probe exchange before child readiness.
use super::*;
use crate::account::client::{ClientError, NativeEgressObservation, PendingPreparation};

pub(super) struct Endpoint {
    prefix: String,
    #[cfg(test)]
    loopback: bool,
}

impl Endpoint {
    pub(super) fn new(raw: &str) -> Result<Self, &'static str> { Self::validated(raw, false) }

    fn validated(raw: &str, loopback: bool) -> Result<Self, &'static str> {
        const ERROR: &str = "invalid browser control egress endpoint";
        if raw.len() > 2048 || !raw.is_ascii() || raw.contains('\\')
            || raw.bytes().any(|b| b <= 32 || b == 127) { return Err(ERROR); }
        let mut url = reqwest::Url::parse(raw).map_err(|_| ERROR)?;
        let local = loopback && url.scheme() == "http" && url.host_str()
            .and_then(|host| host.parse::<std::net::IpAddr>().ok()).is_some_and(|ip| ip.is_loopback());
        if (url.scheme() != "https" && !local) || url.host_str().is_none()
            || !url.username().is_empty() || url.password().is_some() || url.fragment().is_some()
            || url.as_str() != raw || url.query_pairs().any(|(key, _)| key == "framerize_probe") {
            return Err(ERROR);
        }
        let query = url.query().unwrap_or_default().to_owned();
        url.set_query(None);
        let prefix = format!("{url}?{}framerize_probe=", if query.is_empty() { query }
            else { query + "&" });
        Ok(Self { prefix, #[cfg(test)] loopback })
    }

    fn allows(&self, url: &str) -> bool {
        url.strip_prefix(&self.prefix).is_some_and(|nonce| nonce.len() == 32
            && nonce.bytes().all(|b| b.is_ascii_hexdigit()))
    }

    fn probe(&self, scope: &AccountScope, url: &str, budget: Duration)
        -> Result<PendingPreparation<NativeEgressObservation>, ClientError> {
        #[cfg(test)]
        if self.loopback { return scope.observe_egress_for_test(url, budget); }
        scope.observe_egress(url, budget)
    }
}

impl BrowserControlProcess {
    async fn ack_egress(&self, state: &mut State, observed: &NativeEgressObservation,
        control: &PreparationControl, deadline: Instant) -> bool {
        let Some(process) = state.process.as_mut() else { return false; };
        let ready = tokio::select! {
            biased;
            _ = stopped(control) => return false,
            result = tokio::time::timeout_at(deadline, async {
                process.write(json!({"version": 1, "op": "egress_result",
                    "url": observed.url(), "ip": observed.address().to_string()})).await?;
                process.read().await
            }) => result.ok().and_then(Result::ok),
        };
        matches!(ready, Some(Reply::Ready { version: 1, host_generation,
            incarnation, proxy_revision: _ }) if wire::generation_valid(&host_generation)
                && incarnation > 0)
    }

    async fn exchange_egress(&self, state: &mut State, endpoint: &Endpoint,
        scope: &AccountScope, control: &PreparationControl, deadline: Instant)
        -> Option<NativeEgressObservation> {
        let process = state.process.as_mut()?;
        let first = tokio::select! {
            biased;
            _ = stopped(control) => return None,
            result = tokio::time::timeout_at(deadline, async {
                process.write(json!({"version": 1, "op": "prepare", "binding": &self.binding})).await?;
                process.read().await
            }) => result.ok()?.ok()?,
        };
        let Reply::EgressProbe { version: 1, url, budget_ms } = first else { return None; };
        if !endpoint.allows(&url) || !(1..=30_000).contains(&budget_ms) || control.is_cancelled() {
            return None;
        }
        let budget = Duration::from_millis(budget_ms)
            .min(deadline.saturating_duration_since(Instant::now()));
        let pending = endpoint.probe(scope, &url, budget).ok()?;
        let observed = join_probe(pending, control).await?;
        if !observed.matches_scope(scope) || !observed.owner_is_current() || Instant::now() >= deadline {
            return None;
        }
        self.ack_egress(state, &observed, control, deadline).await.then_some(observed)
    }

    pub(super) async fn prepare_account(&self, scope: &AccountScope, session: &Session,
        control: &PreparationControl) -> bool {
        let Some(endpoint) = &self.native_egress else { return false; };
        if control.is_cancelled() || !self.binding.matches_account(scope, session) { return false; }
        let mut state = self.state.lock().await;
        if state.phase != Phase::Fresh || control.is_cancelled() { return false; }
        state.phase = Phase::Uncertain;
        state.process = Process::start(&self.launch).ok();
        let deadline = control.deadline().min(Instant::now() + self.prepare_limit);
        let observed = self.exchange_egress(&mut state, endpoint, scope, control, deadline).await;
        if let Some(observed) = observed {
            if !control.is_cancelled() && Instant::now() < deadline
                && self.binding.matches_account(scope, session) && observed.owner_is_current() {
                state.native_observation = Some(observed);
                state.phase = Phase::Ready;
                return true;
            }
        }
        state.close(false, self.cleanup_limit).await;
        false
    }
}

async fn join_probe(mut pending: PendingPreparation<NativeEgressObservation>,
    control: &PreparationControl) -> Option<NativeEgressObservation> {
    let cancel = pending.cancellation();
    tokio::select! {
        biased;
        _ = stopped(control) => { cancel.cancel(); let _ = pending.await; None },
        result = &mut pending => result.ok(),
    }
}

#[cfg(test)]
#[path = "browser_control_egress_tests.rs"]
mod tests;

//! Startup retains the same account lease later used by native requests.
use super::*;
use crate::account::{client::PreparationResult, sessions::AccountLease};
use crate::service::sessions::Sessions;
use crate::transport::http::{NativeEgress, NativeHttpLimits};
use tokio::time::{Duration, Instant};

pub(super) async fn prepare(peer: Arc<dyn BrowserCredentialControl>, sessions: &Sessions,
    state: &ServiceState, id: AccountId, stop: impl Future<Output = ()> + Send)
    -> Result<Option<AccountLease>, String> {
    let stop = stop.boxed().shared();
    let deadline = Instant::now() + Duration::from_secs(180);
    if stop.clone().now_or_never().is_some() { return Ok(None); }
    if !state.serving_accounts.allows(&id) {
        return Err("browser credential control account is not selected".into());
    }
    let account = state.state.get(id).await
        .map_err(|_| "browser credential control account read failed")?
        .ok_or("browser credential control account unavailable")?;
    if account.id != id { return Err("browser credential control account mismatch".into()); }
    if stop.clone().now_or_never().is_some() { return Ok(None); }
    let route = account.proxy.as_deref().map(NativeEgress::Proxy).unwrap_or(NativeEgress::Direct);
    let lease = sessions.bridge().lease_account_observed(id, account.framer_user_id,
        NativeHttpLimits::default(), route).await
        .map_err(|_| "browser credential control session unavailable")?;
    if stop.clone().now_or_never().is_some() {
        return if lease.finish(false).await { Ok(None) }
            else { Err("browser credential control account cleanup is unconfirmed".into()) };
    }
    let scope = lease.scope().clone();
    let expected = scope.clone();
    let pending = sessions.native().prepare_owned(&scope, deadline, move |session, control| async move {
        let ready = peer.prepare_for_account(&expected, &session, &control).await;
        PreparationResult::finished(Ok(ready))
    });
    let mut pending = match pending {
        Ok(pending) => pending,
        Err(_) => {
            if !lease.finish(false).await {
                return Err("browser credential control account cleanup is unconfirmed".into());
            }
            return Err("browser credential control preparation unavailable".into());
        }
    };
    let cancel = pending.cancellation();
    let (ready, stopped) = tokio::select! {
        biased;
        _ = stop.clone() => { cancel.cancel(); (pending.await, true) },
        result = &mut pending => (result, false),
    };
    if ready == Ok(true) && !stopped && !cancel.is_cancelled()
        && stop.now_or_never().is_none() {
        return Ok(Some(lease));
    }
    if !lease.finish(false).await {
        return Err("browser credential control account cleanup is unconfirmed".into());
    }
    if stopped { Ok(None) }
    else { Err("browser credential control preparation failed".into()) }
}

pub(super) async fn close(sessions: Option<Sessions>, lease: Option<AccountLease>)
    -> Result<(), String> {
    let sessions = match sessions {
        Some(sessions) => AssertUnwindSafe(sessions.shutdown()).catch_unwind().await
            .unwrap_or(Err("native session shutdown panicked")),
        None => Ok(()),
    };
    let lease = match lease {
        Some(lease) => AssertUnwindSafe(lease.finish(false)).catch_unwind().await.unwrap_or(false),
        None => true,
    };
    if !lease { return Err("browser credential control account cleanup is unconfirmed".into()); }
    sessions.map_err(str::to_owned)
}

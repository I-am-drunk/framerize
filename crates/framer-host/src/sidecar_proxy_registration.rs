use super::*;
use framer_core::browser_lifecycle::{BrowserAccountState, BrowserAccountStatus};
use serde::{Deserialize, Serialize};
use std::time::Duration;

const REGISTER_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ProxyRoute {
    pub revision: u64,
    pub proxy: Option<String>,
}

#[derive(Default)]
pub(super) struct ProxyState {
    pub current: ProxyRoute,
    pub pending: Option<ProxyRoute>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RegistrationReply {
    outcome: String,
    route: ProxyRoute,
    status: BrowserAccountStatus,
}

impl SidecarBridge {
    pub(super) async fn route_state(&self, account: &AccountId) -> Arc<Mutex<ProxyState>> {
        self.routes.lock().await.entry(*account).or_default().clone()
    }

    pub(super) async fn route_snapshot(&self, account: &AccountId) -> Result<ProxyRoute, String> {
        let cell = self.route_state(account).await;
        let state = cell.lock().await;
        if state.pending.is_some() {
            return Err("account proxy registration is unconfirmed; retry registration before use".into());
        }
        Ok(state.current.clone())
    }

    pub(super) async fn route_for_command(&self, account: &AccountId) -> Result<ProxyRoute, String> {
        let _admission = self.admit_unscoped()?;
        let cell = self.route_state(account).await;
        let state = cell.lock().await;
        if state.pending.is_some() {
            return Err("account proxy registration is unconfirmed; retry registration before use".into());
        }
        // Registration cannot commit locally while an older command is
        // preparing to start the host. The command's revision is then fenced
        // by that host even if its actual write is delayed or cancelled.
        self.sidecar().await?;
        Ok(state.current.clone())
    }

    pub(super) async fn register_proxy(&self, account: &AccountId, proxy: Option<String>)
        -> Result<(), String> {
        let _admission = self.admit_unscoped()?;
        if proxy.as_ref().map_or(false, String::is_empty) {
            return Err("proxy must be a nonempty string or null".into());
        }
        tokio::time::timeout(REGISTER_TIMEOUT, async {
            let cell = self.route_state(account).await;
            let mut state = cell.lock().await;
            if state.pending.is_some() { self.settle_registration(account, &mut state).await?; }
            if state.current.proxy == proxy {
                let inner = self.inner.lock().await;
                if inner.is_none() { self.check_unused_startup()?; }
                return Ok(());
            }
            let revision = state.current.revision.checked_add(1)
                .ok_or("account proxy revision exhausted")?;
            state.pending = Some(ProxyRoute { revision, proxy });
            self.settle_registration(account, &mut state).await
        }).await.map_err(|_| "account proxy registration timed out; retry registration before use".to_owned())?
    }

    async fn settle_registration(&self, account: &AccountId, state: &mut ProxyState)
        -> Result<(), String> {
        let next = state.pending.clone().ok_or("no pending proxy registration")?;
        {
            let inner = self.inner.lock().await;
            if inner.is_none() {
                self.check_unused_startup()?;
                state.current = next;
                state.pending = None;
                return Ok(());
            }
        }
        let value = self.call_with_policy("register_proxy", json!({
            "account": account.0.to_string(), "expected_revision": state.current.revision,
            "revision": next.revision, "proxy": next.proxy,
        }), REGISTER_TIMEOUT, false).await?;
        if value.get("route").and_then(Value::as_object)
            .map_or(true, |route| !route.contains_key("proxy")) {
            return Err("malformed account proxy registration reply".into());
        }
        let reply: RegistrationReply = serde_json::from_value(value)
            .map_err(|_| "malformed account proxy registration reply".to_owned())?;
        if reply.status.account_id != *account
            || reply.status.host_generation.as_ref().map_or(true, String::is_empty)
            || (reply.status.account_incarnation == 0
                && reply.status.state != BrowserAccountState::Absent) {
            return Err("account proxy registration identity mismatch".into());
        }
        if reply.outcome == "registered" && reply.route == next
            && reply.status.state == BrowserAccountState::Absent && reply.status.active_commands == 0 {
            state.current = next;
            state.pending = None;
            return Ok(());
        }
        if reply.outcome == "busy" && reply.route == state.current { state.pending = None; }
        Err("account proxy change requires confirmed browser closure".into())
    }
}

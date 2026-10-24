//! Standalone child of an external admission/cooldown owner. Never a daemon.
use crate::one_shot_control::{comparison as pair, Failure, Selection};
use futures::future::BoxFuture as FutureResult;
use framer_core::{ids::FramerUserId, port::{Session, SessionStore}};
use framer_host::sidecar::{SidecarBridge, observer::ObserverLimits};
use serde::Deserialize;
use serde_json::json;
use std::{sync::{Arc, Mutex}, time::Duration};
use tokio::sync::Semaphore;
#[path = "browser_comparison_ipc.rs"]
mod ipc;
#[path = "browser_comparison_plan.rs"]
mod plan;
#[path = "browser_comparison_report.rs"]
mod report;
#[cfg(test)]
#[path = "browser_comparison_child_tests.rs"]
mod tests;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Start { command: String, nonce: String, session_cookie: String }
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Recorded { command: String, nonce: String }

struct PrivateStore(Mutex<Option<Session>>);
struct ParentWindow {
    nonce: String,
    selected: Mutex<Option<Selection>>,
    store: Arc<PrivateStore>,
    channel: tokio::sync::Mutex<ipc::Channel>,
}

impl SessionStore for PrivateStore {
    fn load<'a, 'b, 'f>(&'a self, user: &'b FramerUserId)
        -> FutureResult<'f, Result<Option<Session>, String>> where 'a: 'f, 'b: 'f, Self: 'f {
        Box::pin(async move {
            match self.0.lock().map_err(|_| "comparison store unavailable")?.take() {
                Some(session) if session.active_user == *user => Ok(Some(session)),
                _ => Err("comparison session unavailable".into()),
            }
        })
    }
    fn save<'a, 'b, 'f>(&'a self, _: &'b Session)
        -> FutureResult<'f, Result<(), String>> where 'a: 'f, 'b: 'f, Self: 'f {
        Box::pin(async { Err("comparison store is read only".into()) })
    }
    fn delete<'a, 'b, 'f>(&'a self, _: &'b FramerUserId)
        -> FutureResult<'f, Result<(), String>> where 'a: 'f, 'b: 'f, Self: 'f {
        Box::pin(async { Err("comparison store is read only".into()) })
    }
}

impl pair::ComparisonWindow for ParentWindow {
    fn admit(&self) -> FutureResult<'_, Result<Selection, Failure>> {
        Box::pin(async {
            let selected = self.selected.lock().map_err(|_| Failure::Admission)?
                .take().ok_or(Failure::Admission)?;
            let mut channel = self.channel.lock().await;
            channel.send(&json!({"command":"ready", "nonce":self.nonce})).await
                .map_err(|_| Failure::Admission)?;
            let start: Start = channel.receive().await.map_err(|_| Failure::Admission)?;
            if start.command != "start" || start.nonce != self.nonce
                || start.session_cookie.is_empty() || start.session_cookie.len() > 16 * 1024
                || start.session_cookie.contains(['\r', '\n', '\0']) {
                return Err(Failure::Admission);
            }
            *self.store.0.lock().map_err(|_| Failure::Session)? = Some(Session {
                active_user: selected.identity.user.clone(), cookie_header: start.session_cookie,
            });
            Ok(selected)
        })
    }
    fn record(&self, value: &pair::Report) -> FutureResult<'_, Result<(), Failure>> {
        let value = report::bounded(value);
        Box::pin(async move {
            let mut channel = self.channel.lock().await;
            channel.send(&json!({"command":"result", "nonce":self.nonce, "report":value}))
                .await.map_err(|_| Failure::CooldownPublication)?;
            let reply: Recorded = channel.receive().await.map_err(|_| Failure::CooldownPublication)?;
            if reply.command != "recorded" || reply.nonce != self.nonce {
                return Err(Failure::CooldownPublication);
            }
            Ok(())
        })
    }
}

pub(crate) async fn execute(fd: i32) -> Result<bool, ()> {
    use std::os::unix::fs::MetadataExt;
    let mut channel = ipc::Channel::inherited(fd)?;
    let plan: plan::WirePlan = channel.receive().await?;
    let spec = plan.specification()?;
    if !plan.state_parent.is_absolute() || !plan.host_script.is_absolute()
        || std::fs::canonicalize(&plan.state_parent).map_err(|_| ())? != plan.state_parent
        || std::fs::canonicalize(&plan.host_script).map_err(|_| ())? != plan.host_script {
        return Err(());
    }
    let metadata = std::fs::metadata(&plan.state_parent).map_err(|_| ())?;
    if !metadata.is_dir() || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.mode() & 0o077 != 0 { return Err(()); }
    let bridge = Arc::new(SidecarBridge::new_observed_one_shot(
        &plan.state_parent, &plan.host_script).map_err(|_| ())?);
    let observer = bridge.observer_factory(ObserverLimits { scopes: 1, commands: 2,
        cleanup_timeout: Duration::from_secs(30) }).map_err(|_| ())?;
    let store = Arc::new(PrivateStore(Mutex::new(None)));
    let authority = Arc::new(ParentWindow { nonce: plan.nonce.clone(),
        selected: Mutex::new(Some(Selection { identity: spec.identity.clone(),
            context: plan.context.into_attempt(), proxy: plan.proxy })),
        store: store.clone(), channel: tokio::sync::Mutex::new(channel) });
    let permit = Arc::new(Semaphore::new(1)).try_acquire_owned().map_err(|_| ())?;
    let value = pair::launch(spec, plan.baseline_turn, pair::Dependencies {
        authority: authority.clone(), store, observer, permit }).map_err(|_| ())?.await;
    let success = locally_complete(&value);
    authority.channel.lock().await.send(&json!({"command":"done", "nonce":plan.nonce,
        "local_pair_success":success, "failure":value.failure.map(|f| format!("{f:?}"))})).await?;
    Ok(success)
}

fn locally_complete(value: &pair::Report) -> bool {
    value.failure.is_none() && value.account_closed && value.browser_cleanup_confirmed
        && value.native_nonempty_text
        && value.baseline.as_ref().is_some_and(|b| b.accepted)
        && value.native.as_ref().is_some_and(|n| n.status == Some(200)
            && n.failure.is_none() && n.complete_json && n.local_cleanup
            && n.attempts == 1 && n.authorization_calls == 1 && n.transport_starts == 1)
}

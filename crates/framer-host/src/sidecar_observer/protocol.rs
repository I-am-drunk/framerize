use super::*;
use state::ScopeState;

async fn quiet(state: &ScopeState) {
    loop {
        let changed = state.changed.notified();
        tokio::pin!(changed); changed.as_mut().enable();
        if state.lock_phase().active == 0 { return; }
        changed.await;
    }
}

async fn denied(state: &ScopeState, reason: ObserverError) -> CleanupOutcome {
    state.lock_phase().setup_error = Some(reason);
    state.stop();
    quiet(state).await;
    CleanupOutcome::NotStarted { reason: Some(reason) }
}

fn same_scope(v: &Value, state: &ScopeState) -> bool {
    v["scope"] == state.id.to_string()
        && v["account"] == state.account.0.to_string()
        && v["application_generation"] == state.application.0
}

pub(super) async fn run(base: Arc<SidecarBridge>, state: Arc<ScopeState>) -> CleanupOutcome {
    loop {
        let changed = state.changed.notified();
        tokio::pin!(changed); changed.as_mut().enable();
        let (started, sealed) = {
            let p = state.lock_phase(); (p.started, p.sealed)
        };
        if started { break; }
        if sealed { return CleanupOutcome::NotStarted { reason: None }; }
        changed.await;
    }
    if state.lock_phase().stop {
        return denied(&state, ObserverError::Stopped).await;
    }
    let (connection, events) = match wire::connect(&base).await {
        Ok(v) => v,
        Err(e) => return denied(&state, e).await,
    };
    state.lock_phase().connection = Some(connection.clone());
    if let Err(e) = connection.negotiate().await { return denied(&state, e).await; }
    if state.lock_phase().stop {
        return denied(&state, ObserverError::Stopped).await;
    }
    opened(state, connection, events).await
}

async fn opened(state: Arc<ScopeState>, c: Arc<wire::Connection>,
    mut events: broadcast::Receiver<RemoteEvent>) -> CleanupOutcome {
    let opened = c.call("scope_open", state.identity(), None).await;
    match opened {
        Ok(Ok(v)) if same_scope(&v, &state) && v["opened"] == true => {},
        Err(e) => return CleanupOutcome::Unconfirmed(e),
        _ => return CleanupOutcome::Unconfirmed(ObserverError::Protocol),
    }
    state.lock_phase().admitted = true;
    state.changed.notify_waiters();
    let stop = loop {
        let changed = state.changed.notified();
        tokio::pin!(changed); changed.as_mut().enable();
        {
            let p = state.lock_phase();
            if p.stop { break true; }
            if p.sealed && p.active == 0 { break false; }
        }
        tokio::select! {
            _ = changed => {},
            event = events.recv() => match event {
                Ok(RemoteEvent::Cleanup { scope, .. }) if scope != state.id => {},
                _ => return CleanupOutcome::Unconfirmed(ObserverError::GenerationLost),
            }
        }
    };
    let finish = Uuid::new_v4();
    let mut fields = state.identity();
    fields["finish"] = json!(finish.to_string());
    let cmd = if stop { "scope_stop" } else { "scope_seal" };
    match c.call(cmd, fields, None).await {
        Ok(Ok(v)) if same_scope(&v, &state) && v["sealed"] == true
            && v["finish"] == finish.to_string() => {},
        Err(e) => return CleanupOutcome::Unconfirmed(e),
        _ => return CleanupOutcome::Unconfirmed(ObserverError::Protocol),
    }
    finish_scope(&state, &c, &mut events, finish).await
}

async fn finish_scope(state: &ScopeState, c: &wire::Connection,
    events: &mut broadcast::Receiver<RemoteEvent>, finish: Uuid) -> CleanupOutcome {
    loop {
        match events.recv().await {
            Ok(RemoteEvent::Cleanup { scope, .. }) if scope != state.id => continue,
            Ok(RemoteEvent::Cleanup { account, scope, generation, finish: token, clean })
                if scope == state.id && account == state.account
                    && generation == state.application && token == finish => {
                if !clean { return CleanupOutcome::Unconfirmed(ObserverError::Transport); }
                break;
            }
            _ => return CleanupOutcome::Unconfirmed(ObserverError::Protocol),
        }
    }
    quiet(state).await;
    let mut fields = state.identity();
    fields["finish"] = json!(finish.to_string());
    match c.call("scope_forget", fields, None).await {
        Ok(Ok(v)) if same_scope(&v, state) && v["forgotten"] == true
            && v["finish"] == finish.to_string() && c.alive() => {},
        Err(e) => return CleanupOutcome::Unconfirmed(e),
        _ => return CleanupOutcome::Unconfirmed(ObserverError::Protocol),
    }
    CleanupOutcome::Closed(CleanupReceipt {
        account: state.account, application_generation: state.application,
        sidecar_incarnation: c.incarnation, scope: state.id,
        extent: CleanupExtent::AccountBrowserClosed,
    })
}

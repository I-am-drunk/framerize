use framer_server::AppState;
use std::net::SocketAddr;

pub async fn serve(state: AppState, web_dir: &str, addr: SocketAddr,
    stop: impl std::future::Future<Output = ()> + Send + 'static,
) -> Result<(), String> {
    let ownership = state.operation_ownership.clone()
        .ok_or("application request ownership is unavailable")?;
    let sealing = ownership.clone();
    let (sender, receiver) = tokio::sync::oneshot::channel();
    // Retain the runtime, including the private-store lock, through host join.
    let result = framer_server::serve_with_shutdown(state.clone(), web_dir, addr, async move {
        stop.await;
        let _ = sender.send(sealing.seal());
    }).await;
    // Bind failure drops the signal future. Startup work still needs draining.
    let ticket = receiver.await.unwrap_or_else(|_| ownership.seal());
    let closed = close_browser(&state, ticket).await;
    match (result, closed) {
        (Ok(()), closed) => closed,
        (Err(error), Ok(())) => Err(error.to_string()),
        (Err(error), Err(close)) => Err(format!("{error}; shutdown: {close}")),
    }
}

pub async fn close(state: AppState) -> Result<(), String> {
    let ownership = state.operation_ownership.as_ref()
        .ok_or("application request ownership is unavailable")?;
    close_browser(&state, ownership.seal()).await
}

async fn close_browser(state: &AppState,
    ticket: Result<framer_core::operation_gate::DrainTicket, framer_core::operation_gate::GateError>,
) -> Result<(), String> {
    let mut ticket = match ticket {
        Ok(ticket) => ticket,
        Err(error) => { let _ = reap_workers(state, true).await;
            return Err(format!("shutdown admission: {error:?}")); }
    };
    let _drained = match ticket.wait_application().await {
        Ok(drained) => drained,
        Err(error) => { let _ = reap_workers(state, true).await;
            return Err(format!("shutdown drain: {error:?}")); }
    };
    reap_workers(state, false).await?;
    state.browser.shutdown().await
}

async fn reap_workers(state: &AppState, force: bool) -> Result<(), String> {
    match state.policy_generations() {
        Some(manager) => manager.shutdown(force).await
            .map_err(|error| format!("shutdown policy workers: {error:?}")),
        None => Ok(()),
    }
}

#[cfg(test)]
#[path = "shutdown_tests.rs"]
mod tests;

pub fn signal() -> std::io::Result<impl std::future::Future<Output = ()> + Send> {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        let mut terminate = signal(SignalKind::terminate())?;
        let mut interrupt = signal(SignalKind::interrupt())?;
        Ok(async move {
            tokio::select! {
                _ = interrupt.recv() => {}
                _ = terminate.recv() => {}
            }
        })
    }
    #[cfg(not(unix))]
    Ok(async {
        if let Err(error) = tokio::signal::ctrl_c().await {
            eprintln!("shutdown signal failed: {error}");
        }
    })
}

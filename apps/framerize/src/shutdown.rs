use framer_server::AppState;
use std::net::SocketAddr;

pub async fn serve(state: AppState, web_dir: &str, addr: SocketAddr,
    stop: impl std::future::Future<Output = ()> + Send + 'static,
) -> Result<(), String> {
    let ownership = state.operation_ownership.clone()
        .ok_or("application request ownership is unavailable")?;
    let browser = state.browser.clone();
    let sealing = ownership.clone();
    let (sender, receiver) = tokio::sync::oneshot::channel();
    // Retain the runtime, including the private-store lock, through host join.
    let result = framer_server::serve_with_shutdown(state.clone(), web_dir, addr, async move {
        stop.await;
        let _ = sender.send(sealing.seal());
    }).await;
    // Bind failure drops the signal future. Startup work still needs draining.
    let ticket = receiver.await.unwrap_or_else(|_| ownership.seal())
        .map_err(|error| format!("shutdown admission: {error:?}"))?;
    let closed = close_browser(browser, ticket).await;
    match (result, closed) {
        (Ok(()), closed) => closed,
        (Err(error), Ok(())) => Err(error.to_string()),
        (Err(error), Err(close)) => Err(format!("{error}; shutdown: {close}")),
    }
}

pub async fn close(state: AppState) -> Result<(), String> {
    let ownership = state.operation_ownership.as_ref()
        .ok_or("application request ownership is unavailable")?;
    let ticket = ownership.seal().map_err(|error| format!("shutdown admission: {error:?}"))?;
    close_browser(state.browser.clone(), ticket).await
}

async fn close_browser(browser: std::sync::Arc<dyn framer_core::port::BrowserBridge>,
    mut ticket: framer_core::operation_gate::DrainTicket,
) -> Result<(), String> {
    let _drained = ticket.wait_application().await
        .map_err(|error| format!("shutdown drain: {error:?}"))?;
    browser.shutdown().await
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

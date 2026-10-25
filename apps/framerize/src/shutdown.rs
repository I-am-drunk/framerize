use framer_server::AppState;
use std::net::SocketAddr;

pub async fn serve(state: AppState, web_dir: &str, addr: SocketAddr) -> Result<(), String> {
    serve_until(state, web_dir, addr, signal()).await
}

async fn serve_until(state: AppState, web_dir: &str, addr: SocketAddr,
    stop: impl std::future::Future<Output = ()> + Send + 'static,
) -> Result<(), String> {
    let ownership = state.operation_ownership.clone()
        .ok_or("application request ownership is unavailable")?;
    let browser = state.browser.clone();
    let sealing = ownership.clone();
    let (sender, receiver) = tokio::sync::oneshot::channel();
    let result = framer_server::serve_with_shutdown(state, web_dir, addr, async move {
        stop.await;
        let _ = sender.send(sealing.seal());
    }).await;
    // Bind failure drops the signal future. Startup work still needs draining.
    let mut ticket = receiver.await.unwrap_or_else(|_| ownership.seal())
        .map_err(|error| format!("shutdown admission: {error:?}"))?;
    let _drained = ticket.wait_application().await
        .map_err(|error| format!("shutdown drain: {error:?}"))?;
    let closed = browser.shutdown().await;
    result.map_err(|error| error.to_string())?;
    closed
}

#[cfg(test)]
#[path = "shutdown_tests.rs"]
mod tests;

async fn signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        let mut terminate = match signal(SignalKind::terminate()) {
            Ok(signal) => signal,
            Err(error) => {
                eprintln!("cannot listen for shutdown: {error}");
                return;
            }
        };
        tokio::select! {
            result = tokio::signal::ctrl_c() => {
                if let Err(error) = result { eprintln!("shutdown signal failed: {error}"); }
            }
            _ = terminate.recv() => {}
        }
    }
    #[cfg(not(unix))]
    if let Err(error) = tokio::signal::ctrl_c().await {
        eprintln!("shutdown signal failed: {error}");
    }
}

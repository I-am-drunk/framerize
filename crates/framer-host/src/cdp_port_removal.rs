use super::CdpAccountRoute;
use std::{io, path::PathBuf};
use tokio::sync::OwnedMutexGuard;

type RouteGuard = OwnedMutexGuard<CdpAccountRoute>;

pub(super) async fn remove(path: PathBuf, route: RouteGuard)
    -> Result<(RouteGuard, io::Result<()>), String> {
    remove_with(route, move || std::fs::remove_file(path)).await
}

async fn remove_with<F>(route: RouteGuard, removal: F)
    -> Result<(RouteGuard, io::Result<()>), String>
where F: FnOnce() -> io::Result<()> + Send + 'static {
    // A cancelled waiter cannot cancel blocking filesystem work. Keep the
    // account locked until removal actually ends, including its failure path.
    // Return the guard without a gap so startup keeps its original exclusion.
    tokio::task::spawn_blocking(move || {
        let result = removal();
        (route, result)
    }).await.map_err(|_| "could not remove stale DevToolsActivePort".to_owned())
}

#[cfg(test)]
#[path = "cdp_port_removal_tests.rs"]
mod tests;

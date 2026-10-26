use super::{CdpAccountRoute, DeviceRecord};
use std::{future::Future, path::PathBuf};
use tokio::sync::OwnedMutexGuard;

type RouteGuard = OwnedMutexGuard<CdpAccountRoute>;

pub(super) async fn prepare(profile: PathBuf, route: RouteGuard)
    -> Result<(RouteGuard, String), String> {
    let (route, result) = retain(route, async move {
        tokio::fs::create_dir_all(&profile).await.map_err(|e| e.to_string())?;
        DeviceRecord::load_or_create(&profile).await
    }).await?;
    Ok((route, result?))
}

async fn retain<F>(route: RouteGuard, preparation: F)
    -> Result<(RouteGuard, Result<String, String>), String>
where F: Future<Output = Result<String, String>> + Send + 'static {
    // Filesystem work may outlive a cancelled async waiter. Retain both its
    // waiter and the account guard until preparation has actually finished.
    tokio::spawn(async move {
        let result = preparation.await;
        (route, result)
    }).await.map_err(|_| "browser device preparation task failed".to_owned())
}

#[cfg(test)]
#[path = "cdp_device_preparation_tests.rs"]
mod tests;

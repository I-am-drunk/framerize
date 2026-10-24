//! Join the separately sandboxed SDK child before trusting its token or cleanup.
use super::*;
use offline_worker_owner::{Cleanup, SavedDevice, Target, V2Worker, V2Metadata};

#[cfg(test)]
#[path = "resident_sdk_cancel_tests.rs"]
mod cancel_tests;

async fn wait_for_result(run: &mut offline_worker_owner::Run<V2Metadata>,
    stop: impl std::future::Future<Output = ()>)
    -> Result<offline_worker_owner::Minted<V2Metadata>, Failure> {
    let result = {
        let wait = run.result();
        tokio::pin!(wait);
        tokio::select! {
            biased;
            _ = stop => Err(Failure::Cancelled),
            result = &mut wait => result.map_err(failure),
        }
    };
    // An immediately ready stop can win before result() installs its drop guard.
    if matches!(result, Err(Failure::Cancelled | Failure::Deadline)) {
        run.cancel();
    }
    result
}

fn failure(error: offline_worker_owner::Failure) -> Failure {
    if let offline_worker_owner::Failure::V2WorkerStage(stage) = error {
        let mut event = framer_core::observability::Event::new("resident.v2.failed")
            .field("phase", stage.phase());
        if let Some(label) = stage.operation() { event = event.field("operation", label); }
        if let Some(label) = stage.device() { event = event.field("device", label); }
        event.emit();
    }
    Failure::Mint
}

pub(super) async fn mint(worker: &V2Worker, page: &CapturedPage,
    target: &CredentialTarget, control: &PreparationControl, deadline: Instant,
    report: &mut Report) -> Result<offline_worker_owner::Minted<V2Metadata>, Failure> {
    active_until(control, deadline)?;
    let device = SavedDevice::new(page.device_id.clone()).map_err(|_| Failure::Context)?;
    let mut run = worker.begin_with_device(Target { project_id: target.project.0.clone(),
        path: target.path.into(), page_url: page.page_url.clone() }, &device,
        deadline.into_std()).map_err(|_| Failure::Mint)?;
    report.cleanup_confirmed = false;
    let completion = run.completion();
    let result = wait_for_result(&mut run, cancelled(control, deadline)).await;
    let cleanup = completion.wait().await;
    report.cleanup_confirmed = cleanup != Cleanup::Unconfirmed;
    if !report.cleanup_confirmed { return Err(Failure::Cleanup); }
    active_until(control, deadline)?;
    let minted = result?;
    if !matches!(cleanup, Cleanup::Joined { .. })
        || minted.freshness_origin > minted.received_at
        || minted.received_at > std::time::Instant::now() {
        return Err(Failure::Mint);
    }
    Ok(minted)
}

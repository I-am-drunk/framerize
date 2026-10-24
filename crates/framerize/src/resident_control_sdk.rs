//! Join the separately sandboxed SDK child before trusting its token or cleanup.
use super::*;
use offline_worker_owner::{Cleanup, SavedDevice, Target, V2Worker, V2Metadata};

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
    let result = {
        let wait = run.result();
        tokio::pin!(wait);
        tokio::select! {
            biased;
            _ = cancelled(control, deadline) => Err(Failure::Cancelled),
            result = &mut wait => result.map_err(failure),
        }
    }; // Dropping a cancelled result waiter requests worker cancellation.
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

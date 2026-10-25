use super::*;

struct RefusingHost {
    observed: Arc<Bridge>,
    before_launch: bool,
}

#[async_trait::async_trait]
impl DeviceOwner for RefusingHost {
    async fn admit_native_mint(&self, grant: &DeviceGrant, budget_ms: u64)
        -> Result<u64, DeviceFailure> {
        let sequence = self.observed.admit_native_mint(grant, budget_ms).await?;
        if self.before_launch { Err(DeviceFailure::Rejected) } else { Ok(sequence) }
    }

    async fn finish_native_mint(&self, grant: &DeviceGrant, sequence: u64,
        joined: bool) -> Result<bool, DeviceFailure> {
        assert!(!self.before_launch, "refused admission must not finish a mint");
        assert!(self.observed.finish_native_mint(grant, sequence, joined).await?);
        Ok(false)
    }

}

fn refusing_minter(scope: &AccountScope, worker: Arc<V2Worker>,
    observed: &Arc<Bridge>, before_launch: bool) -> Arc<OfflineWorkerMinter> {
    let host = Arc::new(RefusingHost { observed: observed.clone(), before_launch });
    let seed = V2SeedBinding::new(observed.grant.clone(),
        SavedDevice::new("00112233445546778899aabbccddeeff".into()).unwrap());
    Arc::new(OfflineWorkerMinter::with_v2_seed(scope.clone(), worker,
        host, observed.grant.clone(), seed).unwrap())
}

#[tokio::test]
async fn v2_host_admission_refusal_starts_no_worker() {
    let (fixture, worker) = fixture();
    let (client, scope) = account();
    let bridge = Bridge::new(&scope, false);
    let minter = refusing_minter(&scope, worker, &bridge, true);
    let pending = prepare(&client, &scope, minter, "success");
    let completion = pending.completion();
    let result = pending.await;
    tokio::time::timeout(Duration::from_secs(3), completion).await.unwrap().unwrap();
    close(&client, &scope).await;
    assert!(matches!(result, Err(PreparationFailure::Rejected)));
    assert_eq!(bridge.admitted.load(Ordering::SeqCst), 1);
    assert_eq!(bridge.finished.load(Ordering::SeqCst), 0);
    assert!(!fixture.root.join("starts").exists());
}

#[tokio::test]
async fn v2_host_rejection_after_join_discards_token() {
    let (fixture, worker) = fixture();
    let (client, scope) = account();
    let bridge = Bridge::new(&scope, false);
    let minter = refusing_minter(&scope, worker, &bridge, false);
    let pending = prepare(&client, &scope, minter, "success");
    let completion = pending.completion();
    bridge.observe(&fixture).await;
    release(&fixture);
    bridge.at_acknowledgement().await;
    assert!(completion.clone().now_or_never().is_none());
    bridge.acknowledge.add_permits(1);
    let result = pending.await;
    tokio::time::timeout(Duration::from_secs(3), completion).await.unwrap().unwrap();
    close(&client, &scope).await;
    assert!(matches!(result, Err(PreparationFailure::Rejected)));
    assert!(bridge.replied.load(Ordering::SeqCst));
    assert_eq!(bridge.admitted.load(Ordering::SeqCst), 1);
    assert_eq!(bridge.finished.load(Ordering::SeqCst), 1);
    assert_eq!(fs::read_to_string(fixture.root.join("starts")).unwrap(), "1\n");
}

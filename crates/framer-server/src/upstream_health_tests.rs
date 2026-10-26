use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

#[path = "permit_lifetime_fixture.rs"]
mod fixture;

struct ObservedState {
    inner: Arc<dyn StateStore>,
    writes: AtomicUsize,
}

#[async_trait::async_trait]
impl StateStore for ObservedState {
    async fn get(&self, id: AccountId) -> Result<Option<ManagedAccount>, String> {
        self.inner.get(id).await
    }
    async fn list(&self) -> Result<Vec<ManagedAccount>, String> {
        self.inner.list().await
    }
    async fn upsert(&self, account: &ManagedAccount) -> Result<(), String> {
        self.writes.fetch_add(1, Ordering::SeqCst);
        self.inner.upsert(account).await
    }
    async fn remove(&self, id: AccountId) -> Result<(), String> {
        self.writes.fetch_add(1, Ordering::SeqCst);
        self.inner.remove(id).await
    }
}

#[tokio::test]
async fn upstream_403_multibyte_diagnostic_preserves_account_health_and_writes() {
    let mut f = fixture::Fixture::new();
    f.accounts[0].consecutive_failures = 3;
    f.accounts[0].cooldown_step = 2;
    f.accounts[0].cooling_until_ms = Some(u64::MAX);
    f.state.state.upsert(&f.accounts[0]).await.unwrap();
    let observed = Arc::new(ObservedState {
        inner: f.state.state.clone(), writes: AtomicUsize::new(0),
    });
    f.state.runtime_mut().unwrap().state = observed.clone();
    let before = serde_json::to_value(f.state.state.list().await.unwrap()).unwrap();
    // Refusal::Upstream.as_error("/p", body) keeps this sub-500-byte body intact.
    let error = format!("upstream 403 on /p: provider {}é", "a".repeat(210));
    assert_eq!(error.find('é'), Some(239));
    assert_eq!(error.len(), 241);
    for account in &f.accounts {
        record_account_health(&f.state, account, &Err::<(), _>(error.clone())).await;
    }
    assert!(f.state.wave.check().is_none());
    f.state.wave.note_refusal("fixture-wave-a");
    f.state.wave.note_refusal("fixture-wave-b");
    assert!(f.state.wave.check().is_some());
    let ascii = format!("upstream 403 on /p: provider {}", "a".repeat(300));
    for error in [error, ascii] {
        record_account_health(&f.state, &f.accounts[0], &Err::<(), _>(error)).await;
        assert!(f.state.wave.check().is_some());
    }
    assert_eq!(observed.writes.load(Ordering::SeqCst), 0);
    assert_eq!(serde_json::to_value(f.state.state.list().await.unwrap()).unwrap(), before);
    f.no_call();
    assert!(matches!(f.loads.try_recv(), Err(tokio::sync::mpsc::error::TryRecvError::Empty)));
}

#[test]
fn diagnostic_excerpts_keep_120_and_240_byte_caps_on_utf8_boundaries() {
    for limit in [120, 240] {
        assert_eq!(error_excerpt("short é", limit), "short é");
        assert_eq!(error_excerpt(&"a".repeat(limit + 1), limit), "a".repeat(limit));
        assert_eq!(error_excerpt(&format!("{}éz", "a".repeat(limit - 2)), limit),
            format!("{}é", "a".repeat(limit - 2)));
        for scalar in ['é', '€', '🦀'] {
            for cut in 1..scalar.len_utf8() {
                let prefix = "a".repeat(limit - cut);
                let error = format!("{prefix}{scalar}tail");
                assert_eq!(error_excerpt(&error, limit), prefix);
            }
        }
    }
}

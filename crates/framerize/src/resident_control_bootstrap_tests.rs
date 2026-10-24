use super::*;
use framer_core::{ids::{AccountId, FramerUserId, ProjectId, TeamId}, port::Session};
use std::sync::atomic::{AtomicUsize, Ordering};

#[derive(Default)]
struct Store(AtomicUsize);
#[async_trait::async_trait]
impl SessionStore for Store {
    async fn load(&self, _: &FramerUserId) -> Result<Option<Session>, String> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Err("must not read a session before bootstrap".into())
    }
    async fn save(&self, _: &Session) -> Result<(), String> { panic!("unexpected save") }
    async fn delete(&self, _: &FramerUserId) -> Result<(), String> { panic!("unexpected delete") }
}

fn identity() -> Identity {
    Identity { account: AccountId(uuid::Uuid::from_u128(1)),
        user: FramerUserId("fixture-user".into()), team: TeamId("fixture-team".into()),
        project: ProjectId("fixture-project".into()), chat_session: "fixture-chat".into() }
}

async fn preflight(expired: bool) {
    let store = Arc::new(Store::default());
    let sidecar = Arc::new(SidecarBridge::new(std::path::PathBuf::from("/must-not-start")));
    let owner = StartupOwner::new(sidecar, store.clone()).unwrap();
    let deadline = if expired { Instant::now() } else {
        Instant::now() + std::time::Duration::from_secs(10)
    };
    let result = owner.capture(&identity(), &None, "/ai/v3/responses/", deadline).await;
    let failure = match result { Err(failure) => failure, Ok(_) => panic!("unexpected capture") };
    assert!(failure.cleanup_confirmed);
    assert_eq!(store.0.load(Ordering::SeqCst), 0);
    assert!(owner.store.shutdown().await.is_ok());
    assert!(owner.native.shutdown_idle().await.is_ok());
}

#[tokio::test]
async fn missing_resident_host_does_not_read_sessions_or_open_native_account() {
    preflight(false).await;
}

#[tokio::test]
async fn expired_capture_does_not_read_sessions_or_open_native_account() {
    preflight(true).await;
}

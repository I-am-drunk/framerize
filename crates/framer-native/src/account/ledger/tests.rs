use super::*;
use std::{path::PathBuf, process::Command};
use tokio::time::{timeout, Duration};

struct Fixture { root: PathBuf, runtime: PathBuf }
impl Fixture {
    async fn start(&self) -> LedgerOwner {
        let store = Arc::new(framer_store::FileSessionStore::new(self.root.join("sessions")).unwrap());
        LedgerOwner::start(Path::new("/usr/bin/python3"), &self.runtime.join("native.py"),
            &self.root.join("history"), store).await.unwrap()
    }

    fn sessions_available(&self) -> bool {
        framer_store::FileSessionStore::new(self.root.join("sessions")).is_ok()
    }

    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("framerize-ledger-{}", Uuid::new_v4()));
        let runtime = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../runtime").canonicalize().unwrap();
        let output = Command::new("/usr/bin/python3").args(["-I", "-B"])
            .arg(runtime.join("tests_native/seed_ledger.py")).arg(root.join("history"))
            .env_clear().env("PATH", "/usr/bin:/bin").output().unwrap();
        assert!(output.status.success(), "synthetic history setup failed");
        Self { root, runtime }
    }
}

fn account(number: u128) -> AccountId { AccountId(Uuid::from_u128(number)) }

#[tokio::test]
async fn missing_history_refuses_startup_without_creating_a_ledger() {
    let fixture = Fixture::new();
    let missing = fixture.root.join("missing");
    let store = Arc::new(framer_store::FileSessionStore::new(fixture.root.join("sessions")).unwrap());
    let result = LedgerOwner::start(Path::new("/usr/bin/python3"),
        &fixture.runtime.join("native.py"), &missing, store).await;
    assert!(matches!(result, Err(DeviceFailure::Rejected)));
    assert!(!missing.exists());
    assert!(fixture.sessions_available());
}

#[tokio::test]
async fn cancelled_waiter_does_not_strand_a_later_admission() {
    let fixture = Fixture::new();
    let owner = fixture.start().await;
    let first = owner.open_native_device(account(1)).await.unwrap();
    let second = owner.open_native_device(account(1)).await.unwrap();
    let sequence = owner.admit_native_mint(&first, 2000).await.unwrap();
    let mut cancelled = Box::pin(owner.admit_native_mint(&second, 2000));
    assert!(futures::poll!(cancelled.as_mut()).is_pending());
    drop(cancelled);
    owner.finish_native_mint(&first, sequence, true).await.unwrap();
    let next = owner.admit_native_mint(&first, 2000).await.unwrap();
    owner.finish_native_mint(&first, next, true).await.unwrap();
    for grant in [&first, &second] {
        owner.close_native_device(grant, true, false).await.unwrap();
    }
    owner.shutdown().await.unwrap();
}

#[tokio::test]
async fn unconfirmed_mint_survives_process_shutdown_as_dirty_history() {
    let fixture = Fixture::new();
    let owner = fixture.start().await;
    let grant = owner.open_native_device(account(1)).await.unwrap();
    let sequence = owner.admit_native_mint(&grant, 1000).await.unwrap();
    assert!(matches!(owner.finish_native_mint(&grant, sequence, false).await,
        Err(DeviceFailure::Unconfirmed)));
    assert!(owner.shutdown().await.is_err());
    assert!(fixture.sessions_available());
    let next = fixture.start().await;
    assert!(next.open_native_device(account(1)).await.is_err());
    assert!(next.shutdown().await.is_err());
}

#[tokio::test]
async fn abandoned_admission_finishes_reservation_before_next_mint() {
    let fixture = Fixture::new();
    let owner = fixture.start().await;
    let grant = owner.open_native_device(account(1)).await.unwrap();
    let undo = identity(&grant).unwrap();
    let mut args = undo.clone();
    args["budget_ms"] = json!(1000);
    drop(owner.connection.request("admit", args, Undo::Mint(undo)).await.unwrap());
    let next = owner.admit_native_mint(&grant, 1000).await.unwrap();
    assert_eq!(next, 2);
    owner.finish_native_mint(&grant, next, true).await.unwrap();
    owner.close_native_device(&grant, true, false).await.unwrap();
    owner.shutdown().await.unwrap();
}

#[tokio::test]
async fn abandoned_open_reply_closes_only_its_unpublished_grant() {
    let fixture = Fixture::new();
    let owner = fixture.start().await;
    let args = json!({"account": account(1).0, "lease": Uuid::new_v4()});
    for _ in 0..2 {
        let reply = owner.connection.request("open", args.clone(), Undo::Open(args.clone())).await.unwrap();
        drop(reply);
    }
    owner.shutdown().await.unwrap();
    assert!(fixture.sessions_available());
}

#[tokio::test]
async fn pending_mint_does_not_block_finish_or_another_account() {
    let fixture = Fixture::new();
    let owner = fixture.start().await;
    let first = owner.open_native_device(account(1)).await.unwrap();
    let second = owner.open_native_device(account(1)).await.unwrap();
    let other = owner.open_native_device(account(2)).await.unwrap();
    let one = owner.admit_native_mint(&first, 2000).await.unwrap();
    let mut waiting = Box::pin(owner.admit_native_mint(&second, 2000));
    assert!(futures::poll!(waiting.as_mut()).is_pending());
    let two = owner.admit_native_mint(&other, 1000).await.unwrap();
    owner.finish_native_mint(&other, two, true).await.unwrap();
    owner.finish_native_mint(&first, one, true).await.unwrap();
    let three = timeout(Duration::from_secs(2), waiting).await.unwrap().unwrap();
    owner.finish_native_mint(&second, three, true).await.unwrap();
    for grant in [&first, &second, &other] {
        owner.close_native_device(grant, true, false).await.unwrap();
    }
    owner.shutdown().await.unwrap();
}

#[tokio::test]
async fn process_owns_stores_and_preserves_device_after_joined_shutdown() {
    let fixture = Fixture::new();
    let owner = fixture.start().await;
    assert!(!fixture.sessions_available());
    let grant = owner.open_native_device(account(1)).await.unwrap();
    let sequence = owner.admit_native_mint(&grant, 1000).await.unwrap();
    assert!(owner.finish_native_mint(&grant, sequence, true).await.unwrap());
    assert!(owner.close_native_device(&grant, true, false).await.unwrap());
    owner.shutdown().await.unwrap();
    assert!(fixture.sessions_available());
    let reopened = fixture.start().await;
    let next = reopened.open_native_device(account(1)).await.unwrap();
    assert_eq!(next.device, grant.device);
    assert_ne!(next.host_generation, grant.host_generation);
    assert!(matches!(reopened.admit_native_mint(&grant, 1000).await, Err(DeviceFailure::Rejected)));
    reopened.close_native_device(&next, true, false).await.unwrap();
    reopened.shutdown().await.unwrap();
}

impl Drop for Fixture {
    fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.root); }
}

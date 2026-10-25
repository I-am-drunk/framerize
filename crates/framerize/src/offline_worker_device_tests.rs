use super::*;
use std::{fs, os::unix::fs::DirBuilderExt, sync::Mutex};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use tokio::sync::Semaphore;
use framer_api::experimental_native::native_account::{ClientError, CloseError, CloseObserver};

#[path = "offline_worker_v2_tests.rs"]
mod v2;

struct Fixture {
    root: PathBuf,
    worker: Arc<Worker>,
}

impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("framerize-device-bridge-{}", uuid::Uuid::new_v4()));
        fs::DirBuilder::new().mode(0o700).create(&root).unwrap();
        let files: [(&str, &[u8], &str); 3] = [
            ("worker.mjs", include_bytes!("../../../scripts/tests/fixtures/device_mint_pending.mjs"),
                "3dc511b8e330c8106a60040a25383afa14cc3ac74f6c804c49b288487f024933"),
            ("protocol.mjs", include_bytes!("../../../scripts/offline_worker/protocol.mjs"),
                "13ad5f0d8b29a5beaf3dcf7c16398f6d3de98069515b6a8ba8fe8e29502d68df"),
            ("device.mjs", include_bytes!("../../../scripts/offline_worker/device.mjs"),
                "a41e26307400a61ac212f469880108321004f533d210a2aa83a39faca8aeb74f"),
        ];
        let mut pins = vec![pin(PathBuf::from(NODE),
            "53fb205ae78805130177e24bcb459a69a1518c8d98f8965f31d85aae7ea840fc")];
        for (name, bytes, digest) in files {
            fs::write(root.join(name), bytes).unwrap();
            pins.push(pin(root.join(name), digest));
        }
        let worker = Worker::new(super::super::super::offline_worker_owner::Specification {
            node: PathBuf::from(NODE), entry: root.join("worker.mjs"), pins,
        }).unwrap();
        Self { root, worker: Arc::new(worker) }
    }

    async fn started(&self) -> (u32, u64) {
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                if let Ok(value) = fs::read_to_string(self.root.join("ready")) {
                    assert!(value.len() <= 12);
                    let pid: u32 = value.parse().unwrap();
                    assert_eq!(fs::read_link(format!("/proc/{pid}/exe")).unwrap(), PathBuf::from(NODE));
                    let command = fs::read(format!("/proc/{pid}/cmdline")).unwrap();
                    assert!(command.len() <= 16384);
                    assert!(command.split(|b| *b == 0)
                        .any(|arg| arg == self.root.join("worker.mjs").as_os_str().as_bytes()));
                    return (pid, child_start(pid).expect("ready child remains running"));
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        }).await.expect("device-aware fixture must signal readiness")
    }
}

impl Drop for Fixture {
    fn drop(&mut self) { let _ = fs::remove_dir_all(&self.root); }
}

#[async_trait::async_trait]
impl BrowserBridge for Bridge {
    async fn admit_native_mint(&self, grant: &DeviceGrant, budget_ms: u64)
        -> Result<u64, DeviceFailure> {
        self.verify_grant(grant);
        assert!((1..=5000).contains(&budget_ms));
        assert_eq!(self.admitted.fetch_add(1, Ordering::SeqCst), 0);
        Ok(73)
    }

    async fn finish_native_mint(&self, grant: &DeviceGrant, sequence: u64,
        joined: bool) -> Result<bool, DeviceFailure> {
        self.verify_grant(grant);
        assert_eq!(sequence, 73);
        assert!(joined, "host must receive actual worker cleanup confirmation");
        let (pid, start) = self.child.lock().unwrap().expect("observed running worker");
        assert_ne!(child_start(pid), Some(start), "worker must join before host finish");
        assert_eq!(self.finished.fetch_add(1, Ordering::SeqCst), 0);
        self.arriving.add_permits(1);
        self.acknowledge.acquire().await.unwrap().forget();
        self.replied.store(true, Ordering::SeqCst);
        if self.lost_reply { Err(DeviceFailure::Unconfirmed) } else { Ok(true) }
    }

    async fn castle_token(&self, _: &AccountId) -> Result<CastleToken, String> {
        panic!("unexpected legacy token request")
    }
    async fn auth_begin(&self, _: &AccountId, _: &str) -> Result<(), String> {
        panic!("unexpected authentication request")
    }
    async fn auth_complete(&self, _: &AccountId, _: &str)
        -> Result<(String, serde_json::Value), String> {
        panic!("unexpected authentication completion")
    }
}

async fn held_until_acknowledged(settled: CloseObserver, closed: CloseObserver) {
    let premature = tokio::time::timeout(Duration::from_millis(100), async {
        tokio::select! {
            _ = settled => (),
            _ = closed => (),
        }
    }).await;
    assert!(premature.is_err(), "completion and account close must retain host acknowledgement");
}

#[tokio::test]
async fn explicit_cancel_waits_for_worker_join_and_host_acknowledgement() {
    let fixture = Fixture::new();
    let (client, scope) = account();
    let bridge = Bridge::new(&scope, false);
    let pending = prepare(&client, &scope, bridge.minter(&scope, &fixture), "hang");
    bridge.observe(&fixture).await;
    let settled = pending.completion();
    pending.cancellation().cancel();
    bridge.at_acknowledgement().await;
    let closed = client.close_account(&scope).unwrap();
    held_until_acknowledged(settled.clone(), closed.clone()).await;
    bridge.acknowledge.add_permits(1);
    let result = tokio::time::timeout(Duration::from_secs(3), pending).await.unwrap();
    assert!(matches!(result, Err(PreparationFailure::Cancelled)));
    tokio::time::timeout(Duration::from_secs(3), settled).await.unwrap().unwrap();
    tokio::time::timeout(Duration::from_secs(3), closed).await.unwrap().unwrap();
    assert!(bridge.replied.load(Ordering::SeqCst));
}

#[tokio::test]
async fn dropped_result_retains_worker_join_and_host_acknowledgement() {
    let fixture = Fixture::new();
    let (client, scope) = account();
    let bridge = Bridge::new(&scope, false);
    let pending = prepare(&client, &scope, bridge.minter(&scope, &fixture), "hang");
    bridge.observe(&fixture).await;
    let settled = pending.completion();
    drop(pending);
    bridge.at_acknowledgement().await;
    let closed = client.close_account(&scope).unwrap();
    held_until_acknowledged(settled.clone(), closed.clone()).await;
    bridge.acknowledge.add_permits(1);
    tokio::time::timeout(Duration::from_secs(3), settled).await.unwrap().unwrap();
    tokio::time::timeout(Duration::from_secs(3), closed).await.unwrap().unwrap();
    assert!(bridge.replied.load(Ordering::SeqCst));
}

#[tokio::test]
async fn lost_host_acknowledgement_keeps_account_fenced_after_worker_join() {
    let fixture = Fixture::new();
    let (client, scope) = account();
    let bridge = Bridge::new(&scope, true);
    let pending = prepare(&client, &scope, bridge.minter(&scope, &fixture), "hang");
    bridge.observe(&fixture).await;
    let settled = pending.completion();
    pending.cancellation().cancel();
    bridge.at_acknowledgement().await;
    held_until_acknowledged(settled.clone(), scope.closed()).await;
    bridge.acknowledge.add_permits(1);
    let result = tokio::time::timeout(Duration::from_secs(3), pending).await.unwrap();
    assert!(matches!(result, Err(PreparationFailure::CleanupUnconfirmed)));
    assert_eq!(tokio::time::timeout(Duration::from_secs(3), settled).await.unwrap(),
        Err(CloseError::PreparationUnconfirmed));
    assert_eq!(tokio::time::timeout(Duration::from_secs(3), scope.closed()).await.unwrap(),
        Err(CloseError::PreparationUnconfirmed));
    assert!(bridge.replied.load(Ordering::SeqCst));
    assert_fenced(&client, &scope);
}

fn assert_fenced(client: &NativeAccountClient, scope: &AccountScope) {
    let retry = client.prepare_owned(scope,
        tokio::time::Instant::now() + Duration::from_secs(1),
        |_, _| async { PreparationResult::finished(Ok(())) });
    assert!(matches!(retry, Err(ClientError::Draining)));
    let reopened = client.open_account(scope.account(), Session {
        active_user: scope.active_user().clone(), cookie_header: "session=fixture-only".into(),
    }, NativeHttpLimits::default(), NativeEgress::Direct);
    assert!(matches!(reopened, Err(ClientError::AlreadyOpen)));
}

// The host reply is controlled; preparation and worker ownership are real.
struct Bridge {
    grant: Arc<DeviceGrant>,
    child: Mutex<Option<(u32, u64)>>,
    admitted: AtomicUsize,
    finished: AtomicUsize,
    replied: AtomicBool,
    arriving: Semaphore,
    acknowledge: Semaphore,
    lost_reply: bool,
}

impl Bridge {
    async fn observe(&self, fixture: &Fixture) {
        let child = fixture.started().await;
        *self.child.lock().unwrap() = Some(child);
    }

    fn verify_grant(&self, grant: &DeviceGrant) {
        assert_eq!(grant.account, self.grant.account);
        assert_eq!(grant.host_generation, self.grant.host_generation);
        assert_eq!(grant.lease, self.grant.lease);
        assert_eq!(grant.device, self.grant.device);
    }

    fn minter(self: &Arc<Self>, scope: &AccountScope, fixture: &Fixture)
        -> Arc<OfflineWorkerMinter> {
        Arc::new(OfflineWorkerMinter::with_device(scope.clone(), fixture.worker.clone(),
            self.clone(), self.grant.clone()).unwrap())
    }

    async fn at_acknowledgement(&self) {
        tokio::time::timeout(Duration::from_secs(3), self.arriving.acquire())
            .await.expect("worker join must reach host acknowledgement")
            .unwrap().forget();
        assert_eq!(self.admitted.load(Ordering::SeqCst), 1);
        assert_eq!(self.finished.load(Ordering::SeqCst), 1);
        assert!(!self.replied.load(Ordering::SeqCst));
    }

    fn new(scope: &AccountScope, lost_reply: bool) -> Arc<Self> {
        Arc::new(Self {
            grant: Arc::new(DeviceGrant {
                authority: DeviceAuthority::Ledger,
                account: scope.account(), host_generation: "device-bridge-host".into(),
                lease: uuid::Uuid::new_v4(), device: "device-bridge-fixture".into(),
            }),
            child: Mutex::new(None), admitted: AtomicUsize::new(0),
            finished: AtomicUsize::new(0), replied: AtomicBool::new(false),
            arriving: Semaphore::new(0), acknowledge: Semaphore::new(0), lost_reply,
        })
    }
}

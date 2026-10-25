use super::*;
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::os::unix::ffi::OsStrExt;
use crate::authorization::CredentialTarget;
use crate::account::client::NativeAccountClient;
use crate::account::client::PendingPreparation;
use crate::transport::http::NativeEgress;
use crate::transport::http::NativeHttpLimits;
use framer_core::ids::AccountId;
use framer_core::ids::FramerUserId;
use framer_core::ids::ProjectId;
use framer_core::port::Session;

#[path = "offline_worker_device_tests.rs"]
mod device;

fn pin(path: PathBuf, expected: &str) -> crate::worker::process::FilePin {
    assert_eq!(expected.len(), 64);
    let mut sha256 = [0u8; 32];
    for (index, byte) in sha256.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&expected[index * 2..index * 2 + 2], 16).unwrap();
    }
    crate::worker::process::FilePin { path, sha256 }
}

fn account() -> (NativeAccountClient, AccountScope) {
    let client = NativeAccountClient::new().unwrap();
    let scope = client.open_account(AccountId(uuid::Uuid::new_v4()), Session {
        active_user: FramerUserId("offline-adapter-fixture".into()),
        cookie_header: "session=fixture-only".into(),
    }, NativeHttpLimits::default(), NativeEgress::Direct).unwrap();
    (client, scope)
}

fn adapter(scope: &AccountScope) -> Arc<OfflineWorkerMinter> {
    use crate::worker::process::Specification;
    let node = crate::test_support::node();
    let root = crate::test_support::fixture_root();
    let entry = root.join("fake_worker.mjs");
    let pins = vec![
        crate::test_support::node_pin(),
        pin(entry.clone(), "3933fdca4bde11c27b6174d08a5eb790047ccefbe9e89573cc1dc948d269a1bb"),
        pin(root.join("protocol.mjs"), "99117048db3589a307efcd11ce07960f8e80afc0aece3d1a4d000f55ec1622df"),
    ];
    Arc::new(OfflineWorkerMinter::new(scope.clone(),
        Arc::new(Worker::new(Specification { node, entry, pins }).unwrap())))
}

fn prepare(client: &NativeAccountClient, scope: &AccountScope,
    minter: Arc<OfflineWorkerMinter>, mode: &str)
    -> PendingPreparation<MintedCastle> {
    let target = OfflineMintTarget::new(CredentialTarget {
        project: ProjectId(mode.into()), path: "/ai/v3/responses/",
    }, format!("https://framer.com/projects/{mode}")).unwrap();
    client.prepare_owned(scope,
        tokio::time::Instant::now() + Duration::from_secs(5),
        move |_, control| async move {
            minter.mint(&target, &control).await
        }).unwrap()
}

fn child_start(pid: u32) -> Option<u64> {
    let raw = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    raw.rsplit_once(") ")?.1.split_whitespace().nth(19)?.parse().ok()
}

fn running_child() -> Option<(u32, u64)> {
    let tasks: Vec<_> = std::fs::read_dir("/proc/self/task").unwrap().take(129).collect();
    assert!(tasks.len() <= 128);
    let mut pids = BTreeSet::new();
    for task in tasks {
        let path = task.unwrap().path().join("children");
        if let Ok(raw) = std::fs::read_to_string(path) {
            assert!(raw.len() <= 4096);
            pids.extend(raw.split_whitespace().map(|s| s.parse::<u32>().unwrap()));
        }
    }
    let node = crate::test_support::node();
    let entry = crate::test_support::fixture_root().join("fake_worker.mjs");
    let mut found = Vec::new();
    for pid in pids {
        let root = PathBuf::from(format!("/proc/{pid}"));
        if std::fs::read_link(root.join("exe")).ok().as_ref() != Some(&node) { continue; }
        if let Ok(command) = std::fs::read(root.join("cmdline")) {
            assert!(command.len() <= 16384);
            if command.split(|b| *b == 0).any(|a| a == entry.as_os_str().as_bytes()) {
                if let Some(start) = child_start(pid) { found.push((pid, start)); }
            }
        }
    }
    assert!(found.len() <= 1, "concurrent fixture workers");
    found.pop()
}

async fn started_child() -> (u32, u64) {
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if let Some(child) = running_child() { return child; }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }).await.expect("exact fixture child must start before cancellation")
}

async fn close(client: &NativeAccountClient, scope: &AccountScope) {
    tokio::time::timeout(Duration::from_secs(3), client.close_account(scope).unwrap())
        .await.unwrap().unwrap();
}

#[tokio::test]
async fn joined_result_preserves_scope_and_age_across_delayed_exit() {
    let (client, scope) = account();
    let minter = adapter(&scope);
    assert!(minter.scope().same_generation(&scope));
    let before = Instant::now();
    let pending = prepare(&client, &scope, minter, "delayedexit");
    let result = pending.await.unwrap();
    assert_eq!(result.token.0, "fake-fixture-token");
    assert!(result.received_at >= before);
    assert!(result.received_at.elapsed() >= Duration::from_millis(200));
    close(&client, &scope).await;
}

#[tokio::test]
async fn cancellation_interrupts_pending_worker_and_joins_before_account_close() {
    let (client, scope) = account();
    let pending = prepare(&client, &scope, adapter(&scope), "hang");
    let (pid, start) = started_child().await;
    let settled = pending.completion();
    pending.cancellation().cancel();
    let result = tokio::time::timeout(Duration::from_secs(3), pending).await.unwrap();
    assert!(matches!(result, Err(PreparationFailure::Cancelled)));
    settled.await.unwrap();
    assert_ne!(child_start(pid), Some(start), "cancelled child was not reaped");
    close(&client, &scope).await;
}

#[tokio::test]
async fn dropped_result_waiter_retains_join_until_native_completion() {
    let (client, scope) = account();
    let pending = prepare(&client, &scope, adapter(&scope), "hang");
    let (pid, start) = started_child().await;
    let settled = pending.completion();
    drop(pending);
    tokio::time::timeout(Duration::from_secs(3), settled).await.unwrap().unwrap();
    assert_ne!(child_start(pid), Some(start), "dropped-waiter child was not reaped");
    close(&client, &scope).await;
}

#[tokio::test]
async fn invalid_metadata_is_rejected_after_join_without_false_poison() {
    let (client, scope) = account();
    let pending = prepare(&client, &scope, adapter(&scope), "badmeta");
    assert!(matches!(pending.await, Err(PreparationFailure::Rejected)));
    close(&client, &scope).await;
}

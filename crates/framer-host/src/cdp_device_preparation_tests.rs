//! Controlled file operations only; no browser, process or live account.
use super::*;
use crate::{AccountId, BrowserBridge, CdpBridge};
use std::{sync::{mpsc, Arc}, time::Duration};
use tokio::{sync::oneshot, task::JoinHandle, time::timeout};

const LIMIT: Duration = Duration::from_secs(3);
const RECORD: &str = "{ \"castle_device_id\": \"saved-id\", \"future\": [1, true] }\n";
type Preparation = Result<(RouteGuard, Result<String, String>), String>;

struct Pending {
    task: JoinHandle<Preparation>,
    entered: oneshot::Receiver<()>,
    release: mpsc::Sender<()>,
    finished: oneshot::Receiver<bool>,
}

impl Drop for Profile {
    fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); }
}

fn pending(profile: PathBuf, route: RouteGuard, fails: bool) -> Pending {
    let (entered, ready) = oneshot::channel();
    let (release, proceed) = mpsc::channel();
    let (finished, done) = oneshot::channel();
    let task = tokio::spawn(retain(route, async move {
        let result = tokio::task::spawn_blocking(move || {
            let _ = entered.send(());
            proceed.recv_timeout(Duration::from_secs(6))
                .map_err(|_| "fixture release missing".to_owned())?;
            if fails { return Err("fixture publication failed".to_owned()); }
            std::fs::create_dir_all(&profile).map_err(|error| error.to_string())?;
            std::fs::write(DeviceRecord::path(&profile), RECORD).map_err(|error| error.to_string())?;
            Ok("saved-id".to_owned())
        }).await.map_err(|error| error.to_string()).and_then(|result| result);
        let _ = finished.send(result.is_ok());
        result
    }));
    Pending { task, entered: ready, release, finished: done }
}

struct Profile(PathBuf);
impl Profile {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("cdp-preparation-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn bridge(&self) -> Arc<CdpBridge> {
        Arc::new(CdpBridge::new(self.0.join("unused-executable"), &self.0))
    }
}

#[tokio::test]
async fn cancelled_preparation_keeps_same_account_excluded() {
    let profile = Profile::new();
    let bridge = profile.bridge();
    let account = AccountId::new();
    let path = bridge.profile_dir(&account);
    let cell = bridge.route_cell(&account).await;
    let mut work = pending(path.clone(), cell.clone().lock_owned().await, false);
    let entered = timeout(LIMIT, &mut work.entered).await;
    work.task.abort();
    let cancelled = timeout(LIMIT, &mut work.task).await;
    let retained = cell.try_lock().is_err();
    let other = timeout(LIMIT, bridge.register_account_proxy(&AccountId::new(),
        Some("http://127.0.0.1:1".into()))).await;
    let reentering = bridge.clone();
    let mut reentry = tokio::spawn(async move {
        reentering.register_account_proxy(&account, Some("http://127.0.0.1:2".into())).await
    });
    let early = timeout(Duration::from_millis(30), &mut reentry).await;
    let blocked = early.is_err();
    let unpublished = !DeviceRecord::path(&path).exists();
    let released = work.release.send(());
    let finished = timeout(LIMIT, &mut work.finished).await;
    let reentered = match early {
        Ok(result) => result,
        Err(_) => timeout(LIMIT, reentry).await.expect("reentry did not settle"),
    };
    let saved = std::fs::read_to_string(DeviceRecord::path(&path));
    let closed = timeout(LIMIT, bridge.shutdown()).await;
    // Release and settle filesystem work before evaluating regression assertions.
    entered.unwrap().unwrap();
    assert!(matches!(cancelled, Ok(Err(error)) if error.is_cancelled()));
    assert!(retained, "cancelled preparation released the account route early");
    other.unwrap().unwrap();
    assert!(blocked, "same-account work overtook device preparation");
    assert!(unpublished);
    released.unwrap();
    assert!(finished.unwrap().unwrap());
    reentered.unwrap().unwrap();
    assert_eq!(saved.unwrap(), RECORD);
    closed.unwrap().unwrap();
}

#[tokio::test]
async fn shutdown_joins_cancelled_preparation_after_success_or_failure() {
    for fails in [false, true] {
        let profile = Profile::new();
        let bridge = profile.bridge();
        let account = AccountId::new();
        let path = bridge.profile_dir(&account);
        let cell = bridge.route_cell(&account).await;
        let mut work = pending(path.clone(), cell.clone().lock_owned().await, fails);
        let entered = timeout(LIMIT, &mut work.entered).await;
        work.task.abort();
        let cancelled = timeout(LIMIT, &mut work.task).await;
        let closing = bridge.clone();
        let mut shutdown = tokio::spawn(async move { closing.shutdown().await });
        let early = timeout(Duration::from_millis(30), &mut shutdown).await;
        let blocked = early.is_err();
        let released = work.release.send(());
        let finished = timeout(LIMIT, &mut work.finished).await;
        let closed = match early {
            Ok(result) => result,
            Err(_) => timeout(LIMIT, shutdown).await.expect("shutdown did not settle"),
        };
        entered.unwrap().unwrap();
        assert!(matches!(cancelled, Ok(Err(error)) if error.is_cancelled()));
        assert!(blocked, "shutdown returned while device preparation was unfinished");
        released.unwrap();
        assert_eq!(finished.unwrap().unwrap(), !fails);
        closed.unwrap().unwrap();
        assert_eq!(DeviceRecord::path(&path).exists(), !fails);
        assert!(!cell.lock().await.may_have_browser);
        assert!(bridge.instances.lock().await.is_empty());
        assert!(bridge.startup_children.join().await.is_empty());
    }
}

#[tokio::test]
async fn identity_rotation_waits_for_cancelled_preparation() {
    let profile = Profile::new();
    let bridge = profile.bridge();
    let account = AccountId::new();
    let path = bridge.profile_dir(&account);
    let cell = bridge.route_cell(&account).await;
    let mut work = pending(path.clone(), cell.lock_owned().await, false);
    let entered = timeout(LIMIT, &mut work.entered).await;
    work.task.abort();
    let cancelled = timeout(LIMIT, &mut work.task).await;
    let rotating = bridge.clone();
    let mut rotation = tokio::spawn(async move { rotating.rotate_identity(&account).await });
    let early = timeout(Duration::from_millis(30), &mut rotation).await;
    let blocked = early.is_err();
    let released = work.release.send(());
    let finished = timeout(LIMIT, &mut work.finished).await;
    let rotated = match early {
        Ok(result) => result,
        Err(_) => timeout(LIMIT, rotation).await.expect("rotation did not settle"),
    };
    let profile_survived = path.exists();
    let closed = timeout(LIMIT, bridge.shutdown()).await;
    entered.unwrap().unwrap();
    assert!(matches!(cancelled, Ok(Err(error)) if error.is_cancelled()));
    assert!(blocked, "identity rotation overtook unfinished preparation");
    released.unwrap();
    assert!(finished.unwrap().unwrap());
    rotated.unwrap().unwrap();
    assert!(!profile_survived, "cancelled preparation recreated the rotated profile");
    closed.unwrap().unwrap();
}

#[tokio::test]
async fn completed_preparation_preserves_records_guard_and_errors() {
    let cases: [(Option<&[u8]>, bool); 5] = [
        (None, false), (Some(RECORD.as_bytes()), false),
        (Some(b"{"), true), (Some(b"{}"), true), (Some(b"\xff"), true),
    ];
    for (raw, fails) in cases {
        let profile = Profile::new();
        let bridge = profile.bridge();
        let account = AccountId::new();
        let path = bridge.profile_dir(&account);
        let file = DeviceRecord::path(&path);
        if let Some(raw) = raw {
            std::fs::create_dir(&path).unwrap();
            std::fs::write(&file, raw).unwrap();
        }
        let expected_error = if fails { DeviceRecord::load_or_create(&path).await.err() }
            else { None };
        let cell = bridge.route_cell(&account).await;
        let mut route = cell.clone().lock_owned().await;
        route.may_have_browser = true;
        let result = timeout(LIMIT, prepare(path.clone(), route)).await.unwrap();
        if fails {
            assert!(expected_error.is_some());
            assert_eq!(result.err(), expected_error);
            assert_eq!(std::fs::read(&file).unwrap(), raw.unwrap());
        } else {
            let (route, id) = result.unwrap();
            assert!(cell.try_lock().is_err(), "preparation returned an unlocked route");
            assert!(route.may_have_browser, "preparation cleared browser uncertainty");
            assert!(!id.is_empty());
            if let Some(raw) = raw {
                assert_eq!(id, "saved-id");
                assert_eq!(std::fs::read(&file).unwrap(), raw);
            }
            drop(route);
            assert_eq!(DeviceRecord::load_or_create(&path).await.unwrap(), id);
        }
        assert!(cell.try_lock().unwrap().may_have_browser);
    }
    let profile = Profile::new();
    let bridge = profile.bridge();
    let account = AccountId::new();
    let path = bridge.profile_dir(&account);
    std::fs::write(&path, b"not a directory").unwrap();
    let expected = std::fs::create_dir_all(&path).unwrap_err().to_string();
    let cell = bridge.route_cell(&account).await;
    let result = timeout(LIMIT, prepare(path.clone(), cell.clone().lock_owned().await))
        .await.unwrap();
    assert_eq!(result.err(), Some(expected));
    assert!(cell.try_lock().is_ok());
    assert_eq!(std::fs::read(&path).unwrap(), b"not a directory");
}

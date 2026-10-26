//! Controlled blocking removals and temporary files, without browser processes.
use super::*;
use crate::{AccountId, BrowserBridge, CdpBridge};
use std::{sync::{mpsc, Arc}, time::Duration};
use tokio::{sync::oneshot, task::JoinHandle, time::timeout};

const LIMIT: Duration = Duration::from_secs(3);

struct Profile(PathBuf);
impl Profile {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("cdp-removal-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn port(&self) -> PathBuf { self.0.join("DevToolsActivePort") }
    fn bridge(&self) -> Arc<CdpBridge> {
        Arc::new(CdpBridge::new(self.0.join("unused-executable"), &self.0))
    }
}
impl Drop for Profile {
    fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); }
}

struct PendingRemoval {
    task: JoinHandle<Result<(RouteGuard, io::Result<()>), String>>,
    entered: oneshot::Receiver<()>,
    release: mpsc::Sender<()>,
    finished: oneshot::Receiver<Result<(), io::ErrorKind>>,
}

fn pending(path: PathBuf, route: RouteGuard) -> PendingRemoval {
    let (entered, ready) = oneshot::channel();
    let (release, proceed) = mpsc::channel();
    let (finished, done) = oneshot::channel();
    let task = tokio::spawn(remove_with(route, move || {
        let _ = entered.send(());
        proceed.recv_timeout(Duration::from_secs(6)).map_err(|_|
            io::Error::new(io::ErrorKind::TimedOut, "fixture release missing"))?;
        let result = std::fs::remove_file(path);
        let _ = finished.send(result.as_ref().map(|_| ()).map_err(|error| error.kind()));
        result
    }));
    PendingRemoval { task, entered: ready, release, finished: done }
}

#[tokio::test]
async fn cancelled_removal_cannot_overtake_same_account_reentry() {
    let profile = Profile::new();
    std::fs::write(profile.port(), b"old endpoint").unwrap();
    let bridge = profile.bridge();
    let account = AccountId::new();
    let cell = bridge.route_cell(&account).await;
    let mut removal = pending(profile.port(), cell.clone().lock_owned().await);
    let entered = timeout(LIMIT, &mut removal.entered).await;
    removal.task.abort();
    let cancelled = timeout(LIMIT, &mut removal.task).await;
    let retained = cell.try_lock().is_err();
    let other = timeout(LIMIT, bridge.register_account_proxy(&AccountId::new(),
        Some("http://127.0.0.1:1".into()))).await;
    let path = profile.port();
    let same_account = bridge.clone();
    let mut reentry = tokio::spawn(async move {
        same_account.register_account_proxy(&account,
            Some("http://127.0.0.1:2".into())).await?;
        let _route = cell.lock_owned().await;
        std::fs::write(path, b"new endpoint").map_err(|error| error.to_string())
    });
    let early = timeout(Duration::from_millis(30), &mut reentry).await;
    let blocked = early.is_err();
    let released = removal.release.send(());
    let finished = timeout(LIMIT, &mut removal.finished).await;
    let reentered = match early {
        Ok(result) => result,
        Err(_) => timeout(LIMIT, reentry).await.expect("reentry did not settle"),
    };
    let saved = std::fs::read(profile.port());
    let closed = timeout(LIMIT, bridge.shutdown()).await;
    // Settle the blocked operation and its successor before asserting.
    entered.unwrap().unwrap();
    assert!(matches!(cancelled, Ok(Err(error)) if error.is_cancelled()));
    assert!(retained, "cancelled waiter released the account route early");
    other.unwrap().unwrap();
    assert!(blocked, "new account work overtook unfinished removal");
    released.unwrap();
    finished.unwrap().unwrap().unwrap();
    reentered.unwrap().unwrap();
    assert_eq!(saved.unwrap(), b"new endpoint");
    closed.unwrap().unwrap();
}

#[tokio::test]
async fn shutdown_waits_for_cancelled_removal_to_finish() {
    for fails in [false, true] {
        let profile = Profile::new();
        if fails { std::fs::create_dir(profile.port()).unwrap(); }
        else { std::fs::write(profile.port(), b"old endpoint").unwrap(); }
        let bridge = profile.bridge();
        let account = AccountId::new();
        let cell = bridge.route_cell(&account).await;
        let mut removal = pending(profile.port(), cell.clone().lock_owned().await);
        let entered = timeout(LIMIT, &mut removal.entered).await;
        removal.task.abort();
        let cancelled = timeout(LIMIT, &mut removal.task).await;
        let shutting_down = bridge.clone();
        let mut shutdown = tokio::spawn(async move { shutting_down.shutdown().await });
        let early = timeout(Duration::from_millis(30), &mut shutdown).await;
        let blocked = early.is_err();
        let released = removal.release.send(());
        let finished = timeout(LIMIT, &mut removal.finished).await;
        let closed = match early {
            Ok(result) => result,
            Err(_) => timeout(LIMIT, shutdown).await.expect("shutdown did not settle"),
        };
        entered.unwrap().unwrap();
        assert!(matches!(cancelled, Ok(Err(error)) if error.is_cancelled()));
        assert!(blocked, "shutdown reported completion while removal was unfinished");
        released.unwrap();
        assert_eq!(finished.unwrap().unwrap().is_err(), fails);
        closed.unwrap().unwrap();
        assert_eq!(profile.port().exists(), fails);
        assert!(!cell.lock().await.may_have_browser);
        assert!(bridge.instances.lock().await.is_empty());
        assert!(bridge.startup_children.join().await.is_empty());
    }
}

#[tokio::test]
async fn completed_removal_returns_guard_and_original_io_result() {
    for shape in ["file", "missing", "directory"] {
        let profile = Profile::new();
        if shape == "directory" { std::fs::create_dir(profile.port()).unwrap(); }
        if shape == "file" { std::fs::write(profile.port(), b"old endpoint").unwrap(); }
        let expected_error = if shape == "file" { None } else {
            let error = std::fs::remove_file(profile.port()).unwrap_err();
            Some((error.kind(), error.raw_os_error()))
        };
        let bridge = profile.bridge();
        let cell = bridge.route_cell(&AccountId::new()).await;
        let mut route = cell.clone().lock_owned().await;
        route.may_have_browser = true;
        let (route, result) = timeout(LIMIT, remove(profile.port(), route))
            .await.unwrap().unwrap();
        assert!(cell.try_lock().is_err(), "completion released the caller's route");
        assert!(route.may_have_browser, "file removal altered browser uncertainty");
        let actual_error = result.err().map(|error| (error.kind(), error.raw_os_error()));
        assert_eq!(actual_error, expected_error);
        assert_eq!(profile.port().exists(), shape == "directory");
        drop(route);
        assert!(cell.try_lock().is_ok());
    }
}

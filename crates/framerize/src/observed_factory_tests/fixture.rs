use super::super::*;
use framer_api::experimental_native::{native_account::NativeAccountClient,
    native_http::{NativeEgress, NativeHttpLimits}};
use framer_core::ids::{AccountId, FramerUserId};
use framer_host::sidecar::{SidecarBridge, observer::ObserverLimits};
use futures::FutureExt;
use std::{path::PathBuf, sync::Mutex, time::Duration};
use tokio::time::timeout;

pub const BOUND: Duration = Duration::from_secs(2);
pub struct Fixture {
    pub root: PathBuf,
    pub client: NativeAccountClient,
    pub scope: AccountScope,
    pub factory: Factory,
    scopes: Mutex<Vec<AccountScope>>,
    _temp: TempRoot,
}
struct TempRoot(PathBuf);
impl TempRoot {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("factory-case-{}", uuid::Uuid::new_v4()));
        let mut builder = std::fs::DirBuilder::new();
        #[cfg(unix)] {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder.create(&root).unwrap();
        Self(root)
    }
}
impl Drop for TempRoot {
    fn drop(&mut self) {
        // Never touches a pre-existing or shared profile directory.
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
pub fn session() -> Session {
    Session { active_user: FramerUserId("factory-fixture-user".into()),
        cookie_header: "session=offline-factory-fixture".into() }
}
impl Fixture {
    pub fn new(slots: usize) -> Self {
        let temp = TempRoot::new();
        let root = temp.0.clone();
        let script = root.join("camoufox_host.py");
        std::fs::write(&script, "from pathlib import Path\nPath(__file__).with_name('unexpected-launch').write_text('launched')\nraise RuntimeError('no-command factory fixture launched')\n").unwrap();
        std::fs::write(root.join("observed_sidecar.py"), "# Constructor fixture only.\n").unwrap();
        let bridge = Arc::new(SidecarBridge::new_observed_one_shot(&root, &script).unwrap());
        let observer = bridge.observer_factory(ObserverLimits {
            scopes: slots, commands: 4, cleanup_timeout: BOUND,
        }).unwrap();
        let client = NativeAccountClient::new().unwrap();
        let scope = client.open_account(AccountId(uuid::Uuid::new_v4()), session(),
            NativeHttpLimits::default(), NativeEgress::Direct).unwrap();
        let factory = Factory { api: Arc::new(BoundControlClient::new(scope.clone(), NativeEgress::Direct).unwrap()), observer,
            expected: scope.clone(), proxy: None };
        Self { root, client, scope: scope.clone(), factory,
            scopes: Mutex::new(vec![scope]), _temp: temp }
    }
    pub fn for_scope(&self, scope: &AccountScope) -> Factory {
        Factory { api: Arc::new(BoundControlClient::new(scope.clone(), NativeEgress::Direct).unwrap()), observer: self.factory.observer.clone(),
            expected: scope.clone(), proxy: None }
    }
    pub fn open_native(&self, account: AccountId) -> AccountScope {
        let scope = self.client.open_account(account, session(),
            NativeHttpLimits::default(), NativeEgress::Direct).unwrap();
        self.scopes.lock().unwrap().push(scope.clone());
        scope
    }
    pub async fn close(&self, scope: &AccountScope) {
        timeout(BOUND, self.client.close_account(scope).unwrap()).await.unwrap().unwrap();
    }
    pub fn assert_no_launch(&self) {
        assert!(!self.root.join("unexpected-launch").exists(), "unexpected subprocess");
    }
    pub async fn finish(self) {
        let scopes = self.scopes.lock().unwrap().clone();
        for scope in scopes {
            match scope.closed().now_or_never() {
                Some(result) => result.unwrap(),
                None => self.close(&scope).await,
            }
        }
        self.assert_no_launch();
        let supervisor = self.client.supervisor_for_test();
        let root = self.root.clone();
        drop(self);
        timeout(BOUND, async {
            while !supervisor.is_finished() { tokio::task::yield_now().await; }
        }).await.expect("native supervisor did not terminate");
        assert!(!root.exists(), "fixture temporary root survived teardown");
    }
}
pub fn opened(result: Result<OpenedSource, OpenFailure>) -> OpenedSource {
    match result {
        Ok(value) => value,
        Err(OpenFailure::NotStarted) => panic!("unexpected pre-admission rejection"),
        Err(OpenFailure::Unconfirmed) => panic!("unexpected uncertain ownership"),
    }
}
pub fn not_started(result: Result<OpenedSource, OpenFailure>) {
    assert!(matches!(result, Err(OpenFailure::NotStarted)));
}
pub async fn settle(opened: OpenedSource) {
    opened.control.seal();
    assert!(matches!(timeout(BOUND, opened.cleanup).await.unwrap(), SourceCleanup::NotStarted));
    drop(opened.source);
    drop(opened.control);
}

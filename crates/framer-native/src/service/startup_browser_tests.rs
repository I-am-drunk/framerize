use super::*;
use crate::transport::{browser::{BrowserOutcome, BrowserRequest, BrowserTransport},
    http::NativeCancellation};
use futures::future::BoxFuture;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::Semaphore;

#[derive(Clone, Copy)]
enum Preparation { Ready, Fail, Panic }

struct Browser {
    root: PathBuf,
    entered: Arc<Semaphore>,
    release: Arc<Semaphore>,
    dropped: Arc<AtomicBool>,
    outcome: Preparation,
}

struct Waiter(Arc<AtomicBool>);
impl Drop for Waiter {
    fn drop(&mut self) { self.0.store(true, Ordering::SeqCst); }
}

impl Browser {
    fn new(fixture: &Fixture, outcome: Preparation) -> Arc<Self> {
        Arc::new(Self { root: fixture.root.clone(), outcome,
            entered: Arc::new(Semaphore::new(0)), release: Arc::new(Semaphore::new(0)),
            dropped: Arc::new(AtomicBool::new(false)) })
    }
    async fn preparing(&self) {
        timeout(Duration::from_secs(5), self.entered.acquire()).await.unwrap().unwrap().forget();
    }
}

impl BrowserTransport for Browser {
    fn prepare(&self) -> BoxFuture<'static, Result<(), String>> {
        let (root, entered, release, dropped, outcome) = (self.root.clone(),
            self.entered.clone(), self.release.clone(), self.dropped.clone(), self.outcome);
        Box::pin(async move {
            let _waiter = Waiter(dropped);
            assert!(root.join("profiles/.sidecar/camoufox/rate-history.sqlite3").is_file());
            let pid = std::fs::read_to_string(root.join("ledger-pid")).unwrap();
            assert!(PathBuf::from(format!("/proc/{pid}")).exists());
            assert!(FileSessionStore::new(root.join("profiles/.sessions")).is_err());
            entered.add_permits(1);
            release.acquire().await.unwrap().forget();
            match outcome {
                Preparation::Ready => Ok(()),
                Preparation::Fail => Err("synthetic preparation failure".into()),
                Preparation::Panic => panic!("synthetic preparation panic"),
            }
        })
    }

    fn send(&self, _: BrowserRequest, _: NativeCancellation) -> BoxFuture<'static, BrowserOutcome> {
        Box::pin(async { panic!("startup fixture must not send an account request") })
    }
}

async fn browser_fixture() -> Fixture {
    let fixture = fixture().await;
    let source = std::fs::read_to_string(&fixture.config.runtime_script).unwrap();
    let source = source.replace("else:\n", "else:\n    (here / 'ledger-pid').write_text(str(os.getpid()))\n");
    std::fs::write(&fixture.config.runtime_script, source).unwrap();
    fixture
}

async fn assert_released(fixture: &Fixture) {
    if let Ok(pid) = std::fs::read_to_string(fixture.root.join("ledger-pid")) {
        assert!(!PathBuf::from(format!("/proc/{pid}")).exists(), "ledger was not joined");
    }
    assert!(fixture.sessions_available());
    assert!(tokio::net::TcpStream::connect(fixture.config.address).await.is_err());
}

#[tokio::test]
async fn browser_prepares_after_migration_and_ledger_before_serving() {
    let fixture = browser_fixture().await;
    let browser = Browser::new(&fixture, Preparation::Ready);
    let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
    let task = tokio::spawn(serve_with_browser(fixture.config.clone(), browser.clone(),
        async { let _ = stopped.await; }));
    let migration_pid = entered(&fixture).await;
    assert_eq!(browser.entered.available_permits(), 0);
    assert!(!fixture.root.join("ledger-pid").exists());
    assert!(tokio::net::TcpStream::connect(fixture.config.address).await.is_err());
    std::fs::write(fixture.root.join("release"), b"").unwrap();
    browser.preparing().await;
    assert!(!PathBuf::from(format!("/proc/{migration_pid}")).exists());
    let client = reqwest::Client::builder().no_proxy()
        .timeout(Duration::from_millis(100)).build().unwrap();
    assert!(client.get(format!("http://{}/api/health", fixture.config.address)).send().await.is_err());
    browser.release.add_permits(1);
    healthy(fixture.config.address).await;
    stop.send(()).unwrap();
    timeout(Duration::from_secs(5), task).await.unwrap().unwrap().unwrap();
    assert!(browser.dropped.load(Ordering::SeqCst));
    assert_released(&fixture).await;
}

#[tokio::test]
async fn cancelled_or_failed_migration_never_prepares_browser() {
    for failure in [false, true] {
        let fixture = browser_fixture().await;
        let browser = Browser::new(&fixture, Preparation::Ready);
        let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
        let task = tokio::spawn(serve_with_browser(fixture.config.clone(), browser.clone(),
            async { let _ = stopped.await; }));
        let pid = entered(&fixture).await;
        if failure {
            std::fs::write(fixture.root.join("fail"), b"").unwrap();
            std::fs::write(fixture.root.join("release"), b"").unwrap();
        } else { stop.send(()).unwrap(); }
        let result = timeout(Duration::from_secs(7), task).await.unwrap().unwrap();
        assert_eq!(result.is_err(), failure);
        assert!(!PathBuf::from(format!("/proc/{pid}")).exists());
        assert_eq!(browser.entered.available_permits(), 0);
        assert!(!browser.dropped.load(Ordering::SeqCst));
        assert!(!fixture.root.join("ledger-pid").exists());
        assert!(!fixture.config.profiles.join(".sidecar/camoufox/rate-history.sqlite3").exists());
        assert_released(&fixture).await;
    }
}

#[tokio::test]
async fn failed_or_panicking_browser_preparation_joins_ledger() {
    for outcome in [Preparation::Fail, Preparation::Panic] {
        let fixture = browser_fixture().await;
        let browser = Browser::new(&fixture, outcome);
        std::fs::write(fixture.root.join("release"), b"").unwrap();
        let task = tokio::spawn(serve_with_browser(fixture.config.clone(), browser.clone(),
            std::future::pending()));
        browser.preparing().await;
        browser.release.add_permits(1);
        let error = timeout(Duration::from_secs(5), task).await.unwrap().unwrap().unwrap_err();
        let expected = match outcome {
            Preparation::Fail => "browser transport preparation failed",
            Preparation::Panic => "native service failed during startup or serving",
            Preparation::Ready => unreachable!(),
        };
        assert_eq!(error, expected);
        assert!(browser.dropped.load(Ordering::SeqCst));
        assert_released(&fixture).await;
    }
}

#[tokio::test]
async fn cancelled_or_abandoned_browser_preparation_releases_profile_after_join() {
    for abandoned in [false, true] {
        let fixture = browser_fixture().await;
        let browser = Browser::new(&fixture, Preparation::Ready);
        std::fs::write(fixture.root.join("release"), b"").unwrap();
        let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
        let task = tokio::spawn(serve_with_browser(fixture.config.clone(), browser.clone(),
            async { let _ = stopped.await; }));
        browser.preparing().await;
        if abandoned {
            task.abort();
            assert!(task.await.unwrap_err().is_cancelled());
        } else {
            stop.send(()).unwrap();
            timeout(Duration::from_secs(5), task).await.unwrap().unwrap().unwrap();
        }
        timeout(Duration::from_secs(7), async {
            while !fixture.sessions_available() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        }).await.unwrap();
        assert!(browser.dropped.load(Ordering::SeqCst));
        assert_released(&fixture).await;
    }
}

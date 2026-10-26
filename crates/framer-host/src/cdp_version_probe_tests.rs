//! Inert version executables only; no browser, service or network access.
use super::*;
use std::{os::unix::fs::PermissionsExt, time::Duration};
use tokio::time::timeout;

const PREFIX: &str = "#!/usr/bin/env python3\nimport os, pathlib, sys, time\n\
assert sys.argv[1:] == ['--version']\n\
pathlib.Path(__file__).with_name('probe.pid').write_text(str(os.getpid()))\n";

struct Fixture {
    root: PathBuf,
    executable: PathBuf,
}

impl Fixture {
    fn new(body: &str) -> Self {
        let root = std::env::temp_dir().join(format!("cdp-version-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        let executable = root.join("inert-version");
        std::fs::write(&executable, format!("{PREFIX}{body}\n")).unwrap();
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
        Self { root, executable }
    }

    async fn pid(&self) -> u32 {
        loop {
            if let Ok(raw) = std::fs::read_to_string(self.root.join("probe.pid")) {
                if let Ok(pid) = raw.parse() { return pid; }
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    fn assert_exited(pid: u32) {
        assert!(!Path::new(&format!("/proc/{pid}")).exists(), "version probe remains");
    }
}

impl Drop for Fixture {
    fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.root); }
}

async fn cancelled_probe(fixture: &Fixture) -> (Arc<CdpBridge>, AccountId, u32) {
    let bridge = Arc::new(CdpBridge::new(&fixture.executable, fixture.root.join("profiles")));
    let account = AccountId::new();
    let starting = bridge.clone();
    let request = tokio::spawn(async move { starting.instance(&account).await.map(|_| ()) });
    let started = timeout(Duration::from_secs(3), fixture.pid()).await;
    request.abort();
    let stopped = request.await;
    if started.is_err() {
        let _ = timeout(Duration::from_secs(3), bridge.shutdown()).await;
    }
    let pid = started.unwrap();
    assert!(stopped.unwrap_err().is_cancelled());
    (bridge, account, pid)
}

#[tokio::test]
async fn version_output_preserves_user_agent_and_fallback() {
    for (body, version) in [
        ("print('Chromium 123.4')", Some("123.4")),
        ("print('Chromium 124.5'); sys.exit(7)", Some("124.5")),
        ("print('unknown build'); print('Chrome 999.1', file=sys.stderr)", None),
    ] {
        let fixture = Fixture::new(body);
        let bridge = CdpBridge::new(&fixture.executable, fixture.root.join("profiles"));
        let ua = timeout(Duration::from_secs(3),
            desktop_ua(&fixture.executable, &bridge.startup_children)).await.unwrap();
        if let Some(version) = version {
            assert!(ua.contains(&format!("Chrome/{version} ")), "{ua}");
        } else { assert_eq!(ua, DESKTOP_UA); }
        let pid = timeout(Duration::from_secs(2), fixture.pid()).await.unwrap();
        Fixture::assert_exited(pid);
        assert!(bridge.startup_children.join().await.is_empty());
    }
}

#[tokio::test]
async fn missing_version_executable_preserves_fallback() {
    let root = std::env::temp_dir().join(format!("missing-version-{}", uuid::Uuid::new_v4()));
    let executable = root.join("missing");
    let bridge = CdpBridge::new(&executable, &root);
    assert_eq!(desktop_ua(&executable, &bridge.startup_children).await, DESKTOP_UA);
    assert!(bridge.startup_children.join().await.is_empty());
    assert!(!root.exists());
}

#[tokio::test]
async fn cancelled_version_probe_is_joined_before_clean_shutdown() {
    let fixture = Fixture::new("time.sleep(30)");
    let (bridge, account, pid) = cancelled_probe(&fixture).await;
    timeout(Duration::from_secs(3), bridge.shutdown()).await.unwrap().unwrap();
    assert!(!bridge.route_cell(&account).await.lock().await.may_have_browser);
    Fixture::assert_exited(pid);
    assert_eq!(bridge.startup_children.join().await, vec![(None, Ok(()))]);
}

#[tokio::test]
async fn probe_exit_does_not_clear_actual_browser_uncertainty() {
    let fixture = Fixture::new("time.sleep(30)");
    let (bridge, account, pid) = cancelled_probe(&fixture).await;
    let route = bridge.route_cell(&account).await;
    route.lock().await.may_have_browser = true;
    let result = timeout(Duration::from_secs(3), bridge.shutdown()).await.unwrap();
    assert_eq!(result.unwrap_err(), "could not confirm every owned browser exited cleanly");
    assert!(route.lock().await.may_have_browser);
    assert!(bridge.instances.lock().await.is_empty());
    Fixture::assert_exited(pid);
}

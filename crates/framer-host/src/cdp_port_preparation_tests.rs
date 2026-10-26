//! Disposable executables only; no actual browser, service or network access.
use super::*;
use std::{os::unix::fs::PermissionsExt, time::Duration};
use tokio::time::timeout;

const SCRIPT: &str = r#"#!/usr/bin/env python3
import os, pathlib, sys, time
root = pathlib.Path(__file__).parent
is_probe = sys.argv[1:] == ['--version']
with (root / 'invocations').open('a') as log:
    log.write(('probe' if is_probe else 'browser') + '\n')
if is_probe:
    print('Chromium 123.4')
    raise SystemExit(0)
(root / 'browser.pid').write_text(str(os.getpid()))
time.sleep(30)
"#;

struct Fixture { root: PathBuf, executable: PathBuf, profiles: PathBuf }

impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("cdp-port-prepare-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        let executable = root.join("inert-chrome");
        std::fs::write(&executable, SCRIPT).unwrap();
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
        let profiles = root.join("profiles");
        Self { root, executable, profiles }
    }

    fn bridge(&self, engine: EngineKind) -> Arc<CdpBridge> {
        Arc::new(CdpBridge::with_engine(&self.executable, &self.profiles, engine))
    }

    async fn browser_pid(&self) -> u32 {
        loop {
            if let Ok(raw) = std::fs::read_to_string(self.root.join("browser.pid")) {
                if let Ok(pid) = raw.parse() { return pid; }
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    fn assert_exited(&self) {
        if let Ok(raw) = std::fs::read_to_string(self.root.join("browser.pid")) {
            let pid: u32 = raw.parse().unwrap();
            assert!(!Path::new(&format!("/proc/{pid}")).exists(), "fixture child remains");
        }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.root); }
}

#[tokio::test]
async fn unremovable_port_entry_prevents_owned_process_dispatch() {
    for engine in [EngineKind::FullChrome, EngineKind::HeadlessShell] {
        let fixture = Fixture::new();
        let bridge = fixture.bridge(engine);
        let account = AccountId::new();
        let port_file = bridge.profile_dir(&account).join("DevToolsActivePort");
        std::fs::create_dir_all(&port_file).unwrap();
        let result = timeout(Duration::from_secs(2), bridge.instance(&account)).await;
        // The baseline starts a disposable child; settle it before asserting.
        let closed = timeout(Duration::from_secs(4), bridge.shutdown()).await;
        fixture.assert_exited();
        closed.unwrap().unwrap();
        assert!(!fixture.root.join("invocations").exists(), "a process was dispatched");
        assert_eq!(result.unwrap().err().as_deref(),
            Some("could not remove stale DevToolsActivePort"));
        assert!(port_file.is_dir());
        assert!(bridge.instances.lock().await.is_empty());
        assert!(bridge.startup_children.join().await.is_empty());
        assert!(!bridge.route_cell(&account).await.lock().await.may_have_browser);
    }
}

#[tokio::test]
async fn missing_and_removable_port_entries_allow_owned_startup() {
    for engine in [EngineKind::FullChrome, EngineKind::HeadlessShell] {
        for stale_file in [false, true] {
            let fixture = Fixture::new();
            let bridge = fixture.bridge(engine);
            let account = AccountId::new();
            let profile = bridge.profile_dir(&account);
            std::fs::create_dir_all(&profile).unwrap();
            let port_file = profile.join("DevToolsActivePort");
            if stale_file { std::fs::write(&port_file, "1\n").unwrap(); }
            let starting = bridge.clone();
            let request = tokio::spawn(async move { starting.instance(&account).await.map(|_| ()) });
            let started = timeout(Duration::from_secs(3), fixture.browser_pid()).await;
            request.abort();
            let cancelled = request.await;
            let closed = timeout(Duration::from_secs(4), bridge.shutdown()).await;
            fixture.assert_exited();
            closed.unwrap().unwrap();
            started.expect("owned startup did not reach the disposable executable");
            assert!(cancelled.unwrap_err().is_cancelled());
            assert!(!port_file.exists());
            assert_eq!(std::fs::read_to_string(fixture.root.join("invocations")).unwrap(),
                "probe\nbrowser\n");
            assert_eq!(bridge.startup_children.join().await, vec![(Some(account), Ok(()))]);
            assert!(bridge.instances.lock().await.is_empty());
            assert!(!bridge.route_cell(&account).await.lock().await.may_have_browser);
        }
    }
}

#[tokio::test]
async fn attach_keeps_best_effort_local_port_removal() {
    let fixture = Fixture::new();
    let bridge = CdpBridge::with_engine("bad address", &fixture.profiles, EngineKind::Attach);
    let account = AccountId::new();
    let port_file = bridge.profile_dir(&account).join("DevToolsActivePort");
    std::fs::create_dir_all(&port_file).unwrap();
    let error = timeout(Duration::from_secs(2), bridge.instance(&account))
        .await.unwrap().err().expect("invalid attach address must be rejected");
    assert_ne!(error, "could not remove stale DevToolsActivePort");
    assert!(bridge.route_cell(&account).await.lock().await.may_have_browser);
    assert!(port_file.is_dir());
    assert!(!fixture.root.join("invocations").exists());
    assert!(bridge.startup_children.join().await.is_empty());
    timeout(Duration::from_secs(2), bridge.shutdown()).await.unwrap().unwrap();
}

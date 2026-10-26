//! Stalled disposable version probes only; no browser or network execution.
use super::*;
use std::{os::unix::fs::PermissionsExt, time::Duration};
use tokio::time::timeout;

struct Fixture {
    root: PathBuf,
    executable: PathBuf,
}

impl Fixture {
    fn new(body: &str) -> Self {
        let root = std::env::temp_dir().join(format!("cdp-version-deadline-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        let executable = root.join("inert-version");
        let script = format!("#!/usr/bin/env python3\nimport os, pathlib, sys, time\n\
            assert sys.argv[1:] == ['--version']\n\
            pathlib.Path(__file__).with_name('probe.pid').write_text(str(os.getpid()))\n\
            {body}\ntime.sleep(30)\n");
        std::fs::write(&executable, script).unwrap();
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
        Self { root, executable }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.root); }
}

async fn stalled_probe(body: &str, browser_uncertain: bool) {
    let fixture = Fixture::new(body);
    let bridge = CdpBridge::new(&fixture.executable, fixture.root.join("profiles"));
    let account = AccountId::new();
    let route = bridge.route_cell(&account).await;
    route.lock().await.may_have_browser = browser_uncertain;
    let result = timeout(Duration::from_secs(8),
        desktop_ua(&fixture.executable, &bridge.startup_children)).await;
    // Join our disposable child before checking the regression result.
    let shutdown = timeout(Duration::from_secs(4), bridge.shutdown()).await;
    let joined = timeout(Duration::from_secs(1), bridge.startup_children.join()).await;
    let pid = std::fs::read_to_string(fixture.root.join("probe.pid")).unwrap();
    assert!(!Path::new(&format!("/proc/{}", pid.trim())).exists(), "probe remains");
    assert_eq!(result.expect("stalled version probe escaped its deadline"), DESKTOP_UA);
    assert_eq!(joined.unwrap(), vec![(None, Ok(()))]);
    if browser_uncertain {
        assert_eq!(shutdown.unwrap().unwrap_err(),
            "could not confirm every owned browser exited cleanly");
    } else {
        shutdown.unwrap().unwrap();
    }
    assert_eq!(route.lock().await.may_have_browser, browser_uncertain);
    assert!(bridge.instances.lock().await.is_empty());
}

#[tokio::test]
async fn stalled_version_probe_times_out_with_retained_cleanup() {
    stalled_probe("pass", false).await;
}

#[tokio::test]
async fn unfinished_version_output_preserves_fallback_and_browser_uncertainty() {
    stalled_probe("print('Chromium 123.4', flush=True)", true).await;
}

use super::*;

async fn fixture() -> Fixture {
    let mut fixture = Fixture::new().await;
    let history = fixture.config.profiles.join(".sidecar/camoufox");
    std::fs::rename(history.join("rate-history.sqlite3"), fixture.root.join("prepared.sqlite3")).unwrap();
    let runtime = fixture.config.runtime_script.parent().unwrap().to_owned();
    let script = fixture.root.join("migration-runtime.py");
    let previous = fixture.root.join("previous.json");
    std::fs::write(&previous, b"{}").unwrap();
    let source = format!(concat!(
        "import os, pathlib, runpy, shutil, sys, time\n",
        "here = pathlib.Path(__file__).parent\n",
        "if '--expire-legacy-from' in sys.argv:\n",
        "    (here / 'entered').write_text(str(os.getpid()))\n",
        "    while not (here / 'release').exists(): time.sleep(0.01)\n",
        "    if (here / 'fail').exists(): raise SystemExit(7)\n",
        "    root = pathlib.Path(sys.argv[sys.argv.index('--root') + 1])\n",
        "    shutil.copyfile(here / 'prepared.sqlite3', root / 'rate-history.sqlite3')\n",
        "else:\n",
        "    sys.path.insert(0, {runtime:?})\n",
        "    runpy.run_path({entry:?}, run_name='__main__')\n"
    ), runtime=runtime.to_str().unwrap(), entry=fixture.config.runtime_script.to_str().unwrap());
    std::fs::write(&script, source).unwrap();
    fixture.config.runtime_script = script;
    fixture.config.expire_legacy_from = Some(previous);
    let reservation = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    fixture.config.address = reservation.local_addr().unwrap();
    fixture
}

async fn entered(fixture: &Fixture) -> u32 {
    timeout(Duration::from_secs(5), async {
        loop {
            if let Ok(pid) = std::fs::read_to_string(fixture.root.join("entered")) {
                if let Ok(pid) = pid.parse() { break pid; }
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }).await.unwrap()
}

async fn healthy(address: SocketAddr) {
    let client = reqwest::Client::builder().no_proxy()
        .timeout(Duration::from_millis(100)).build().unwrap();
    timeout(Duration::from_secs(5), async {
        loop {
            if let Ok(reply) = client.get(format!("http://{address}/api/health")).send().await {
                if reply.status() == 200 { break; }
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }).await.unwrap();
}

#[tokio::test]
async fn migration_holds_profile_and_listener_until_history_is_published() {
    let fixture = fixture().await;
    let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
    let task = tokio::spawn(serve(fixture.config.clone(), async { let _ = stopped.await; }));
    let pid = entered(&fixture).await;
    assert!(!fixture.sessions_available());
    assert!(tokio::net::TcpStream::connect(fixture.config.address).await.is_err());
    std::fs::write(fixture.root.join("release"), b"").unwrap();
    healthy(fixture.config.address).await;
    assert!(!PathBuf::from(format!("/proc/{pid}")).exists());
    assert!(!fixture.sessions_available());
    stop.send(()).unwrap();
    timeout(Duration::from_secs(5), task).await.unwrap().unwrap().unwrap();
    assert!(fixture.sessions_available());
    // Leaving the explicit flag in service configuration cannot repeat expiry.
    std::fs::remove_file(fixture.root.join("previous.json")).unwrap();
    std::fs::remove_file(fixture.root.join("release")).unwrap();
    let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
    let task = tokio::spawn(serve(fixture.config.clone(), async { let _ = stopped.await; }));
    healthy(fixture.config.address).await;
    stop.send(()).unwrap();
    timeout(Duration::from_secs(5), task).await.unwrap().unwrap().unwrap();
    assert!(fixture.sessions_available());
}

#[tokio::test]
async fn cancelled_or_failed_migration_joins_before_releasing_the_profile() {
    for failure in [false, true] {
        let fixture = fixture().await;
        let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
        let task = tokio::spawn(serve(fixture.config.clone(), async { let _ = stopped.await; }));
        let pid = entered(&fixture).await;
        assert!(!fixture.sessions_available());
        if failure {
            std::fs::write(fixture.root.join("fail"), b"").unwrap();
            std::fs::write(fixture.root.join("release"), b"").unwrap();
        } else { stop.send(()).unwrap(); }
        let result = timeout(Duration::from_secs(7), task).await.unwrap().unwrap();
        assert_eq!(result.is_err(), failure);
        assert!(!PathBuf::from(format!("/proc/{pid}")).exists());
        assert!(fixture.sessions_available());
        assert!(!fixture.config.profiles.join(".sidecar/camoufox/rate-history.sqlite3").exists());
        assert!(tokio::net::TcpStream::connect(fixture.config.address).await.is_err());
    }
}

#[tokio::test]
async fn abandoned_startup_still_reaps_its_migration_child() {
    let fixture = fixture().await;
    let task = tokio::spawn(serve(fixture.config.clone(), std::future::pending()));
    let pid = entered(&fixture).await;
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    timeout(Duration::from_secs(7), async {
        while !fixture.sessions_available() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }).await.unwrap();
    assert!(!PathBuf::from(format!("/proc/{pid}")).exists());
    assert!(!fixture.config.profiles.join(".sidecar/camoufox/rate-history.sqlite3").exists());
}

#[tokio::test]
async fn explicit_expiry_does_not_bypass_real_runtime_proof() {
    let mut fixture = fixture().await;
    fixture.config.runtime_script = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../runtime/native.py").canonicalize().unwrap();
    let error = serve(fixture.config.clone(), std::future::pending()).await.unwrap_err();
    assert!(error.contains("legacy migration failed"));
    assert!(fixture.sessions_available());
    assert!(!fixture.config.profiles.join(".sidecar/camoufox/rate-history.sqlite3").exists());
}

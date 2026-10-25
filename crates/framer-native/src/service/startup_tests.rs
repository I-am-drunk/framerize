use super::*;
use sha2::{Digest, Sha256};
use std::process::Command;
use tokio::time::{timeout, Duration};

#[path = "startup_migration_tests.rs"]
mod migration;

struct Fixture { root: PathBuf, config: ServeConfig }
impl Fixture {
    async fn new() -> Self {
        let root = std::env::temp_dir().join(format!("framerize-native-startup-{}", uuid::Uuid::new_v4()));
        let profiles = root.join("profiles");
        std::fs::create_dir_all(profiles.join(".sessions")).unwrap();
        let runtime = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../runtime")
            .canonicalize().unwrap();
        let result = Command::new("/usr/bin/python3").args(["-I", "-B"])
            .arg(runtime.join("tests_native/seed_ledger.py")).arg(profiles.join(".sidecar/camoufox"))
            .env_clear().env("PATH", "/usr/bin:/bin").output().unwrap();
        assert!(result.status.success());
        let database = root.join("accounts.db");
        let store = SqliteStateStore::open(database.to_str().unwrap()).unwrap();
        let id = uuid::Uuid::from_u128(1);
        let account = serde_json::from_value(serde_json::json!({
            "id": id, "alias": "fixture", "framer_user_id": "fixture-user",
            "castle_device_id": "fixture-device", "current_team": "fixture-team",
            "current_project": "fixture-project", "state": framer_core::account::AccountState::Ready,
            "credit": null, "consecutive_failures": 0, "cooldown_step": 0,
        })).unwrap();
        store.upsert(&account).await.unwrap();
        let node = root.join("inert-node");
        let entry = root.join("inert-entry.js");
        let contents = b"This fixture must never execute.\n";
        for path in [&node, &entry] { std::fs::write(path, contents).unwrap(); }
        let digest = format!("{:x}", Sha256::digest(contents));
        let worker_config = root.join("worker.json");
        std::fs::write(&worker_config, serde_json::to_vec(&serde_json::json!({
            "node": node, "entry": entry, "pins": [
                {"path": node, "sha256": digest}, {"path": entry, "sha256": digest}]
        })).unwrap()).unwrap();
        let config = ServeConfig { database, profiles,
            python: PathBuf::from("/usr/bin/python3"), runtime_script: runtime.join("native.py"),
            worker_config: worker_config.to_str().unwrap().into(),
            selected_accounts: vec![id.to_string()], address: "127.0.0.1:0".parse().unwrap(),
            expire_legacy_from: None };
        Self { root, config }
    }
    fn sessions_available(&self) -> bool {
        FileSessionStore::new(self.config.profiles.join(".sessions")).is_ok()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.root); }
}

#[tokio::test]
async fn missing_history_and_mismatched_pins_refuse_startup() {
    for missing_history in [true, false] {
        let fixture = Fixture::new().await;
        if missing_history {
            std::fs::remove_dir_all(fixture.config.profiles.join(".sidecar")).unwrap();
        } else { std::fs::write(fixture.root.join("inert-entry.js"), b"changed").unwrap(); }
        let error = serve(fixture.config.clone(), std::future::pending()).await.unwrap_err();
        assert!(error.contains(if missing_history { "migrated account history" } else { "input verification" }));
        assert!(fixture.sessions_available());
    }
}

#[tokio::test]
async fn existing_profile_owner_prevents_startup() {
    let fixture = Fixture::new().await;
    let owner = FileSessionStore::new(fixture.config.profiles.join(".sessions")).unwrap();
    let error = serve(fixture.config.clone(), std::future::pending()).await.unwrap_err();
    assert!(error.contains("session store ownership unavailable"));
    drop(owner);
    assert!(fixture.sessions_available());
}

#[tokio::test]
async fn unknown_selected_account_refuses_before_starting_the_ledger() {
    let fixture = Fixture::new().await;
    let mut config = fixture.config.clone();
    config.selected_accounts = vec![uuid::Uuid::from_u128(3).to_string()];
    config.runtime_script = fixture.root.join("must-not-launch.py");
    let error = serve(config, std::future::pending()).await.unwrap_err();
    assert!(error.contains("not in this profile"));
    assert!(fixture.sessions_available());
    // The same profile must be reusable after this preflight refusal.
    serve(fixture.config.clone(), async {}).await.unwrap();
}

#[tokio::test]
async fn loopback_service_joins_shutdown_and_releases_the_profile() {
    let fixture = Fixture::new().await;
    let reservation = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let mut config = fixture.config.clone();
    config.address = reservation.local_addr().unwrap();
    let url = format!("http://{}", config.address);
    drop(reservation);
    let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
    let task = tokio::spawn(serve(config, async { let _ = stopped.await; }));
    let client = reqwest::Client::builder().no_proxy().timeout(Duration::from_millis(200)).build().unwrap();
    timeout(Duration::from_secs(5), async {
        loop {
            if let Ok(reply) = client.get(format!("{url}/api/health")).send().await {
                assert_eq!(reply.status(), 200);
                assert_eq!(reply.headers()["x-framerize-pid"], std::process::id().to_string());
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }).await.unwrap();
    assert!(!fixture.sessions_available());
    let invalid = client.post(format!("{url}/api/offline/complete")).json(&serde_json::json!({}))
        .send().await.unwrap();
    assert!(invalid.status().is_client_error());
    stop.send(()).unwrap();
    timeout(Duration::from_secs(5), task).await.unwrap().unwrap().unwrap();
    assert!(fixture.sessions_available());
    serve(fixture.config.clone(), async {}).await.unwrap();
}

#[tokio::test]
async fn unexpected_ledger_exit_stops_the_listener_and_releases_ownership() {
    let fixture = Fixture::new().await;
    let script = fixture.root.join("exiting-ledger.py");
    std::fs::write(&script, concat!(
        "import pathlib, sys, time\n",
        "print('{\"ready\":1}', flush=True)\n",
        "while not (pathlib.Path(sys.argv[-1]) / 'exit').exists(): time.sleep(0.01)\n",
        "raise SystemExit(7)\n",
    )).unwrap();
    let reservation = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = reservation.local_addr().unwrap();
    let mut config = fixture.config.clone();
    config.address = address;
    config.runtime_script = script;
    drop(reservation);
    let task = tokio::spawn(serve(config, std::future::pending()));
    let client = reqwest::Client::builder().no_proxy()
        .timeout(Duration::from_millis(200)).build().unwrap();
    timeout(Duration::from_secs(5), async {
        loop {
            if let Ok(reply) = client.get(format!("http://{address}/api/health")).send().await {
                assert_eq!(reply.status(), 200);
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }).await.unwrap();
    assert!(!fixture.sessions_available());
    std::fs::write(fixture.config.profiles.join(".sidecar/camoufox/exit"), b"").unwrap();
    let result = timeout(Duration::from_secs(5), task).await.unwrap().unwrap();
    assert_eq!(result.unwrap_err(), "native ledger shutdown is unconfirmed");
    assert!(tokio::net::TcpStream::connect(address).await.is_err());
    assert!(fixture.sessions_available());
    serve(fixture.config.clone(), async {}).await.unwrap();
}

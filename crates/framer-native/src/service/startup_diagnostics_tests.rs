//! Exercise startup and the real event sink in separate processes.
use super::*;
use std::path::Path;
use std::process::{Child, Stdio};

const CHILD_CASE: &str = "FRAMER_TEST_STARTUP_DIAGNOSTICS_CASE";
const CHILD_ROOT: &str = "FRAMER_TEST_STARTUP_DIAGNOSTICS_ROOT";

struct JoinedChild(Child);
impl Drop for JoinedChild {
    fn drop(&mut self) { let _ = self.0.kill(); let _ = self.0.wait(); }
}

fn isolated(name: &str, run: impl FnOnce(&Path)) {
    if std::env::var(CHILD_CASE).ok().as_deref() == Some(name) {
        let root = PathBuf::from(std::env::var_os(CHILD_ROOT).unwrap());
        run(&root);
        std::fs::write(root.join("completed"), name).unwrap();
        return;
    }
    let root = std::env::temp_dir().join(format!("native-startup-log-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&root).unwrap();
    let module = module_path!().split_once("::").unwrap().1;
    let mut child = JoinedChild(Command::new(std::env::current_exe().unwrap())
        .args(["--exact", &format!("{module}::{name}"), "--nocapture", "--test-threads=1"])
        .env(CHILD_CASE, name).env(CHILD_ROOT, &root)
        .stdin(Stdio::null()).stdout(Stdio::inherit()).stderr(Stdio::inherit())
        .spawn().unwrap());
    let until = std::time::Instant::now() + Duration::from_secs(20);
    loop {
        if let Some(status) = child.0.try_wait().unwrap() {
            assert!(status.success(), "isolated fixture {name}: {status}");
            assert_eq!(std::fs::read_to_string(root.join("completed")).unwrap(), name);
            std::fs::remove_dir_all(root).unwrap();
            return;
        }
        assert!(std::time::Instant::now() < until, "isolated fixture timed out");
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn service_case(name: &str, broken_log: bool, non_utf8: bool) {
    isolated(name, |_| {
        tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap()
            .block_on(serving_case(broken_log, non_utf8));
    });
}

#[test]
fn startup_initializes_real_request_diagnostics() {
    service_case("startup_initializes_real_request_diagnostics", false, false);
}

#[test]
fn unavailable_log_keeps_serving_and_cleanup_working() {
    service_case("unavailable_log_keeps_serving_and_cleanup_working", true, false);
}

#[cfg(unix)]
#[test]
fn non_utf8_profile_keeps_its_actual_log_destination() {
    service_case("non_utf8_profile_keeps_its_actual_log_destination", false, true);
}

fn rows(root: &Path) -> Vec<serde_json::Value> {
    let text = std::fs::read_dir(root.join(".logs")).unwrap().map(|entry|
        std::fs::read_to_string(entry.unwrap().path()).unwrap()).collect::<String>();
    text.lines().map(|line| serde_json::from_str(line).unwrap()).collect()
}

#[test]
fn unavailable_log_recovers_without_panicking_or_replacing_state() {
    isolated("unavailable_log_recovers_without_panicking_or_replacing_state", |root| {
        let path = root.join(".logs");
        std::fs::write(&path, b"preserve-this-file").unwrap();
        framer_core::observability::init(root.to_str().unwrap());
        let result = std::panic::catch_unwind(|| {
            framer_core::observability::Event::new("diagnostic.unavailable").emit();
        });
        assert!(result.is_ok(), "an unavailable log must not panic");
        assert_eq!(std::fs::read(&path).unwrap(), b"preserve-this-file");
        std::fs::remove_file(&path).unwrap();
        std::fs::create_dir(&path).unwrap();
        framer_core::observability::Event::new("diagnostic.recovered").emit();
        let records = rows(root);
        assert_eq!(records.len(), 1);
        assert_eq!(records[0]["event"], "diagnostic.recovered");
    });
}

fn request() -> serde_json::Value {
    serde_json::json!({
        "account_id":"00000000-0000-0000-0000-000000000001",
        "user_id":"fixture-user", "team_id":"fixture-team", "project_id":"fixture-project",
        "conversation_id":"fixture-conversation",
        "page_url":"https://framer.com/projects/fixture-project",
        "route":"/ai/v3/responses/", "feature_category":"fixture-category",
        "feature_name":"fixture-feature", "step":"fixture-step",
        "turn_id":"fixture-turn", "user_prompt":"private-diagnostic-fixture",
        "timeout_ms":3000, "token_limit":32,
        "request":{"model":"gpt-4.1", "input":"private-diagnostic-fixture", "stream":false}
    })
}

async fn serving_case(broken_log: bool, non_utf8: bool) {
    let mut fixture = Fixture::new().await;
    #[cfg(unix)]
    if non_utf8 {
        use std::os::unix::ffi::OsStringExt;
        let profiles = fixture.root.join(std::ffi::OsString::from_vec(b"profiles-\x80".to_vec()));
        std::fs::rename(&fixture.config.profiles, &profiles).unwrap();
        fixture.config.profiles = profiles;
    }
    #[cfg(not(unix))]
    assert!(!non_utf8);
    if broken_log {
        std::fs::write(fixture.config.profiles.join(".logs"), b"preserve-this-file").unwrap();
    }
    let reservation = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let mut config = fixture.config.clone();
    config.address = reservation.local_addr().unwrap();
    let url = format!("http://{}", config.address);
    drop(reservation);
    let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
    let mut task = tokio::spawn(serve(config, async { let _ = stopped.await; }));
    let client = reqwest::Client::builder().no_proxy().timeout(Duration::from_secs(2)).build().unwrap();
    let ready = timeout(Duration::from_secs(5), async {
        loop {
            if let Ok(reply) = client.get(format!("{url}/api/health")).send().await {
                if reply.status().is_success() { break; }
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }).await;
    let reply = if ready.is_ok() {
        Some(client.post(format!("{url}/api/offline/complete")).json(&request()).send().await)
    } else { None };
    let _ = stop.send(());
    let finished = timeout(Duration::from_secs(5), &mut task).await;
    if finished.is_err() { task.abort(); let _ = task.await; }
    ready.unwrap();
    finished.unwrap().unwrap().unwrap();
    assert!(fixture.sessions_available());
    let reply = reply.unwrap().unwrap();
    assert!(reply.status().is_client_error() || reply.status().is_server_error());
    let body: serde_json::Value = reply.json().await.unwrap();
    if broken_log {
        assert_eq!(std::fs::read(fixture.config.profiles.join(".logs")).unwrap(), b"preserve-this-file");
        return;
    }
    let records = rows(&fixture.config.profiles);
    let terminal: Vec<_> = records.iter().filter(|row|
        matches!(row["event"].as_str(), Some("offline.complete" | "offline.request.finished"))).collect();
    assert_eq!(terminal.len(), 2, "real startup must activate both request recorders");
    assert_eq!(terminal[0]["request_id"], terminal[1]["request_id"]);
    for row in terminal {
        assert_eq!(row["completion"]["failure"], body["error"]["code"]);
        assert_eq!(row["completion"]["cleanup_confirmed"], true);
        assert!(row["completion"]["attempt"].is_null(), "no upstream attempt is expected");
    }
    let serialized = serde_json::to_string(&records).unwrap();
    for private in ["private-diagnostic-fixture", "fixture-user", "fixture-team",
        "fixture-project", "00000000-0000-0000-0000-000000000001"] {
        assert!(!serialized.contains(private), "private request data reached diagnostics");
    }
}

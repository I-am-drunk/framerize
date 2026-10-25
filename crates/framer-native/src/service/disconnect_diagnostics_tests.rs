//! Inert serving ports only. Each case isolates the process-global logger.
use super::*;
use futures::StreamExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use serde_json::Value;

const CHILD_CASE: &str = "FRAMER_TEST_DISCONNECT_DIAGNOSTICS_CASE";
const CHILD_ROOT: &str = "FRAMER_TEST_DISCONNECT_DIAGNOSTICS_ROOT";
const TERMINAL: &str = "offline.request.finished";

struct Directory(PathBuf);
impl Drop for Directory {
    fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); }
}
struct JoinedChild(Child);
impl Drop for JoinedChild {
    fn drop(&mut self) { let _ = self.0.kill(); let _ = self.0.wait(); }
}

fn isolated(name: &str, run: impl FnOnce(&Path)) {
    if std::env::var(CHILD_CASE).ok().as_deref() == Some(name) {
        let root = PathBuf::from(std::env::var_os(CHILD_ROOT).expect("fixture root"));
        run(&root);
        std::fs::write(root.join("completed"), name).unwrap();
        return;
    }
    let root = Directory(std::env::temp_dir().join(format!("native-disconnect-{}", uuid::Uuid::new_v4())));
    std::fs::create_dir(&root.0).unwrap();
    let module = module_path!().split_once("::").unwrap().1;
    let mut child = JoinedChild(Command::new(std::env::current_exe().unwrap())
        .args(["--exact", &format!("{module}::{name}"), "--nocapture", "--test-threads=1"])
        .env(CHILD_CASE, name).env(CHILD_ROOT, &root.0)
        .stdin(Stdio::null()).stdout(Stdio::inherit()).stderr(Stdio::inherit())
        .spawn().expect("start isolated fixture"));
    let until = std::time::Instant::now() + Duration::from_secs(30);
    loop {
        if let Some(status) = child.0.try_wait().expect("observe fixture child") {
            assert!(status.success(), "isolated fixture {name}: {status}");
            assert_eq!(std::fs::read_to_string(root.0.join("completed")).unwrap(), name,
                "the exact child test must actually run");
            return;
        }
        assert!(std::time::Instant::now() < until, "isolated fixture {name} timed out");
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[derive(Clone, Copy)]
enum Client { Json, SseUnpolled, SsePolled }
enum Case { Disconnect(Client, bool), BrokenLog }

fn run_case(name: &str, case: Case) {
    isolated(name, |root| {
        if matches!(case, Case::BrokenLog) {
            std::fs::write(root.join(".logs"), b"task-owned regular file").unwrap();
        }
        framer_core::observability::init(root.to_str().unwrap());
        let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        runtime.block_on(async {
            match case {
                Case::Disconnect(client, uncertain) => disconnected(root, client, uncertain).await,
                Case::BrokenLog => broken_log(root).await,
            }
        });
    });
}

fn rows(root: &Path) -> Vec<Value> {
    let mut rows = Vec::new();
    for file in std::fs::read_dir(root.join(".logs")).unwrap() {
        let path = file.unwrap().path();
        if path.extension().and_then(|v| v.to_str()) != Some("ndjson") { continue; }
        assert!(std::fs::metadata(&path).unwrap().len() <= 65_536);
        let text = std::fs::read_to_string(path).unwrap();
        assert!(text.is_empty() || text.ends_with('\n'));
        rows.extend(text.lines().map(|line| serde_json::from_str::<Value>(line).unwrap()));
    }
    rows
}

fn terminal_count(root: &Path) -> usize {
    rows(root).iter().filter(|row| row["event"] == TERMINAL).count()
}

async fn disconnected(root: &Path, client: Client, uncertain: bool) {
    let (state, ports, mut spec, sessions) = fixture();
    // Cancellation, not the fixture's shorter default deadline, owns this outcome.
    spec.deadline = Instant::now() + Duration::from_secs(10);
    spec.payload["input"] = "disconnect-private-payload".into();
    let account = spec.identity.account.0.to_string();
    let other = ports.other_id.0.to_string();
    ports.issue.try_acquire().unwrap().forget();
    ports.uncertain.store(uncertain, Ordering::SeqCst);
    let (sender, receiver) = tokio::sync::mpsc::channel(1);
    let run = if matches!(client, Client::Json) {
        drop(sender);
        start(state.clone(), &ports, spec, &sessions)
    } else {
        launch_with_source(state.clone(), spec, Arc::new(Builder(ports.clone())),
            sessions, Some(sender)).unwrap()
    };
    let diagnostic_id = run.diagnostic_id.to_string();
    assert_ne!(diagnostic_id, account);
    wait(&ports.issuing).await;
    assert_eq!(terminal_count(root), 0, "issuance is still held");
    if matches!(client, Client::Json) {
        drop(run.result);
        drop(receiver);
    } else {
        let mut body = crate::service::stream::response(run.result, receiver,
            FreeRoute::Responses, "gpt-4.1".into(), run.diagnostic_id)
            .into_body().into_data_stream();
        if matches!(client, Client::SsePolled) {
            let opening = tokio::time::timeout(Duration::from_secs(2), body.next())
                .await.unwrap().unwrap().unwrap();
            assert!(!opening.is_empty());
            assert!(body.next().now_or_never().is_none(), "held issuance cannot finish the stream");
        }
        drop(body);
    }
    tokio::time::timeout(Duration::from_secs(2), async {
        while !ports.cancelled.load(Ordering::SeqCst) { tokio::task::yield_now().await; }
    }).await.unwrap();
    assert!(run.cleanup.clone().now_or_never().is_none());
    assert_eq!(terminal_count(root), 0, "disconnect must not report unfinished cleanup");
    ports.issue.add_permits(1);
    let joined = tokio::time::timeout(Duration::from_secs(2), run.cleanup).await.unwrap();
    assert_eq!(joined, if uncertain { Err(FailureKind::Cleanup) } else { Ok(()) });
    let records = rows(root);
    let terminal: Vec<_> = records.iter().filter(|row| row["event"] == TERMINAL).collect();
    assert_eq!(terminal.len(), 1);
    let row = terminal[0];
    assert_eq!(row["request_id"], diagnostic_id);
    assert_eq!(row["completion"]["failure"], if uncertain { "Cleanup" } else { "Cancelled" });
    assert_eq!(row["completion"]["cleanup_confirmed"], !uncertain);
    assert!(records.iter().all(|row| row["event"] != "offline.complete"));
    assert_eq!(ports.builds.load(Ordering::SeqCst), 1);
    assert_private_absent(&records, &[&account, &account[..8], &other, &other[..8]]);
}

fn assert_private_absent(records: &[Value], identities: &[&str]) {
    for row in records {
        let text = row.to_string();
        for marker in identities.iter().copied().chain([
            "fixture", "session=", "disconnect-private-payload",
        ]) {
            assert!(!text.contains(marker), "private fixture value reached a diagnostic");
        }
        for key in ["account", "session", "request", "response", "context", "token", "cookie"] {
            assert!(row.get(key).is_none(), "unexpected private diagnostic field: {key}");
        }
    }
}

async fn broken_log(root: &Path) {
    let (state, ports, spec, sessions) = fixture();
    let run = start(state, &ports, spec, &sessions);
    let diagnostic_id = run.diagnostic_id;
    let outcome = finish(run).await;
    assert_eq!(outcome.failure, Some(FailureKind::Authorization));
    assert!(outcome.cleanup_confirmed && outcome.response.is_none());
    // Exercise the result-consumer recorder too. Both use the non-directory.
    crate::service::http::record_completion(&outcome, diagnostic_id);
    assert_eq!(outcome.failure, Some(FailureKind::Authorization));
    assert!(outcome.cleanup_confirmed && outcome.response.is_none());
    let scope = ports.scope.lock().unwrap().clone().unwrap();
    assert_eq!(scope.closed().await, Ok(()));
    assert_eq!(std::fs::read(root.join(".logs")).unwrap(), b"task-owned regular file");
}

#[test]
fn dropped_json_retains_one_terminal_diagnostic() {
    run_case("dropped_json_retains_one_terminal_diagnostic", Case::Disconnect(Client::Json, false));
}
#[test]
fn unpolled_sse_retains_one_terminal_diagnostic() {
    run_case("unpolled_sse_retains_one_terminal_diagnostic", Case::Disconnect(Client::SseUnpolled, false));
}
#[test]
fn polled_sse_retains_one_terminal_diagnostic() {
    run_case("polled_sse_retains_one_terminal_diagnostic", Case::Disconnect(Client::SsePolled, false));
}

#[test]
fn dropped_json_preserves_uncertain_cleanup_diagnostic() {
    run_case("dropped_json_preserves_uncertain_cleanup_diagnostic", Case::Disconnect(Client::Json, true));
}
#[test]
fn unpolled_sse_preserves_uncertain_cleanup_diagnostic() {
    run_case("unpolled_sse_preserves_uncertain_cleanup_diagnostic", Case::Disconnect(Client::SseUnpolled, true));
}
#[test]
fn polled_sse_preserves_uncertain_cleanup_diagnostic() {
    run_case("polled_sse_preserves_uncertain_cleanup_diagnostic", Case::Disconnect(Client::SsePolled, true));
}
#[test]
fn logger_panics_preserve_request_result_and_cleanup() {
    run_case("logger_panics_preserve_request_result_and_cleanup", Case::BrokenLog);
}

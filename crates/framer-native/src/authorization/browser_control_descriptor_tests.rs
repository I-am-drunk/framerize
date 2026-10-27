use super::*;
use crate::account::client::NativeAccountClient;
use crate::authorization::control::browser_control_fixture::{fixture, target, Ports};
use crate::transport::http::{NativeEgress, NativeHttpLimits};
use tokio::sync::Semaphore;

struct HeldShutdown { entered: Semaphore, release: Semaphore }
impl BrowserCredentialControl for HeldShutdown {
    fn prepare(&self) -> BoxFuture<'_, bool> { async { false }.boxed() }
    fn mint<'a>(&'a self, _: &'a AccountScope, _: &'a Session, _: &'a DeviceGrant,
        _: &'a OfflineMintTarget, _: u64, _: &'a PreparationControl)
        -> BoxFuture<'a, PreparationResult<BrowserControlMint>> {
        async { PreparationResult::unconfirmed() }.boxed()
    }
    fn finish<'a>(&'a self, _: &'a DeviceGrant, _: bool, _: bool) -> BoxFuture<'a, bool> {
        async { false }.boxed()
    }
    fn shutdown(&self) -> BoxFuture<'_, bool> {
        async move {
            self.entered.add_permits(1);
            self.release.acquire().await.unwrap().forget();
            true
        }.boxed()
    }
}

fn grant(scope: &AccountScope) -> DeviceGrant {
    DeviceGrant { account: scope.account(), host_generation: "a".repeat(32),
        lease: uuid::Uuid::new_v4(), device: "fixture-device".into(),
        authority: crate::account::device::DeviceAuthority::Ledger }
}

#[tokio::test]
async fn shutdown_closes_admission_before_joining_the_existing_peer() {
    let (client, scope) = fixture::scope(NativeEgress::Direct);
    let control = configured(settings(&scope));
    let held = Arc::new(HeldShutdown { entered: Semaphore::new(0), release: Semaphore::new(0) });
    control.state.lock().unwrap().peer = Some(held.clone());
    let closing = control.clone();
    let job = tokio::spawn(async move { closing.shutdown().await });
    let entered = timeout(Duration::from_secs(2), held.entered.acquire()).await;
    let refused = control.install(&scope, &fixture::session()).is_err();
    let no_mint_peer = control.peer(true).unwrap().is_none();
    let pending = !job.is_finished();
    held.release.add_permits(1);
    let result = job.await.unwrap();
    close(&client, &scope).await;
    assert!(entered.is_ok() && refused && no_mint_peer && pending && result);
}

#[tokio::test]
async fn finish_awaits_the_owned_peer_and_preserves_cleanup_flags() {
    let (client, scope) = fixture::scope(NativeEgress::Direct);
    let control = configured(settings(&scope));
    let ports = Ports::new();
    ports.finish_release.try_acquire().unwrap().forget();
    control.state.lock().unwrap().peer = Some(ports.clone());
    let peer = control.clone(); let grant = grant(&scope);
    let finishing = tokio::spawn(async move { peer.finish(&grant, true, true).await });
    let entered = timeout(Duration::from_secs(2), ports.finish_entered.acquire()).await;
    let waiting = !finishing.is_finished();
    ports.finish_release.add_permits(1);
    let result = finishing.await.unwrap();
    let flags = ports.finished.lock().unwrap().clone();
    let closed = control.shutdown().await;
    close(&client, &scope).await;
    assert!(entered.is_ok() && waiting && result && closed);
    assert_eq!(flags, [(true, true)]);
}

struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("control-descriptor-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for Directory {
    fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); }
}

fn digest(bytes: &[u8]) -> String { format!("{:x}", Sha256::digest(bytes)) }

fn waiting_child(scope: &AccountScope, root: &Directory) -> Arc<ConfiguredControl> {
    let mut value = settings(scope);
    value.python = std::env::var_os("FRAMERIZE_TEST_CONTROL_PYTHON").unwrap().into();
    value.python_sha256 = std::env::var("FRAMERIZE_TEST_CONTROL_PYTHON_SHA256").unwrap();
    assert_eq!(digest(&std::fs::read(&value.python).unwrap()), value.python_sha256);
    value.script = root.0.join("child.py");
    value.state_directory = root.0.clone();
    value.prepare_ms = 3000;
    let code = b"import json, os, pathlib, sys, time\nroot = pathlib.Path(sys.argv[sys.argv.index('--state-directory') + 1])\nassert json.loads(sys.stdin.readline())['op'] == 'prepare'\n(root / 'started').write_text(str(os.getpid()))\nwhile True: time.sleep(0.01)\n";
    std::fs::write(&value.script, code).unwrap();
    value.script_sha256 = digest(code);
    configured(value)
}

async fn started(root: &Directory) -> Option<i32> {
    timeout(Duration::from_secs(2), async {
        loop {
            if let Ok(value) = std::fs::read_to_string(root.0.join("started")) {
                if let Ok(pid) = value.parse() { return pid; }
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }).await.ok()
}

#[tokio::test]
async fn cancelled_real_child_remains_owned_until_reaped() {
    let root = Directory::new();
    let (client, scope) = fixture::scope(NativeEgress::Direct);
    let peer = waiting_child(&scope, &root);
    let owner = peer.clone(); let expected = scope.clone();
    let pending = client.prepare_owned(&scope, Instant::now() + Duration::from_secs(4),
        move |session, stop| async move {
            PreparationResult::finished(Ok(owner.prepare_for_account(&expected, &session, &stop).await))
        }).unwrap();
    let pid = started(&root).await;
    pending.cancellation().cancel();
    let prepared = pending.await;
    let shutdown = peer.shutdown().await;
    close(&client, &scope).await;
    let pid = pid.expect("inert child reached preparation");
    let absent = unsafe { libc::kill(pid, 0) } == -1
        && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH);
    assert!(absent, "child remains after preparation and shutdown joined");
    assert_eq!(prepared, Err(PreparationFailure::Cancelled));
    assert!(!shutdown, "forced cleanup must not become confirmed cleanup");
    assert!(peer.peer(true).unwrap().is_none());
}

#[tokio::test]
async fn binding_rejects_closed_generation_and_uses_reopened_session() {
    let (client, old) = fixture::scope(NativeEgress::Direct);
    let value = settings(&old);
    fixture::close(&client, &old).await;
    let mut current = fixture::session(); current.cookie_header = "session=reopened-fixture".into();
    let fresh = client.open_account(old.account(), current.clone(), NativeHttpLimits::default(),
        NativeEgress::Direct).unwrap();
    let stale = value.binding(&old, &current).is_err();
    let binding = value.binding(&fresh, &current);
    close(&client, &fresh).await;
    assert!(stale && !old.same_generation(&fresh));
    assert_eq!(binding.unwrap().session, "reopened-fixture");
}

#[tokio::test]
async fn installation_is_single_use_and_shutdown_before_prepare_is_permanent() {
    let (client, scope) = fixture::scope(NativeEgress::Direct);
    let control = configured(settings(&scope));
    let peer = control.install(&scope, &fixture::session()).unwrap();
    let twice = control.install(&scope, &fixture::session()).is_err();
    let closed = control.shutdown().await;
    let closed_peer = peer.shutdown().await;
    let fresh = configured(settings(&scope));
    let fresh_closed = fresh.shutdown().await;
    let after_close = fresh.install(&scope, &fixture::session()).is_err();
    close(&client, &scope).await;
    assert!(twice && closed && closed_peer && fresh_closed && after_close);
}
fn configured(settings: Settings) -> Arc<ConfiguredControl> {
    settings.validate().unwrap();
    Arc::new(ConfiguredControl { settings, state: SyncMutex::default() })
}

#[tokio::test]
async fn rejected_binding_cannot_be_retried_with_a_different_identity() {
    let (client, scope) = fixture::scope(NativeEgress::Direct);
    let control = configured(settings(&scope));
    let mut wrong = fixture::session(); wrong.active_user.0.push('x');
    let rejected = control.install(&scope, &wrong).is_err();
    let retry = control.install(&scope, &fixture::session()).is_err();
    let no_peer = control.peer(false).unwrap().is_none();
    let closed = control.shutdown().await;
    close(&client, &scope).await;
    assert!(rejected && retry && no_peer && closed);
}

#[tokio::test]
async fn cancelled_scoped_preparation_never_installs_a_peer() {
    let (client, scope) = fixture::scope(NativeEgress::Direct);
    let peer = configured(settings(&scope));
    let control = peer.clone(); let expected = scope.clone();
    let result = client.prepare_owned(&scope, Instant::now() + Duration::from_secs(2),
        move |session, stop| async move {
            stop.cancel();
            PreparationResult::finished(Ok(control.prepare_for_account(&expected, &session, &stop).await))
        }).unwrap().await;
    let no_peer = peer.peer(false).unwrap().is_none();
    let closed = peer.shutdown().await;
    close(&client, &scope).await;
    assert_eq!(result, Err(PreparationFailure::Cancelled));
    assert!(no_peer && closed);
}

fn document() -> serde_json::Value {
    json!({"version":1, "account":"00000000-0000-0000-0000-000000000001",
        "user":"bound-control-fixture", "device":"fixture-device",
        "page_url":"https://framer.com/projects/fixture-project",
        "wrapper_url":"https://app.framerstatic.com/fixture-wrapper.mjs",
        "post_path":"/ai/v3/responses/", "python":"/descriptor-fixture/python",
        "python_sha256":"a".repeat(64), "script":"/descriptor-fixture/child.py",
        "script_sha256":"b".repeat(64), "state_directory":"/descriptor-fixture/state",
        "egress_endpoint":"https://example.com/ip?preserved=", "proxy_revision":0,
        "prepare_ms":180000, "probe_ms":30000, "witness_ms":180000})
}

#[test]
fn descriptor_rejects_credentials_arguments_and_unknown_fields_without_echoing_them() {
    let root = Directory::new();
    for field in ["session", "cookie_header", "bearer", "proxy", "arguments", "unknown"] {
        let mut value = document();
        value[field] = json!("fixture-private-never-echo");
        let (path, hash) = save(&root, &value);
        assert_eq!(load_browser_control(&path, &hash).err(), Some(ERROR));
    }
    for bytes in [b"{malformed fixture-private-never-echo".to_vec(),
        serde_json::to_vec(&document()).unwrap().into_iter().chain([0xff]).collect()] {
        let path = root.0.join("invalid.json");
        std::fs::write(&path, &bytes).unwrap();
        assert_eq!(load_browser_control(&path, &digest(&bytes)).err(), Some(ERROR));
    }
}

#[test]
fn descriptor_rejects_unbounded_timings_and_unpinned_launch_values() {
    let root = Directory::new();
    for (field, value) in [("version", json!(2)), ("prepare_ms", json!(0)),
        ("prepare_ms", json!(180001)), ("probe_ms", json!(30001)),
        ("witness_ms", json!(180001)), ("probe_ms", json!(0.5)),
        ("python", json!("relative")), ("state_directory", json!("relative")),
        ("python_sha256", json!("a".repeat(63))), ("script_sha256", json!("z".repeat(64)))] {
        let mut item = document(); item[field] = value;
        let (path, hash) = save(&root, &item);
        assert_eq!(load_browser_control(&path, &hash).err(), Some(ERROR), "{field}");
    }
}

fn settings(scope: &AccountScope) -> Settings {
    let mut value = document();
    value["account"] = json!(scope.account());
    value["user"] = json!(scope.active_user());
    serde_json::from_value(value).unwrap()
}

#[test]
fn descriptor_rejects_credential_urls_reserved_nonces_and_invalid_context() {
    let root = Directory::new();
    for (field, value) in [("page_url", "https://other.example/projects/test"),
        ("page_url", "https://secret@framer.com/projects/test"),
        ("wrapper_url", "https://example.com/wrapper#hidden"),
        ("egress_endpoint", "https://example.com/ip?%66ramerize_probe="),
        ("egress_endpoint", "http://example.com/ip"), ("post_path", "/path?hidden=1"),
        ("account", "00000000-0000-0000-0000-000000000000"), ("device", "bad device")] {
        let mut item = document(); item[field] = json!(value);
        let (path, hash) = save(&root, &item);
        assert_eq!(load_browser_control(&path, &hash).err(), Some(ERROR), "{field}");
    }
}

#[test]
fn generated_arguments_preserve_exact_endpoint_and_fractional_limits() {
    let mut value: Settings = serde_json::from_value(document()).unwrap();
    value.prepare_ms = 1001; value.probe_ms = 17; value.witness_ms = 2002;
    value.proxy_revision = u64::MAX;
    let args = value.launch().arguments;
    let args: Vec<_> = args.iter().map(|s| s.to_str().unwrap()).collect();
    assert_eq!(args, vec!["--egress-endpoint", "https://example.com/ip?preserved=",
        "--state-directory", "/descriptor-fixture/state", "--proxy-revision", "18446744073709551615",
        "--prepare-seconds", "1.001", "--probe-seconds", "0.017", "--witness-seconds", "2.002"]);
    assert!(args.iter().all(|s| !s.contains("cookie") && !s.contains("bound-control-fixture")));
}

async fn close(client: &NativeAccountClient, scope: &AccountScope) {
    fixture::close(client, scope).await;
    client.shutdown_idle().await.unwrap();
}

#[tokio::test]
async fn binding_uses_the_owned_session_and_exact_private_account_proxy() {
    for proxy in [None, Some("http://fixture-user:fixture-password@proxy.invalid:8080")] {
        let route = proxy.map(NativeEgress::Proxy).unwrap_or(NativeEgress::Direct);
        let (client, scope) = fixture::scope(route);
        let settings = settings(&scope);
        let expected = scope.clone();
        let binding = client.prepare_owned(&scope, Instant::now() + Duration::from_secs(2),
            move |session, _| async move {
                PreparationResult::finished(settings.binding(&expected, &session)
                    .map_err(|_| PreparationFailure::Rejected))
            }).unwrap().await;
        close(&client, &scope).await;
        let binding = binding.unwrap();
        assert_eq!(binding.account, scope.account());
        assert_eq!(binding.user, *scope.active_user());
        assert_eq!(binding.session, "fixture-only-cookie");
        assert_eq!(binding.proxy.as_deref(), proxy);
    }
}

#[tokio::test]
async fn binding_rejects_other_accounts_users_and_conflicting_device_cookies() {
    let (client, scope) = fixture::scope(NativeEgress::Direct);
    let mut value = settings(&scope);
    value.account = AccountId::new();
    let account_refused = value.binding(&scope, &fixture::session()).is_err();
    value.account = scope.account(); value.user.0.push('x');
    let user_refused = value.binding(&scope, &fixture::session()).is_err();
    let mut session = fixture::session(); session.cookie_header.push_str("; __cuid=other-device");
    let device_refused = settings(&scope).binding(&scope, &session).is_err();
    close(&client, &scope).await;
    assert!(account_refused && user_refused && device_refused);
}

fn save(root: &Directory, value: &serde_json::Value) -> (PathBuf, String) {
    let bytes = serde_json::to_vec(value).unwrap();
    let path = root.0.join("descriptor.json");
    std::fs::write(&path, &bytes).unwrap();
    (path, digest(&bytes))
}

#[tokio::test]
async fn descriptor_load_is_pinned_inert_and_symlink_compatible() {
    let root = Directory::new();
    let (path, hash) = save(&root, &document());
    let peer = load_browser_control(&path, &hash).unwrap();
    assert!(peer.requires_account_scope());
    assert!(!peer.prepare().await);
    assert!(peer.shutdown().await);
    let link = root.0.join("link");
    std::os::unix::fs::symlink(&path, &link).unwrap();
    assert!(load_browser_control(&link, &hash).is_ok());
    assert_eq!(load_browser_control(&path, &"0".repeat(64)).err(), Some(ERROR));
    let mut bytes = std::fs::read(&path).unwrap();
    bytes.push(b' ');
    std::fs::write(&path, bytes).unwrap();
    assert_eq!(load_browser_control(&path, &hash).err(), Some(ERROR));
    assert_eq!(std::fs::read_dir(&root.0).unwrap().count(), 2);
}

#[test]
fn descriptor_refuses_nonregular_missing_relative_and_oversized_files() {
    let root = Directory::new();
    let large = root.0.join("large");
    std::fs::File::create(&large).unwrap().set_len(MAX_DESCRIPTOR + 1).unwrap();
    for path in [root.0.as_path(), large.as_path(), &root.0.join("missing"), Path::new("relative")] {
        assert_eq!(read_descriptor(path, &"0".repeat(64)).err(), Some(ERROR));
    }
}

#[cfg(target_os = "linux")]
#[test]
fn descriptor_fifo_refuses_before_a_writer_is_available() {
    use std::{ffi::CString, fs::OpenOptions, os::unix::fs::OpenOptionsExt, sync::mpsc};
    let root = Directory::new();
    let fifo = root.0.join("descriptor.fifo");
    let name = CString::new(fifo.as_os_str().as_encoded_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
    let (started_tx, started_rx) = mpsc::channel();
    let (done_tx, done_rx) = mpsc::channel();
    let path = fifo.clone();
    let worker = std::thread::spawn(move || {
        started_tx.send(()).unwrap();
        let result = read_descriptor(&path, &"0".repeat(64));
        let _ = done_tx.send(result.is_err());
        result
    });
    let started = started_rx.recv_timeout(Duration::from_secs(3));
    let early = done_rx.recv_timeout(Duration::from_secs(1));
    let rescue = early.is_err().then(|| OpenOptions::new().read(true).write(true)
        .custom_flags(libc::O_NONBLOCK).open(&fifo));
    let joined = worker.join();
    drop(rescue);
    assert!(started.is_ok());
    assert_eq!(joined.unwrap().err(), Some(ERROR));
    assert_eq!(early, Ok(true), "descriptor open needed a rescue writer");
}

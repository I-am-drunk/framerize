use super::*;
use crate::account::client::NativeAccountClient;
use crate::account::device::DeviceAuthority;
use crate::authorization::control::browser_control_fixture::{fixture, target};
use crate::transport::http::{NativeHttp, NativeHttpLimits, egress_probe_fixture::{Peer, response, NONCE}};
use framer_core::ids::AccountId;
use std::sync::Arc;

struct Case {
    client: NativeAccountClient,
    scope: AccountScope,
    peer: Arc<BrowserControlProcess>,
    grant: Arc<DeviceGrant>,
    directory: PathBuf,
}

fn sha(path: &Path) -> String { format!("{:x}", Sha256::digest(std::fs::read(path).unwrap())) }

fn good_reply() -> Vec<u8> { response(200, "application/json", br#"{"ip":"203.0.113.8"}"#) }

const CHILD: &str = r#"import json, os, pathlib, sys, time
mode, url, directory = sys.argv[1:]
root = pathlib.Path(directory)
def mark(name, value='yes'):
    (root / name).write_text(value)
def send(value):
    print(json.dumps(value), flush=True)
mark('pid', str(os.getpid()))
assert json.loads(sys.stdin.readline())['op'] == 'prepare'
ready = dict(version=1, op='ready', host_generation='a'*32, incarnation=1, proxy_revision=0)
if mode == 'premature':
    send(ready)
else:
    probe = dict(version=1, op='egress_probe', url=url, budget_ms=2000)
    if mode == 'wrong-url': probe['url'] = url.replace('/ip?', '/other?')
    if mode == 'bad-version': probe['version'] = 2
    if mode == 'bad-budget': probe['budget_ms'] = 0
    if mode == 'excess-budget': probe['budget_ms'] = 30001
    send(probe)
    reply = json.loads(sys.stdin.readline())
    assert reply == dict(version=1, op='egress_result', url=url, ip='203.0.113.8')
    mark('acknowledged')
    if mode == 'hold-ready':
        mark('waiting')
        while not (root / 'release').exists(): time.sleep(0.005)
    send(probe if mode == 'duplicate' else ready)
for line in sys.stdin:
    command = json.loads(line)
    if command['op'] == 'mint':
        mark('minted')
        send(dict(version=1, op='minted', grant=command['grant'], token='fixture-token'))
    elif command['op'] in ('shutdown', 'finish'):
        mark('closed')
        send(dict(version=1, op='closed', grant=command.get('grant'),
                  cleanup_confirmed=True, reason='finished'))
        sys.exit(0)
    else:
        sys.exit(2)
"#;

impl Drop for Case {
    fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.directory); }
}

fn case(address: std::net::SocketAddr, url: &str, mode: &str) -> Case {
    let client = NativeAccountClient::new().unwrap();
    let account = AccountId::new();
    let user = fixture::session().active_user;
    let scope = client.open_account_for_test(account, fixture::session(), ||
        NativeHttp::for_loopback(account, user, address, NativeHttpLimits::default())).unwrap();
    let python = PathBuf::from(std::env::var_os("FRAMERIZE_TEST_CONTROL_PYTHON").unwrap());
    let expected = std::env::var("FRAMERIZE_TEST_CONTROL_PYTHON_SHA256").unwrap();
    assert_eq!(sha(&python), expected);
    let directory = std::env::temp_dir().join(format!("control-egress-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&directory).unwrap();
    let script = directory.join("child.py");
    std::fs::write(&script, CHILD).unwrap();
    let launch = BrowserControlLaunch { python, python_sha256: expected,
        script_sha256: sha(&script), script, arguments: vec![mode.into(), url.into(),
            directory.as_os_str().to_owned()] };
    finish_case(client, scope, launch, directory, url)
}

fn finish_case(client: NativeAccountClient, scope: AccountScope, launch: BrowserControlLaunch,
    directory: PathBuf, url: &str) -> Case {
    let grant = Arc::new(DeviceGrant { account: scope.account(), host_generation: "a".repeat(32),
        lease: uuid::Uuid::new_v4(), device: "fixture-device".into(), authority: DeviceAuthority::Ledger });
    let binding = BrowserControlBinding { account: scope.account(), user: scope.active_user().clone(),
        session: "fixture-only-cookie".into(), device: grant.device.clone(),
        page_url: target().page_url().into(), wrapper_url: "https://example.com/wrapper.mjs".into(),
        post_path: target().path().into(), proxy: None };
    let mut peer = BrowserControlProcess::new(launch, binding).unwrap();
    let endpoint = url.split("&framerize_probe=").next().unwrap();
    peer.native_egress = Some(Endpoint::validated(endpoint, true).unwrap());
    peer.prepare_limit = Duration::from_secs(3);
    peer.cleanup_limit = Duration::from_secs(2);
    Case { client, scope, peer: Arc::new(peer), grant, directory }
}

impl Case {
    fn mint_on(&self, scope: &AccountScope) -> PendingPreparation<String> {
        let peer = self.peer.clone();
        let expected = scope.clone();
        let grant = self.grant.clone();
        self.client.prepare_owned(scope, Instant::now() + Duration::from_secs(3),
            move |session, control| async move {
                let result = peer.mint(&expected, &session, &grant, &target(), 1000, &control).await;
                PreparationResult { cleanup_confirmed: result.cleanup_confirmed,
                    value: result.value.map(|mint| mint.token.0) }
            }).unwrap()
    }

    async fn marker(&self, name: &str) -> bool {
        timeout(Duration::from_secs(2), async {
            while !self.directory.join(name).exists() {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        }).await.is_ok()
    }

    fn reaped(&self) -> bool {
        let pid: i32 = std::fs::read_to_string(self.directory.join("pid")).unwrap().parse().unwrap();
        (unsafe { libc::kill(pid, 0) }) == -1
            && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH)
    }

    fn prepare(&self, budget: Duration) -> PendingPreparation<bool> {
        let peer = self.peer.clone();
        let scope = self.scope.clone();
        self.client.prepare_owned(&self.scope, Instant::now() + budget,
            move |session, control| async move {
                PreparationResult::finished(Ok(peer.prepare_for_account(&scope, &session, &control).await))
            }).unwrap()
    }

    async fn close(&self) -> (bool, bool) {
        let control = self.peer.shutdown().await;
        let account = match self.client.close_account(&self.scope) {
            Ok(closing) => closing.await.is_ok(),
            Err(ClientError::StaleScope) => true,
            Err(_) => false,
        };
        (control, account && self.client.shutdown_idle().await.is_ok())
    }
}

#[test]
fn endpoint_preserves_raw_query_and_refuses_existing_nonce_or_credentials() {
    let raw = "https://example.com/ip?flag&blank=&raw=%2f+";
    let endpoint = Endpoint::new(raw).unwrap();
    assert_eq!(endpoint.prefix, format!("{raw}&framerize_probe="));
    for bad in ["http://example.com/ip", "https://user@example.com/ip", "https://EXAMPLE.com/ip",
        "https://example.com/ip#", "https://example.com/ip?framerize_probe",
        "https://example.com/ip?framerize_probe=", "https://example.com/ip?%66ramerize_probe=",
        "https://example.com/../ip", "https://example.com/\\ip", "https://example.com/ip\n"] {
        assert!(Endpoint::new(bad).is_err(), "{bad}");
    }
}

#[test]
fn endpoint_accepts_only_its_exact_url_and_fixed_length_nonce() {
    let endpoint = Endpoint::new("https://example.com/ip?x=%2f+").unwrap();
    let valid = format!("{}{}", endpoint.prefix, NONCE);
    assert!(endpoint.allows(&valid));
    for bad in [valid.replace("/ip?", "/else?"), valid.replace("%2f", "%2F"),
        valid.replace("example.com", "other.example"), format!("{valid}&extra=1"),
        valid.replace(NONCE, ""), valid.replace(NONCE, &"z".repeat(32)), format!("{valid}0")] {
        assert!(!endpoint.allows(&bad), "{bad}");
    }
}

async fn refused_without_probe(mode: &str) {
    let listener = fixture::listener().await;
    let address = listener.local_addr().unwrap();
    let url = format!("http://{address}/ip?flag&framerize_probe={NONCE}");
    let case = case(address, &url, mode);
    let prepared = case.prepare(Duration::from_secs(5)).await;
    let closed = case.close().await;
    assert_eq!(prepared, Ok(false));
    assert_eq!(closed, (false, true));
    assert!(case.reaped());
    assert!(!case.directory.join("acknowledged").exists());
    assert!(listener.accept().now_or_never().is_none());
    assert!(case.peer.state.lock().await.process.is_none());
}

#[tokio::test]
async fn premature_ready_is_refused_without_a_native_request() {
    refused_without_probe("premature").await;
}

#[tokio::test]
async fn different_endpoint_is_refused_without_a_native_request() {
    refused_without_probe("wrong-url").await;
}

#[tokio::test]
async fn a_second_probe_cannot_replace_the_ready_reply() {
    let server = Peer::start(Some(good_reply())).await;
    let case = case(server.address, &server.url, "duplicate");
    let prepared = case.prepare(Duration::from_secs(5)).await;
    let closed = case.close().await;
    let (_, joined) = server.settle().await;
    assert_eq!(prepared, Ok(false));
    assert_eq!(closed, (false, true));
    assert!(joined && case.reaped());
    assert!(case.directory.join("acknowledged").exists());
    assert!(case.peer.state.lock().await.native_observation.is_none());
}

#[tokio::test]
async fn a_rejected_native_reply_is_never_acknowledged_to_the_child() {
    let server = Peer::start(Some(response(200, "text/html", b"invalid"))).await;
    let case = case(server.address, &server.url, "normal");
    let prepared = case.prepare(Duration::from_secs(5)).await;
    let closed = case.close().await;
    let (_, joined) = server.settle().await;
    assert_eq!(prepared, Ok(false));
    assert_eq!(closed, (false, true));
    assert!(joined && case.reaped());
    assert!(!case.directory.join("acknowledged").exists());
}

#[tokio::test]
async fn cancellation_joins_the_native_probe_and_reaps_the_child() {
    let mut server = Peer::start(None).await;
    let case = case(server.address, &server.url, "normal");
    let pending = case.prepare(Duration::from_secs(5));
    let completion = pending.completion();
    let entered = server.entered().await;
    pending.cancellation().cancel();
    let prepared = pending.await;
    let settled = completion.await;
    let closed = case.close().await;
    let (_, joined) = server.settle().await;
    assert!(entered && joined && case.reaped());
    assert_eq!(prepared, Err(PreparationFailure::Cancelled));
    assert_eq!(settled, Ok(()));
    assert_eq!(closed, (false, true));
    assert!(!case.directory.join("acknowledged").exists());
}

#[tokio::test]
async fn preparation_deadline_joins_the_native_probe_before_refusal() {
    let mut server = Peer::start(None).await;
    let mut case = case(server.address, &server.url, "normal");
    Arc::get_mut(&mut case.peer).unwrap().prepare_limit = Duration::from_secs(1);
    let pending = case.prepare(Duration::from_secs(5));
    let entered = server.entered().await;
    let prepared = pending.await;
    let closed = case.close().await;
    let (_, joined) = server.settle().await;
    assert!(entered && joined && case.reaped());
    assert_eq!(prepared, Ok(false));
    assert_eq!(closed, (false, true));
}

#[tokio::test]
async fn a_dropped_prepare_waiter_keeps_joined_account_cleanup() {
    let mut server = Peer::start(None).await;
    let case = case(server.address, &server.url, "normal");
    let pending = case.prepare(Duration::from_secs(5));
    let completion = pending.completion();
    let entered = server.entered().await;
    drop(pending);
    let settled = completion.await;
    let closed = case.close().await;
    let (_, joined) = server.settle().await;
    assert!(entered && joined && case.reaped());
    assert_eq!(settled, Ok(()));
    assert_eq!(closed, (false, true));
}

#[tokio::test]
async fn account_close_cancels_preparation_and_joins_both_owners() {
    let mut server = Peer::start(None).await;
    let case = case(server.address, &server.url, "normal");
    let pending = case.prepare(Duration::from_secs(5));
    let entered = server.entered().await;
    let closing = case.client.close_account(&case.scope).unwrap();
    let prepared = pending.await;
    let account = closing.await;
    let closed = case.close().await;
    let (_, joined) = server.settle().await;
    assert!(entered && joined && case.reaped());
    assert_eq!(account, Ok(()));
    assert_eq!(prepared, Err(PreparationFailure::Cancelled));
    assert_eq!(closed, (false, true));
}

#[tokio::test]
async fn cancellation_after_probe_cannot_publish_child_readiness() {
    let server = Peer::start(Some(good_reply())).await;
    let case = case(server.address, &server.url, "hold-ready");
    let pending = case.prepare(Duration::from_secs(5));
    let waiting = case.marker("waiting").await;
    pending.cancellation().cancel();
    let prepared = pending.await;
    let closed = case.close().await;
    let (_, joined) = server.settle().await;
    assert!(waiting && joined && case.reaped());
    assert_eq!(prepared, Err(PreparationFailure::Cancelled));
    assert_eq!(closed, (false, true));
    assert!(case.peer.state.lock().await.native_observation.is_none());
}

#[tokio::test]
async fn replacing_the_account_generation_refuses_mint_before_child_delivery() {
    let server = Peer::start(Some(good_reply())).await;
    let case = case(server.address, &server.url, "normal");
    let prepared = case.prepare(Duration::from_secs(5)).await;
    let retired = case.client.close_account(&case.scope).unwrap().await;
    let account = case.scope.account();
    let user = case.scope.active_user().clone();
    let replacement = case.client.open_account_for_test(account, fixture::session(), ||
        NativeHttp::for_loopback(account, user, server.address, NativeHttpLimits::default())).unwrap();
    let minted = case.mint_on(&replacement).await;
    let replacement_closed = case.client.close_account(&replacement).unwrap().await;
    let closed = case.close().await;
    let (_, joined) = server.settle().await;
    assert_eq!(prepared, Ok(true));
    assert_eq!(retired, Ok(()));
    assert_eq!(replacement_closed, Ok(()));
    assert_eq!(minted, Err(PreparationFailure::Rejected));
    assert_eq!(closed, (true, true));
    assert!(joined && case.reaped());
    assert!(!case.scope.same_generation(&replacement));
    assert!(!case.directory.join("minted").exists());
}

#[tokio::test]
async fn invalid_probe_version_and_budgets_are_refused_before_request() {
    for mode in ["bad-version", "bad-budget", "excess-budget"] { refused_without_probe(mode).await; }
}

#[tokio::test]
async fn probe_acknowledgement_precedes_ready_and_same_generation_mint() {
    let server = Peer::start(Some(good_reply())).await;
    let case = case(server.address, &server.url, "hold-ready");
    let pending = case.prepare(Duration::from_secs(5));
    let waiting = case.marker("waiting").await;
    let incomplete = pending.completion().now_or_never().is_none();
    std::fs::write(case.directory.join("release"), b"release").unwrap();
    let prepared = pending.await;
    let bound = case.peer.state.lock().await.native_observation.as_ref()
        .is_some_and(|value| value.matches_scope(&case.scope) && value.owner_is_current());
    let minted = case.mint_on(&case.scope).await;
    let finished = case.peer.finish(&case.grant, true, false).await;
    let closed = case.close().await;
    let (wire, joined) = server.settle().await;
    assert!(waiting && incomplete && bound && joined && finished);
    assert_eq!(prepared, Ok(true));
    assert_eq!(minted.as_deref(), Ok("fixture-token"));
    assert_eq!(closed, (true, true));
    assert!(case.reaped());
    assert!(case.peer.state.lock().await.native_observation.is_none());
    assert!(wire.starts_with(&format!("GET /ip?keep=%2F+&flag&framerize_probe={NONCE} ")));
    assert!(!wire.to_ascii_lowercase().contains("cookie:"));
    assert!(!wire.to_ascii_lowercase().contains("authorization:"));
}

#[tokio::test]
async fn scoped_mode_refuses_unbound_and_mismatched_prepare_before_spawn() {
    for field in 0..4 {
        let listener = fixture::listener().await;
        let address = listener.local_addr().unwrap();
        let url = format!("http://{address}/ip?flag&framerize_probe={NONCE}");
        let mut case = case(address, &url, "normal");
        let binding = &mut Arc::get_mut(&mut case.peer).unwrap().binding;
        match field { 0 => binding.account = AccountId::new(), 1 => binding.user.0.push('x'),
            2 => binding.session.push('x'), _ => binding.proxy = Some("http://127.0.0.1:9".into()) }
        let scoped = case.peer.requires_account_scope();
        let unbound = case.peer.prepare().await;
        let prepared = case.prepare(Duration::from_secs(3)).await;
        let closed = case.close().await;
        assert!(scoped && !unbound && !prepared.unwrap());
        assert_eq!(closed, (true, true));
        assert!(!case.directory.join("pid").exists());
        assert!(listener.accept().now_or_never().is_none());
    }
}

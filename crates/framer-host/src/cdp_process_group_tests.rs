//! Disposable Python children and loopback CDP only; no browser or live account.
use super::*;
use std::{os::unix::fs::PermissionsExt, time::Duration};
use tokio::{io::{AsyncReadExt, AsyncWriteExt}, time::timeout};

async fn member(group: i32) -> Child {
    let mut child = Command::new("python3").args(["-I", "-B", "-c",
        "import sys; sys.stdout.write('R'); sys.stdout.flush(); sys.stdin.read(1)"])
        .process_group(group).stdin(Stdio::piped()).stdout(Stdio::piped())
        .stderr(Stdio::null()).kill_on_drop(true).spawn().unwrap();
    let mut ready = [0; 1];
    let result = timeout(Duration::from_secs(3),
        child.stdout.as_mut().unwrap().read_exact(&mut ready)).await;
    if !matches!(result, Ok(Ok(_))) {
        finish(&mut child).await;
        panic!("inert group member did not become ready");
    }
    assert_eq!(ready, [b'R']);
    child
}

async fn finish(child: &mut Child) -> std::process::ExitStatus {
    let _ = child.start_kill();
    timeout(Duration::from_secs(3), child.wait()).await.unwrap().unwrap()
}

#[tokio::test]
async fn cancelled_startup_joins_its_group_without_killing_another_group() {
    let fixture = LaunchFixture::new();
    let bridge = Arc::new(CdpBridge::new(&fixture.executable, fixture.root.join("profiles")));
    let account = AccountId::new();
    let starting = bridge.clone();
    let request = tokio::spawn(async move { starting.instance(&account).await.map(|_| ()) });
    let id = timeout(Duration::from_secs(3), fixture.id()).await;
    if id.is_err() {
        request.abort();
        let _ = request.await;
        let _ = timeout(Duration::from_secs(3), bridge.shutdown()).await;
        panic!("inert browser did not start");
    }
    let id = id.unwrap();
    let mut owned = member(id as i32).await;
    let owned_id = owned.id().unwrap();
    let mut unrelated = member(0).await;
    let unrelated_id = unrelated.id().unwrap();
    request.abort();
    let cancelled = request.await;
    let mut byte = [0; 1];
    let eof = timeout(Duration::from_secs(2),
        owned.stdout.as_mut().unwrap().read(&mut byte)).await;
    // Hold the killed member unreaped. A kill request is not group absence.
    let early = timeout(Duration::from_millis(50), bridge.shutdown()).await;
    let fenced = bridge.route_cell(&account).await.lock().await.may_have_browser;
    let unrelated_running = unrelated.try_wait().unwrap().is_none();
    let owned_exit = finish(&mut owned).await;
    finish(&mut unrelated).await;
    let closed = timeout(Duration::from_secs(3), bridge.shutdown()).await;
    let repeated = timeout(Duration::from_secs(3), bridge.shutdown()).await;
    assert!(cancelled.unwrap_err().is_cancelled());
    assert!(matches!(eof, Ok(Ok(0))), "owned group member was not signaled");
    assert!(early.is_err(), "shutdown confirmed an unreaped group member");
    assert!(fenced);
    assert!(unrelated_running, "another process group was killed");
    assert!(!owned_exit.success());
    closed.unwrap().unwrap();
    repeated.unwrap().unwrap();
    assert!(!bridge.route_cell(&account).await.lock().await.may_have_browser);
    for id in [id, owned_id, unrelated_id] {
        assert!(!Path::new(&format!("/proc/{id}")).exists());
    }
}

const STARTUP: &str = r#"#!/usr/bin/env python3
import os, pathlib, sys, time
if '--version' in sys.argv:
    print('Chromium 1.2.3.4')
    raise SystemExit(0)
pathlib.Path(__file__).with_name('browser.pid').write_text(str(os.getpid()))
time.sleep(60)
"#;

struct LaunchFixture { root: PathBuf, executable: PathBuf }

async fn publish(bridge: &CdpBridge, account: AccountId, mut child: Child)
    -> tokio::task::JoinHandle<()> {
    let group = cdp_process_group::ProcessGroup::for_child(&child);
    let mut input = child.stdin.take();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let peer = tokio::spawn(async move {
        let (socket, _) = timeout(Duration::from_secs(3), listener.accept()).await.unwrap().unwrap();
        let mut ws = tokio_tungstenite::accept_async(socket).await.unwrap();
        while let Some(Ok(frame)) = ws.next().await {
            if frame.is_close() { break; }
            let command: Value = serde_json::from_str(frame.to_text().unwrap()).unwrap();
            assert_eq!(command["method"], "Browser.close");
            ws.send(Message::Text(json!({"id": command["id"], "result": {}})
                .to_string().into())).await.unwrap();
            if let Some(mut input) = input.take() { input.write_all(b"x").await.unwrap(); }
        }
    });
    let (ws, _) = connect_async(format!("ws://{address}")).await.unwrap();
    let cell = bridge.route_cell(&account).await;
    let mut route = cell.lock().await;
    route.may_have_browser = true;
    route.group = group;
    bridge.instances.lock().await.insert(account,
        Arc::new(Mutex::new(CdpInstance { child: Some(child), ws, next_id: 0, pending: Default::default() })));
    peer
}

impl Drop for LaunchFixture {
    fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.root); }
}

async fn wait_absent(id: u32) {
    while Path::new(&format!("/proc/{id}")).exists() {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

#[tokio::test]
async fn reaped_leader_cannot_authorize_another_group_signal() {
    let mut leader = member(0).await;
    let leader_id = leader.id().unwrap();
    let group = cdp_process_group::ProcessGroup::for_child(&leader).unwrap();
    let mut owned = member(leader_id as i32).await;
    let owned_id = owned.id().unwrap();
    // Child::wait closes its stored stdin, so retain the fixture input here.
    let mut owned_input = owned.stdin.take().unwrap();
    leader.stdin.take().unwrap().write_all(b"x").await.unwrap();
    let leader_exit = timeout(Duration::from_secs(3), leader.wait()).await;
    let occupied = timeout(Duration::from_millis(50), group.wait_empty()).await;
    group.request_kill(&leader);
    let premature = timeout(Duration::from_millis(50), owned.wait()).await;
    let _ = owned_input.write_all(b"x").await;
    drop(owned_input);
    let owned_exit = timeout(Duration::from_secs(3), owned.wait()).await;
    if leader_exit.is_err() { finish(&mut leader).await; }
    if owned_exit.is_err() { finish(&mut owned).await; }
    let empty = timeout(Duration::from_secs(3), group.wait_empty()).await;
    assert!(leader_exit.unwrap().unwrap().success());
    assert!(occupied.is_err());
    assert!(premature.is_err(), "a reaped leader authorized group signaling");
    assert!(owned_exit.unwrap().unwrap().success());
    empty.unwrap().unwrap();
    group.wait_empty().await.unwrap();
    assert!(!Path::new(&format!("/proc/{owned_id}")).exists());
}

#[tokio::test]
async fn clean_leader_exit_keeps_the_account_fenced_until_its_group_is_gone() {
    let child = member(0).await;
    let leader_id = child.id().unwrap();
    let mut owned = member(leader_id as i32).await;
    let owned_id = owned.id().unwrap();
    let bridge = Arc::new(CdpBridge::new("unused", "/uncreated-cdp-group"));
    let account = AccountId::new();
    let mut peer = publish(&bridge, account, child).await;
    let closing_bridge = bridge.clone();
    let mut closing = tokio::spawn(async move { closing_bridge.shutdown().await });
    let leader_gone = timeout(Duration::from_secs(3), wait_absent(leader_id)).await;
    let early = timeout(Duration::from_millis(50), &mut closing).await;
    if early.is_err() { closing.abort(); let _ = closing.await; }
    let fenced = bridge.route_cell(&account).await.lock().await.may_have_browser;
    let still_running = owned.try_wait().unwrap().is_none();
    if let Some(mut input) = owned.stdin.take() { let _ = input.write_all(b"x").await; }
    let exited = timeout(Duration::from_secs(3), owned.wait()).await;
    if exited.is_err() { finish(&mut owned).await; }
    let closed = timeout(Duration::from_secs(3), bridge.shutdown()).await;
    let repeated = timeout(Duration::from_secs(3), bridge.shutdown()).await;
    let peer_exit = timeout(Duration::from_secs(3), &mut peer).await;
    if peer_exit.is_err() { peer.abort(); let _ = peer.await; }
    leader_gone.unwrap();
    assert!(early.is_err(), "a clean leader exit hid a surviving group member");
    assert!(fenced, "account fence cleared before the group exited");
    assert!(still_running, "graceful shutdown forcibly killed a group member");
    assert!(exited.unwrap().unwrap().success());
    closed.unwrap().unwrap();
    repeated.unwrap().unwrap();
    peer_exit.unwrap().unwrap();
    assert!(!bridge.route_cell(&account).await.lock().await.may_have_browser);
    assert!(bridge.instances.lock().await.is_empty());
    assert!(!Path::new(&format!("/proc/{owned_id}")).exists());
}

impl LaunchFixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("cdp-group-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        let executable = root.join("inert-chrome");
        std::fs::write(&executable, STARTUP).unwrap();
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
        Self { root, executable }
    }

    async fn id(&self) -> u32 {
        loop {
            if let Ok(raw) = std::fs::read_to_string(self.root.join("browser.pid")) {
                if let Ok(id) = raw.parse() { return id; }
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }
}

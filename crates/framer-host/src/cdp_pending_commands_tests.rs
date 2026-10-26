use super::*;
use tokio::net::{TcpListener, TcpStream};
use tokio::time::timeout;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, tungstenite::protocol::Role};

const WAIT: Duration = Duration::from_secs(3);
const SHORT: Duration = Duration::from_millis(40);
type Peer = WebSocketStream<TcpStream>;

struct Profile(PathBuf);
impl Profile {
    fn new(account: AccountId) -> Self {
        let root = std::env::temp_dir().join(format!("cdp-pending-{}", uuid::Uuid::new_v4()));
        let profile = root.join(account.0.to_string());
        std::fs::create_dir_all(&profile).unwrap();
        std::fs::write(DeviceRecord::path(&profile),
            r#"{"castle_device_id":"fixture-saved-device"}"#).unwrap();
        Self(root)
    }
}
impl Drop for Profile {
    fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); }
}

async fn pair() -> (CdpInstance, Peer) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let (client, (peer, _)) = tokio::try_join!(
        TcpStream::connect(listener.local_addr().unwrap()), listener.accept()).unwrap();
    let ws = WebSocketStream::from_raw_socket(MaybeTlsStream::Plain(client), Role::Client, None).await;
    let peer = WebSocketStream::from_raw_socket(peer, Role::Server, None).await;
    (CdpInstance { child: None, ws, next_id: 0, pending: Default::default() }, peer)
}

async fn command(peer: &mut Peer) -> Value {
    let frame = timeout(WAIT, peer.next()).await.unwrap().unwrap().unwrap();
    serde_json::from_str(frame.to_text().unwrap()).unwrap()
}

async fn send(peer: &mut Peer, value: Value) {
    timeout(WAIT, peer.send(Message::Text(value.to_string().into()))).await.unwrap().unwrap();
}

async fn finish(instance: &mut CdpInstance, peer: &mut Peer) {
    let (closed, frame) = tokio::join!(instance.disconnect_observed(WAIT),
        timeout(WAIT, peer.next()));
    closed.unwrap();
    assert!(frame.unwrap().unwrap().unwrap().is_close());
}

async fn install(instance: CdpInstance, engine: EngineKind) -> (CdpBridge, AccountId) {
    let bridge = CdpBridge::with_engine("unused", "/fixture-uncreated", engine);
    let account = AccountId::new();
    let route = bridge.route_cell(&account).await;
    let mut route = route.lock().await;
    route.pending = instance.pending.clone();
    route.may_have_browser = true;
    bridge.instances.lock().await.insert(account, Arc::new(Mutex::new(instance)));
    (bridge, account)
}

async fn abandoned(instance: &mut CdpInstance, peer: &mut Peer) -> Value {
    let (result, request) = tokio::join!(
        timeout(SHORT, instance.eval("pending_fixture()".into())), command(peer));
    assert!(result.is_err());
    request
}

async fn stalled_start(listener: TcpListener, sent: tokio::sync::oneshot::Sender<()>) {
    let address = listener.local_addr().unwrap();
    let (mut http, _) = listener.accept().await.unwrap();
    let mut request = Vec::new();
    while !request.ends_with(b"\r\n\r\n") { request.push(http.read_u8().await.unwrap()); }
    assert!(request.starts_with(b"GET /json/list HTTP/1.1\r\n"));
    let body = json!([{"type":"page", "url":"https://framer.com/projects/fixture",
        "webSocketDebuggerUrl":format!("ws://{address}/page")}]).to_string();
    let reply = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{body}", body.len());
    http.write_all(reply.as_bytes()).await.unwrap();
    drop(http);
    let (socket, _) = listener.accept().await.unwrap();
    let mut peer = tokio_tungstenite::accept_async(socket).await.unwrap();
    let ready = command(&mut peer).await;
    send(&mut peer, json!({"id":ready["id"], "result":{"result":{"value":true}}})).await;
    let seed = command(&mut peer).await;
    assert!(seed["params"]["expression"].as_str().unwrap().contains("fixture-saved-device"));
    sent.send(()).unwrap();
    assert!(!matches!(peer.next().await, Some(Ok(frame)) if !frame.is_close()));
}

#[tokio::test]
async fn cancelled_attach_start_retains_its_unobserved_command() {
    let account = AccountId::new();
    let profile = Profile::new(account);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let bridge = CdpBridge::with_engine(listener.local_addr().unwrap().to_string(),
        &profile.0, EngineKind::Attach);
    let (sent, received) = tokio::sync::oneshot::channel();
    let cancel = async {
        let mut starting = Box::pin(bridge.instance(&account));
        tokio::select! {
            _ = received => {},
            _ = &mut starting => panic!("startup returned before its seed reply"),
        }
        drop(starting);
    };
    timeout(WAIT, async { tokio::join!(cancel, stalled_start(listener, sent)); }).await.unwrap();
    assert!(bridge.instances.lock().await.is_empty());
    assert_eq!(bridge.instance(&account).await.err().unwrap(), UNCONFIRMED);
    for _ in 0..2 {
        assert_eq!(timeout(WAIT, bridge.shutdown()).await.unwrap().unwrap_err(), UNCONFIRMED);
        assert!(bridge.route_cell(&account).await.lock().await.may_have_browser);
    }
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn cancelled_partial_send_keeps_attach_cleanup_unconfirmed() {
    let listener = tokio::net::TcpSocket::new_v4().unwrap();
    listener.set_recv_buffer_size(1024).unwrap();
    listener.bind("127.0.0.1:0".parse().unwrap()).unwrap();
    let listener = listener.listen(1).unwrap();
    let client = tokio::net::TcpSocket::new_v4().unwrap();
    client.set_send_buffer_size(1024).unwrap();
    let (client, (mut peer, _)) = tokio::try_join!(
        client.connect(listener.local_addr().unwrap()), listener.accept()).unwrap();
    let ws = WebSocketStream::from_raw_socket(MaybeTlsStream::Plain(client), Role::Client, None).await;
    let mut instance = CdpInstance { child: None, ws, next_id: 0, pending: Default::default() };
    let mut prefix = [0u8; 16];
    let (result, seen) = tokio::join!(timeout(SHORT, instance.eval("x".repeat(1024 * 1024))),
        timeout(WAIT, peer.read_exact(&mut prefix)));
    seen.unwrap().unwrap();
    assert!(result.is_err(), "the fixture did not cancel a blocked send");
    drop(peer);
    let (bridge, _) = install(instance, EngineKind::Attach).await;
    assert_eq!(timeout(WAIT, bridge.shutdown()).await.unwrap().unwrap_err(), UNCONFIRMED);
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn owned_group_exit_settles_commands_but_close_reply_alone_does_not() {
    let (mut instance, mut peer) = pair().await;
    abandoned(&mut instance, &mut peer).await;
    let mut child = Command::new("python3").args(["-I", "-B", "-c",
        "import sys; sys.stdin.buffer.read(1)"]).stdin(Stdio::piped())
        .stdout(Stdio::null()).stderr(Stdio::null()).process_group(0)
        .kill_on_drop(true).spawn().unwrap();
    let mut input = child.stdin.take().unwrap();
    let group = cdp_process_group::ProcessGroup::for_child(&child);
    let pending = instance.pending.clone();
    instance.child = Some(child);
    let (bridge, account) = install(instance, EngineKind::FullChrome).await;
    bridge.route_cell(&account).await.lock().await.group = group.clone();
    let saved = bridge.instances.lock().await[&account].clone();
    let responder = async {
        let request = match timeout(WAIT, peer.next()).await {
            Ok(Some(Ok(frame))) => serde_json::from_str::<Value>(frame.to_text().unwrap()).ok(),
            _ => None,
        };
        if let Some(request) = &request {
            send(&mut peer, json!({"id":request["id"], "result":{}})).await;
        }
        request
    };
    let (first, request) = tokio::join!(timeout(SHORT, bridge.shutdown()), responder);
    let still_pending = !pending.is_empty();
    let released = input.write_all(b"x").await;
    drop(input);
    let _ = timeout(WAIT, peer.close(None)).await;
    // Sending a close frame leaves the transport open until the peer is driven.
    // An exited browser drops it; do the same before retrying shutdown.
    drop(peer);
    let closed = timeout(WAIT, bridge.shutdown()).await;
    let mut saved = saved.lock().await;
    let child = saved.child.as_mut().unwrap();
    if let Some(group) = &group { group.request_kill(child); }
    let _ = child.start_kill();
    let exited = timeout(WAIT, cdp_process_group::wait_for_exit(child, group.as_deref())).await;
    released.unwrap();
    exited.unwrap().unwrap();
    assert!(first.is_err(), "a close acknowledgement is not child/group exit");
    assert_eq!(request.unwrap()["method"], "Browser.close");
    assert!(still_pending);
    closed.unwrap().unwrap();
    assert!(pending.is_empty());
    assert!(bridge.instances.lock().await.is_empty());
    assert!(!bridge.route_cell(&account).await.lock().await.may_have_browser);
}

#[tokio::test]
async fn lost_or_malformed_replies_cannot_confirm_attach_cleanup() {
    for malformed in [false, true] {
        let (mut instance, mut peer) = pair().await;
        let responder = async move {
            let request = command(&mut peer).await;
            if malformed {
                send(&mut peer, json!({"id":request["id"], "private":"fixture-secret"})).await;
            }
            peer.close(None).await.unwrap();
        };
        let call = async {
            if malformed { instance.eval_long("fixture_secret()".into()).await }
            else { instance.eval("fixture_secret()".into()).await }
        };
        let (result, ()) = tokio::join!(timeout(WAIT, call), responder);
        assert_eq!(result.unwrap().unwrap_err(), UNCONFIRMED);
        let (bridge, account) = install(instance, EngineKind::Attach).await;
        for _ in 0..2 {
            assert_eq!(timeout(WAIT, bridge.shutdown()).await.unwrap().unwrap_err(), UNCONFIRMED);
            assert!(bridge.instances.lock().await.contains_key(&account));
        }
    }
}

#[tokio::test]
async fn timeout_and_cancelled_shutdown_retain_observation_until_late_reply() {
    let (mut instance, mut peer) = pair().await;
    let (result, request) = tokio::join!(
        instance.cmd_deadline("Runtime.evaluate", json!({"awaitPromise":true}), SHORT),
        command(&mut peer));
    assert_eq!(result.unwrap_err(), "cdp eval timed out");
    assert_eq!(timeout(WAIT, instance.disconnect_observed(SHORT)).await.unwrap().unwrap_err(),
        UNCONFIRMED);
    assert!(timeout(SHORT, peer.next()).await.is_err(), "disconnected unobserved work");
    let (bridge, account) = install(instance, EngineKind::Attach).await;
    assert!(timeout(SHORT, bridge.shutdown()).await.is_err());
    assert!(bridge.instances.lock().await.contains_key(&account));
    send(&mut peer, json!({"id":request["id"], "result":{"result":{"value":"done"}}})).await;
    let (result, frame) = tokio::join!(timeout(WAIT, bridge.shutdown()),
        timeout(WAIT, peer.next()));
    result.unwrap().unwrap();
    assert!(frame.unwrap().unwrap().unwrap().is_close());
    assert!(bridge.instances.lock().await.is_empty());
    bridge.shutdown().await.unwrap();
}

#[tokio::test]
async fn cancelled_reply_blocks_reuse_until_its_terminal_response() {
    for terminal_error in [false, true] {
        let (mut instance, mut peer) = pair().await;
        let first = abandoned(&mut instance, &mut peer).await;
        send(&mut peer, json!({"method":"Runtime.consoleAPICalled", "params":{}})).await;
        send(&mut peer, json!({"id":99, "result":{}})).await;
        assert!(timeout(SHORT, instance.cmd("Page.enable", json!({}))).await.is_err());
        assert!(timeout(SHORT, peer.next()).await.is_err(), "dispatched before observation");
        let (mut independent, mut other) = pair().await;
        let responder = async {
            let request = command(&mut other).await;
            send(&mut other, json!({"id":request["id"], "result":{"independent":true}})).await;
        };
        let (value, ()) = tokio::join!(independent.cmd("Page.enable", json!({})), responder);
        assert_eq!(value.unwrap(), json!({"independent":true}));
        finish(&mut independent, &mut other).await;
        let reply = if terminal_error {
            json!({"id":first["id"], "error":{"code":-32000, "message":"fixture"}})
        } else { json!({"id":first["id"], "result":{"result":{"value":"late"}}}) };
        send(&mut peer, reply).await;
        let responder = async {
            let request = command(&mut peer).await;
            assert_eq!(request["id"].as_u64(), Some(first["id"].as_u64().unwrap() + 1));
            send(&mut peer, json!({"id":request["id"], "result":{"accepted":true}})).await;
        };
        let (value, ()) = tokio::join!(instance.cmd("Page.enable", json!({})), responder);
        assert_eq!(value.unwrap(), json!({"accepted":true}));
        finish(&mut instance, &mut peer).await;
    }
}

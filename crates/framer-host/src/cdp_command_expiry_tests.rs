use super::*;
use futures_util::{SinkExt, StreamExt};
use tokio::io::AsyncReadExt;
use tokio::net::{TcpListener, TcpStream};
use tokio::time::timeout;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};
use tokio_tungstenite::tungstenite::protocol::Role;

const WAIT: Duration = Duration::from_secs(3);

fn frame(value: Value) -> Vec<u8> {
    let payload = value.to_string().into_bytes();
    assert!(payload.len() < 126);
    let mut bytes = vec![0x81, payload.len() as u8];
    bytes.extend(payload);
    bytes
}

async fn pair(buffered: Vec<u8>) -> (CdpInstance, TcpStream) {
    let listener = timeout(WAIT, TcpListener::bind("127.0.0.1:0"))
        .await.unwrap().unwrap();
    let address = listener.local_addr().unwrap();
    let connected = timeout(WAIT, async {
        tokio::try_join!(TcpStream::connect(address), listener.accept())
    }).await;
    drop(listener);
    let (client, (peer, _)) = connected.unwrap().unwrap();
    let ws = timeout(WAIT, WebSocketStream::from_partially_read(
        MaybeTlsStream::Plain(client), buffered, Role::Client, None,
    )).await.unwrap();
    (CdpInstance { child: None, ws, next_id: 0, pending: Default::default() }, peer)
}

async fn collect(peer: TcpStream) -> (Result<usize, String>, Vec<u8>) {
    let mut reader = peer.take(8192);
    let mut bytes = Vec::new();
    let result = timeout(WAIT, reader.read_to_end(&mut bytes)).await
        .map_err(|_| "fixture socket drain timed out".to_owned())
        .and_then(|value| value.map_err(|error| error.to_string()));
    drop(reader);
    (result, bytes)
}

#[tokio::test]
async fn expired_command_does_not_dispatch_or_record_work() {
    for method in ["Page.enable", "Browser.close"] {
        let (mut instance, peer) = pair(Vec::new()).await;
        let result = timeout(WAIT,
            instance.cmd_deadline(method, json!({}), Duration::ZERO)).await;
        let empty = instance.pending.is_empty();
        let next_id = instance.next_id;
        drop(instance);
        let (drained, bytes) = collect(peer).await;
        assert!(drained.is_ok(), "fixture peer did not settle: {drained:?}");
        assert!(empty, "expired command created pending work");
        assert_eq!(result.unwrap().unwrap_err(), "cdp eval timed out");
        assert_eq!(next_id, 0, "expired command consumed a correlation id");
        assert!(bytes.is_empty(), "expired command reached the peer");
    }
}

#[tokio::test]
async fn expired_observation_keeps_buffered_terminal_replies() {
    for terminal in [json!({"id":7,"result":{"kept":true}}),
        json!({"id":7,"error":{"code":-32000,"message":"fixture"}})] {
        let (mut instance, peer) = pair(frame(terminal.clone())).await;
        instance.pending.record(7).unwrap();
        let expired = timeout(WAIT,
            instance.next_reply(Instant::now() - Duration::from_secs(1))).await;
        let retained = !instance.pending.is_empty();
        let recovered = if retained {
            Some(timeout(WAIT, instance.next_reply(Instant::now() + WAIT)).await)
        } else { None };
        let empty = instance.pending.is_empty();
        drop(instance);
        let (drained, bytes) = collect(peer).await;
        assert!(drained.is_ok(), "fixture peer did not settle: {drained:?}");
        assert!(retained, "expired observation consumed a terminal reply");
        assert_eq!(expired.unwrap().unwrap_err(), "cdp eval timed out");
        assert_eq!(recovered.unwrap().unwrap().unwrap(), (7, terminal));
        assert!(empty, "fresh observation did not settle the command");
        assert!(bytes.is_empty());
    }
}

#[tokio::test]
async fn expired_drain_keeps_existing_pending_work() {
    let mut buffered = frame(json!({"method":"Runtime.consoleAPICalled","params":{}}));
    buffered.extend(frame(json!({"id":99,"result":{}})));
    buffered.extend(frame(json!({"id":11,"result":{}})));
    let (mut instance, peer) = pair(buffered).await;
    instance.pending.record(11).unwrap();
    let expired = timeout(WAIT, instance.disconnect_observed(Duration::ZERO)).await;
    let retained = !instance.pending.is_empty();
    let recovered = if retained {
        Some(timeout(WAIT, instance.observe_pending(Instant::now() + WAIT)).await)
    } else { None };
    let empty = instance.pending.is_empty();
    drop(instance);
    let (drained, bytes) = collect(peer).await;
    assert!(drained.is_ok(), "fixture peer did not settle: {drained:?}");
    assert!(retained, "expired drain consumed outstanding work");
    assert_eq!(expired.unwrap().unwrap_err(), UNCONFIRMED);
    recovered.unwrap().unwrap().unwrap();
    assert!(empty, "fresh drain did not observe the retained terminal reply");
    assert!(bytes.is_empty(), "expired drain disconnected the peer");
}

async fn answer_fresh(peer: TcpStream) -> Result<Vec<Value>, String> {
    let mut socket = WebSocketStream::from_raw_socket(peer, Role::Server, None).await;
    let result = async {
        let mut seen = Vec::new();
        for _ in 0..2 {
            let incoming = socket.next().await.ok_or("fixture peer closed")?
                .map_err(|error| error.to_string())?;
            let command: Value = serde_json::from_str(incoming.to_text()
                .map_err(|error| error.to_string())?)
                .map_err(|error| error.to_string())?;
            let id = command["id"].as_u64().ok_or("fixture command has no id")?;
            let complete = command["method"] == "Runtime.evaluate";
            seen.push(command);
            socket.send(Message::Text(json!({"id":id,"result":{"fresh":true}})
                .to_string().into())).await.map_err(|error| error.to_string())?;
            if complete { return Ok(seen); }
        }
        Err("fixture exceeded its two-command bound".to_owned())
    }.await;
    drop(socket);
    result
}

#[tokio::test]
async fn fresh_budget_can_settle_then_dispatch_after_expired_attempt() {
    let (mut instance, peer) = pair(frame(json!({"id":3,"result":{}}))).await;
    instance.next_id = 3;
    instance.pending.record(3).unwrap();
    let expired = timeout(WAIT,
        instance.cmd_deadline("Page.enable", json!({}), Duration::ZERO)).await;
    let retained = instance.pending.0.lock().map(|pending| pending.clone())
        .map_err(|_| "fixture pending lock was poisoned");
    let (fresh, replied) = tokio::join!(
        timeout(WAIT, instance.cmd_deadline("Runtime.evaluate", json!({}), WAIT)),
        timeout(WAIT, answer_fresh(peer)),
    );
    let empty = instance.pending.is_empty();
    drop(instance);
    assert_eq!(retained.unwrap(), BTreeSet::from([3]));
    assert_eq!(expired.unwrap().unwrap_err(), "cdp eval timed out");
    assert_eq!(fresh.unwrap().unwrap(), json!({"fresh":true}));
    let seen = replied.unwrap().unwrap();
    assert_eq!(seen.len(), 1, "expired attempt dispatched an extra command");
    assert_eq!(seen[0]["id"], 4);
    assert_eq!(seen[0]["method"], "Runtime.evaluate");
    assert!(empty, "fresh terminal reply left the instance fenced");
}

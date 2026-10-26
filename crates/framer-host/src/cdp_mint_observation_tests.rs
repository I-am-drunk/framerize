use super::*;
use std::time::Duration;
use tokio::net::{TcpListener, TcpStream};
use tokio::time::timeout;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, tungstenite::protocol::Role};

type Peer = WebSocketStream<TcpStream>;
const WAIT: Duration = Duration::from_secs(3);

async fn connected() -> (CdpBridge, AccountId, Peer) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let (client, (peer, _)) = tokio::try_join!(
        TcpStream::connect(listener.local_addr().unwrap()), listener.accept()).unwrap();
    let ws = WebSocketStream::from_raw_socket(MaybeTlsStream::Plain(client), Role::Client, None).await;
    let peer = WebSocketStream::from_raw_socket(peer, Role::Server, None).await;
    let bridge = CdpBridge::with_engine("unused", "/fixture-uncreated", EngineKind::Attach);
    let account = AccountId::new();
    let pending = bridge.route_cell(&account).await.lock().await.pending.clone();
    bridge.instances.lock().await.insert(account,
        Arc::new(Mutex::new(CdpInstance { child: None, ws, next_id: 0, pending })));
    (bridge, account, peer)
}

async fn command(peer: &mut Peer) -> Value {
    let frame = timeout(WAIT, peer.next()).await.unwrap().unwrap().unwrap();
    let request: Value = serde_json::from_str(frame.to_text().unwrap()).unwrap();
    assert_eq!(request["method"], "Runtime.evaluate");
    assert_eq!(request["params"]["awaitPromise"], true);
    request
}

#[tokio::test]
async fn unobserved_mint_returns_cleanup_uncertainty_without_retry() {
    let (bridge, account, mut peer) = connected().await;
    let disconnect = async move {
        command(&mut peer).await;
        peer.close(None).await.unwrap();
    };
    let (result, ()) = tokio::join!(timeout(WAIT, bridge.castle_token(&account)), disconnect);
    let closed = timeout(WAIT, bridge.shutdown()).await;
    assert_eq!(closed.unwrap().unwrap_err(), cdp_pending_commands::UNCONFIRMED);
    assert_eq!(result.expect("unobserved mint entered the retry delay").err().unwrap(),
        cdp_pending_commands::UNCONFIRMED);
}

#[tokio::test]
async fn observed_mint_errors_and_empty_results_keep_the_retry_budget() {
    let (bridge, account, mut peer) = connected().await;
    let token = "fixture-token".repeat(50);
    let expected = token.clone();
    let replies = async move {
        for (index, mut reply) in [
            json!({"error":{"code":-32000, "message":"completed fixture failure"}}),
            json!({"result":{"result":{"type":"undefined"}}}),
            json!({"result":{"result":{"type":"string", "value":token}}}),
        ].into_iter().enumerate() {
            let request = command(&mut peer).await;
            assert_eq!(request["id"].as_u64(), Some(index as u64 + 1));
            reply["id"] = request["id"].clone();
            peer.send(Message::Text(reply.to_string().into())).await.unwrap();
        }
        peer
    };
    let (result, mut peer) = timeout(Duration::from_secs(8), async {
        tokio::join!(bridge.castle_token(&account), replies)
    }).await.expect("observed retries did not finish");
    let (closed, frame) = tokio::join!(timeout(WAIT, bridge.shutdown()),
        timeout(WAIT, peer.next()));
    closed.unwrap().unwrap();
    assert!(frame.unwrap().unwrap().unwrap().is_close());
    assert_eq!(result.unwrap().0, expected);
    assert!(bridge.instances.lock().await.is_empty());
}

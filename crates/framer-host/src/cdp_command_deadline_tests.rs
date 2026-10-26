use super::*;
use std::time::Duration;
use tokio::io::AsyncReadExt;
use tokio::net::{TcpSocket, TcpStream};
use tokio::time::timeout;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};
use tokio_tungstenite::tungstenite::protocol::Role;

async fn pair() -> (CdpInstance, TcpStream) {
    let listener = TcpSocket::new_v4().unwrap();
    listener.set_recv_buffer_size(1024).unwrap();
    listener.bind("127.0.0.1:0".parse().unwrap()).unwrap();
    let listener = listener.listen(1).unwrap();
    let client = TcpSocket::new_v4().unwrap();
    client.set_send_buffer_size(1024).unwrap();
    let (client, (peer, _)) = tokio::try_join!(
        client.connect(listener.local_addr().unwrap()), listener.accept()).unwrap();
    let ws = WebSocketStream::from_raw_socket(MaybeTlsStream::Plain(client), Role::Client, None).await;
    (CdpInstance { child: None, ws, next_id: 0, pending: Default::default() }, peer)
}

async fn joined<T>(mut peer: tokio::task::JoinHandle<T>) -> T {
    match timeout(Duration::from_secs(3), &mut peer).await {
        Ok(result) => result.unwrap(),
        Err(_) => {
            peer.abort();
            let _ = peer.await;
            panic!("inert command peer did not settle");
        }
    }
}

#[tokio::test]
async fn command_deadline_covers_a_blocked_socket_write() {
    let (mut instance, mut peer) = pair().await;
    let params = json!({"expression": "x".repeat(1024 * 1024)});
    let result = timeout(Duration::from_secs(1), instance.cmd_deadline(
        "Runtime.evaluate", params, Duration::from_millis(100))).await;
    drop(instance);
    let mut sent = Vec::new();
    timeout(Duration::from_secs(3), peer.read_to_end(&mut sent)).await.unwrap().unwrap();
    assert!(!sent.is_empty() && sent.len() < 1024 * 1024, "fixture did not stall the write");
    assert_eq!(result.expect("command write outlived its deadline").unwrap_err(),
        "cdp eval timed out");
}

async fn reply(reply: Option<Value>) -> Result<Value, String> {
    let (mut instance, peer) = pair().await;
    let server = tokio::spawn(async move {
        let mut peer = WebSocketStream::from_raw_socket(peer, Role::Server, None).await;
        let command = peer.next().await.unwrap().unwrap().into_text().unwrap();
        let command: Value = serde_json::from_str(&command).unwrap();
        assert_eq!(command["method"], "Page.enable");
        if let Some(mut reply) = reply {
            reply["id"] = command["id"].clone();
            peer.send(Message::Text(reply.to_string().into())).await.unwrap();
        }
        match peer.next().await {
            None | Some(Err(_)) => {},
            Some(Ok(frame)) => assert!(frame.is_close()),
        }
    });
    let result = timeout(Duration::from_secs(1), instance.cmd_deadline(
        "Page.enable", json!({}), Duration::from_millis(250))).await;
    drop(instance);
    joined(server).await;
    result.expect("command exceeded the fixture deadline")
}

#[tokio::test]
async fn completed_write_still_has_a_bounded_reply_wait() {
    assert_eq!(reply(None).await.unwrap_err(), "cdp eval timed out");
}

#[tokio::test]
async fn command_reply_values_and_protocol_errors_are_preserved() {
    assert_eq!(reply(Some(json!({"result": {"accepted": true}}))).await.unwrap(),
        json!({"accepted": true}));
    let error = reply(Some(json!({"error": {"code": -32601, "message": "fixture failure"}})))
        .await.unwrap_err();
    assert!(error.starts_with("cdp error: ") && error.contains("fixture failure"));
}

use super::*;
use tokio::io::AsyncWriteExt;

#[tokio::test]
async fn owned_browser_closes_and_exits_but_attached_browser_only_disconnects() {
    for owned in [false, true] {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let child = owned.then(|| Command::new("python3").args(["-I", "-B", "-c",
            "import sys; sys.stdin.read(1)"]).stdin(Stdio::piped())
            .kill_on_drop(true).spawn().unwrap());
        let mut child = child;
        let input = child.as_mut().map(|child| child.stdin.take().unwrap());
        let peer = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.unwrap();
            let mut socket = tokio_tungstenite::accept_async(socket).await.unwrap();
            let frame = socket.next().await.unwrap().unwrap();
            if let Some(mut input) = input {
                let value: Value = serde_json::from_str(frame.to_text().unwrap()).unwrap();
                assert_eq!(value["method"], "Browser.close");
                socket.send(Message::Text(json!({"id":value["id"],"result":{}})
                    .to_string().into())).await.unwrap();
                input.write_all(b"x").await.unwrap();
            } else {
                assert!(frame.is_close(), "attached browser received a CDP command");
            }
        });
        let (ws, _) = connect_async(format!("ws://{address}")).await.unwrap();
        let bridge = CdpBridge::new("unused", "/fixture-unused");
        bridge.instances.lock().await.insert(AccountId::new(),
            Arc::new(Mutex::new(CdpInstance { child, ws, next_id: 0 })));
        tokio::time::timeout(std::time::Duration::from_secs(3), bridge.shutdown())
            .await.unwrap().unwrap();
        peer.await.unwrap();
        assert!(bridge.instances.lock().await.is_empty());
        assert!(bridge.instance(&AccountId::new()).await.err().unwrap().contains("shutting down"));
    }
}

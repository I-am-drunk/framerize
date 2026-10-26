//! Stalled loopback upgrades only. No browser or executable is launched.
use super::*;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::time::timeout;

struct Profile(PathBuf);
impl Drop for Profile {
    fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); }
}

struct Fixture {
    profile: Profile,
    address: String,
    peer: tokio::task::JoinHandle<()>,
    upgraded: tokio::sync::oneshot::Receiver<()>,
}

async fn headers(socket: &mut tokio::net::TcpStream) -> Vec<u8> {
    let mut request = Vec::new();
    while !request.ends_with(b"\r\n\r\n") {
        assert!(request.len() < 4096);
        request.push(socket.read_u8().await.unwrap());
    }
    request
}

impl Fixture {
    async fn new() -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let profile = Profile(std::env::temp_dir().join(format!("cdp-upgrade-{}", uuid::Uuid::new_v4())));
        std::fs::create_dir(&profile.0).unwrap();
        std::fs::write(profile.0.join("DevToolsActivePort"), format!("{}\n", address.port())).unwrap();
        let (entered, upgraded) = tokio::sync::oneshot::channel();
        let peer = tokio::spawn(async move {
            let (mut http, _) = listener.accept().await.unwrap();
            assert!(headers(&mut http).await.starts_with(b"GET /json/list HTTP/1.1\r\n"));
            let body = json!([{"type":"page", "url":FRAMER_HOME,
                "webSocketDebuggerUrl":format!("ws://{address}/page")}]).to_string();
            let reply = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{body}", body.len());
            http.write_all(reply.as_bytes()).await.unwrap();
            drop(http);
            let (mut socket, _) = listener.accept().await.unwrap();
            assert!(headers(&mut socket).await.starts_with(b"GET /page HTTP/1.1\r\n"));
            entered.send(()).unwrap();
            // Never send the upgrade response. The caller must close its socket.
            assert_eq!(socket.read(&mut [0; 1]).await.unwrap(), 0);
        });
        Self { profile, address: address.to_string(), peer, upgraded }
    }

    fn start(&self, owned: bool) -> tokio::task::JoinHandle<Result<WsStream, String>> {
        let profile = self.profile.0.clone();
        let address = self.address.clone();
        tokio::spawn(async move {
            if owned { bring_up_ws(&profile).await }
            else { attach_ws(Path::new(&address), &AccountId::new()).await }
        })
    }

    async fn finish(mut self) {
        let joined = timeout(Duration::from_secs(2), &mut self.peer).await;
        if joined.is_err() {
            self.peer.abort();
            let _ = self.peer.await;
        }
        joined.unwrap().unwrap();
    }
}

async fn deadline_closes(owned: bool) {
    let fixture = Fixture::new().await;
    let mut request = fixture.start(owned);
    let result = timeout(Duration::from_secs(8), &mut request).await;
    if result.is_err() {
        request.abort();
        let _ = request.await;
    }
    fixture.finish().await;
    assert_eq!(result.unwrap().unwrap().err().unwrap(),
        "devtools WebSocket connection timed out");
}

#[tokio::test]
async fn owned_upgrade_deadline_closes_stalled_socket() {
    deadline_closes(true).await;
}

#[tokio::test]
async fn attach_upgrade_deadline_closes_stalled_socket() {
    deadline_closes(false).await;
}

#[tokio::test]
async fn cancelled_upgrade_closes_stalled_socket() {
    for owned in [true, false] {
        let mut fixture = Fixture::new().await;
        let request = fixture.start(owned);
        let entered = timeout(Duration::from_secs(3), &mut fixture.upgraded).await;
        request.abort();
        let result = request.await;
        fixture.finish().await;
        entered.unwrap().unwrap();
        assert!(result.unwrap_err().is_cancelled());
    }
}

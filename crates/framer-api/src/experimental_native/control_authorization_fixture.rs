use super::*;
use crate::experimental_native::{native_account::NativeAccountClient, native_http::NativeHttpLimits};
use framer_core::ids::{AccountId, FramerUserId};
use tokio::{net::TcpListener, io::{AsyncReadExt, AsyncWriteExt}, task::JoinHandle};
use std::time::Duration;

#[derive(Default)]
pub struct RecordingBrowser {
    pub calls: std::sync::Mutex<Vec<(&'static str, AccountId)>>,
}
#[async_trait::async_trait]
impl BrowserBridge for RecordingBrowser {
    async fn castle_token(&self, account: &AccountId) -> Result<framer_core::port::CastleToken, String> {
        self.calls.lock().unwrap().push(("token", *account));
        Ok(framer_core::port::CastleToken("fixture-only-token".into()))
    }
    async fn seed_session(&self, account: &AccountId, cookie: &str) -> Result<(), String> {
        assert_eq!(cookie, session().cookie_header);
        self.calls.lock().unwrap().push(("seed", *account));
        Ok(())
    }
    async fn auth_begin(&self, _: &AccountId, _: &str) -> Result<(), String> {
        panic!("unexpected login attempt")
    }
    async fn auth_complete(&self, _: &AccountId, _: &str) -> Result<(String, serde_json::Value), String> {
        panic!("unexpected login completion")
    }
}

pub fn prepare(client: &NativeAccountClient, account: &AccountScope,
    source: BoundBrowserSessionSource) -> crate::experimental_native::native_account::PendingPreparation<String> {
    let expected = account.clone();
    client.prepare_owned(account, tokio::time::Instant::now() + Duration::from_secs(5),
        move |session, control| async move {
            let issued = source.issue(&expected, &session, &control).await;
            PreparationResult { value: issued.value.map(|material| material.bearer),
                cleanup_confirmed: issued.cleanup_confirmed }
        }).unwrap()
}

pub const BOUND: Duration = Duration::from_secs(2);
pub fn session() -> Session {
    Session { active_user: FramerUserId("bound-control-fixture".into()),
        cookie_header: "session=fixture-only-cookie".into() }
}
pub fn scope(egress: NativeEgress<'_>) -> (NativeAccountClient, AccountScope) {
    let client = NativeAccountClient::new().unwrap();
    let scope = client.open_account(AccountId(uuid::Uuid::new_v4()), session(),
        NativeHttpLimits::default(), egress).unwrap();
    (client, scope)
}
pub async fn close(client: &NativeAccountClient, scope: &AccountScope) {
    tokio::time::timeout(BOUND, client.close_account(scope).unwrap()).await.unwrap().unwrap();
}
pub async fn listener() -> TcpListener {
    TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).await.unwrap()
}
pub fn token() -> serde_json::Value {
    serde_json::json!({"accessToken": "fixture-bearer",
        "expiresAt": (chrono::Utc::now() + chrono::Duration::minutes(10)).to_rfc3339()})
}
pub fn response(status: u16, body: &str, headers: &str) -> String {
    format!("HTTP/1.1 {status} Fixture\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n{headers}\r\n{body}", body.len())
}

pub fn bound(scope: &AccountScope, address: std::net::SocketAddr,
    egress: NativeEgress<'_>) -> BoundControlClient {
    assert!(scope.matches_egress(&egress));
    BoundControlClient { expected: scope.clone(),
        transport: crate::transport::Transport::control_for_loopback(address, egress).unwrap() }
}

pub async fn read_request(socket: &mut tokio::net::TcpStream) -> String {
    let mut bytes = Vec::new();
    loop {
        let mut part = [0_u8; 2048];
        let count = socket.read(&mut part).await.unwrap();
        assert!(count > 0, "fixture request ended before headers");
        bytes.extend_from_slice(&part[..count]);
        assert!(bytes.len() <= 16 * 1024, "fixture request exceeds bound");
        if bytes.windows(4).any(|part| part == b"\r\n\r\n") { break; }
    }
    String::from_utf8(bytes).unwrap()
}

pub fn serve_once(listener: TcpListener, reply: String) -> JoinHandle<String> {
    tokio::spawn(async move {
        tokio::time::timeout(BOUND, async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let request = read_request(&mut socket).await;
            socket.write_all(reply.as_bytes()).await.unwrap();
            socket.shutdown().await.unwrap();
            request
        }).await.unwrap()
    })
}

pub async fn result(worker: JoinHandle<String>) -> String {
    tokio::time::timeout(BOUND, worker).await.unwrap().unwrap()
}

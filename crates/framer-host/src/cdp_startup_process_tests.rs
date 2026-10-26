//! Inert Python stand-ins and loopback CDP only, never a browser or live URL.
use super::*;
use std::{os::unix::fs::PermissionsExt, path::PathBuf, time::Duration};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::time::timeout;

const SCRIPT: &str = r#"#!/usr/bin/env python3
import os, pathlib, sys, time
if '--version' in sys.argv:
    print('Chromium 1.2.3.4')
    raise SystemExit(0)
profile = pathlib.Path(next(arg.split('=', 1)[1] for arg in sys.argv
                            if arg.startswith('--user-data-dir=')))
(profile / 'fixture.pid').write_text(str(os.getpid()))
port = %PORT%
if port is not None:
    (profile / 'DevToolsActivePort').write_text(str(port) + '\n')
time.sleep(30)
"#;

struct Fixture {
    root: PathBuf,
    executable: PathBuf,
    profiles: PathBuf,
}

impl Fixture {
    fn new(port: Option<u16>) -> Self {
        let root = std::env::temp_dir().join(format!("cdp-startup-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        let executable = root.join("inert-chrome");
        let port = port.map(|port| port.to_string()).unwrap_or_else(|| "None".into());
        std::fs::write(&executable, SCRIPT.replace("%PORT%", &port)).unwrap();
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
        let profiles = root.join("profiles");
        Self { root, executable, profiles }
    }

    fn bridge(&self, engine: EngineKind) -> Arc<CdpBridge> {
        Arc::new(CdpBridge::with_engine(&self.executable, &self.profiles, engine))
    }

    fn pid_file(&self, account: AccountId) -> PathBuf {
        self.profiles.join(account.0.to_string()).join("fixture.pid")
    }

    fn assert_exited(&self, account: AccountId) {
        let pid = std::fs::read_to_string(self.pid_file(account)).unwrap();
        let pid: u32 = pid.parse().unwrap();
        assert!(!PathBuf::from(format!("/proc/{pid}")).exists(), "startup child remains");
    }
}

impl Drop for Fixture {
    fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.root); }
}

async fn stall_after_connect(listener: tokio::net::TcpListener,
    reached: tokio::sync::oneshot::Sender<()>) {
    let address = listener.local_addr().unwrap();
    let (mut http, _) = timeout(Duration::from_secs(3), listener.accept()).await.unwrap().unwrap();
    let mut request = [0; 1024];
    timeout(Duration::from_secs(2), http.read(&mut request)).await.unwrap().unwrap();
    let body = json!([{"type": "page", "url": FRAMER_HOME,
        "webSocketDebuggerUrl": format!("ws://{address}")}])
        .to_string();
    let reply = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
    http.write_all(reply.as_bytes()).await.unwrap();
    drop(http);
    let (socket, _) = timeout(Duration::from_secs(3), listener.accept()).await.unwrap().unwrap();
    let mut ws = timeout(Duration::from_secs(2), tokio_tungstenite::accept_async(socket))
        .await.unwrap().unwrap();
    let frame = timeout(Duration::from_secs(2), ws.next()).await.unwrap().unwrap().unwrap();
    let command: Value = serde_json::from_str(frame.to_text().unwrap()).unwrap();
    assert_eq!(command["method"], "Runtime.evaluate");
    reached.send(()).unwrap();
    let closed = timeout(Duration::from_secs(3), ws.next()).await.unwrap();
    if let Some(Ok(frame)) = closed {
        assert!(frame.is_close(), "cancelled startup kept its CDP transport");
    }
}

#[tokio::test]
async fn failed_owned_startup_is_reaped_before_clean_shutdown() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let fixture = Fixture::new(Some(listener.local_addr().unwrap().port()));
    let peer = tokio::spawn(async move {
        let (mut socket, _) = timeout(Duration::from_secs(3), listener.accept()).await.unwrap().unwrap();
        let mut request = [0; 1024];
        timeout(Duration::from_secs(2), socket.read(&mut request)).await.unwrap().unwrap();
        socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\n[!")
            .await.unwrap();
    });
    let bridge = fixture.bridge(EngineKind::FullChrome);
    let account = AccountId::new();
    let attempt = timeout(Duration::from_secs(5), bridge.instance(&account)).await;
    let closed = timeout(Duration::from_secs(3), bridge.shutdown()).await;
    let joined = timeout(Duration::from_secs(3), peer).await;
    joined.unwrap().unwrap();
    assert_eq!(attempt.unwrap().err().unwrap(), OWNED_STARTUP_UNCONFIRMED);
    closed.unwrap().unwrap();
    fixture.assert_exited(account);
    assert!(bridge.instances.lock().await.is_empty());
}

#[path = "cdp_startup_error_tests.rs"]
mod startup_errors;

#[tokio::test]
async fn cancelled_owned_startup_is_reaped_before_clean_shutdown() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let fixture = Fixture::new(Some(listener.local_addr().unwrap().port()));
    let (reached, connected) = tokio::sync::oneshot::channel();
    let peer = tokio::spawn(stall_after_connect(listener, reached));
    let bridge = fixture.bridge(EngineKind::HeadlessShell);
    let account = AccountId::new();
    let starting = bridge.clone();
    let request = tokio::spawn(async move { starting.instance(&account).await.map(|_| ()) });
    let started = timeout(Duration::from_secs(5), connected).await;
    request.abort();
    let cancelled = request.await;
    let closed = timeout(Duration::from_secs(3), bridge.shutdown()).await;
    let joined = timeout(Duration::from_secs(3), peer).await;
    joined.unwrap().unwrap();
    started.unwrap().unwrap();
    assert!(cancelled.unwrap_err().is_cancelled());
    closed.unwrap().unwrap();
    fixture.assert_exited(account);
    assert!(bridge.instances.lock().await.is_empty());
}

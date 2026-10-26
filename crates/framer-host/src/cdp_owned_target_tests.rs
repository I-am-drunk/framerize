//! Discovery uses temporary port files and loopback peers, never a browser.
use super::*;
use std::sync::atomic::AtomicUsize;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::time::timeout;
use tokio_tungstenite::tungstenite::handshake::server::{Request, Response};

struct Profile(PathBuf);
impl Drop for Profile {
    fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); }
}

struct Observation {
    result: Result<Result<(), String>, tokio::time::error::Elapsed>,
    requests: usize,
    selected: Option<String>,
}

fn target(kind: &str, url: &str, path: &str) -> Value {
    json!({"type":kind, "url":url, "webSocketDebuggerUrl":format!("%WS%{path}")})
}

async fn stop(task: tokio::task::JoinHandle<()>) {
    task.abort();
    match task.await {
        Ok(()) => (),
        Err(error) if error.is_cancelled() => (),
        Err(error) => panic!("inert discovery peer failed: {error}"),
    }
}

async fn discover(replies: Vec<(u16, String)>) -> Observation {
    assert!(!replies.is_empty());
    let http = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let websocket = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let ws_url = format!("ws://{}", websocket.local_addr().unwrap());
    let profile = Profile(std::env::temp_dir().join(format!("owned-target-{}", uuid::Uuid::new_v4())));
    std::fs::create_dir(&profile.0).unwrap();
    std::fs::write(profile.0.join("DevToolsActivePort"),
        format!("{}\n", http.local_addr().unwrap().port())).unwrap();
    let requests = Arc::new(AtomicUsize::new(0));
    let observed = requests.clone();
    let http_peer = tokio::spawn(async move {
        loop {
            let (mut socket, _) = http.accept().await.unwrap();
            let mut request = Vec::new();
            while !request.ends_with(b"\r\n\r\n") {
                assert!(request.len() < 4096, "oversized discovery request");
                request.push(socket.read_u8().await.unwrap());
            }
            assert!(request.starts_with(b"GET /json/list HTTP/1.1\r\n"));
            let index = observed.fetch_add(1, Ordering::SeqCst);
            let (status, body) = &replies[index.min(replies.len() - 1)];
            let body = body.replace("%WS%", &ws_url);
            let header = format!("HTTP/1.1 {status} Fixture\r\nContent-Length: {}\r\n\r\n", body.len());
            socket.write_all(format!("{header}{body}").as_bytes()).await.unwrap();
            let _ = socket.shutdown().await;
        }
    });
    let selected = Arc::new(std::sync::Mutex::new(None));
    let capture = selected.clone();
    let ws_peer = tokio::spawn(async move {
        let (socket, _) = websocket.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_hdr_async(socket,
            move |request: &Request, response: Response| {
                *capture.lock().unwrap() = Some(request.uri().path().to_owned());
                Ok(response)
            }).await.unwrap();
        assert!(socket.next().await.unwrap().unwrap().is_close());
    });
    let result = timeout(Duration::from_secs(25), async {
        let mut socket = bring_up_ws(&profile.0).await?;
        socket.close(None).await.map_err(|error| error.to_string())
    }).await;
    stop(http_peer).await;
    stop(ws_peer).await;
    let selected = selected.lock().unwrap().clone();
    Observation { result, requests: requests.load(Ordering::SeqCst), selected }
}

#[tokio::test]
async fn owned_discovery_skips_other_origins_before_connecting() {
    let mut targets: Vec<_> = [
        "https://not-framer.com/", "https://framer.com.other.invalid/",
        "https://framer.com@other.invalid/", "https://other.invalid/framer.com",
        "http://framer.com/", "https://api.framer.com/", "https://framer.com:444/",
        "about:blank", "https://other.invalid/?next=https://framer.com/",
    ].into_iter().map(|url| target("page", url, "/wrong")).collect();
    targets.push(target("service_worker", FRAMER_HOME, "/worker"));
    targets.push(target("page", FRAMER_HOME, "/right"));
    let observed = discover(vec![(200, json!(targets).to_string())]).await;
    observed.result.unwrap().unwrap();
    assert_eq!(observed.selected.as_deref(), Some("/right"));
    assert_eq!(observed.requests, 1);
}

#[tokio::test]
async fn owned_discovery_waits_for_framer_readiness_and_keeps_first_match() {
    let observed = discover(vec![
        (503, "fixture not ready".into()),
        (200, "[]".into()),
        (200, json!([target("service_worker", FRAMER_HOME, "/worker"),
            target("page", "about:blank", "/blank")]).to_string()),
        (200, json!([{"type":"page", "url":FRAMER_HOME}]).to_string()),
        (200, json!([target("page", "https://FRAMER.COM:443/projects/fixture", "/first"),
            target("page", FRAMER_HOME, "/second")]).to_string()),
    ]).await;
    observed.result.unwrap().unwrap();
    assert_eq!(observed.selected.as_deref(), Some("/first"));
    assert_eq!(observed.requests, 5);
}

#[tokio::test]
async fn owned_discovery_exhaustion_never_connects_to_an_unrelated_page() {
    let body = json!([target("page", "https://other.invalid/", "/wrong")]).to_string();
    let observed = discover(vec![(200, body)]).await;
    assert_eq!(observed.result.unwrap().unwrap_err(), "no page target");
    assert_eq!(observed.requests, 60);
    assert_eq!(observed.selected, None);
}

#[tokio::test]
async fn owned_discovery_malformed_json_stays_terminal() {
    let ready = json!([target("page", FRAMER_HOME, "/ready")]).to_string();
    let observed = discover(vec![(200, "[!".into()), (200, ready)]).await;
    assert_eq!(observed.result.unwrap().unwrap_err(), "expected value at line 1 column 2");
    assert_eq!(observed.requests, 1);
    assert_eq!(observed.selected, None);
}

#[tokio::test]
async fn owned_discovery_websocket_errors_stay_terminal() {
    let broken = json!([{"type":"page", "url":FRAMER_HOME,
        "webSocketDebuggerUrl":"http://127.0.0.1/"}]).to_string();
    let ready = json!([target("page", FRAMER_HOME, "/ready")]).to_string();
    let observed = discover(vec![(200, broken), (200, ready)]).await;
    assert_eq!(observed.result.unwrap().unwrap_err(), "URL error: URL scheme not supported");
    assert_eq!(observed.requests, 1);
    assert_eq!(observed.selected, None);
}

#[tokio::test]
async fn owned_discovery_http_exhaustion_keeps_transport_error() {
    let observed = discover(vec![(503, String::new())]).await;
    assert_eq!(observed.result.unwrap().unwrap_err(),
        "devtools http never came up: devtools HTTP status 503");
    assert_eq!(observed.requests, 60);
    assert_eq!(observed.selected, None);
}

use super::*;
use super::super::{NativeEgress, NativeHttpLimits};
use framer_core::ids::{AccountId, FramerUserId};
use tokio::{io::{AsyncReadExt, AsyncWriteExt}, net::TcpListener, sync::oneshot};

pub(crate) const NONCE: &str = "0123456789abcdef0123456789abcdef";
pub(crate) struct Peer {
    pub url: String,
    pub address: std::net::SocketAddr,
    entered: oneshot::Receiver<()>,
    stop: Option<oneshot::Sender<()>>,
    task: tokio::task::JoinHandle<Option<(String, bool)>>,
}

pub(crate) fn response(status: u16, media: &str, body: &[u8]) -> Vec<u8> {
    let mut data = format!("HTTP/1.1 {status} Fixture\r\nContent-Type: {media}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()).into_bytes();
    data.extend_from_slice(body);
    data
}

fn transport(address: std::net::SocketAddr) -> NativeHttp {
    NativeHttp::for_loopback(AccountId::new(), FramerUserId("fixture-user".into()),
        address, NativeHttpLimits::default()).unwrap()
}

#[test]
fn probe_requires_canonical_https_and_exactly_one_nonce() {
    let url = format!("https://example.com/ip?flag&blank=&raw=%2f+&framerize_probe={NONCE}");
    let budget = Duration::from_secs(1);
    let request = EgressProbeRequest::new(&url, budget).unwrap();
    assert_eq!(request.url.as_str(), url);
    for invalid in [url.replace("https:", "http:"), url.replace("example.com", "user:pass@example.com"),
        url.replace("example.com", "EXAMPLE.com"), url.replace("example.com", "@example.com"),
        format!("{url}#"), format!("{url}#fragment"), format!("{url}&framerize_probe"),
        format!("{url}&framerize_probe="), format!("{url}&%66ramerize_probe="),
        url.replace(NONCE, ""), url.replace(NONCE, "short"), url.replace(NONCE, &"z".repeat(32)),
        url.replace("/ip?", "/x/../ip?"), url.replace("/ip?", "/\\ip?"), format!("{url}\n"),
        "https://example.com/ip".to_owned()] {
        assert!(EgressProbeRequest::new(&invalid, budget).is_err(), "{invalid}");
    }
    assert!(EgressProbeRequest::new(&url, Duration::ZERO).is_err());
    assert!(EgressProbeRequest::new(&url, Duration::from_secs(31)).is_err());
}

#[tokio::test]
async fn probe_uses_the_existing_client_and_omits_credentials() {
    let peer = Peer::start(Some(response(200, "application/json; charset=utf-8",
        br#"{"ip":"203.0.113.8"}"#))).await;
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert("x-fixture-client", "original".parse().unwrap());
    let client = NativeHttp::build_with_builder(AccountId::new(), FramerUserId("fixture-user".into()),
        format!("http://{}", peer.address), NativeHttpLimits::default(), false, NativeEgress::Direct,
        reqwest::Client::builder().default_headers(headers)).unwrap();
    let request = EgressProbeRequest::for_loopback(&peer.url, Duration::from_secs(1)).unwrap();
    let expected = peer.url.clone();
    let result = client.observe_egress(request, NativeCancellation::new()).await;
    let (wire, closed) = peer.settle().await;
    let observed = result.unwrap();
    assert!(closed);
    assert_eq!(observed.url, expected);
    assert_eq!(observed.address.to_string(), "203.0.113.8");
    assert!(wire.starts_with(&format!("GET /ip?keep=%2F+&flag&framerize_probe={NONCE} HTTP/1.1\r\n")));
    assert!(wire.contains("x-fixture-client: original\r\n"));
    assert!(wire.contains("cache-control: no-store\r\n"));
    for field in ["authorization:", "cookie:", "origin:", "referer:", "accept-encoding:"] {
        assert!(!wire.to_ascii_lowercase().contains(field), "{field}");
    }
}

#[tokio::test]
async fn probe_rejects_status_media_schema_and_oversize() {
    let valid = br#"{"ip":"203.0.113.8"}"#.to_vec();
    let mut cases = vec![(302, "application/json", valid.clone()),
        (200, "text/html", valid), (200, "application/json", vec![0xff]),
        (200, "application/json", vec![b' '; MAX_BODY + 1])];
    for body in [r#"{"ip":"203.0.113.8","extra":1}"#, r#"{"ip":"::1","ip":"::1"}"#,
        r#"{"ip":"invalid"}"#, r#"{"ip":false}"#, r#"{"ip":"fe80::1%eth0"}"#, "{}"] {
        cases.push((200, "application/json", body.as_bytes().to_vec()));
    }
    for (status, media, body) in cases {
        let peer = Peer::start(Some(response(status, media, &body))).await;
        let client = transport(peer.address);
        let request = EgressProbeRequest::for_loopback(&peer.url, Duration::from_secs(1)).unwrap();
        let result = client.observe_egress(request, NativeCancellation::new()).await;
        let (_, joined) = peer.settle().await;
        assert!(joined);
        assert_eq!(result.err(), Some(EgressProbeError::Rejected));
        assert_eq!(client.in_flight.load(std::sync::atomic::Ordering::Acquire), 0);
    }
}

#[tokio::test]
async fn cancelled_probe_drops_response_before_return() {
    let mut peer = Peer::start(None).await;
    let client = std::sync::Arc::new(transport(peer.address));
    let request = EgressProbeRequest::for_loopback(&peer.url, Duration::from_secs(2)).unwrap();
    let cancellation = NativeCancellation::new();
    let worker_cancel = cancellation.clone();
    let worker_client = client.clone();
    let task = tokio::spawn(async move { worker_client.observe_egress(request, worker_cancel).await });
    let entered = peer.entered().await;
    cancellation.cancel();
    let result = task.await;
    let (_, closed) = peer.settle().await;
    assert!(entered && closed);
    assert_eq!(result.unwrap().err(), Some(EgressProbeError::Cancelled));
    assert_eq!(client.in_flight.load(std::sync::atomic::Ordering::Acquire), 0);
}

#[tokio::test]
async fn probe_deadline_releases_its_transport_capacity() {
    let peer = Peer::start(None).await;
    let client = transport(peer.address);
    let request = EgressProbeRequest::for_loopback(&peer.url, Duration::from_millis(100)).unwrap();
    let result = client.observe_egress(request, NativeCancellation::new()).await;
    let (_, closed) = peer.settle().await;
    assert!(closed);
    assert_eq!(result.err(), Some(EgressProbeError::Deadline));
    assert_eq!(client.in_flight.load(std::sync::atomic::Ordering::Acquire), 0);
}

struct NoBrowser;
impl crate::transport::browser::BrowserTransport for NoBrowser {
    fn send(&self, _: crate::transport::browser::BrowserRequest, _: NativeCancellation)
        -> futures::future::BoxFuture<'static, crate::transport::browser::BrowserOutcome> {
        panic!("probe must not use a browser sender")
    }
}

#[tokio::test]
async fn probe_shares_capacity_and_refuses_browser_fallback() {
    use futures::FutureExt;
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let url = format!("http://{address}/ip?framerize_probe={NONCE}");
    let client = transport(address);
    let held = Permit::acquire(&client.in_flight, 1).unwrap();
    let request = EgressProbeRequest::for_loopback(&url, Duration::from_secs(1)).unwrap();
    let busy = client.observe_egress(request, NativeCancellation::new()).await;
    let count = client.in_flight.load(std::sync::atomic::Ordering::Acquire);
    drop(held);
    let client = client.with_browser(std::sync::Arc::new(NoBrowser));
    let request = EgressProbeRequest::for_loopback(&url, Duration::from_secs(1)).unwrap();
    let browser = client.observe_egress(request, NativeCancellation::new()).await;
    assert_eq!(busy.err(), Some(EgressProbeError::Rejected));
    assert_eq!(count, 1);
    assert_eq!(browser.err(), Some(EgressProbeError::Rejected));
    assert!(listener.accept().now_or_never().is_none());
    assert_eq!(client.in_flight.load(std::sync::atomic::Ordering::Acquire), 0);
}

impl Peer {
    pub async fn start(reply: Option<Vec<u8>>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (stop, stopped) = oneshot::channel();
        let (ready, entered) = oneshot::channel();
        let task = tokio::spawn(async move {
            tokio::select! {
                _ = stopped => None,
                result = exchange(listener, ready, reply) => result,
            }
        });
        Self { url: format!("http://{address}/ip?keep=%2F+&flag&framerize_probe={NONCE}"),
            address, entered, stop: Some(stop), task }
    }

    pub async fn entered(&mut self) -> bool {
        matches!(tokio::time::timeout(Duration::from_secs(2), &mut self.entered).await, Ok(Ok(())))
    }

    pub async fn settle(mut self) -> (String, bool) {
        match tokio::time::timeout(Duration::from_secs(2), &mut self.task).await {
            Ok(Ok(Some(result))) => result,
            Ok(_) => (String::new(), false),
            Err(_) => { let _ = self.stop.take().unwrap().send(());
                let _ = self.task.await; (String::new(), false) }
        }
    }
}

async fn exchange(listener: TcpListener, ready: oneshot::Sender<()>, reply: Option<Vec<u8>>)
    -> Option<(String, bool)> {
    let (mut socket, _) = listener.accept().await.ok()?;
    let mut request = Vec::new();
    while !request.windows(4).any(|part| part == b"\r\n\r\n") {
        let mut part = [0; 512];
        let size = socket.read(&mut part).await.ok()?;
        if size == 0 || request.len() + size > 8192 { return None; }
        request.extend_from_slice(&part[..size]);
    }
    let _ = ready.send(());
    let closed = if let Some(reply) = reply {
        socket.write_all(&reply).await.ok()?;
        socket.shutdown().await.ok()?;
        true
    } else {
        socket.read(&mut [0; 1]).await.ok()? == 0
    };
    Some((String::from_utf8(request).ok()?, closed))
}

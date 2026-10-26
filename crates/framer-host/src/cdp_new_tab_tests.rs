use super::*;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::time::timeout;

async fn request(socket: &mut TcpStream) -> Vec<u8> {
    let mut bytes = Vec::new();
    while !bytes.ends_with(b"\r\n\r\n") && !bytes.ends_with(b"\n\n") {
        assert!(bytes.len() < 16 * 1024);
        bytes.push(socket.read_u8().await.unwrap());
    }
    bytes
}

fn framed(body: &[u8]) -> Vec<u8> {
    let mut response = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n", body.len()).into_bytes();
    response.extend_from_slice(body);
    response
}

async fn joined<T>(mut task: tokio::task::JoinHandle<T>) -> T {
    match timeout(Duration::from_secs(8), &mut task).await {
        Ok(result) => result.unwrap(),
        Err(_) => {
            task.abort();
            let _ = task.await;
            panic!("inert HTTP peer did not settle");
        }
    }
}

async fn exchange(response: Vec<u8>, keep_open: bool)
    -> (Result<Vec<u8>, String>, Vec<u8>, bool) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let peer = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let request = request(&mut socket).await;
        socket.write_all(&response).await.unwrap();
        let closed = keep_open && matches!(timeout(Duration::from_secs(2),
            socket.read(&mut [0u8; 1])).await, Ok(Ok(0)));
        (request, closed)
    });
    let url = format!("http://{address}/json/new?https%3A%2F%2Fframer.com%2Flogin");
    let mut call = Box::pin(reqwest_put(&url));
    let first = timeout(Duration::from_secs(1), &mut call).await;
    let (request, closed) = joined(peer).await;
    let result = match first {
        Ok(result) => result,
        Err(_) => {
            let _ = timeout(Duration::from_secs(1), call).await;
            Err("fixture PUT exceeded caller wait".into())
        }
    };
    (result, request, closed)
}

#[tokio::test]
async fn new_tab_put_reads_crlf_body_and_emits_valid_http() {
    let body: &[u8] = br#"{"id":"created","url":"https://framer.com/login"}"#;
    let close_delimited = [b"HTTP/1.0 200 OK\r\n\r\n".as_slice(), body].concat();
    for response in [framed(body), close_delimited] {
        let (result, request, _) = exchange(response, false).await;
        assert_eq!(result.unwrap(), body);
        assert!(request.starts_with(b"PUT /json/new?https%3A%2F%2Fframer.com%2Flogin HTTP/1.1\r\n"));
        assert!(request.ends_with(b"\r\n\r\n"));
        assert!(request.windows(19).any(|part| part == b"Content-Length: 0\r\n"));
    }
}

#[tokio::test]
async fn new_tab_put_finishes_before_a_persistent_peer_closes() {
    let body: &[u8] = br#"{"id":"created"}"#;
    let (result, _, closed) = exchange(framed(body), true).await;
    assert_eq!(result.unwrap(), body);
    assert!(closed, "the framed response did not release the client socket");
}

#[tokio::test]
async fn new_tab_put_rejects_failed_truncated_or_unbounded_replies() {
    let mut responses = vec![
        b"HTTP/1.1 503 Unavailable\r\nContent-Length: 2\r\n\r\n{}".to_vec(),
        b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\n{}".to_vec(),
        b"HTTP/1.1 200 OK\r\nContent-Length: 8388609\r\n\r\n".to_vec(),
        b"HTTP/1.1 200 OK\r\nContent-Length: invalid\r\n\r\n".to_vec(),
        b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n0\r\n\r\n".to_vec(),
        b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nContent-Length: 3\r\n\r\n{}".to_vec(),
    ];
    responses.push(format!("HTTP/1.1 200 OK\r\nX-Fill: {}\r\n\r\n", "x".repeat(16 * 1024)).into_bytes());
    let mut oversized = b"HTTP/1.0 200 OK\r\n\r\n".to_vec();
    oversized.resize(oversized.len() + 8 * 1024 * 1024 + 1, b'x');
    responses.push(oversized);
    for response in responses {
        let (result, _, _) = exchange(response, false).await;
        assert!(result.is_err());
    }
}

#[tokio::test]
async fn cancelled_new_tab_put_closes_its_socket() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (entered, ready) = tokio::sync::oneshot::channel();
    let peer = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        request(&mut socket).await;
        entered.send(()).unwrap();
        matches!(timeout(Duration::from_millis(500), socket.read(&mut [0u8; 1]))
            .await, Ok(Ok(0)))
    });
    let call = tokio::spawn(async move {
        reqwest_put(&format!("http://{address}/json/new?fixture")).await
    });
    timeout(Duration::from_secs(2), ready).await.unwrap().unwrap();
    call.abort();
    assert!(call.await.unwrap_err().is_cancelled());
    assert!(joined(peer).await, "cancelled PUT retained its socket");
}

#[tokio::test]
async fn new_tab_put_deadline_closes_its_socket() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let peer = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        request(&mut socket).await;
        matches!(timeout(Duration::from_secs(6), socket.read(&mut [0u8; 1]))
            .await, Ok(Ok(0)))
    });
    let result = timeout(Duration::from_secs(7),
        reqwest_put(&format!("http://{address}/json/new?fixture"))).await;
    let closed = joined(peer).await;
    assert_eq!(result.unwrap().unwrap_err(), "devtools HTTP request timed out");
    assert!(closed, "expired PUT retained its socket");
}

#[tokio::test]
async fn attach_creates_framer_tab_when_no_suitable_page_exists() {
    let http = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = http.local_addr().unwrap();
    let ws = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let websocket_url = format!("ws://{}", ws.local_addr().unwrap());
    let ws_peer = tokio::spawn(async move {
        let (socket, _) = ws.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_async(socket).await.unwrap();
        assert!(socket.next().await.unwrap().unwrap().is_close());
    });
    let http_peer = tokio::spawn(async move {
        let mut requests = Vec::new();
        let created = json!({"webSocketDebuggerUrl": websocket_url}).to_string();
        for (index, body) in [b"[]".as_slice(), created.as_bytes()].into_iter().enumerate() {
            let (mut socket, _) = http.accept().await.unwrap();
            requests.push(request(&mut socket).await);
            socket.write_all(&framed(body)).await.unwrap();
            if index == 1 {
                let _ = timeout(Duration::from_secs(2), socket.read(&mut [0u8; 1])).await;
            }
        }
        requests
    });
    let connected = timeout(Duration::from_secs(4),
        attach_ws(Path::new(&address.to_string()), &AccountId::new())).await;
    let opened = match connected {
        Ok(Ok(mut socket)) => matches!(timeout(Duration::from_secs(2), socket.close(None))
            .await, Ok(Ok(()))),
        _ => false,
    };
    if !opened { ws_peer.abort(); }
    let requests = joined(http_peer).await;
    if opened { joined(ws_peer).await; }
    else { let _ = ws_peer.await; }
    assert!(opened, "new Framer tab did not connect");
    assert_eq!(requests.len(), 2);
    assert!(requests[0].starts_with(b"GET /json/list HTTP/1.1\r\n"));
    assert!(requests[1].starts_with(b"PUT /json/new?https%3A%2F%2Fframer.com%2Flogin HTTP/1.1\r\n"));
}

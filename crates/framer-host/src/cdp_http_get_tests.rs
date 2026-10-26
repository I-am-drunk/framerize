use super::reqwest_get;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::time::timeout;

async fn read_request(socket: &mut TcpStream) -> Vec<u8> {
    let mut bytes = Vec::new();
    while !bytes.ends_with(b"\r\n\r\n") {
        assert!(bytes.len() < 16 * 1024);
        bytes.push(socket.read_u8().await.unwrap());
    }
    bytes
}

fn framed(body: &[u8]) -> Vec<u8> {
    let mut bytes = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n", body.len()).into_bytes();
    bytes.extend_from_slice(body);
    bytes
}

async fn joined<T>(mut peer: tokio::task::JoinHandle<T>) -> T {
    match timeout(Duration::from_secs(8), &mut peer).await {
        Ok(result) => result.unwrap(),
        Err(_) => {
            peer.abort();
            let _ = peer.await;
            panic!("inert GET peer did not settle");
        }
    }
}

async fn exchange(reply: Vec<u8>, keep_open: bool)
    -> (Result<Vec<u8>, String>, Vec<u8>, bool) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let peer = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let request = read_request(&mut socket).await;
        // Invalid replies may be refused before the peer finishes writing.
        let _ = socket.write_all(&reply).await;
        let closed = keep_open && matches!(timeout(Duration::from_secs(2),
            socket.read(&mut [0u8; 1])).await, Ok(Ok(0)));
        (request, closed)
    });
    let url = format!("http://{address}/json/list?fixture=1");
    let result = timeout(Duration::from_secs(6), reqwest_get(&url)).await;
    let (request, closed) = joined(peer).await;
    (result.expect("GET exceeded its deadline"), request, closed)
}

#[tokio::test]
async fn get_preserves_framed_and_close_delimited_discovery() {
    let body: &[u8] = br#"[{"type":"page","url":"https://framer.com/login"}]"#;
    let close_delimited = [b"HTTP/1.0 200 OK\r\n\r\n".as_slice(), body].concat();
    for (reply, keep_open) in [(framed(body), true), (close_delimited, false)] {
        let (result, request, closed) = exchange(reply, keep_open).await;
        assert_eq!(result.unwrap(), body);
        assert!(request.starts_with(b"GET /json/list?fixture=1 HTTP/1.1\r\n"));
        assert!(!keep_open || closed, "framed GET retained its socket");
    }
}

#[tokio::test]
async fn get_rejects_failed_truncated_and_conflicting_replies() {
    for reply in [
        b"HTTP/1.1 503 Unavailable\r\nContent-Length: 2\r\n\r\n[]".as_slice(),
        b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\n[]",
        b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nContent-Length: 3\r\n\r\n[]",
        b"HTTP/1.1 200 OK\r\nContent-Length: invalid\r\n\r\n[]",
        b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n2\r\n[]\r\n0\r\n\r\n",
    ] {
        let (result, _, _) = exchange(reply.to_vec(), false).await;
        assert!(result.is_err(), "invalid GET reply was accepted");
    }
}

#[tokio::test]
async fn get_bounds_headers_and_bodies() {
    let headers = format!("HTTP/1.1 200 OK\r\nX-Fill: {}\r\nContent-Length: 2\r\n\r\n[]",
        "x".repeat(16 * 1024)).into_bytes();
    let mut body = b"HTTP/1.0 200 OK\r\n\r\n".to_vec();
    body.resize(body.len() + 8 * 1024 * 1024 + 1, b'x');
    for reply in [headers, body,
        b"HTTP/1.1 200 OK\r\nContent-Length: 8388609\r\n\r\n".to_vec()] {
        let (result, _, _) = exchange(reply, false).await;
        assert!(result.is_err(), "unbounded GET reply was accepted");
    }
}

#[tokio::test]
async fn cancelled_get_closes_its_socket_before_peer_reply() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (entered, ready) = tokio::sync::oneshot::channel();
    let (observe, aborted) = tokio::sync::oneshot::channel();
    let peer = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        read_request(&mut socket).await;
        entered.send(()).unwrap();
        aborted.await.unwrap();
        matches!(timeout(Duration::from_secs(1), socket.read(&mut [0u8; 1]))
            .await, Ok(Ok(0)))
    });
    let call = tokio::spawn(async move {
        reqwest_get(&format!("http://{address}/json/list")).await
    });
    timeout(Duration::from_secs(2), ready).await.unwrap().unwrap();
    call.abort();
    let cancelled = matches!(call.await, Err(error) if error.is_cancelled());
    observe.send(()).unwrap();
    let closed = joined(peer).await;
    assert!(cancelled);
    assert!(closed, "cancelled GET retained its socket");
}

#[tokio::test]
async fn close_delimited_get_waits_for_eof_after_a_quiet_gap() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let peer = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        read_request(&mut socket).await;
        socket.write_all(b"HTTP/1.0 200 OK\r\n\r\n[").await.unwrap();
        tokio::time::sleep(Duration::from_millis(1100)).await;
        let _ = socket.write_all(b"]").await;
    });
    let result = timeout(Duration::from_secs(6),
        reqwest_get(&format!("http://{address}/json/list"))).await;
    joined(peer).await;
    assert_eq!(result.unwrap().unwrap(), b"[]");
}

#[tokio::test]
async fn get_deadline_closes_a_trickling_response() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let peer = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        read_request(&mut socket).await;
        socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 1000\r\n\r\n").await.unwrap();
        let (mut reader, mut writer) = socket.into_split();
        let mut tick = tokio::time::interval(Duration::from_millis(200));
        let mut byte = [0u8; 1];
        timeout(Duration::from_secs(6), async {
            loop {
                tokio::select! {
                    result = reader.read(&mut byte) => {
                        return matches!(result, Ok(0) | Err(_));
                    }
                    _ = tick.tick() => {
                        if writer.write_all(b"x").await.is_err() { return true; }
                    }
                }
            }
        }).await.unwrap_or(false)
    });
    let result = timeout(Duration::from_secs(7),
        reqwest_get(&format!("http://{address}/json/list"))).await;
    let closed = joined(peer).await;
    assert_eq!(result.unwrap().unwrap_err(), "http get timed out");
    assert!(closed, "GET continued reading after its total deadline");
}

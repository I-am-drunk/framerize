use super::*;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::time::timeout;
use tokio_tungstenite::tungstenite::handshake::server::{Request, Response};

async fn joined<T>(mut task: tokio::task::JoinHandle<T>) -> T {
    match timeout(Duration::from_secs(2), &mut task).await {
        Ok(result) => result.unwrap(),
        Err(_) => {
            task.abort();
            let _ = task.await;
            panic!("inert CDP peer did not settle");
        }
    }
}

async fn selected(targets: Vec<(&str, &str, &str)>) -> String {
    let http = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = http.local_addr().unwrap();
    let websocket = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let ws_address = websocket.local_addr().unwrap();
    let list: Vec<_> = targets.into_iter().map(|(kind, url, path)| json!({
        "type": kind, "url": url,
        "webSocketDebuggerUrl": format!("ws://{ws_address}{path}")
    })).collect();
    let http_peer = tokio::spawn(async move {
        let (mut socket, _) = http.accept().await.unwrap();
        let mut request = Vec::new();
        while !request.ends_with(b"\r\n\r\n") {
            request.push(socket.read_u8().await.unwrap());
        }
        assert!(request.starts_with(b"GET /json/list HTTP/1.1\r\n"));
        let body = serde_json::to_vec(&list).unwrap();
        let header = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n", body.len());
        socket.write_all(header.as_bytes()).await.unwrap();
        socket.write_all(&body).await.unwrap();
        socket.shutdown().await.unwrap();
    });
    let ws_peer = tokio::spawn(async move {
        let (socket, _) = websocket.accept().await.unwrap();
        let selected = Arc::new(std::sync::Mutex::new(String::new()));
        let capture = selected.clone();
        let mut socket = tokio_tungstenite::accept_hdr_async(socket,
            move |request: &Request, response: Response| {
                *capture.lock().unwrap() = request.uri().path().to_owned();
                Ok(response)
            }).await.unwrap();
        assert!(socket.next().await.unwrap().unwrap().is_close());
        let path = selected.lock().unwrap().clone();
        path
    });
    let connected = timeout(Duration::from_secs(2),
        attach_ws(Path::new(&address.to_string()), &AccountId::new())).await;
    let closed = match connected {
        Ok(Ok(mut socket)) => timeout(Duration::from_secs(2), socket.close(None)).await,
        _ => {
            http_peer.abort();
            ws_peer.abort();
            let _ = http_peer.await;
            let _ = ws_peer.await;
            panic!("attachment to inert target failed");
        }
    };
    joined(http_peer).await;
    let path = joined(ws_peer).await;
    closed.unwrap().unwrap();
    path
}

#[tokio::test]
async fn unrelated_framer_mentions_do_not_select_the_attached_page() {
    for url in [
        "https://not-framer.com/", "https://framer.com.other.invalid/",
        "https://other.invalid/framer.com", "https://framer.com@other.invalid/",
        "https://other.invalid/?next=https://framer.com/",
        "https://other.invalid/#framer.com",
    ] {
        let path = selected(vec![("page", url, "/wrong"),
            ("page", "https://framer.com/projects/fixture", "/right")]).await;
        assert_eq!(path, "/right");
    }
}

#[tokio::test]
async fn different_scheme_port_or_subdomain_is_not_the_framer_app() {
    for url in [
        "http://framer.com/", "ftp://framer.com/", "https://api.framer.com/",
        "https://framer.com:444/", "https://framer.com:65536/",
        "about:blank#framer.com", "data:text/plain,framer.com",
    ] {
        let path = selected(vec![("page", url, "/wrong"),
            ("page", "https://framer.com/", "/right")]).await;
        assert_eq!(path, "/right");
    }
}

#[tokio::test]
async fn real_framer_pages_keep_first_page_priority() {
    for url in [
        "https://framer.com/", "https://framer.com/projects/fixture",
        "https://framer.com/login?redirect=%2Fprojects#token=fixture",
        "https://framer.com:443/login",
    ] {
        let path = selected(vec![("service_worker", "https://framer.com/", "/worker"),
            ("page", url, "/first"), ("page", "https://framer.com/login", "/second")]).await;
        assert_eq!(path, "/first");
    }
}

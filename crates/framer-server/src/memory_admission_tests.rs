use super::*;
use axum::{http::Request as HttpRequest, routing::post, Router};
use futures::StreamExt;
use std::sync::atomic::AtomicI64;
use tower::ServiceExt;

fn fixture() -> (Arc<Admission>, Arc<AtomicI64>) {
    let free = Arc::new(AtomicI64::new(i64::MAX));
    let sample = free.clone();
    (Arc::new(Admission { reserved: AtomicU64::new(0),
        sample: Arc::new(move || Some(i128::from(sample.load(Ordering::SeqCst)))) }), free)
}
fn request() -> Request {
    HttpRequest::post("/openai/v1/responses").body(Body::empty()).unwrap()
}

#[tokio::test]
async fn pressure_refuses_new_request_while_existing_stream_completes() {
    let (gate, free) = fixture();
    let calls = Arc::new(AtomicU64::new(0));
    let counted = calls.clone();
    let (tx, rx) = tokio::sync::mpsc::channel::<Result<axum::body::Bytes, std::io::Error>>(2);
    let receiver = Arc::new(std::sync::Mutex::new(Some(rx)));
    let app = Router::new().route("/openai/v1/responses", post(move || {
        counted.fetch_add(1, Ordering::SeqCst);
        let rx = receiver.lock().unwrap().take().unwrap();
        async { Body::from_stream(futures::stream::unfold(rx, |mut rx| async {
            rx.recv().await.map(|chunk| (chunk, rx))
        })) }
    })).layer(axum::middleware::from_fn_with_state(gate.clone(), middleware));
    let accepted = app.clone().oneshot(request()).await.unwrap();
    assert_eq!(accepted.status(), StatusCode::OK);
    let mut stream = accepted.into_body().into_data_stream();
    tx.send(Ok("first".into())).await.unwrap();
    assert_eq!(stream.next().await.unwrap().unwrap(), "first");
    free.store(-1, Ordering::SeqCst);
    let refused = app.oneshot(request()).await.unwrap();
    assert_eq!(refused.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(refused.headers()["retry-after"], "5");
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(gate.reserved.load(Ordering::SeqCst), REQUEST_RESERVE);
    tx.send(Ok("last".into())).await.unwrap();
    drop(tx);
    assert_eq!(stream.next().await.unwrap().unwrap(), "last");
    assert!(stream.next().await.is_none());
    drop(stream);
    assert_eq!(gate.reserved.load(Ordering::SeqCst), 0);
}

#[test]
fn host_headroom_uses_available_memory_and_ten_percent_reserve() {
    assert_eq!(host_headroom("MemTotal: 1000 kB\nMemAvailable: 300 kB\n"), Some(200 * 1024));
    assert_eq!(host_headroom("MemTotal: 1000 kB\nMemAvailable: 90 kB\n"), Some(-10 * 1024));
    assert_eq!(host_headroom("MemFree: 2 kB\n"), None);
    assert_eq!(host_headroom("MemTotal: broken\nMemAvailable: 100 kB\n"), None);
}

#[test]
fn tighter_cgroup_ancestor_limits_new_requests() {
    let root = std::env::temp_dir().join(format!("memory-admission-{}", uuid::Uuid::new_v4()));
    let proc = root.join("proc");
    let cg = root.join("cgroup");
    std::fs::create_dir_all(proc.join("self")).unwrap();
    std::fs::create_dir_all(cg.join("parent/child")).unwrap();
    std::fs::write(proc.join("meminfo"), "MemTotal: 1000 kB\nMemAvailable: 500 kB\n").unwrap();
    std::fs::write(proc.join("self/cgroup"), "0::/parent/child\n").unwrap();
    for (path, value) in [("parent/child/memory.current", "10"),
        ("parent/child/memory.max", "max"), ("parent/memory.current", "899"),
        ("parent/memory.max", "1000")] {
        std::fs::write(cg.join(path), value).unwrap();
    }
    let constrained = headroom(&proc, &cg);
    std::fs::write(cg.join("parent/memory.current"), "901").unwrap();
    let breached = headroom(&proc, &cg);
    std::fs::remove_dir_all(&root).unwrap();
    assert_eq!(constrained, Some(1));
    assert_eq!(breached, Some(-1));
}

#[tokio::test]
async fn unread_response_reserves_room_until_disposal_and_recovers() {
    let (gate, free) = fixture();
    free.store(REQUEST_RESERVE as i64, Ordering::SeqCst);
    let app = Router::new().route("/openai/v1/responses", post(|| async { "ok" }))
        .route("/api/health", axum::routing::get(|| async { "ok" }))
        .layer(axum::middleware::from_fn_with_state(gate.clone(), middleware));
    let first = app.clone().oneshot(request()).await.unwrap();
    assert_eq!(first.status(), StatusCode::OK);
    let second = app.clone().oneshot(request()).await.unwrap();
    assert_eq!(second.status(), StatusCode::SERVICE_UNAVAILABLE);
    let body = axum::body::to_bytes(second.into_body(), 1024).await.unwrap();
    assert_eq!(serde_json::from_slice::<serde_json::Value>(&body).unwrap()["error"]["code"], "memory_pressure");
    free.store(-1, Ordering::SeqCst);
    let health = app.clone().oneshot(HttpRequest::get("/api/health").body(Body::empty()).unwrap()).await.unwrap();
    assert_eq!(health.status(), StatusCode::OK);
    drop(first);
    assert_eq!(gate.reserved.load(Ordering::SeqCst), 0);
    free.store(REQUEST_RESERVE as i64, Ordering::SeqCst);
    assert_eq!(app.oneshot(request()).await.unwrap().status(), StatusCode::OK);
    assert_eq!(gate.reserved.load(Ordering::SeqCst), 0);
}

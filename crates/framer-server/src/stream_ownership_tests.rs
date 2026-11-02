use super::{proxy_response_stream, AbortOnDrop, ClientDialect};
use axum::body::{to_bytes, Body, Bytes};
use framer_core::port::ProxyResponse;
use futures::{stream, StreamExt};
use std::sync::{Arc, Weak};
use std::time::Duration;
use tokio::sync::{mpsc, oneshot};

struct ReleaseOnDrop {
    _payload: Arc<Vec<u8>>,
    dropped: Option<oneshot::Sender<()>>,
}

impl Drop for ReleaseOnDrop {
    fn drop(&mut self) {
        if let Some(tx) = self.dropped.take() {
            let _ = tx.send(());
        }
    }
}

struct Producer {
    receiver: mpsc::Receiver<Result<ProxyResponse, String>>,
    task: AbortOnDrop<()>,
    started: oneshot::Receiver<()>,
    resume: oneshot::Sender<()>,
    dropped: oneshot::Receiver<()>,
    payload: Weak<Vec<u8>>,
}

fn producer(bytes: &'static [u8]) -> Producer {
    let payload = Arc::new(vec![0x5a; 2 * 1024 * 1024]);
    let weak = Arc::downgrade(&payload);
    let (dropped_tx, dropped) = oneshot::channel();
    let held = ReleaseOnDrop { _payload: payload, dropped: Some(dropped_tx) };
    let (started_tx, started) = oneshot::channel();
    let (resume, resume_rx) = oneshot::channel();
    let (tx, receiver) = mpsc::channel(1);
    let task = AbortOnDrop(tokio::spawn(async move {
        let _held = held;
        let _ = started_tx.send(());
        if resume_rx.await.is_ok() {
            let split = bytes.len().min(17);
            let response = ProxyResponse {
                status: 200,
                content_type: "text/event-stream".into(),
                stream: Box::pin(stream::iter([
                    Ok(Bytes::new()),
                    Ok(Bytes::from_static(&bytes[..split])),
                    Ok(Bytes::from_static(&bytes[split..])),
                ])),
                queue_wait_ms: 0,
            };
            let _ = tx.send(Ok(response)).await;
        }
    }));
    Producer { receiver, task, started, resume, dropped, payload: weak }
}

async fn released(dropped: oneshot::Receiver<()>, payload: Weak<Vec<u8>>) {
    tokio::time::timeout(Duration::from_secs(2), dropped)
        .await.expect("producer did not release its resources")
        .expect("drop notification was lost");
    assert!(payload.upgrade().is_none(), "request payload is still owned");
}

#[tokio::test]
async fn dropping_an_unpolled_body_cancels_its_producer() {
    let job = producer(b"data: unused\n\n");
    let body = Body::from_stream(proxy_response_stream(job.receiver, job.task, ClientDialect::OpenAiChat));
    job.started.await.unwrap();
    assert!(job.payload.upgrade().is_some());
    drop(body);
    released(job.dropped, job.payload).await;
    // Keep this sender alive until after cancellation has been verified.
    drop(job.resume);
}

#[tokio::test]
async fn disconnect_while_waiting_leaves_another_stream_working() {
    let cancelled = producer(b"data: cancelled\n\n");
    let healthy = producer(b"data: healthy\n\n");
    let mut waiting = Box::pin(proxy_response_stream(cancelled.receiver, cancelled.task, ClientDialect::OpenAiChat));
    let healthy_body = Body::from_stream(proxy_response_stream(healthy.receiver, healthy.task, ClientDialect::OpenAiChat));
    cancelled.started.await.unwrap();
    healthy.started.await.unwrap();
    {
        let next = waiting.next();
        futures::pin_mut!(next);
        assert!(futures::poll!(next).is_pending());
    }
    drop(waiting);
    released(cancelled.dropped, cancelled.payload).await;
    assert!(healthy.payload.upgrade().is_some());
    healthy.resume.send(()).unwrap();
    assert_eq!(
        to_bytes(healthy_body, 1024).await.unwrap().as_ref(),
        b"data: healthy\n\n",
    );
    released(healthy.dropped, healthy.payload).await;
    drop(cancelled.resume);
}

#[tokio::test]
async fn completed_stream_keeps_exact_bytes_and_releases_its_producer() {
    const TEXT: &str = "data: {\"text\":\"café 日本語 😀\"}\n\ndata: [DONE]\n\n";
    let job = producer(TEXT.as_bytes());
    let body = Body::from_stream(proxy_response_stream(job.receiver, job.task, ClientDialect::OpenAiChat));
    job.started.await.unwrap();
    job.resume.send(()).unwrap();
    assert_eq!(to_bytes(body, 4096).await.unwrap().as_ref(), TEXT.as_bytes());
    released(job.dropped, job.payload).await;
}

#[tokio::test]
async fn producer_error_is_a_terminal_event_with_the_recorded_cause() {
    let (tx, rx) = mpsc::channel(1);
    let task = AbortOnDrop(tokio::spawn(async move {
        tx.send(Err("fixture error\ncontinued".into())).await.unwrap();
    }));
    let body = Body::from_stream(proxy_response_stream(rx, task, ClientDialect::OpenAiChat));
    let wire = String::from_utf8(to_bytes(body, 4096).await.unwrap().to_vec()).unwrap();
    let data = wire.lines().find_map(|line| line.strip_prefix("data: ")).unwrap();
    let event: serde_json::Value = serde_json::from_str(data).unwrap();
    assert_eq!(event["error"]["message"], "fixture error\ncontinued");
    assert!(wire.contains("data: [DONE]"));
    assert!(!wire.contains(": upstream error:"));
}

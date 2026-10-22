use super::*;
use futures::StreamExt;
use std::{sync::atomic::{AtomicUsize, Ordering}, time::Duration};

struct Count(AtomicUsize);
impl Cancellation for Count { fn cancel(&self) { self.0.fetch_add(1, Ordering::SeqCst); } }

fn metadata() -> port::AttemptMetadata {
    port::AttemptMetadata { status: 200, usage_header: Some("fixture usage".into()),
        original_content_type: Some("text/event-stream".into()), step_index: 1,
        elapsed: Duration::from_millis(7) }
}

#[tokio::test]
async fn routing_stream_head_precedes_cleanup_and_keeps_body_storage() {
    let control = Control::new();
    let bytes = axum::body::Bytes::from_static(b"data: fixture\n\n");
    let expected = bytes.clone();
    let headers = Arc::new(port::RawStreamHeaders { status: 200, usage_header: None,
        original_content_type: Some(Arc::from("text/event-stream")),
        content_type: Arc::from("text/event-stream"), elapsed: Duration::from_millis(3) });
    let head = port::RawStreamHead { headers: headers.clone(),
        body: Box::pin(futures::stream::iter([Ok(bytes)])) };
    let (release, released) = oneshot::channel();
    let run = InvocationRun { result: async { Ok(head) }.boxed(),
        cleanup: async { released.await.unwrap() }.boxed().shared(),
        cancellation: Arc::new(Count(AtomicUsize::new(0))),
        completed_attempts: Some(Arc::new(|| vec![metadata()])),
    };
    let (sender, receiver) = oneshot::channel();
    let worker_control = control.clone();
    let task = tokio::spawn(async move {
        owned::forward(run, Instant::now() + Duration::from_secs(2),
            worker_control, &mut Some(sender)).await
    });
    let mut delivered = tokio::time::timeout(Duration::from_millis(250), receiver)
        .await.unwrap().unwrap().unwrap();
    assert!(Arc::ptr_eq(&headers, &delivered.headers));
    assert!(!task.is_finished());
    let chunk = delivered.body.next().await.unwrap().unwrap();
    assert_eq!(chunk.as_ptr(), expected.as_ptr());
    assert_eq!(&chunk[..], &expected[..]);
    assert!(delivered.body.next().await.is_none());
    let progress = control.progress.lock().unwrap().clone().unwrap();
    assert_eq!(progress()[0].step_index, 1);
    assert_eq!(progress()[0].usage_header.as_deref(), Some("fixture usage"));
    release.send(Ok(())).unwrap();
    assert!(task.await.unwrap().confirmed);
}

#[tokio::test]
async fn routing_unpolled_result_drop_cancels_a_later_inner_invocation() {
    let control = Control::new();
    let (_sender, receiver) = oneshot::channel::<Result<port::RawResult, DispatchError>>();
    let run = result(receiver, async { Ok(()) }.boxed().shared(), control.clone());
    drop(run.result);
    let count = Arc::new(Count(AtomicUsize::new(0)));
    let inner = InvocationRun::<port::RawResult> {
        result: futures::future::pending().boxed(), cleanup: async { Ok(()) }.boxed().shared(),
        cancellation: count.clone(), completed_attempts: None,
    };
    control.attach(&inner);
    assert_eq!(count.0.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn routing_completed_result_drop_does_not_cancel_a_stream() {
    let control = Control::new();
    let (sender, receiver) = oneshot::channel();
    let run = result(receiver, async { Ok(()) }.boxed().shared(), control.clone());
    assert!(sender.send(Ok(port::RawResult { body: vec![1], metadata: metadata() })).is_ok());
    assert_eq!(run.result.await.unwrap().body, vec![1]);
    assert!(!*control.stop.borrow());
}

#[tokio::test]
async fn routing_failed_result_waits_for_cleanup_and_retains_metadata() {
    let (release, released) = oneshot::channel();
    let (started, ready) = oneshot::channel();
    let error = DispatchError { kind: FailureKind::Authorization, upstream: None,
        attempts: vec![metadata()] };
    let run = InvocationRun::<port::RawResult> {
        result: async move { let _ = started.send(()); Err(error) }.boxed(),
        cleanup: async { released.await.unwrap() }.boxed().shared(),
        cancellation: Arc::new(Count(AtomicUsize::new(0))), completed_attempts: None,
    };
    let (sender, mut receiver) = oneshot::channel();
    let task = tokio::spawn(async move {
        let mut sender = Some(sender);
        let completed = owned::forward(run, Instant::now() + Duration::from_secs(2),
            Control::new(), &mut sender).await;
        (completed, sender)
    });
    ready.await.unwrap();
    assert!(matches!(receiver.try_recv(), Err(oneshot::error::TryRecvError::Empty)));
    assert!(!task.is_finished());
    release.send(Err(FailureKind::Cleanup)).unwrap();
    let (completed, sender) = task.await.unwrap();
    assert!(sender.is_some() && !completed.confirmed);
    let error = completed.error.unwrap();
    assert_eq!(error.kind, FailureKind::Authorization);
    assert_eq!(error.attempts.len(), 1);
    assert_eq!(error.attempts[0].step_index, 1);
    assert_eq!(error.attempts[0].usage_header.as_deref(), Some("fixture usage"));
}

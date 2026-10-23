use super::*;
use std::{sync::atomic::{AtomicUsize, Ordering}, time::Duration};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

struct Count(AtomicUsize);
impl Cancellation for Count { fn cancel(&self) { self.0.fetch_add(1, Ordering::SeqCst); } }

struct RefusalBody { bytes: Box<[u8]>, _credit: OwnedSemaphorePermit }
impl AsRef<[u8]> for RefusalBody { fn as_ref(&self) -> &[u8] { &self.bytes } }

fn refused() -> (InvocationRun<port::ComposedStreamHead>,
    oneshot::Sender<Result<(), FailureKind>>, Arc<Count>, Arc<Semaphore>) {
    let credit = Arc::new(Semaphore::new(1));
    let body = axum::body::Bytes::from_owner(RefusalBody {
        bytes: Box::from(&b"{\"error\":\"fixture refusal\"}"[..]),
        _credit: credit.clone().try_acquire_owned().unwrap(),
    });
    let metadata = port::AttemptMetadata { status: 403, usage_header: None,
        original_content_type: Some("application/json".into()), step_index: 0,
        elapsed: Duration::from_millis(7) };
    let error = DispatchError { kind: FailureKind::Upstream,
        upstream: Some(port::UpstreamFailure { body, metadata: metadata.clone() }),
        attempts: vec![metadata] };
    let (release, released) = oneshot::channel();
    let retained = credit.clone();
    let cancellation = Arc::new(Count(AtomicUsize::new(0)));
    let run = InvocationRun { result: async { Err(error) }.boxed(),
        cleanup: async move {
            let _returned = retained.acquire_owned().await.unwrap();
            released.await.unwrap()
        }.boxed().shared(), cancellation: cancellation.clone(), completed_attempts: None };
    (run, release, cancellation, credit)
}

#[tokio::test]
async fn routing_collected_refusal_keeps_cleanup_before_error_publication() {
    let (source, release, _, credit) = refused();
    let mut error = source.result.await.err().unwrap();
    // A collected result owns ordinary bytes, not a composed JSON lease.
    let upstream = error.upstream.as_mut().unwrap();
    upstream.body = axum::body::Bytes::copy_from_slice(&upstream.body);
    assert_eq!(credit.available_permits(), 1);
    let run = InvocationRun::<port::RawResult> {
        result: async { Err(error) }.boxed(), cleanup: source.cleanup,
        cancellation: source.cancellation, completed_attempts: None,
    };
    let (sender, mut receiver) = oneshot::channel();
    let mut task = tokio::spawn(async move {
        let mut sender = Some(sender);
        let completion = owned::forward(run, Instant::now() + Duration::from_secs(2),
            Control::new(), &mut sender).await;
        (completion, sender)
    });
    assert!(tokio::time::timeout(Duration::from_millis(20), &mut task).await.is_err());
    assert!(matches!(receiver.try_recv(), Err(oneshot::error::TryRecvError::Empty)));
    release.send(Ok(())).unwrap();
    let (completion, sender) = tokio::time::timeout(Duration::from_secs(1), task).await.unwrap().unwrap();
    assert!(completion.confirmed && sender.is_some());
    let error = completion.error.unwrap();
    assert_eq!(error.kind, FailureKind::Upstream);
    assert_eq!(error.upstream.unwrap().body.as_ref(), b"{\"error\":\"fixture refusal\"}");
}

#[tokio::test]
async fn routing_refusal_drop_releases_body_but_never_confirms_failed_cleanup() {
    for abandoned in [false, true] {
        let (run, release, _, credit) = refused();
        let (sender, receiver) = oneshot::channel();
        let task = tokio::spawn(async move {
            owned::forward(run, Instant::now() + Duration::from_secs(2),
                Control::new(), &mut Some(sender)).await
        });
        if abandoned {
            drop(receiver);
        } else {
            let error = tokio::time::timeout(Duration::from_millis(250), receiver)
                .await.unwrap().unwrap().err().unwrap();
            assert_eq!(error.kind, FailureKind::Upstream);
            drop(error);
        }
        release.send(Err(FailureKind::Cleanup)).unwrap();
        let completion = tokio::time::timeout(Duration::from_secs(1), task).await.unwrap().unwrap();
        assert!(!completion.confirmed);
        assert_eq!(credit.available_permits(), 1);
    }
}

#[tokio::test]
async fn routing_refusal_transfers_body_before_cleanup_and_retains_last_slice() {
    for cancel in [false, true] {
        let (run, release, cancellation, credit) = refused();
        let control = Control::new();
        let (sender, receiver) = oneshot::channel();
        let forwarded = control.clone();
        let mut task = tokio::spawn(async move {
            let mut sender = Some(sender);
            let completion = owned::forward(run, Instant::now() + Duration::from_secs(2),
                forwarded, &mut sender).await;
            (completion, sender.is_none())
        });
        let error = tokio::time::timeout(Duration::from_millis(250), receiver)
            .await.unwrap().unwrap().err().unwrap();
        assert_eq!(error.kind, FailureKind::Upstream);
        assert_eq!(control.failure(), FailureKind::Upstream);
        assert_eq!(error.attempts.len(), 1);
        let upstream = error.upstream.unwrap();
        assert_eq!(upstream.metadata.status, 403);
        assert_eq!(upstream.body.as_ref(), b"{\"error\":\"fixture refusal\"}");
        let held = upstream.body.slice(1..3);
        drop(upstream);
        release.send(Ok(())).unwrap();
        if cancel { control.cancel(); }
        assert_eq!(cancellation.0.load(Ordering::SeqCst), usize::from(cancel));
        assert_eq!(credit.available_permits(), 0);
        assert!(tokio::time::timeout(Duration::from_millis(20), &mut task).await.is_err());
        drop(held);
        let (completion, sent) = tokio::time::timeout(Duration::from_secs(1), task).await.unwrap().unwrap();
        assert!(sent && completion.confirmed && completion.error.is_none());
        assert_eq!(credit.available_permits(), 1);
    }
}

use super::*;
use framer_core::native_dispatch::{FailureKind, RawStreamCompletion, RawStreamHeaders,
    ComposedAttemptMetadata, ComposedStreamCompletion};
use futures::{FutureExt, pin_mut};
use std::{sync::Arc, time::Duration};
use tokio::sync::oneshot;

fn raw(result: Result<(), FailureKind>) -> NativeStreamCompletion {
    NativeStreamCompletion::Raw(RawStreamCompletion { result, headers: None,
        emitted_bytes: 7, elapsed: Duration::from_millis(4) })
}

#[tokio::test]
async fn native_evidence_waits_for_cleanup_after_body_failure() {
    let (send_body, body) = oneshot::channel();
    let (send_cleanup, cleanup) = oneshot::channel();
    let observed = observe(NativeStreamObservers {
        completion: async { body.await.unwrap() }.boxed().shared(),
        cleanup: async { cleanup.await.unwrap() }.boxed().shared(),
    });
    pin_mut!(observed);
    assert!(observed.as_mut().now_or_never().is_none());
    send_body.send(raw(Err(FailureKind::Cancelled))).unwrap_or_else(|_| panic!("body receiver"));
    assert!(observed.as_mut().now_or_never().is_none());
    send_cleanup.send(Ok(())).unwrap();
    assert!(observed.await);
}

#[tokio::test]
async fn native_evidence_keeps_cleanup_failure_and_waits_for_body() {
    let (send_body, body) = oneshot::channel();
    let (send_cleanup, cleanup) = oneshot::channel();
    let observed = observe(NativeStreamObservers {
        completion: async { body.await.unwrap() }.boxed().shared(),
        cleanup: async { cleanup.await.unwrap() }.boxed().shared(),
    });
    pin_mut!(observed);
    send_cleanup.send(Err(FailureKind::Cleanup)).unwrap();
    assert!(observed.as_mut().now_or_never().is_none());
    send_body.send(raw(Ok(()))).unwrap_or_else(|_| panic!("body receiver"));
    assert!(!observed.await);
}

#[test]
fn native_evidence_keeps_only_bounded_fields_for_both_stream_kinds() {
    let headers = Arc::new(RawStreamHeaders { status: 200,
        usage_header: Some(Arc::from("private-fixture-header")),
        original_content_type: Some(Arc::from("private-fixture-content-type")),
        content_type: Arc::from("private-fixture-content-type"), elapsed: Duration::ZERO });
    let raw = NativeStreamCompletion::Raw(RawStreamCompletion {
        result: Err(FailureKind::Cancelled), headers: Some(headers.clone()),
        emitted_bytes: 7, elapsed: Duration::from_millis(4) });
    let composed = NativeStreamCompletion::Composed(ComposedStreamCompletion {
        result: Ok(()), attempts: vec![ComposedAttemptMetadata { headers,
            response_id: Some(Arc::from("private-fixture-id")),
            usage: Some(Arc::new(serde_json::json!({"private-fixture-usage": 9}))),
            step_index: 0, elapsed: Duration::ZERO }].into(),
        emitted_bytes: 7, elapsed: Duration::from_millis(4) });
    for completion in [raw, composed] {
        let record = fields(&completion, false);
        assert!(!record.to_string().contains("private-fixture"));
        assert_eq!(record.as_object().unwrap().len(), 7);
        assert!(record.as_object().unwrap().values().all(|v| !v.is_object() && !v.is_array()));
        assert_eq!(record["cleanup_confirmed"], false);
        assert_eq!(record["observed_attempts"], 1);
        assert_eq!(record["last_status"], 200);
        assert_eq!(record["yielded_bytes"], 7);
        assert_eq!(record["body_elapsed_ms"], 4);
    }
}

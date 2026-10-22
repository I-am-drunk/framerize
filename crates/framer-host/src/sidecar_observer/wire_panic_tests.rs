use super::*;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::AtomicBool;

fn poison(pending: &PendingReplies) {
    assert!(catch_unwind(AssertUnwindSafe(|| {
        let _map = pending.lock().unwrap();
        panic!("fixture pending-map poison");
    })).is_err());
}

#[test]
fn poisoned_reply_map_drop_fences_generation_and_fails_remaining_waiter() {
    let pending: PendingReplies = Arc::new(StdMutex::new(HashMap::new()));
    let (first, mut first_result) = oneshot::channel();
    let (second, mut second_result) = oneshot::channel();
    pending.lock().unwrap().extend([(1, first), (2, second)]);
    let healthy = Arc::new(AtomicBool::new(true));
    let (events, mut received) = broadcast::channel(4);
    poison(&pending);
    assert!(catch_unwind(AssertUnwindSafe(|| {
        let _reply = ObservedReply { id: 1, pending: pending.clone(),
            healthy: healthy.clone(), events: events.clone() };
        panic!("fixture reply-owner unwind");
    })).is_err());
    assert!(matches!(first_result.try_recv(), Err(oneshot::error::TryRecvError::Closed)));
    assert_eq!(second_result.try_recv().unwrap().unwrap_err(), "observed generation is unconfirmed");
    assert!(!healthy.load(Ordering::SeqCst));
    assert!(matches!(received.try_recv(), Ok(RemoteEvent::Lost)));
    assert!(pending.lock().unwrap_or_else(std::sync::PoisonError::into_inner).is_empty());
}

#[test]
fn failing_poisoned_pending_map_reports_all_waiters_without_panicking() {
    let pending: PendingReplies = Arc::new(StdMutex::new(HashMap::new()));
    let (first, mut first_result) = oneshot::channel();
    let (second, mut second_result) = oneshot::channel();
    pending.lock().unwrap().extend([(1, first), (2, second)]);
    poison(&pending);
    assert!(catch_unwind(AssertUnwindSafe(|| fail_pending(&pending, "fixture lost"))).is_ok());
    assert_eq!(first_result.try_recv().unwrap().unwrap_err(), "fixture lost");
    assert_eq!(second_result.try_recv().unwrap().unwrap_err(), "fixture lost");
    assert!(pending.lock().unwrap_or_else(std::sync::PoisonError::into_inner).is_empty());
}

#[test]
fn frame_drop_contains_callback_panic_before_releasing_writer() {
    let pipe = Mutex::new(());
    let observed_locked = AtomicBool::new(false);
    let result = catch_unwind(AssertUnwindSafe(|| {
        let frame = FrameGuard { writer: pipe.try_lock().unwrap(), complete: false,
            on_lost: || {
                observed_locked.store(pipe.try_lock().is_err(), Ordering::SeqCst);
                panic!("fixture invalidate panic");
            } };
        drop(frame);
    }));
    assert!(result.is_ok());
    assert!(observed_locked.load(Ordering::SeqCst));
    assert!(pipe.try_lock().is_ok());
}

#[test]
fn frame_callback_panic_during_outer_unwind_preserves_original_panic() {
    let pipe = Mutex::new(());
    let observed_locked = AtomicBool::new(false);
    let result = catch_unwind(AssertUnwindSafe(|| {
        let _frame = FrameGuard { writer: pipe.try_lock().unwrap(), complete: false,
            on_lost: || {
                observed_locked.store(pipe.try_lock().is_err(), Ordering::SeqCst);
                panic!("fixture nested invalidate panic");
            } };
        panic!("fixture original unwind");
    }));
    let panic = result.expect_err("original panic disappeared");
    assert_eq!(panic.downcast_ref::<&str>().copied(), Some("fixture original unwind"));
    assert!(observed_locked.load(Ordering::SeqCst));
    assert!(pipe.try_lock().is_ok());
}

#[test]
fn unpoisoned_reply_drop_keeps_other_waiter_and_generation_healthy() {
    let pending: PendingReplies = Arc::new(StdMutex::new(HashMap::new()));
    let (first, mut first_result) = oneshot::channel();
    let (second, mut second_result) = oneshot::channel();
    pending.lock().unwrap().extend([(1, first), (2, second)]);
    let healthy = Arc::new(AtomicBool::new(true));
    let (events, mut received) = broadcast::channel(4);
    drop(ObservedReply { id: 1, pending: pending.clone(),
        healthy: healthy.clone(), events: events.clone() });
    assert!(matches!(first_result.try_recv(), Err(oneshot::error::TryRecvError::Closed)));
    assert!(matches!(second_result.try_recv(), Err(oneshot::error::TryRecvError::Empty)));
    assert!(healthy.load(Ordering::SeqCst));
    assert!(matches!(received.try_recv(), Err(broadcast::error::TryRecvError::Empty)));
    assert_eq!(pending.lock().unwrap().len(), 1);
    assert!(pending.lock().unwrap().contains_key(&2));
}

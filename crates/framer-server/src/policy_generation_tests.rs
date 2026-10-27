//! Manager ordering controls with a synthetic process verdict, not OS reap proofs.
use super::*;
use std::time::Duration;

const BOUND: Duration = Duration::from_secs(2);
fn manager(limit: usize) -> Arc<GenerationManager> {
    let scope = crate::operation_ownership::ServingOwnership::new_unmanaged(
        Arc::new(framer_core::wave::WaveGuard::new()), limit).unwrap();
    GenerationManager::new(limit, scope).unwrap()
}
fn ready(manager: &Arc<GenerationManager>) -> (u64, oneshot::Sender<()>) {
    let (process, exit) = PolicyProcess::fixture(manager.limit * 2 + 1);
    let (inbox, _) = Inbox::new(manager.limit);
    { let mut state = inbox.state.lock().unwrap(); state.hello = None; state.ready = true; }
    let worker = Worker::new(process, inbox);
    let mut state = manager.lock().unwrap();
    assert!(state.slots.len() < 2);
    let id = state.next; state.next += 1;
    let (cancel_stage, _) = watch::channel(false);
    state.slots.insert(id, Slot { phase: GenerationPhase::Ready, pins: 0,
        unconfirmed: false, worker: Some(worker), starting: None, retiring: None, cancel_stage });
    (id, exit)
}
async fn pending<F: std::future::Future>(mut future: std::pin::Pin<&mut F>) {
    std::future::poll_fn(|cx| {
        assert!(future.as_mut().poll(cx).is_pending()); std::task::Poll::Ready(())
    }).await;
}

#[tokio::test]
async fn empty_manager_stays_legacy_and_shutdown_is_idempotent() {
    let manager = manager(3);
    assert!(manager.select().unwrap().is_none());
    assert!(manager.snapshot().unwrap().is_empty());
    manager.shutdown(false).await.unwrap();
    manager.shutdown(false).await.unwrap();
    assert!(matches!(manager.select(), Err(GenerationError::Closed)));
}

#[tokio::test]
async fn activation_and_rollback_change_only_future_pins() {
    let manager = manager(3);
    let (a, _a_exit) = ready(&manager);
    let (b, _b_exit) = ready(&manager);
    manager.activate(a).unwrap();
    let first = manager.select().unwrap().unwrap();
    manager.activate(b).unwrap();
    let second = manager.select().unwrap().unwrap();
    manager.activate(a).unwrap();
    let third = manager.select().unwrap().unwrap();
    assert_eq!((first.generation(), second.generation(), third.generation()), (a, b, a));
    assert_ne!(first.operation(), third.operation());
    assert!(matches!(manager.select(), Err(GenerationError::Full)));
    for pin in [first, second, third] { pin.confirm(); }
    tokio::time::timeout(BOUND, manager.shutdown(false)).await.unwrap().unwrap();
}

#[tokio::test]
async fn retained_last_pin_holds_retirement_and_the_second_slot() {
    let manager = manager(3);
    let (a, _a_exit) = ready(&manager);
    let (b, _b_exit) = ready(&manager);
    manager.activate(a).unwrap();
    let pin = manager.select().unwrap().unwrap();
    let retained = pin.clone();
    pin.confirm(); drop(pin);
    manager.activate(b).unwrap();
    let mut retiring = Box::pin(manager.retire(a));
    pending(retiring.as_mut()).await;
    assert_eq!(manager.activate(a), Err(GenerationError::NotReady));
    assert_eq!(manager.stage("/unused-third-worker".into(), [0; 32]).await,
        Err(GenerationError::Full));
    assert!(manager.snapshot().unwrap().iter().any(|s| s.generation == a && s.pins == 1));
    let completion = manager.lock().unwrap().slots.get(&a).unwrap().retiring.clone().unwrap();
    drop(retiring); // Its job remains owned by the manager.
    drop(retained);
    tokio::time::timeout(BOUND, completion).await.unwrap().unwrap();
    assert!(!manager.snapshot().unwrap().iter().any(|s| s.generation == a));
    manager.shutdown(false).await.unwrap();
}

#[tokio::test]
async fn unconfirmed_pin_stays_fenced_after_local_reaping() {
    let manager = manager(2);
    let (a, _exit) = ready(&manager);
    manager.activate(a).unwrap();
    let pin = manager.select().unwrap().unwrap();
    pin.unconfirmed(); pin.confirm(); drop(pin);
    assert_eq!(manager.retire(a).await, Err(GenerationError::Unconfirmed));
    let snapshots = manager.snapshot().unwrap();
    assert_eq!(snapshots[0].phase, GenerationPhase::ReapedUnconfirmed);
    assert_eq!(snapshots[0].pins, 0);
    assert!(!snapshots[0].alive);
    assert!(matches!(manager.select(), Err(GenerationError::Unconfirmed)));
    assert_eq!(manager.shutdown(false).await, Err(GenerationError::Unconfirmed));
}

#[tokio::test]
async fn forced_shutdown_interrupts_a_retirement_waiting_for_pins() {
    let manager = manager(2);
    let (a, _exit) = ready(&manager);
    manager.activate(a).unwrap();
    let pin = manager.select().unwrap().unwrap();
    let mut normal = Box::pin(manager.shutdown(false));
    pending(normal.as_mut()).await;
    assert_eq!(tokio::time::timeout(BOUND, manager.shutdown(true)).await.unwrap(),
        Err(GenerationError::Unconfirmed));
    assert_eq!(normal.await, Err(GenerationError::Unconfirmed));
    assert!(!manager.snapshot().unwrap()[0].alive);
    pin.confirm(); drop(pin);
    assert!(manager.snapshot().unwrap()[0].unconfirmed);
}

#[tokio::test]
async fn failed_candidate_spawn_preserves_active_selection_and_releases_slot() {
    let manager = manager(2);
    let (a, _exit) = ready(&manager);
    manager.activate(a).unwrap();
    assert_eq!(manager.stage(std::env::temp_dir().join(uuid::Uuid::new_v4().to_string()),
        [0; 32]).await, Err(GenerationError::Process));
    assert_eq!(manager.snapshot().unwrap().len(), 1);
    let pin = manager.select().unwrap().unwrap();
    assert_eq!(pin.generation(), a);
    pin.confirm(); drop(pin);
    manager.shutdown(false).await.unwrap();
}

#[tokio::test]
async fn dead_ready_candidate_cannot_revive_or_replace_active_worker() {
    let manager = manager(2);
    let (a, _a_exit) = ready(&manager);
    let (b, b_exit) = ready(&manager);
    let mut alive = manager.lock().unwrap().slots[&b].worker.as_ref().unwrap().alive.clone();
    manager.activate(a).unwrap();
    b_exit.send(()).unwrap();
    tokio::time::timeout(BOUND, alive.changed()).await.unwrap().unwrap();
    assert_eq!(manager.activate(b), Err(GenerationError::NotReady));
    let pin = manager.select().unwrap().unwrap();
    assert_eq!(pin.generation(), a);
    pin.confirm(); drop(pin);
    manager.shutdown(false).await.unwrap();
}

#[tokio::test]
async fn shared_unconfirmed_gate_prevents_legacy_fallback() {
    let manager = manager(2);
    drop(manager.ownership.external_cleanup().unwrap());
    assert!(!manager.ownership.accepting_requests());
    assert!(matches!(manager.select(), Err(GenerationError::Unconfirmed)));
    manager.shutdown(false).await.unwrap();
    assert!(!manager.ownership.accepting_requests());
}

#[tokio::test]
async fn routing_uses_only_reserved_weak_operation_handlers() {
    use std::sync::atomic::AtomicUsize;
    let manager = manager(2);
    let (a, _exit) = ready(&manager);
    manager.activate(a).unwrap();
    let pin = manager.select().unwrap().unwrap();
    let op = pin.operation();
    let calls = Arc::new(AtomicUsize::new(0));
    let seen = calls.clone();
    let handler: Dispatch = Arc::new(move |_| { seen.fetch_add(1, Ordering::SeqCst); Ok(()) });
    let _channels = pin.route(&handler).unwrap();
    assert!(pin.route(&handler).is_err());
    let inbox = pin.0.worker.inbox.clone();
    let message = || Decoded { message: Message::Done { op,
        outcome: framer_worker_wire::message::DoneOutcome::Completed, diag_dropped: 0 }, wire_bytes: 50 };
    inbox.dispatch(message()).unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    drop(handler);
    assert!(inbox.dispatch(message()).is_err());
    pin.confirm(); drop(pin);
    assert!(inbox.state.lock().unwrap().operations.is_empty());
    manager.shutdown(false).await.unwrap();
}

#[tokio::test]
async fn hello_must_be_first_and_declare_the_exact_route() {
    let (inbox, hello) = Inbox::new(1);
    assert!(inbox.dispatch(Decoded { message: Message::Ready { generation: 1 }, wire_bytes: 20 }).is_err());
    assert!(hello.await.is_err());
}

#[tokio::test]
async fn hello_does_not_admit_operation_frames_before_ready() {
    let (inbox, hello) = Inbox::new(1);
    inbox.dispatch(Decoded { message: Message::Hello { wire: framer_worker_wire::VERSION,
        build: framer_worker_wire::message::Text::new("fixture".into()).unwrap(),
        routes: vec![framer_worker_wire::message::Route::FreeResponses] }, wire_bytes: 80 }).unwrap();
    hello.await.unwrap();
    assert!(inbox.dispatch(Decoded { message: Message::Done { op: 1,
        outcome: framer_worker_wire::message::DoneOutcome::Completed,
        diag_dropped: 0 }, wire_bytes: 50 }).is_err());
}

#[tokio::test]
async fn retiring_a_starting_slot_cancels_and_joins_its_owned_stage() {
    let manager = manager(2);
    let (id, _exit) = ready(&manager);
    let joined = Arc::new(AtomicBool::new(false));
    {
        let mut state = manager.lock().unwrap();
        let slot = state.slots.get_mut(&id).unwrap();
        slot.phase = GenerationPhase::Starting;
        let mut cancelled = slot.cancel_stage.subscribe();
        let observed = joined.clone();
        slot.starting = Some(async move {
            cancelled.changed().await.unwrap();
            assert!(*cancelled.borrow()); observed.store(true, Ordering::SeqCst);
        }.boxed().shared());
    }
    tokio::time::timeout(BOUND, manager.retire(id)).await.unwrap().unwrap();
    assert!(joined.load(Ordering::SeqCst));
    assert!(manager.snapshot().unwrap().is_empty());
}

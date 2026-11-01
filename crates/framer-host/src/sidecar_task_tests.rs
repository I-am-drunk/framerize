use super::*;
use tokio::sync::oneshot;

#[tokio::test]
async fn cancelled_queued_writer_is_joined_without_sending_a_frame() {
    use tokio::io::AsyncReadExt;
    let tasks = TransportTasks::new();
    let (writer, mut reader) = tokio::io::duplex(4);
    let writer = Arc::new(Mutex::new(writer));
    let locked = writer.lock().await;
    let mut write = Box::pin(super::super::write_registered_command(
        writer.clone(), serde_json::json!({"cmd": "fixture"}), Some(&tasks)));
    assert!(futures::poll!(write.as_mut()).is_pending());
    drop(write);
    drop(locked);
    drop(writer);
    tasks.seal().await;
    assert_eq!(tasks.join().await.unwrap(), 1);
    let mut received = Vec::new();
    reader.read_to_end(&mut received).await.unwrap();
    assert!(received.is_empty());
}

#[tokio::test]
async fn task_capacity_is_bounded_without_dropping_owned_work() {
    let tasks = TransportTasks::new();
    let (release, waiting) = watch::channel(false);
    for _ in 0..TASK_LIMIT {
        let mut waiting = waiting.clone();
        tasks.spawn(async move {
            while !*waiting.borrow_and_update() {
                waiting.changed().await.unwrap();
            }
        }).await.unwrap();
    }
    assert!(tasks.spawn(async { panic!("rejected task executed"); }).await.is_err());
    assert_eq!(tasks.state.lock().await.tasks.len(), TASK_LIMIT);
    release.send_replace(true);
    tasks.seal().await;
    assert_eq!(tasks.join().await.unwrap(), TASK_LIMIT as u64);
}

#[tokio::test]
async fn cancelling_join_keeps_task_until_its_actual_completion() {
    let tasks = TransportTasks::new();
    let (release, blocked) = oneshot::channel();
    tasks.spawn(async move { let _ = blocked.await; }).await.unwrap();
    tasks.seal().await;
    let mut join = Box::pin(tasks.join());
    assert!(futures::poll!(join.as_mut()).is_pending());
    drop(join);
    assert_eq!(tasks.state.lock().await.tasks.len(), 1);
    release.send(()).unwrap();
    assert_eq!(tasks.join().await.unwrap(), 1);
    assert_eq!(tasks.state.lock().await.tasks.len(), 0);
    assert!(tasks.spawn(async {}).await.is_err());
}

#[tokio::test]
async fn sealing_rejects_late_child_registration_while_join_holds_lock() {
    let tasks = TransportTasks::new();
    let owner = Arc::downgrade(&tasks);
    let (release, blocked) = oneshot::channel();
    tasks.spawn(async move {
        blocked.await.unwrap();
        assert!(owner.upgrade().unwrap().spawn(async {}).await.is_err());
    }).await.unwrap();
    tasks.seal().await;
    let mut join = Box::pin(tasks.join());
    assert!(futures::poll!(join.as_mut()).is_pending());
    release.send(()).unwrap();
    assert_eq!(join.await.unwrap(), 1);
}

#[tokio::test]
async fn dropped_writer_waiter_keeps_the_started_frame_owned() {
    use tokio::io::AsyncReadExt;
    let tasks = TransportTasks::new();
    let (writer, mut reader) = tokio::io::duplex(4);
    let fields = serde_json::json!({"cmd": "fixture", "data": "bounded payload"});
    let expected = format!("{fields}\n").into_bytes();
    let mut write = Box::pin(super::super::write_registered_command(
        Arc::new(Mutex::new(writer)), fields, Some(&tasks)));
    assert!(futures::poll!(write.as_mut()).is_pending());
    let mut first = [0u8; 1];
    reader.read_exact(&mut first).await.unwrap();
    drop(write);
    tasks.seal().await;
    let mut join = Box::pin(tasks.join());
    assert!(futures::poll!(join.as_mut()).is_pending());
    let mut received = first.to_vec();
    reader.read_to_end(&mut received).await.unwrap();
    assert_eq!(join.await.unwrap(), 1);
    assert_eq!(received, expected);
}

#[tokio::test]
async fn panic_is_sticky_but_other_owned_tasks_still_join() {
    let tasks = TransportTasks::new();
    let (release, blocked) = oneshot::channel();
    tasks.spawn(async move { let _ = blocked.await; }).await.unwrap();
    tasks.spawn(async { panic!("fixture task panic"); }).await.unwrap();
    tasks.seal().await;
    let mut join = Box::pin(tasks.join());
    assert!(futures::poll!(join.as_mut()).is_pending());
    release.send(()).unwrap();
    let cause = join.await.unwrap_err();
    assert!(cause.contains("browser transport task panicked"), "{cause}");
    assert_eq!(tasks.state.lock().await.tasks.len(), 0);
    assert_eq!(tasks.join().await.unwrap_err(), cause);
    assert!(tasks.spawn(async {}).await.is_err());
}

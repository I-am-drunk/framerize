use super::*;

#[tokio::test]
async fn resident_reservation_rejects_competing_admission_without_poisoning() {
    let (state, ports) = owned();
    drop(acquire(&state).await.unwrap());
    let control = crate::admission::admit_existing(&state, identity(), "gpt-4.1",
        Instant::now() + Duration::from_secs(3)).await.unwrap();
    let before = slot(&state, id(1));
    ports.events.lock().unwrap().clear();
    *ports.status_mode.lock().unwrap() = StatusMode::Error;
    let result = lease_account(&state, id(1)).await;
    assert!(matches!(result, Err(ApiError::PoolBusy(_))));
    let after = slot(&state, id(1));
    assert!(!after.uncertain, "known reservation must not create uncertainty");
    assert_eq!(after.users, before.users);
    assert_eq!(after.status, before.status);
    assert!(ports.events.lock().unwrap().is_empty(), "refuse before status RPC");
    *ports.status_mode.lock().unwrap() = StatusMode::Ready;
    control.completion.revalidate(&state, &control.identity, &control.proxy)
        .await.unwrap();
    let other = lease_account(&state, id(2)).await.unwrap();
    assert_eq!(other.account(), id(2));
    drop(other);
    drop(control);
    let resumed = lease_account(&state, id(1)).await.unwrap();
    assert!(!slot(&state, id(1)).uncertain);
    drop(resumed);
}

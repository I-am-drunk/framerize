use super::*;
#[path = "fixture.rs"]
mod fixture;
use fixture::*;
use tokio::time::timeout;

#[tokio::test]
async fn material_waits_for_original_cleanup_before_publication() {
    let (client, scope) = account();
    let (source, probe) = source(&scope, &scope, Inner::Success, Cleanup::Closed, Opening::Started);
    let pending = prepare(&client, &scope, source, probe.clone());
    gate(&probe.source_entered).await;
    assert!(pending.completion().now_or_never().is_none());
    probe.source_release.add_permits(1);
    gate(&probe.cleanup_entered).await;
    assert_eq!(count(&probe.seals), 1);
    assert!(probe.results.lock().unwrap().is_empty());
    assert!(pending.completion().now_or_never().is_none());
    probe.cleanup_release.add_permits(1);
    assert_eq!(settled(pending).await, Ok("fixture-bearer".into()));
    assert_eq!(count(&probe.stops), 0);
    intact(&probe);
    close(&client, &scope).await;
}

#[tokio::test]
async fn different_account_is_rejected_before_factory_open() {
    let (client, scope) = account();
    let (other_client, other) = account();
    let (source, probe) = source(&scope, &scope, Inner::Success, Cleanup::Closed, Opening::Started);
    let pending = prepare_as(&client, &scope, other.clone(), source, probe.clone(), None);
    assert_eq!(settled(pending).await, Err(PreparationFailure::Rejected));
    assert_eq!(count(&probe.opens), 0);
    assert_eq!(count(&probe.calls), 0);
    close(&client, &scope).await;
    close(&other_client, &other).await;
}

#[tokio::test]
async fn mismatched_session_user_is_rejected_before_factory_open() {
    let (client, scope) = account();
    let (source, probe) = source(&scope, &scope, Inner::Success, Cleanup::Closed, Opening::Started);
    let mut wrong = session();
    wrong.active_user.0 = "different-fixture-user".into();
    let pending = prepare_as(&client, &scope, scope.clone(), source, probe.clone(), Some(wrong));
    assert_eq!(settled(pending).await, Err(PreparationFailure::Rejected));
    assert_eq!(count(&probe.opens), 0);
    close(&client, &scope).await;
}

#[tokio::test]
async fn same_account_replacement_is_rejected_before_factory_open() {
    let (client, old) = account();
    let (source, probe) = source(&old, &old, Inner::Success, Cleanup::Closed, Opening::Started);
    close(&client, &old).await;
    let next = reopen(&client, &old);
    assert_eq!(old.account(), next.account());
    assert_ne!(old.generation(), next.generation());
    let pending = prepare(&client, &next, source, probe.clone());
    assert_eq!(settled(pending).await, Err(PreparationFailure::Rejected));
    assert_eq!(count(&probe.opens), 0);
    close(&client, &next).await;
}

#[tokio::test]
async fn concurrent_duplicate_use_does_not_open_or_interrupt_first_source() {
    let (client, scope) = account();
    let (source, probe) = source(&scope, &scope, Inner::Success, Cleanup::Closed, Opening::Started);
    let first = prepare(&client, &scope, source.clone(), probe.clone());
    gate(&probe.source_entered).await;
    let duplicate = prepare(&client, &scope, source, probe.clone());
    assert_eq!(settled(duplicate).await, Err(PreparationFailure::Rejected));
    assert_eq!(count(&probe.opens), 1);
    assert_eq!(count(&probe.calls), 1);
    assert!(first.completion().now_or_never().is_none());
    probe.release_all();
    assert!(settled(first).await.is_ok());
    intact(&probe);
    close(&client, &scope).await;
}

#[tokio::test]
async fn dropped_waiter_retains_issued_work_and_account_barrier() {
    let (client, scope) = account();
    let (source, probe) = source(&scope, &scope, Inner::Success, Cleanup::Closed, Opening::Started);
    let pending = prepare(&client, &scope, source, probe.clone());
    let completion = pending.completion();
    gate(&probe.source_entered).await;
    drop(pending);
    gate(&probe.stopped).await;
    let closed = client.close_account(&scope).unwrap();
    assert!(completion.clone().now_or_never().is_none());
    assert!(closed.clone().now_or_never().is_none());
    fenced(&client, &scope);
    intact(&probe);
    probe.source_release.add_permits(1);
    gate(&probe.cleanup_entered).await;
    assert!(completion.clone().now_or_never().is_none());
    assert!(closed.clone().now_or_never().is_none());
    assert!(probe.results.lock().unwrap().is_empty());
    probe.cleanup_release.add_permits(1);
    timeout(BOUND, completion).await.unwrap().unwrap();
    timeout(BOUND, closed).await.unwrap().unwrap();
    assert_eq!(probe.results.lock().unwrap()[0], Snapshot {
        value: Err(PreparationFailure::Cancelled), confirmed: true });
    intact(&probe);
}

#[tokio::test]
async fn cancellation_during_source_awaits_original_work_and_cleanup() {
    let (client, scope) = account();
    let (source, probe) = source(&scope, &scope, Inner::Success, Cleanup::Closed, Opening::Started);
    let pending = prepare(&client, &scope, source, probe.clone());
    gate(&probe.source_entered).await;
    pending.cancellation().cancel();
    gate(&probe.stopped).await;
    assert!(pending.completion().now_or_never().is_none());
    intact(&probe);
    probe.source_release.add_permits(1);
    gate(&probe.cleanup_entered).await;
    assert!(pending.completion().now_or_never().is_none());
    probe.cleanup_release.add_permits(1);
    assert_eq!(settled(pending).await, Err(PreparationFailure::Cancelled));
    assert_eq!(probe.results.lock().unwrap()[0], Snapshot {
        value: Err(PreparationFailure::Cancelled), confirmed: true });
    intact(&probe);
    close(&client, &scope).await;
}

#[tokio::test]
async fn cancellation_during_cleanup_suppresses_material_without_dropping_observer() {
    let (client, scope) = account();
    let (source, probe) = source(&scope, &scope, Inner::Success, Cleanup::Closed, Opening::Started);
    probe.source_release.add_permits(1);
    let pending = prepare(&client, &scope, source, probe.clone());
    gate(&probe.cleanup_entered).await;
    pending.cancellation().cancel();
    gate(&probe.stopped).await;
    assert!(pending.completion().now_or_never().is_none());
    intact(&probe);
    probe.cleanup_release.add_permits(1);
    assert_eq!(settled(pending).await, Err(PreparationFailure::Cancelled));
    assert_eq!(probe.results.lock().unwrap()[0].value, Err(PreparationFailure::Cancelled));
    intact(&probe);
    close(&client, &scope).await;
}

#[tokio::test]
async fn factory_wrong_account_is_stopped_and_joined_without_issuing() {
    let (client, scope) = account();
    let (other_client, other) = account();
    let (source, probe) = source(&scope, &other, Inner::Success, Cleanup::Closed, Opening::Started);
    let pending = prepare(&client, &scope, source, probe.clone());
    gate(&probe.cleanup_entered).await;
    assert_eq!(count(&probe.calls), 0);
    assert!(count(&probe.stops) >= 1);
    assert_eq!(count(&probe.seals), 1);
    assert!(pending.completion().now_or_never().is_none());
    probe.cleanup_release.add_permits(1);
    assert_eq!(settled(pending).await, Err(PreparationFailure::CleanupUnconfirmed));
    assert!(!probe.results.lock().unwrap()[0].confirmed);
    fenced(&client, &scope);
    intact(&probe);
    close(&other_client, &other).await;
}

#[tokio::test]
async fn factory_old_generation_cannot_settle_same_account_replacement() {
    let (client, old) = account();
    close(&client, &old).await;
    let next = reopen(&client, &old);
    let (source, probe) = source(&next, &old, Inner::Success, Cleanup::Closed, Opening::Started);
    let pending = prepare(&client, &next, source, probe.clone());
    gate(&probe.cleanup_entered).await;
    assert_eq!(count(&probe.calls), 0);
    assert!(count(&probe.stops) >= 1);
    assert!(pending.completion().now_or_never().is_none());
    probe.cleanup_release.add_permits(1);
    assert_eq!(settled(pending).await, Err(PreparationFailure::CleanupUnconfirmed));
    assert!(!probe.results.lock().unwrap()[0].confirmed);
    fenced(&client, &next);
    intact(&probe);
}

#[tokio::test]
async fn cleanup_receipt_wrong_account_keeps_native_fence() {
    let (client, scope) = account();
    let mut identity = SourceIdentity::from_scope(&scope);
    identity.account = AccountId(uuid::Uuid::new_v4());
    let (source, probe) = source(&scope, &scope, Inner::Success,
        Cleanup::Identity(identity), Opening::Started);
    probe.release_all();
    let pending = prepare(&client, &scope, source, probe.clone());
    assert_eq!(settled(pending).await, Err(PreparationFailure::CleanupUnconfirmed));
    assert_eq!(probe.results.lock().unwrap()[0], Snapshot {
        value: Err(PreparationFailure::CleanupUnconfirmed), confirmed: false });
    fenced(&client, &scope);
    intact(&probe);
}

#[tokio::test]
async fn cleanup_receipt_wrong_generation_keeps_native_fence() {
    let (client, scope) = account();
    let mut identity = SourceIdentity::from_scope(&scope);
    identity.generation += 1;
    let (source, probe) = source(&scope, &scope, Inner::Success,
        Cleanup::Identity(identity), Opening::Started);
    probe.release_all();
    let pending = prepare(&client, &scope, source, probe.clone());
    assert_eq!(settled(pending).await, Err(PreparationFailure::CleanupUnconfirmed));
    assert!(!probe.results.lock().unwrap()[0].confirmed);
    fenced(&client, &scope);
    intact(&probe);
}

#[tokio::test]
async fn cleanup_failure_does_not_publish_successful_material() {
    let (client, scope) = account();
    let (source, probe) = source(&scope, &scope, Inner::Success, Cleanup::Unconfirmed, Opening::Started);
    probe.release_all();
    let pending = prepare(&client, &scope, source, probe.clone());
    let completion = pending.completion();
    assert_eq!(settled(pending).await, Err(PreparationFailure::CleanupUnconfirmed));
    assert!(timeout(BOUND, completion).await.unwrap().is_err());
    assert!(!probe.results.lock().unwrap()[0].confirmed);
    fenced(&client, &scope);
    intact(&probe);
}

#[tokio::test]
async fn lost_cleanup_worker_stops_source_and_keeps_native_fence() {
    let (client, scope) = account();
    let (source, probe) = source(&scope, &scope, Inner::Success, Cleanup::Lost, Opening::Started);
    probe.release_all();
    let pending = prepare(&client, &scope, source, probe.clone());
    let completion = pending.completion();
    assert_eq!(settled(pending).await, Err(PreparationFailure::CleanupUnconfirmed));
    assert!(timeout(BOUND, completion).await.unwrap().is_err());
    assert!(count(&probe.stops) >= 1);
    assert!(probe.cleanup_abandoned.load(Ordering::SeqCst));
    assert!(!probe.source_abandoned.load(Ordering::SeqCst));
    assert!(probe.results.lock().unwrap().is_empty());
    fenced(&client, &scope);
}

#[tokio::test]
async fn closed_cleanup_never_upgrades_inner_uncertainty() {
    let (client, scope) = account();
    let (source, probe) = source(&scope, &scope, Inner::Unconfirmed, Cleanup::Closed, Opening::Started);
    probe.release_all();
    let pending = prepare(&client, &scope, source, probe.clone());
    assert_eq!(settled(pending).await, Err(PreparationFailure::CleanupUnconfirmed));
    assert_eq!(probe.results.lock().unwrap()[0], Snapshot {
        value: Err(PreparationFailure::CleanupUnconfirmed), confirmed: false });
    fenced(&client, &scope);
    intact(&probe);
}

#[tokio::test]
async fn not_started_cleanup_contradicts_successful_material() {
    let (client, scope) = account();
    let (source, probe) = source(&scope, &scope, Inner::Success, Cleanup::NotStarted, Opening::Started);
    probe.release_all();
    let pending = prepare(&client, &scope, source, probe.clone());
    assert_eq!(settled(pending).await, Err(PreparationFailure::CleanupUnconfirmed));
    assert!(!probe.results.lock().unwrap()[0].confirmed);
    fenced(&client, &scope);
    intact(&probe);
}

#[tokio::test]
async fn not_started_cleanup_preserves_confirmed_rejection() {
    let (client, scope) = account();
    let (source, probe) = source(&scope, &scope, Inner::Rejected, Cleanup::NotStarted, Opening::Started);
    probe.release_all();
    let pending = prepare(&client, &scope, source, probe.clone());
    assert_eq!(settled(pending).await, Err(PreparationFailure::Rejected));
    assert_eq!(probe.results.lock().unwrap()[0], Snapshot {
        value: Err(PreparationFailure::Rejected), confirmed: true });
    intact(&probe);
    close(&client, &scope).await;
}

#[tokio::test]
async fn factory_not_started_is_unavailable_with_confirmed_cleanup() {
    let (client, scope) = account();
    let (source, probe) = source(&scope, &scope, Inner::Success, Cleanup::Closed, Opening::NotStarted);
    let pending = prepare(&client, &scope, source, probe.clone());
    assert_eq!(settled(pending).await, Err(PreparationFailure::Unavailable));
    assert_eq!(probe.results.lock().unwrap()[0], Snapshot {
        value: Err(PreparationFailure::Unavailable), confirmed: true });
    assert_eq!(count(&probe.opens), 1);
    assert_eq!(count(&probe.calls), 0);
    assert_eq!(count(&probe.seals), 0);
    close(&client, &scope).await;
}

#[tokio::test]
async fn factory_unconfirmed_poison_is_distinct_from_not_started() {
    let (client, scope) = account();
    let (source, probe) = source(&scope, &scope, Inner::Success, Cleanup::Closed, Opening::Unconfirmed);
    let pending = prepare(&client, &scope, source, probe.clone());
    assert_eq!(settled(pending).await, Err(PreparationFailure::CleanupUnconfirmed));
    assert_eq!(probe.results.lock().unwrap()[0], Snapshot {
        value: Err(PreparationFailure::CleanupUnconfirmed), confirmed: false });
    assert_eq!(count(&probe.opens), 1);
    assert_eq!(count(&probe.calls), 0);
    fenced(&client, &scope);
}

#[tokio::test]
async fn cancelled_before_admission_does_not_consume_single_use_source() {
    let (client, scope) = account();
    let (source, probe) = source(&scope, &scope, Inner::Success, Cleanup::Closed, Opening::Started);
    let attempted = source.clone();
    let requested = scope.clone();
    let pending = client.prepare_owned(&scope,
        tokio::time::Instant::now() + std::time::Duration::from_secs(30),
        move |session, control| async move {
            control.cancel();
            let result = attempted.issue(&requested, &session, &control).await;
            assert!(result.cleanup_confirmed);
            assert!(matches!(result.value, Err(PreparationFailure::Cancelled)));
            PreparationResult::<String>::finished(Err(PreparationFailure::Cancelled))
        }).unwrap();
    assert_eq!(settled(pending).await, Err(PreparationFailure::Cancelled));
    assert_eq!(count(&probe.opens), 0);
    probe.release_all();
    let retried = prepare(&client, &scope, source, probe.clone());
    assert_eq!(settled(retried).await, Ok("fixture-bearer".into()));
    assert_eq!(count(&probe.opens), 1);
    close(&client, &scope).await;
}

#[tokio::test]
async fn cleanup_guard_marks_unpolled_observer_drop_only_after_drop() {
    let (client, scope) = account();
    let (source, probe) = source(&scope, &scope, Inner::Success, Cleanup::Closed, Opening::Started);
    let opened = source.factory.open(&scope).ok().expect("fixture open failed");
    assert_eq!(count(&probe.opens), 1);
    assert_eq!(probe.cleanup_entered.available_permits(), 0);
    intact(&probe);
    drop(opened.cleanup);
    assert!(probe.cleanup_abandoned.load(Ordering::SeqCst));
    assert!(!probe.source_abandoned.load(Ordering::SeqCst));
    assert_eq!(probe.cleanup_entered.available_permits(), 0);
    assert_eq!(count(&probe.calls), 0);
    close(&client, &scope).await;
}

#[tokio::test]
async fn cleanup_guard_marks_pending_observer_drop_only_after_drop() {
    let (client, scope) = account();
    let (source, probe) = source(&scope, &scope, Inner::Success, Cleanup::Closed, Opening::Started);
    let mut opened = source.factory.open(&scope).ok().expect("fixture open failed");
    assert!(opened.cleanup.as_mut().now_or_never().is_none());
    assert_eq!(probe.cleanup_entered.available_permits(), 1);
    intact(&probe);
    drop(opened.cleanup);
    assert!(probe.cleanup_abandoned.load(Ordering::SeqCst));
    assert!(!probe.source_abandoned.load(Ordering::SeqCst));
    assert_eq!(count(&probe.calls), 0);
    close(&client, &scope).await;
}

#[tokio::test]
async fn cleanup_guard_finishes_without_abandonment_after_observer_completion() {
    let (client, scope) = account();
    let (source, probe) = source(&scope, &scope, Inner::Success, Cleanup::Closed, Opening::Started);
    let mut opened = source.factory.open(&scope).ok().expect("fixture open failed");
    assert!(opened.cleanup.as_mut().now_or_never().is_none());
    intact(&probe);
    probe.cleanup_release.add_permits(1);
    let result = timeout(BOUND, opened.cleanup).await.expect("fixture cleanup timed out");
    assert!(matches!(result, SourceCleanup::Closed(value)
        if value == SourceIdentity::from_scope(&scope)));
    assert_eq!(probe.cleanup_entered.available_permits(), 1);
    assert_eq!(count(&probe.calls), 0);
    intact(&probe);
    close(&client, &scope).await;
}

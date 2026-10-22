use super::*;
#[path = "fixture.rs"]
mod fixture;
use fixture::*;
use framer_core::ids::AccountId;
use tokio::time::timeout;

#[tokio::test]
async fn wrong_account_rejection_preserves_empty_host_registry() {
    let fixture = Fixture::new(1);
    let other = fixture.open_native(AccountId(uuid::Uuid::new_v4()));
    not_started(fixture.factory.open(&other));
    let admitted = opened(fixture.factory.open(&fixture.scope));
    assert!(admitted.scope.same_generation(&fixture.scope));
    settle(admitted).await;
    fixture.finish().await;
}

#[tokio::test]
async fn same_account_replacement_requires_its_exact_native_generation() {
    let fixture = Fixture::new(1);
    fixture.close(&fixture.scope).await;
    let replacement = fixture.open_native(fixture.scope.account());
    assert_eq!(replacement.account(), fixture.scope.account());
    assert_ne!(replacement.generation(), fixture.scope.generation());
    not_started(fixture.factory.open(&replacement));
    let exact = fixture.for_scope(&replacement);
    settle(opened(exact.open(&replacement))).await;
    drop(exact);
    fixture.finish().await;
}

#[tokio::test]
async fn no_runtime_rejection_leaves_account_and_capacity_available() {
    let fixture = Fixture::new(1);
    let factory = fixture.for_scope(&fixture.scope);
    let scope = fixture.scope.clone();
    let (sent, received) = tokio::sync::oneshot::channel();
    let worker = std::thread::spawn(move || {
        assert!(tokio::runtime::Handle::try_current().is_err());
        assert!(sent.send(factory.open(&scope)).is_ok());
    });
    let result = timeout(BOUND, received).await.expect("outside-runtime thread stalled")
        .expect("outside-runtime result lost");
    worker.join().expect("outside-runtime thread panicked");
    not_started(result);
    settle(opened(fixture.factory.open(&fixture.scope))).await;
    fixture.finish().await;
}

#[tokio::test]
async fn open_and_seal_without_command_yields_not_started_and_releases_scope() {
    let fixture = Fixture::new(1);
    let first = opened(fixture.factory.open(&fixture.scope));
    assert!(first.scope.same_generation(&fixture.scope));
    settle(first).await;
    fixture.assert_no_launch();
    settle(opened(fixture.factory.open(&fixture.scope))).await;
    fixture.finish().await;
}

#[tokio::test]
async fn busy_same_account_rejection_does_not_replace_admitted_owner() {
    // A spare slot distinguishes AccountBusy from global capacity rejection.
    let fixture = Fixture::new(2);
    let first = opened(fixture.factory.open(&fixture.scope));
    let duplicate = fixture.for_scope(&fixture.scope);
    not_started(duplicate.open(&fixture.scope));
    assert!(first.scope.same_generation(&fixture.scope));
    settle(first).await;
    settle(opened(duplicate.open(&fixture.scope))).await;
    drop(duplicate);
    fixture.finish().await;
}

#[tokio::test]
async fn capacity_rejection_preserves_existing_owner_and_recovers_after_join() {
    let fixture = Fixture::new(1);
    let other = fixture.open_native(AccountId(uuid::Uuid::new_v4()));
    let second_factory = fixture.for_scope(&other);
    let first = opened(fixture.factory.open(&fixture.scope));
    not_started(second_factory.open(&other));
    settle(first).await;
    let second = opened(second_factory.open(&other));
    assert!(second.scope.same_generation(&other));
    settle(second).await;
    drop(second_factory);
    fixture.finish().await;
}

#[tokio::test]
async fn repeated_no_command_stop_and_seal_do_not_poison_account() {
    let fixture = Fixture::new(1);
    let first = opened(fixture.factory.open(&fixture.scope));
    for _ in 0..3 {
        first.control.request_stop();
        first.control.seal();
    }
    settle(first).await;
    settle(opened(fixture.factory.open(&fixture.scope))).await;
    fixture.finish().await;
}

#[tokio::test]
async fn egress_mismatch_is_rejected_before_host_scope_admission() {
    let fixture = Fixture::new(1);
    let mut mismatched = fixture.for_scope(&fixture.scope);
    mismatched.proxy = Some("http://127.0.0.1:9".into());
    not_started(mismatched.open(&fixture.scope));
    settle(opened(fixture.factory.open(&fixture.scope))).await;
    drop(mismatched);
    fixture.finish().await;
}

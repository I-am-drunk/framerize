//! Deterministic ownership fixtures. No runtime, thread, network or live state.
use super::*;
use std::{pin::Pin, task::{Context, Poll}};

fn limits() -> LeaseLimits {
    LeaseLimits { accounts: 8, users: 8, waiters: 16,
        user_id_bytes: 64, total_user_bytes: 256 }
}
fn coordinator() -> AccountLeaseCoordinator { AccountLeaseCoordinator::new(limits()).unwrap() }
fn poll<F: Future>(f: Pin<&mut F>) -> Poll<F::Output> {
    f.poll(&mut Context::from_waker(futures::task::noop_waker_ref()))
}
fn ready<F: Future>(f: F) -> F::Output {
    match poll(Box::pin(f).as_mut()) {
        Poll::Ready(value) => value,
        Poll::Pending => panic!("ownership unexpectedly blocked"),
    }
}
fn pending<F: Future>(f: &mut Pin<Box<F>>) { assert!(poll(f.as_mut()).is_pending()); }
fn bind(c: &AccountLeaseCoordinator, account: AccountId, user: &str) {
    let mut change = ready(c.begin_change(account).unwrap()).unwrap();
    ready(change.fence_user(user).unwrap()).unwrap();
    change.stage_binding(account, Some(user)).unwrap();
    change.finish(ChangeResult::Completed).unwrap();
}
fn operation(c: &AccountLeaseCoordinator, account: AccountId, user: &str) -> OperationLease {
    ready(c.operation(account, user).unwrap()).unwrap()
}

#[test]
fn lease_transports_share_one_owner_while_other_accounts_progress() {
    let c = coordinator(); let legacy = c.clone(); let native = c.clone();
    let a = AccountId::new(); let b = AccountId::new();
    bind(&c, a, "same-user"); bind(&c, b, "same-user");
    let held = operation(&legacy, a, "same-user");
    let mut next = Box::pin(native.operation(a, "same-user").unwrap());
    pending(&mut next);
    operation(&native, b, "same-user").complete().unwrap();
    assert_eq!(c.stats().unwrap().operations, 1);
    held.complete().unwrap();
    ready(next).unwrap().complete().unwrap();
    assert_eq!(c.stats().unwrap().operations, 0);
    assert_eq!(c.stats().unwrap().waiters, 0);
}

#[test]
fn lease_unpolled_and_cancelled_waiters_release_their_bounded_slots() {
    let mut l = limits(); l.waiters = 2;
    let c = AccountLeaseCoordinator::new(l).unwrap(); let a = AccountId::new();
    bind(&c, a, "user");
    let held = operation(&c, a, "user");
    let unpolled = c.operation(a, "user").unwrap();
    let mut polled = Box::pin(c.operation(a, "user").unwrap());
    pending(&mut polled);
    assert_eq!(c.operation(a, "user").err(), Some(LeaseError::QueueFull));
    drop(unpolled); drop(polled);
    assert_eq!(c.stats().unwrap().waiters, 0);
    held.complete().unwrap();
    operation(&c, a, "user").complete().unwrap();
}

#[test]
fn lease_cancelling_primary_drain_releases_only_unissued_exclusion() {
    let c = coordinator(); let a = AccountId::new(); bind(&c, a, "user");
    let held = operation(&c, a, "user");
    let mut change = Box::pin(c.begin_change(a).unwrap()); pending(&mut change);
    let mut next = Box::pin(c.operation(a, "user").unwrap()); pending(&mut next);
    assert!(c.stats().unwrap().lifecycle_active);
    drop(change);
    assert!(!c.stats().unwrap().lifecycle_active);
    pending(&mut next); // The still-running transport owner was not released.
    held.complete().unwrap();
    ready(next).unwrap().complete().unwrap();
    assert_eq!(c.stats().unwrap().poisoned_accounts, 0);
}

#[test]
fn lease_unknown_login_fences_primary_then_every_discovered_user_account() {
    let c = coordinator(); let a = AccountId::new(); let b = AccountId::new();
    let d = AccountId::new(); let other = AccountId::new();
    bind(&c, b, "target"); bind(&c, d, "target"); bind(&c, other, "other");
    let b_run = operation(&c, b, "target"); let d_run = operation(&c, d, "target");
    let mut change = ready(c.begin_change(a).unwrap()).unwrap();
    assert_eq!(change.account(), a);
    let mut fence = Box::pin(change.fence_user("target").unwrap()); pending(&mut fence);
    let mut waiting = Box::pin(c.operation(b, "target").unwrap()); pending(&mut waiting);
    operation(&c, other, "other").complete().unwrap();
    b_run.complete().unwrap(); pending(&mut fence); pending(&mut waiting);
    d_run.complete().unwrap(); ready(fence).unwrap();
    let affected = change.fenced_accounts().unwrap();
    assert_eq!(affected.len(), 3);
    assert!(affected.contains(&a) && affected.contains(&b) && affected.contains(&d));
    change.stage_binding(a, Some("target")).unwrap();
    change.finish(ChangeResult::Completed).unwrap();
    ready(waiting).unwrap().complete().unwrap();
    operation(&c, a, "target").complete().unwrap();
}

#[test]
fn lease_cross_login_queue_holds_no_account_and_cannot_deadlock_expansion() {
    let c = coordinator(); let a = AccountId::new(); let b = AccountId::new();
    bind(&c, a, "alice"); bind(&c, b, "bob");
    let mut first = ready(c.begin_change(a).unwrap()).unwrap();
    let mut second = Box::pin(c.begin_change(b).unwrap()); pending(&mut second);
    // Queued B does not own B, so A can fence Bob's accounts and commit.
    ready(first.fence_user("bob").unwrap()).unwrap();
    first.stage_binding(a, Some("bob")).unwrap();
    first.finish(ChangeResult::Completed).unwrap();
    let mut second = ready(second).unwrap();
    ready(second.fence_user("alice").unwrap()).unwrap();
    second.stage_binding(b, Some("alice")).unwrap();
    second.finish(ChangeResult::Completed).unwrap();
    operation(&c, a, "bob").complete().unwrap();
    operation(&c, b, "alice").complete().unwrap();
}

#[test]
fn lease_operation_wait_rechecks_version_after_atomic_rebind() {
    let c = coordinator(); let a = AccountId::new(); bind(&c, a, "old");
    let held = operation(&c, a, "old");
    let mut stale = Box::pin(c.operation(a, "old").unwrap()); pending(&mut stale);
    let mut changing = Box::pin(c.begin_change(a).unwrap()); pending(&mut changing);
    held.complete().unwrap();
    let mut change = ready(changing).unwrap();
    ready(change.fence_user("new").unwrap()).unwrap();
    change.stage_binding(a, Some("new")).unwrap();
    assert_eq!(c.operation(a, "new").err(), Some(LeaseError::BindingMismatch));
    pending(&mut stale);
    change.finish(ChangeResult::Completed).unwrap();
    assert_eq!(ready(stale).err(), Some(LeaseError::BindingMismatch));
    assert_eq!(c.operation(a, "old").err(), Some(LeaseError::BindingMismatch));
    operation(&c, a, "new").complete().unwrap();
}

#[test]
fn lease_queued_lifecycle_rechecks_replaced_account_binding() {
    let c = coordinator(); let a = AccountId::new(); bind(&c, a, "old");
    let mut first = ready(c.begin_change(a).unwrap()).unwrap();
    let mut stale = Box::pin(c.begin_change(a).unwrap()); pending(&mut stale);
    ready(first.fence_user("new").unwrap()).unwrap();
    first.stage_binding(a, Some("new")).unwrap();
    first.finish(ChangeResult::Completed).unwrap();
    assert_eq!(ready(stale).err(), Some(LeaseError::BindingMismatch));
    assert_eq!(c.stats().unwrap().waiters, 0);
    ready(c.begin_change(a).unwrap()).unwrap().finish(ChangeResult::Completed).unwrap();
}

#[test]
fn lease_binding_changes_require_owned_and_drained_user_fences() {
    let c = coordinator(); let a = AccountId::new(); let b = AccountId::new();
    let outside = AccountId::new(); bind(&c, a, "old"); bind(&c, b, "target");
    bind(&c, outside, "outside");
    let b_run = operation(&c, b, "target");
    let mut change = ready(c.begin_change(a).unwrap()).unwrap();
    assert_eq!(change.stage_binding(a, Some("target")), Err(LeaseError::NotOwned));
    let mut pending_fence = Box::pin(change.fence_user("target").unwrap());
    pending(&mut pending_fence); drop(pending_fence);
    assert_eq!(change.stage_binding(a, Some("target")), Err(LeaseError::NotDrained));
    assert_eq!(change.fenced_accounts(), Err(LeaseError::NotDrained));
    b_run.complete().unwrap();
    ready(change.fence_user("target").unwrap()).unwrap();
    assert_eq!(change.stage_binding(outside, None), Err(LeaseError::NotOwned));
    change.stage_binding(a, Some("target")).unwrap();
    change.finish(ChangeResult::Completed).unwrap();
    operation(&c, outside, "outside").complete().unwrap();
}

#[test]
fn lease_abandoned_operation_poisons_exact_account_not_its_peers() {
    let c = coordinator(); let a = AccountId::new(); let b = AccountId::new();
    bind(&c, a, "user"); bind(&c, b, "user");
    drop(operation(&c, a, "user"));
    assert_eq!(c.operation(a, "user").err(), Some(LeaseError::Poisoned));
    assert_eq!(c.begin_change(a).err(), Some(LeaseError::Poisoned));
    operation(&c, b, "user").complete().unwrap();
    assert_eq!(c.stats().unwrap().poisoned_accounts, 1);
    assert_eq!(c.stats().unwrap().poisoned_users, 0);
}

#[test]
fn lease_unconfirmed_login_retains_account_and_unbound_target_user_poison() {
    let c = coordinator(); let a = AccountId::new(); let b = AccountId::new();
    let mut change = ready(c.begin_change(a).unwrap()).unwrap();
    ready(change.fence_user("target").unwrap()).unwrap();
    drop(change);
    assert_eq!(c.begin_change(a).err(), Some(LeaseError::Poisoned));
    assert_eq!(c.stats().unwrap().poisoned_accounts, 1);
    assert_eq!(c.stats().unwrap().poisoned_users, 1);
    let mut other = ready(c.begin_change(b).unwrap()).unwrap();
    assert_eq!(other.fence_user("target").err(), Some(LeaseError::Poisoned));
    assert_eq!(other.stage_binding(b, Some("target")), Err(LeaseError::NotOwned));
    other.finish(ChangeResult::Completed).unwrap();
    assert_eq!(c.stats().unwrap().accounts, 1);
    assert_eq!(c.stats().unwrap().users, 1);
}

#[test]
fn lease_retryable_failure_stays_closed_until_owned_reconciliation() {
    let c = coordinator(); let a = AccountId::new(); let b = AccountId::new();
    bind(&c, a, "old"); bind(&c, b, "target");
    let mut change = ready(c.begin_change(a).unwrap()).unwrap();
    ready(change.fence_user("target").unwrap()).unwrap();
    change.stage_binding(a, Some("target")).unwrap();
    change.finish(ChangeResult::RetryableFailure).unwrap();
    assert_eq!(c.operation(a, "old").err(), Some(LeaseError::Closed));
    assert_eq!(c.operation(b, "target").err(), Some(LeaseError::Closed));
    assert_eq!(c.operation(a, "target").err(), Some(LeaseError::BindingMismatch));
    let mut retry = ready(c.begin_change(a).unwrap()).unwrap();
    ready(retry.fence_user("target").unwrap()).unwrap();
    retry.stage_binding(a, Some("target")).unwrap();
    retry.finish(ChangeResult::Completed).unwrap();
    operation(&c, a, "target").complete().unwrap();
    operation(&c, b, "target").complete().unwrap();
    assert_eq!(c.stats().unwrap().closed_accounts, 0);
    assert_eq!(c.stats().unwrap().closed_users, 0);
}

#[test]
fn lease_dropped_user_expansion_cannot_be_reported_as_completed() {
    for poll_first in [false, true] {
        let c = coordinator(); let a = AccountId::new(); let b = AccountId::new();
        bind(&c, b, "target"); let held = operation(&c, b, "target");
        let mut change = ready(c.begin_change(a).unwrap()).unwrap();
        let mut fence = Box::pin(change.fence_user("target").unwrap());
        if poll_first { pending(&mut fence); }
        drop(fence);
        assert_eq!(change.finish(ChangeResult::Completed), Err(LeaseError::NotDrained));
        held.complete().unwrap();
        assert_eq!(c.operation(b, "target").err(), Some(LeaseError::Poisoned));
        assert_eq!(c.begin_change(a).err(), Some(LeaseError::Poisoned));
        assert_eq!(c.stats().unwrap().poisoned_users, 1);
        assert!(!c.stats().unwrap().lifecycle_active);
    }
}

#[test]
fn lease_cancelled_lifecycle_queue_leaks_no_account_or_waiter_records() {
    let mut l = limits(); l.accounts = 1;
    let c = AccountLeaseCoordinator::new(l).unwrap();
    let first = ready(c.begin_change(AccountId::new()).unwrap()).unwrap();
    let mut second = Box::pin(c.begin_change(AccountId::new()).unwrap());
    let third = c.begin_change(AccountId::new()).unwrap(); pending(&mut second);
    assert_eq!(c.stats().unwrap().accounts, 1);
    drop(second); drop(third);
    assert_eq!(c.stats().unwrap().waiters, 0);
    first.finish(ChangeResult::Completed).unwrap();
    assert_eq!(c.stats().unwrap(), LeaseStats::default());
}

#[test]
fn lease_account_limit_retains_poisoned_accounts_instead_of_evicting_them() {
    let mut l = limits(); l.accounts = 1;
    let c = AccountLeaseCoordinator::new(l).unwrap();
    let a = AccountId::new(); let b = AccountId::new(); bind(&c, a, "a");
    assert_eq!(ready(c.begin_change(b).unwrap()).err(), Some(LeaseError::AccountLimit));
    assert_eq!(c.stats().unwrap().waiters, 0);
    drop(operation(&c, a, "a"));
    assert_eq!(ready(c.begin_change(b).unwrap()).err(), Some(LeaseError::AccountLimit));
    assert_eq!(c.stats().unwrap().accounts, 1);
    assert_eq!(c.stats().unwrap().poisoned_accounts, 1);
}

#[test]
fn lease_user_limit_retains_unbound_poison_and_does_not_allocate_on_rejection() {
    let mut l = limits(); l.users = 1;
    let c = AccountLeaseCoordinator::new(l).unwrap();
    let mut first = ready(c.begin_change(AccountId::new()).unwrap()).unwrap();
    ready(first.fence_user("one").unwrap()).unwrap();
    first.finish(ChangeResult::Unconfirmed).unwrap();
    let mut second = ready(c.begin_change(AccountId::new()).unwrap()).unwrap();
    assert_eq!(second.fence_user("two").err(), Some(LeaseError::UserLimit));
    assert_eq!(second.fence_user("one").err(), Some(LeaseError::Poisoned));
    second.finish(ChangeResult::Completed).unwrap();
    assert_eq!(c.stats().unwrap().users, 1);
    assert_eq!(c.stats().unwrap().user_bytes, 3);
}

#[test]
fn lease_user_identifier_storage_is_bounded_and_does_not_retain_input_capacity() {
    let mut l = limits(); l.user_id_bytes = 4; l.total_user_bytes = 6;
    let c = AccountLeaseCoordinator::new(l).unwrap();
    let mut change = ready(c.begin_change(AccountId::new()).unwrap()).unwrap();
    assert_eq!(change.fence_user("").err(), Some(LeaseError::InvalidUser));
    assert_eq!(change.fence_user("large").err(), Some(LeaseError::InvalidUser));
    let mut oversized = String::with_capacity(4096); oversized.push_str("abcd");
    ready(change.fence_user(&oversized).unwrap()).unwrap();
    assert_eq!(c.stats().unwrap().user_bytes, 4);
    assert_eq!(change.fence_user("efg").err(), Some(LeaseError::UserBytesLimit));
    ready(change.fence_user("ef").unwrap()).unwrap();
    assert_eq!(c.stats().unwrap().user_bytes, 6);
    change.finish(ChangeResult::Completed).unwrap();
    assert_eq!(c.stats().unwrap(), LeaseStats::default());
}

#[test]
fn lease_removal_reclaims_only_unused_open_records_after_commit() {
    let c = coordinator(); let a = AccountId::new(); let b = AccountId::new();
    bind(&c, a, "alice"); bind(&c, b, "bob");
    let mut change = ready(c.begin_change(a).unwrap()).unwrap();
    ready(change.fence_user("alice").unwrap()).unwrap();
    change.stage_binding(a, None).unwrap();
    let mut old = Box::pin(c.operation(a, "alice").unwrap()); pending(&mut old);
    change.finish(ChangeResult::Completed).unwrap();
    assert_eq!(ready(old).err(), Some(LeaseError::BindingMismatch));
    assert_eq!(c.operation(a, "alice").err(), Some(LeaseError::UnknownAccount));
    assert_eq!(c.stats().unwrap().accounts, 1);
    assert_eq!(c.stats().unwrap().users, 1);
    operation(&c, b, "bob").complete().unwrap();
}

#[test]
fn lease_user_reconciliation_does_not_reopen_a_separate_closed_account() {
    let c = coordinator(); let a = AccountId::new(); let b = AccountId::new();
    bind(&c, a, "shared"); bind(&c, b, "shared");
    let mut failed = ready(c.begin_change(a).unwrap()).unwrap();
    ready(failed.fence_user("shared").unwrap()).unwrap();
    failed.finish(ChangeResult::RetryableFailure).unwrap();
    let mut other = ready(c.begin_change(b).unwrap()).unwrap();
    ready(other.fence_user("shared").unwrap()).unwrap();
    other.finish(ChangeResult::Completed).unwrap();
    assert_eq!(c.operation(a, "shared").err(), Some(LeaseError::Closed));
    operation(&c, b, "shared").complete().unwrap();
    ready(c.begin_change(a).unwrap()).unwrap().finish(ChangeResult::Completed).unwrap();
    operation(&c, a, "shared").complete().unwrap();
}

#[test]
fn lease_multiple_binding_updates_publish_together_under_one_owner() {
    let c = coordinator(); let a = AccountId::new(); let b = AccountId::new();
    bind(&c, a, "old"); bind(&c, b, "old");
    let mut change = ready(c.begin_change(a).unwrap()).unwrap();
    ready(change.fence_user("old").unwrap()).unwrap();
    ready(change.fence_user("new").unwrap()).unwrap();
    change.stage_binding(a, Some("new")).unwrap();
    change.stage_binding(b, Some("new")).unwrap();
    assert_eq!(c.operation(a, "new").err(), Some(LeaseError::BindingMismatch));
    assert_eq!(c.operation(b, "new").err(), Some(LeaseError::BindingMismatch));
    let mut old_a = Box::pin(c.operation(a, "old").unwrap()); pending(&mut old_a);
    let mut old_b = Box::pin(c.operation(b, "old").unwrap()); pending(&mut old_b);
    change.finish(ChangeResult::Completed).unwrap();
    assert_eq!(ready(old_a).err(), Some(LeaseError::BindingMismatch));
    assert_eq!(ready(old_b).err(), Some(LeaseError::BindingMismatch));
    operation(&c, a, "new").complete().unwrap();
    operation(&c, b, "new").complete().unwrap();
}

#[test]
fn lease_unstarted_cancellation_preserves_binding_version_and_availability() {
    let c = coordinator(); let a = AccountId::new(); bind(&c, a, "old");
    let queued = c.operation(a, "old").unwrap();
    ready(c.begin_change(a).unwrap()).unwrap().cancel_unstarted().unwrap();
    ready(queued).unwrap().complete().unwrap();
    let unknown = AccountId::new();
    ready(c.begin_change(unknown).unwrap()).unwrap().cancel_unstarted().unwrap();
    assert_eq!(c.stats().unwrap().accounts, 1);
    assert_eq!(c.stats().unwrap().poisoned_accounts, 0);
}

#[test]
fn lease_unstarted_cancellation_preserves_previous_reconciliation_requirement() {
    let c = coordinator(); let a = AccountId::new(); bind(&c, a, "old");
    ready(c.begin_change(a).unwrap()).unwrap().finish(ChangeResult::RetryableFailure).unwrap();
    let before = c.stats().unwrap();
    ready(c.begin_change(a).unwrap()).unwrap().cancel_unstarted().unwrap();
    assert_eq!(before, c.stats().unwrap());
    assert_eq!(c.operation(a, "old").err(), Some(LeaseError::Closed));
}

#[test]
fn lease_unstarted_cancellation_rejects_fenced_or_staged_changes() {
    let c = coordinator(); let a = AccountId::new(); bind(&c, a, "old");
    let mut fenced = ready(c.begin_change(a).unwrap()).unwrap();
    ready(fenced.fence_user("new").unwrap()).unwrap();
    assert!(fenced.cancel_unstarted().is_err());
    assert_eq!(c.stats().unwrap().poisoned_accounts, 1);
    assert_eq!(c.stats().unwrap().poisoned_users, 1);
    let b = AccountId::new(); bind(&c, b, "other");
    let mut staged = ready(c.begin_change(b).unwrap()).unwrap();
    staged.stage_binding(b, None).unwrap();
    assert!(staged.cancel_unstarted().is_err());
    assert_eq!(c.stats().unwrap().poisoned_accounts, 2);
}

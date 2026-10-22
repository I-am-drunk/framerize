use super::*;
use super::super::tests::local_state;
use std::panic::{catch_unwind, AssertUnwindSafe};

fn poison_phase(state: &ScopeState) {
    assert!(catch_unwind(AssertUnwindSafe(|| {
        let _phase = state.phase.lock().unwrap();
        panic!("fixture phase poison");
    })).is_err());
}
fn uncertain(state: &ScopeState) {
    let phase = state.lock_phase();
    assert!(phase.stop && phase.sealed);
    assert!(matches!(phase.outcome,
        Some(CleanupOutcome::Unconfirmed(ObserverError::WorkerLost))));
}
fn retained(state: &Arc<ScopeState>, registry: &Registry) {
    let accounts = registry.accounts.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    assert!(Arc::ptr_eq(accounts.get(&state.account).unwrap(), state));
    assert_eq!(registry.slots.available_permits(), registry.limits.scopes - 1);
}

#[test]
fn phase_poison_is_sticky_and_rejects_clean_publication() {
    let (state, registry) = local_state();
    poison_phase(&state);
    state.publish(CleanupOutcome::NotStarted { reason: None });
    uncertain(&state);
    assert!(matches!(state.begin(&registry), Err(ObserverError::Sealed)));
}

#[test]
fn cleanup_observer_drop_with_poisoned_phase_does_not_double_panic() {
    let (state, _) = local_state();
    poison_phase(&state);
    assert!(catch_unwind(AssertUnwindSafe(|| {
        let _observer = CleanupObserver { state: state.clone() };
        panic!("fixture outer unwind");
    })).is_err());
    uncertain(&state);
}

#[test]
fn command_drop_with_poisoned_phase_keeps_uncertainty_and_retires_count() {
    let (state, registry) = local_state();
    let command = state.begin(&registry).unwrap();
    poison_phase(&state);
    assert!(catch_unwind(AssertUnwindSafe(move || {
        let _command = command;
        panic!("fixture command unwind");
    })).is_err());
    uncertain(&state);
    assert_eq!(state.lock_phase().active, 0);
    assert_eq!(registry.commands.available_permits(), registry.limits.commands);
}

#[test]
fn run_guard_drop_with_poisoned_phase_retains_registered_owner() {
    let (state, registry) = local_state();
    registry.accounts.lock().unwrap().insert(state.account, state.clone());
    poison_phase(&state);
    assert!(catch_unwind(AssertUnwindSafe(|| {
        let _guard = RunGuard { state: state.clone(), registry: registry.clone(), finished: false };
        panic!("fixture supervisor unwind");
    })).is_err());
    uncertain(&state);
    retained(&state, &registry);
}

#[tokio::test]
async fn poisoned_registry_retains_unstarted_owner_and_rejects_replacement() {
    let (state, registry) = local_state();
    registry.accounts.lock().unwrap().insert(state.account, state.clone());
    assert!(catch_unwind(AssertUnwindSafe(|| {
        let _accounts = registry.accounts.lock().unwrap();
        panic!("fixture registry poison");
    })).is_err());
    assert!(catch_unwind(AssertUnwindSafe(|| {
        let _guard = RunGuard { state: state.clone(), registry: registry.clone(), finished: false };
        panic!("fixture outer supervisor unwind");
    })).is_err());
    uncertain(&state);
    retained(&state, &registry);
    let factory = ObserverFactory { bridge: state._owner.clone(), registry: registry.clone() };
    assert!(matches!(factory.one_shot_scope(state.account, ApplicationGeneration(999)),
        Err(ObserverError::AccountUnconfirmed)));
    retained(&state, &registry);
}

#[tokio::test]
async fn poisoned_registry_prevents_supervisor_clean_removal() {
    let (state, registry) = local_state();
    registry.accounts.lock().unwrap().insert(state.account, state.clone());
    state.seal();
    assert!(catch_unwind(AssertUnwindSafe(|| {
        let _accounts = registry.accounts.lock().unwrap();
        panic!("fixture final-removal poison");
    })).is_err());
    let guard = RunGuard { state: state.clone(), registry: registry.clone(), finished: false };
    let mut supervisor = Box::pin(supervise(state._owner.clone(), registry.clone(), guard));
    assert!(futures::poll!(supervisor.as_mut()).is_pending());
    uncertain(&state);
    retained(&state, &registry);
    drop(supervisor);
    uncertain(&state);
    retained(&state, &registry);
}

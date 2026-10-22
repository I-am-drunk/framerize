use super::*;
use crate::operation_gate::{OperationGate, ControlGuard, DrainLease, GuardAdmission, GuardCheckError};
use std::sync::Arc;

async fn held(wave: Arc<WaveGuard>) -> (ControlGuard, DrainLease) {
    let gate = OperationGate::new(1).unwrap();
    let control = ControlGuard::bind(gate.clone(), wave);
    let mut ticket = gate.seal().unwrap();
    (control, ticket.wait().await.unwrap())
}

#[tokio::test]
async fn typed_clear_observation_preserves_recent_refusal_and_escalation_history() {
    let wave = Arc::new(WaveGuard::new());
    wave.note_refusal("fixture-account-a");
    let before = {
        let inner = wave.inner.lock().unwrap();
        (inner.refusals.clone(), inner.last_refusal, inner.pause_len, inner.paused_until)
    };
    let (control, lease) = held(wave.clone()).await;
    assert_eq!(control.check(&lease), Ok(GuardAdmission::KnownClear));
    let after = {
        let inner = wave.inner.lock().unwrap();
        (inner.refusals.clone(), inner.last_refusal, inner.pause_len, inner.paused_until)
    };
    assert_eq!(before, after);
    wave.note_refusal("fixture-account-b");
    assert!(matches!(control.check(&lease), Ok(GuardAdmission::Paused(_))));
    assert_eq!(wave.inner.lock().unwrap().pause_len, PAUSE_BASE * 2);
}

#[tokio::test]
async fn typed_pause_returns_remaining_without_resetting_guard() {
    let wave = Arc::new(WaveGuard::new());
    wave.note_refusal("fixture-account-a"); wave.note_refusal("fixture-account-b");
    let before = {
        let inner = wave.inner.lock().unwrap();
        (inner.refusals.clone(), inner.last_refusal, inner.pause_len, inner.paused_until)
    };
    let (control, lease) = held(wave.clone()).await;
    let Ok(GuardAdmission::Paused(remaining)) = control.check(&lease) else { panic!("pause required") };
    assert!(!remaining.is_zero()); assert!(remaining <= PAUSE_BASE);
    let after = {
        let inner = wave.inner.lock().unwrap();
        (inner.refusals.clone(), inner.last_refusal, inner.pause_len, inner.paused_until)
    };
    assert_eq!(before, after);
}

#[tokio::test]
async fn typed_poison_is_unavailable_even_when_legacy_check_returns_none() {
    let wave = Arc::new(WaveGuard::new());
    let (control, lease) = held(wave.clone()).await;
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _lock = wave.inner.lock().unwrap(); panic!("fixture wave poison");
    }));
    assert_eq!(wave.check(), None);
    assert_eq!(control.check(&lease), Err(GuardCheckError::Unavailable));
}

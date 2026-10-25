use super::*;
use super::native_control::{admit, AdmissionError, HealthOutcome};

#[tokio::test]
async fn outcomes_preserve_fresh_fields_and_extend_existing_cooldown() {
    let (state, ports, spec, _) = fixture();
    let mut admitted = admit(&state, spec.identity, "gpt-4.1", spec.deadline).await.unwrap();
    admitted.completion.start();
    let until = framer_core::rotation::now_ms() + 600_000;
    {
        let mut account = ports.account.lock().unwrap();
        account.alias = "fresh alias".into();
        account.cooling_until_ms = Some(until);
        account.cooldown_step = 3;
    }
    state.wave.note_refusal("one"); state.wave.note_refusal("two");
    for outcome in [HealthOutcome::Complete, HealthOutcome::Unclassified] {
        admitted.completion.publish(&state, &admitted.identity, &admitted.proxy, outcome).await.unwrap();
    }
    assert!(ports.events.lock().unwrap().is_empty());
    assert!(state.wave.check().is_some());
    admitted.completion.publish(&state, &admitted.identity, &admitted.proxy,
        HealthOutcome::RiskRefused).await.unwrap();
    let account = ports.account.lock().unwrap().clone();
    assert_eq!(account.alias, "fresh alias");
    assert_eq!(account.cooling_until_ms, Some(until));
    assert_eq!(account.cooldown_step, 4);
    admitted.pool.complete().unwrap();
    admitted.completion.complete().unwrap();
}

async fn wait_for_users(state: &ServiceState, count: usize) {
    tokio::time::timeout(Duration::from_secs(2), async {
        while state.serving_accounts.residents.snapshot().unwrap().residents
            .iter().map(|slot| slot.users).sum::<usize>() != count {
            tokio::task::yield_now().await;
        }
    }).await.unwrap();
}

#[tokio::test]
async fn cancelled_capacity_wait_retains_ownership_until_it_finishes() {
    let (state, ports, spec, _) = fixture();
    let first = admit(&state, spec.identity.clone(), "gpt-4.1", spec.deadline).await.unwrap();
    let waiting = state.clone();
    let task = tokio::spawn(async move {
        admit(&waiting, spec.identity, "gpt-4.1", spec.deadline).await
    });
    wait_for_users(&state, 3).await;
    task.abort();
    assert!(matches!(task.await, Err(error) if error.is_cancelled()));
    assert_eq!(state.serving_accounts.residents.snapshot().unwrap().residents[0].users, 3);
    drop(first);
    wait_for_users(&state, 0).await;
    assert!(ports.events.lock().unwrap().is_empty());
}

#[tokio::test]
async fn invalid_admission_never_reads_private_sessions() {
    for change in 0..7 {
        let (mut state, ports, spec, _) = fixture();
        let mut identity = spec.identity;
        let mut deadline = spec.deadline;
        let expected = match change {
            0 => { identity.user.0.push('x'); AdmissionError::InvalidScope }
            1 => { identity.team.0.push('x'); AdmissionError::InvalidScope }
            2 => { identity.project.0.push('x'); AdmissionError::InvalidScope }
            3 => { identity.account = AccountId::new(); AdmissionError::Ineligible }
            4 => { deadline = Instant::now(); AdmissionError::InvalidRequest }
            5 => { state.operation_ownership = None; AdmissionError::Ineligible }
            _ => {
                state.wave.note_refusal("one"); state.wave.note_refusal("two");
                AdmissionError::Overloaded
            }
        };
        let error = admit(&state, identity, "gpt-4.1", deadline).await.err().unwrap();
        assert_eq!(error, expected);
        assert!(ports.events.lock().unwrap().is_empty());
        assert_eq!(state.serving_accounts.residents.snapshot().unwrap().occupied, 0);
    }
}

#[tokio::test]
async fn capacity_wait_revalidates_identity_proxy_and_wave() {
    for change in 0..3 {
        let (state, ports, spec, _) = fixture();
        let first = admit(&state, spec.identity.clone(), "gpt-4.1", spec.deadline).await.unwrap();
        let waiting = state.clone();
        let task = tokio::spawn(async move {
            admit(&waiting, spec.identity, "gpt-4.1", spec.deadline).await
        });
        wait_for_users(&state, 3).await;
        match change {
            0 => ports.account.lock().unwrap().current_project = Some(ProjectId("changed".into())),
            1 => ports.account.lock().unwrap().proxy = Some("http://127.0.0.1:9".into()),
            _ => { state.wave.note_refusal("one"); state.wave.note_refusal("two"); }
        }
        drop(first);
        let error = task.await.unwrap().err().unwrap();
        assert_eq!(error, if change == 2 { AdmissionError::Overloaded } else { AdmissionError::InvalidScope });
        wait_for_users(&state, 0).await;
        assert!(ports.events.lock().unwrap().is_empty());
    }
}

use super::*;

#[tokio::test]
async fn different_store_or_operation_owner_cannot_bypass_session_ownership() {
    for replace_store in [true, false] {
        let (mut state, ports, spec, sessions) = fixture();
        if replace_store { state.sessions = ports.clone(); }
        else {
            state.operation_ownership = Some(
                framer_server::operation_ownership::ServingOwnership::new_unmanaged(
                    state.wave.clone(), 32).unwrap());
        }
        let result = launch_with_source(state, spec, Arc::new(Builder(ports.clone())), sessions, None);
        assert_eq!(result.err().unwrap().kind, FailureKind::InvalidScope);
        assert!(ports.events.lock().unwrap().is_empty());
    }
}

#[tokio::test]
async fn account_completion_keeps_shared_store_available_for_next_request() {
    let (state, ports, spec, sessions) = fixture();
    let next = Specification { identity: spec.identity.clone(), route: spec.route,
        payload: spec.payload.clone(), context: AttemptContext {
            feature_category: spec.context.feature_category.clone(),
            feature_name: spec.context.feature_name.clone(), step: spec.context.step.clone(),
            turn_id: spec.context.turn_id.clone(), user_prompt: spec.context.user_prompt.clone(),
        },
        deadline: spec.deadline, token_limit: spec.token_limit };
    let first = finish(start(state.clone(), &ports, spec, &sessions)).await;
    assert!(first.cleanup_confirmed);
    ports.read.add_permits(1);
    ports.issue.add_permits(1);
    let second = finish(start(state.clone(), &ports, next, &sessions)).await;
    assert!(second.cleanup_confirmed);
    assert_eq!(ports.builds.load(Ordering::SeqCst), 2);
    sessions.shutdown().await.unwrap();
}

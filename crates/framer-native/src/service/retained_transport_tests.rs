use super::*;

async fn retain(ports: &Ports, sessions: &Sessions) -> AccountScope {
    let account = ports.account.lock().unwrap().clone();
    let lease = sessions.bridge().lease_account_observed(account.id,
        account.framer_user_id, NativeHttpLimits::default(), NativeEgress::Direct)
        .await.unwrap();
    let scope = lease.scope().clone();
    assert!(lease.finish(true).await);
    ports.read.add_permits(1);
    scope
}

#[tokio::test]
async fn serving_reuses_idle_transport_then_closes_it_after_credential_refusal() {
    let (state, ports, spec, sessions) = fixture();
    let cached = retain(&ports, &sessions).await;
    let outcome = finish(start(state, &ports, spec, &sessions)).await;
    assert!(outcome.cleanup_confirmed && outcome.response.is_none());
    assert!(outcome.failure.is_some());
    assert_eq!(ports.builds.load(Ordering::SeqCst), 1);
    let observed = ports.scope.lock().unwrap().clone().unwrap();
    assert!(cached.same_generation(&observed));
    assert_eq!(cached.closed().now_or_never(), Some(Ok(())));
    sessions.shutdown().await.unwrap();
}

#[tokio::test]
async fn serving_replaces_idle_transport_before_using_a_changed_proxy() {
    let (state, ports, spec, sessions) = fixture();
    let cached = retain(&ports, &sessions).await;
    ports.account.lock().unwrap().proxy = Some("http://127.0.0.1:9".into());
    let outcome = finish(start(state, &ports, spec, &sessions)).await;
    assert!(outcome.cleanup_confirmed && outcome.response.is_none());
    assert!(outcome.failure.is_some());
    assert_eq!(ports.builds.load(Ordering::SeqCst), 1);
    let observed = ports.scope.lock().unwrap().clone().unwrap();
    assert!(!cached.same_generation(&observed));
    assert_eq!(cached.closed().now_or_never(), Some(Ok(())));
    assert_eq!(observed.closed().now_or_never(), Some(Ok(())));
    sessions.shutdown().await.unwrap();
}

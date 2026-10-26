//! Production AppState sharing with inert ports. No browser or network I/O.
use super::*;
use crate::operation_ownership::with_owned_operations;
#[path = "permit_lifetime_fixture.rs"]
mod fixture;

#[tokio::test]
async fn two_routers_share_capacity_registration_and_shutdown() {
    use tower::ServiceExt;
    use axum::{body::Body, http::{Request, StatusCode}};
    let f = fixture::Fixture::new();
    let a = with_owned_operations(f.state, 32).unwrap();
    let b = AppState::new(a.runtime().clone());
    let registration = a.serving_accounts.registration.lock().await;
    assert!(b.serving_accounts.registration.try_lock().is_err());
    drop(registration);
    let held = a.gate.acquire("fixture-shared-capacity").await;
    assert!(b.gate.acquire_now("fixture-shared-capacity").await.is_none());
    drop(held);
    assert!(b.gate.acquire_now("fixture-shared-capacity").await.is_some());
    let _seal = a.operation_ownership.as_ref().unwrap().seal().unwrap();
    for state in [a, b] {
        let response = crate::router(state, "/fixture-no-web").oneshot(
            Request::builder().uri("/api/health").body(Body::empty()).unwrap()).await.unwrap();
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    }
}

#[test]
fn shared_runtime_rotates_and_saturates_without_affecting_another_runtime() {
    let f = fixture::Fixture::new();
    let a = &f.state;
    let b = AppState::new(a.runtime().clone());
    let independent = fixture::Fixture::new();
    let rotation = a.serving_accounts.rotation();
    let first = a.serving_accounts.select(f.accounts.clone(), None).unwrap().id;
    let second = b.serving_accounts.select(f.accounts.clone(), None).unwrap().id;
    assert_ne!(first, second);
    let team = &f.accounts[0].current_team.as_ref().unwrap().0;
    rotation.mark_saturated(team, 60_000);
    assert!(b.serving_accounts.rotation().is_saturated(team));
    assert_eq!(b.serving_accounts.select(f.accounts.clone(), None).unwrap().id, f.accounts[1].id);
    assert!(!independent.state.serving_accounts.rotation().is_saturated(team));
    assert_eq!(rotation.next_scope_idx("fixture-account", 2), 0);
    assert_eq!(b.serving_accounts.rotation().next_scope_idx("fixture-account", 2), 1);
    assert_eq!(independent.state.serving_accounts.rotation().next_scope_idx("fixture-account", 2), 0);
}

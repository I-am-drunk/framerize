use super::*;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use framer_core::account::AccountState;
use framer_core::ids::{FramerUserId, ProjectId, TeamId};

#[path = "serving_account_load_tests.rs"]
mod load;

fn account(n: u128) -> ManagedAccount {
    ManagedAccount {
        id: AccountId(uuid::Uuid::from_u128(n)),
        alias: format!("fixture-{n}"),
        email: Some(format!("fixture-{n}@example.invalid")),
        framer_user_id: FramerUserId(format!("user-{n}")),
        castle_device_id: format!("device-{n}"),
        proxy: Some(format!("http://proxy-{n}.invalid:8080")),
        current_team: Some(TeamId(format!("team-{n}"))),
        current_project: Some(ProjectId(format!("project-{n}"))),
        extra_workspaces: Vec::new(),
        state: AccountState::Ready,
        credit: None,
        consecutive_failures: 0,
        cooldown_step: 0,
        cooling_until_ms: None,
    }
}

fn policy(ids: &[u128]) -> ServingAccountPolicy {
    let raw = ids.iter().map(|n| uuid::Uuid::from_u128(*n).to_string())
        .collect::<Vec<_>>().join(",");
    ServingAccountPolicy::parse(Some(&raw)).unwrap()
}

#[test]
fn exhausted_retry_keeps_earliest_allowed_cooldown() {
    let mut first = account(1);
    let mut second = account(2);
    let mut disabled = account(3);
    first.cooling_until_ms = Some(61_000);
    second.cooling_until_ms = Some(31_000);
    disabled.cooling_until_ms = Some(2_000);
    let excluded = vec![first.id.0.to_string(), second.id.0.to_string()];
    let pool = vec![first, second, disabled];
    let selected = policy(&[1, 2]);
    assert!(matches!(selected.select_fresh_at(pool.clone(), &excluded, 1_000),
        Err(ApiError::AccountCooling { retry_after_secs: 30 })));
    assert!(matches!(selected.select_fresh_at(pool.clone(), &excluded, 31_000),
        Err(ApiError::AccountCooling { retry_after_secs: 30 })));
    assert!(matches!(selected.select_fresh_at(pool, &excluded, 61_000),
        Err(ApiError::ServingCapacity(_))));
}

#[test]
fn unset_policy_preserves_unrestricted_admission() {
    let policy = ServingAccountPolicy::parse(None).unwrap();
    assert_eq!(policy.configured_count(), None);
    assert!(policy.allows(&account(1).id));
    assert!(policy.allows(&account(2).id));
}

#[test]
fn comma_separated_uuids_accept_whitespace_and_case() {
    let raw = " 00000000-0000-0000-0000-00000000000A,\n00000000-0000-0000-0000-00000000000b ";
    let policy = ServingAccountPolicy::parse(Some(raw)).unwrap();
    assert_eq!(policy.configured_count(), Some(2));
    assert!(policy.allows(&account(10).id));
    assert!(policy.allows(&account(11).id));
    assert!(!policy.allows(&account(12).id));
}

#[test]
fn empty_policy_is_an_initialization_error() {
    for raw in ["", " ", "\n\t"] {
        assert!(ServingAccountPolicy::parse(Some(raw)).is_err());
    }
}

#[test]
fn partial_or_malformed_ids_fail_closed() {
    for raw in ["00000000", "*", "all", "not-a-uuid", "00000000-0000-0000-0000-00000000000g"] {
        assert!(ServingAccountPolicy::parse(Some(raw)).is_err());
    }
}

#[test]
fn blank_entries_are_rejected_instead_of_ignored() {
    let id = account(1).id.0.to_string();
    for raw in [format!("{id},"), format!(",{id}"), format!("{id}, ,{id}")] {
        assert!(ServingAccountPolicy::parse(Some(&raw)).is_err());
    }
}

#[test]
fn duplicate_ids_are_rejected_after_normalization() {
    let lower = account(10).id.0.to_string();
    let upper = lower.to_uppercase();
    assert!(ServingAccountPolicy::parse(Some(&format!("{lower},{upper}"))).is_err());
}

#[test]
fn explicit_parked_account_never_falls_back() {
    let pool = vec![account(1), account(2)];
    let requested = pool[1].id.0.to_string();
    let error = policy(&[1]).select(pool, Some(&requested)).unwrap_err();
    assert!(matches!(error, ApiError::ServingCapacity(_)));
    assert_eq!(error.into_response().status(), StatusCode::SERVICE_UNAVAILABLE);
}

#[test]
fn unknown_explicit_id_preserves_existing_error() {
    let error = policy(&[1]).select(vec![account(1)], Some("missing")).unwrap_err();
    assert!(matches!(&error, ApiError::Internal(message) if message == "unknown account id"));
    assert_eq!(error.into_response().status(), StatusCode::INTERNAL_SERVER_ERROR);
}

#[test]
fn explicit_cooling_id_rejects_without_substituting_a_ready_account() {
    let mut requested = account(1);
    requested.consecutive_failures = 9;
    requested.cooling_until_ms = Some(u64::MAX);
    let id = requested.id.0.to_string();
    let error = policy(&[1, 2]).select_at(vec![requested, account(2)], Some(&id), 0).unwrap_err();
    assert!(matches!(error, ApiError::AccountCooling { retry_after_secs }
        if retry_after_secs == u64::MAX.div_ceil(1000)));
}

#[test]
fn automatic_selection_never_loads_a_healthier_parked_account() {
    let mut allowed = account(1);
    allowed.consecutive_failures = 2;
    let pool = vec![account(2), allowed];
    let policy = policy(&[1]);
    for _ in 0..8 {
        assert_eq!(policy.select(pool.clone(), None).unwrap().id, account(1).id);
    }
}

#[test]
fn selection_preserves_failure_ranking_within_allowed_pool() {
    let mut worse = account(1);
    worse.consecutive_failures = 3;
    let pool = vec![worse, account(2), account(3)];
    assert_eq!(policy(&[1, 2]).select(pool, None).unwrap().id, account(2).id);
}

#[test]
fn selection_preserves_cooldown_avoidance_within_allowed_pool() {
    let mut cooling = account(1);
    cooling.cooling_until_ms = Some(u64::MAX);
    let pool = vec![account(3), cooling, account(2)];
    assert_eq!(policy(&[1, 2]).select(pool, None).unwrap().id, account(2).id);
}

#[test]
fn all_cooling_returns_earliest_allowed_wait_without_fallback() {
    let mut later = account(1);
    later.cooling_until_ms = Some(610_000);
    let mut sooner = account(2);
    sooner.cooling_until_ms = Some(310_001);
    let pool = vec![account(3), later, sooner];
    assert!(matches!(policy(&[1, 2]).select_at(pool, None, 10_000),
        Err(ApiError::AccountCooling { retry_after_secs: 301 })));
}

#[test]
fn selected_but_unprovisioned_account_is_not_made_eligible() {
    let mut incomplete = account(1);
    incomplete.current_project = None;
    let pool = vec![incomplete, account(2), account(3)];
    assert_eq!(policy(&[1, 2]).select(pool, None).unwrap().id, account(2).id);
}

#[test]
fn retry_selection_excludes_both_tried_and_parked_accounts() {
    let tried = vec![account(1).id.0.to_string()];
    let pool = vec![account(1), account(3), account(2)];
    assert_eq!(policy(&[1, 2]).select_fresh(pool, &tried).unwrap().id, account(2).id);
}

#[test]
fn exhausted_serving_pool_does_not_expand_into_parked_accounts() {
    let tried = vec![account(1).id.0.to_string()];
    let error = policy(&[1]).select_fresh(vec![account(1), account(2)], &tried).unwrap_err();
    assert!(matches!(error, ApiError::ServingCapacity(_)));
}

#[test]
fn unknown_configured_id_does_not_expand_into_stored_accounts() {
    let policy = policy(&[99]);
    assert_eq!(policy.configured_count(), Some(1));
    let error = policy.select(vec![account(1)], None).unwrap_err();
    assert!(matches!(error, ApiError::ServingCapacity(_)));
}

#[test]
fn unrestricted_empty_pool_and_retry_preserve_original_errors() {
    let policy = ServingAccountPolicy::default();
    assert!(matches!(policy.select(Vec::new(), None), Err(ApiError::Internal(message))
        if message == "no eligible account"));
    let tried = vec![account(1).id.0.to_string()];
    assert!(matches!(policy.select_fresh(vec![account(1)], &tried), Err(ApiError::Internal(message))
        if message == "pool exhausted: every account was refused for this request"));
}

#[test]
fn admission_preserves_record_identity_proxy_and_health() {
    let mut parked = account(2);
    parked.consecutive_failures = 7;
    parked.cooldown_step = 3;
    parked.cooling_until_ms = Some(u64::MAX);
    let pool = vec![account(1), parked];
    let before = serde_json::to_value(&pool).unwrap();
    let policy = policy(&[1]);
    let selected = policy.select(pool.clone(), None).unwrap();
    assert_eq!(serde_json::to_value(&selected).unwrap(), before[0]);
    assert!(policy.require(&pool[1].id).is_err());
    assert_eq!(serde_json::to_value(&pool).unwrap(), before);
}

#[test]
fn six_allowed_accounts_bound_selection_from_eleven_records() {
    let pool: Vec<_> = (1..=11).map(account).collect();
    let policy = policy(&[1, 2, 3, 4, 5, 6]);
    assert_eq!(policy.configured_count(), Some(6));
    for n in 1..=11 {
        assert_eq!(policy.require(&account(n).id).is_ok(), n <= 6);
    }
    for _ in 0..24 {
        let selected = policy.select(pool.clone(), None).unwrap();
        assert!(selected.id.0.as_u128() <= 6);
    }
    assert_eq!(pool.len(), 11);
}

#[tokio::test]
async fn capacity_http_status_is_distinct_without_remapping_existing_errors() {
    let error = policy(&[1]).require(&account(2).id).unwrap_err();
    let response = error.into_response();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let bytes = axum::body::to_bytes(response.into_body(), 1024).await.unwrap();
    assert_eq!(bytes.as_ref(), b"account is disabled by the pool configuration; its saved session is retained");

    // Identical text from a pre-existing internal error is still an HTTP 500.
    let response = ApiError::Internal(String::from_utf8(bytes.to_vec()).unwrap()).into_response();
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let body = axum::body::to_bytes(response.into_body(), 1024).await.unwrap();
    assert_eq!(body, bytes);
}

#[path = "serving_accounts_endpoint_tests.rs"]
mod endpoints;

#[test]
fn explicit_ready_account_is_not_replaced_by_a_healthier_peer() {
    let mut a = account(1);
    a.consecutive_failures = 9;
    a.cooling_until_ms = Some(1000);
    let id = a.id.0.to_string();
    let chosen = policy(&[1, 2]).select_at(vec![a, account(2)], Some(&id), 1000).unwrap();
    assert_eq!(chosen.id, account(1).id);
    assert_eq!(chosen.consecutive_failures, 9);
}

#[test]
fn unprovisioned_cooldown_does_not_masquerade_as_available_capacity() {
    let mut a = account(1);
    a.current_project = None;
    a.cooling_until_ms = Some(610_000);
    assert!(matches!(policy(&[1]).select_at(vec![a], None, 1000),
        Err(ApiError::ServingCapacity(_))));
}

#[test]
fn cooling_account_reenters_at_its_deadline_without_state_changes() {
    let mut a = account(1);
    a.cooling_until_ms = Some(1001);
    a.consecutive_failures = 9;
    let p = policy(&[1]);
    assert!(matches!(p.select_at(vec![a.clone()], None, 1000),
        Err(ApiError::AccountCooling { retry_after_secs: 1 })));
    let selected = p.select_at(vec![a.clone()], None, 1001).unwrap();
    assert_eq!(serde_json::to_value(selected).unwrap(), serde_json::to_value(a).unwrap());
}

#[test]
fn retry_wait_excludes_tried_parked_and_unprovisioned_accounts() {
    let mut pool: Vec<_> = (1..=4).map(account).collect();
    pool[1].cooling_until_ms = Some(400_000);
    pool[3].current_project = None;
    let tried = vec![pool[0].id.0.to_string()];
    assert!(matches!(policy(&[1, 2, 4]).select_fresh_at(pool, &tried, 100_000),
        Err(ApiError::AccountCooling { retry_after_secs: 300 })));
}

#[test]
fn unrestricted_admission_also_respects_the_full_cooldown() {
    let mut a = account(1);
    a.cooling_until_ms = Some(601_000);
    let p = ServingAccountPolicy::default();
    for _ in 0..3 {
        assert!(matches!(p.select_at(vec![a.clone()], None, 1000),
            Err(ApiError::AccountCooling { retry_after_secs: 600 })));
    }
    assert_eq!(a.cooling_until_ms, Some(601_000));
}

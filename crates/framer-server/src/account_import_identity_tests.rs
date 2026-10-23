//! An imported session must belong to the explicitly selected login.
use super::*;

#[cfg(feature = "native-dispatch")]
struct UnusedNative;

#[cfg(feature = "native-dispatch")]
impl framer_core::native_dispatch::InvocationProvider for UnusedNative {
    fn policy(&self) -> framer_core::native_dispatch::Policy {
        panic!("account import must not invoke native inference")
    }
    fn contains_account(&self, _account: AccountId) -> bool {
        panic!("account import must not invoke native inference")
    }
    fn prepare(&self, _request: framer_core::native_dispatch::Preparation,
        _permit: tokio::sync::OwnedSemaphorePermit)
        -> BoxFuture<'_, Result<Box<dyn framer_core::native_dispatch::OwnedInvocation>,
            framer_core::native_dispatch::DispatchError>> {
        panic!("account import must not invoke native inference")
    }
}

#[tokio::test(flavor = "current_thread")]
async fn mismatched_login_refuses_before_save_or_workspace_actions() {
    let (state, ports) = fixture(true);
    let before = serde_json::to_value(&*ports.records.lock().unwrap()).unwrap();
    let body = serde_json::from_value(json!({
        "account_id": account(2).id,
        "session_cookie": "fixture-only",
        "expected_framer_user_id": "other-user"
    })).unwrap();
    let result = crate::import_account(State(state), Json(body)).await;
    assert!(matches!(result, Err(ApiError::InvalidRequest(ref message))
        if message == "session belongs to a different Framer user"));
    assert_eq!(ports.client_calls.load(Ordering::SeqCst), 1);
    assert_eq!(ports.session_calls.load(Ordering::SeqCst), 0);
    assert_eq!(ports.state_writes.load(Ordering::SeqCst), 0);
    assert_eq!(ports.browser_calls.load(Ordering::SeqCst), 0);
    assert!(ports.saved.lock().unwrap().is_empty());
    assert!(ports.registered_proxies.lock().unwrap().is_empty());
    assert_eq!(serde_json::to_value(&*ports.records.lock().unwrap()).unwrap(), before);
}

#[tokio::test(flavor = "current_thread")]
async fn matching_login_preserves_normal_import() {
    let (state, ports) = fixture(true);
    let first_before = serde_json::to_value(&ports.records.lock().unwrap()[0]).unwrap();
    let body = serde_json::from_value(json!({
        "account_id": account(2).id,
        "session_cookie": "fixture-only",
        "expected_framer_user_id": "user-2",
        "team_id": "team-2"
    })).unwrap();
    let result = crate::import_account(State(state), Json(body)).await.unwrap();
    assert_eq!(result.0["status"], "imported");
    assert_eq!(result.0["framer_user_id"], "user-2");
    assert_eq!(ports.session_calls.load(Ordering::SeqCst), 1);
    assert_eq!(ports.saved.lock().unwrap()[0].active_user.0, "user-2");
    assert_eq!(ports.browser_calls.load(Ordering::SeqCst), 0);
    assert_eq!(serde_json::to_value(&ports.records.lock().unwrap()[0]).unwrap(), first_before);
}

#[tokio::test(flavor = "current_thread")]
async fn malformed_expected_login_refuses_before_any_io() {
    for expected in [String::new(), "   ".to_string(), "a".repeat(129)] {
        let (state, ports) = fixture(false);
        let body = serde_json::from_value(json!({
            "account_id": account(2).id,
            "session_cookie": "fixture-only",
            "expected_framer_user_id": expected
        })).unwrap();
        let result = crate::import_account(State(state), Json(body)).await;
        assert!(matches!(result, Err(ApiError::InvalidRequest(ref message))
            if message == "expected Framer user is invalid"));
        ports.assert_rejected_before_io(0);
        assert!(ports.saved.lock().unwrap().is_empty());
        assert!(ports.registered_proxies.lock().unwrap().is_empty());
    }
}

#[cfg(feature = "native-dispatch")]
#[tokio::test(flavor = "current_thread")]
async fn eligible_native_import_does_not_start_browser_prewarm() {
    let (mut state, ports) = fixture(true);
    state.serving_accounts = Arc::new(policy(&[1, 2]));
    state.native_dispatch = Some(Arc::new(UnusedNative));
    let body = serde_json::from_value(json!({
        "account_id": account(2).id,
        "session_cookie": "fixture-only",
        "expected_framer_user_id": "user-2",
        "team_id": "team-2"
    })).unwrap();
    let result = crate::import_account(State(state), Json(body)).await.unwrap();
    assert_eq!(result.0["serving_allowed"], true);
    assert_eq!(ports.session_calls.load(Ordering::SeqCst), 1);
    assert_eq!(ports.registered_proxies.lock().unwrap().len(), 1);
    assert_eq!(Arc::strong_count(&ports), 1, "browser prewarm retained account ports");
    tokio::task::yield_now().await;
    assert_eq!(ports.browser_calls.load(Ordering::SeqCst), 0);
}

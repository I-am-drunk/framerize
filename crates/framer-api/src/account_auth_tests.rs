use framer_core::ids::AccountId;
#[tokio::test]
async fn account_cache_invalidation_preserves_unrelated_account_credentials() {
    let api = crate::HttpFramerClient::new();
    let replaced = AccountId(uuid::Uuid::new_v4());
    let unrelated = AccountId(uuid::Uuid::new_v4());
    let session = framer_core::Session {
        active_user: framer_core::FramerUserId("fixture-user".into()),
        cookie_header: "fixture-only".into(),
    };
    for id in [replaced, unrelated, replaced] {
        api.ai_token.lock().unwrap().push((id, crate::auth::AiToken::for_session(
            &session, "fixture-cache-token".to_owned(), i64::MAX)));
    }
    api.invalidate_account_authorization(&replaced);
    let cache = api.ai_token.lock().unwrap();
    assert_eq!(cache.len(), 1);
    assert_eq!(cache[0].0, unrelated);
}

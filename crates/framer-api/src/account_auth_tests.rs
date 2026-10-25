use framer_core::ids::AccountId;
#[tokio::test]
async fn account_cache_invalidation_preserves_unrelated_account_credentials() {
    let api = crate::HttpFramerClient::new();
    let replaced = AccountId(uuid::Uuid::new_v4());
    let unrelated = AccountId(uuid::Uuid::new_v4());
    for id in [replaced, unrelated, replaced] {
        api.ai_token.lock().unwrap().push((id, crate::auth::AiToken {
            bearer: "fixture-cache-token".to_owned(), expires_at_ms: i64::MAX,
        }));
    }
    api.invalidate_account_authorization(&replaced);
    let cache = api.ai_token.lock().unwrap();
    assert_eq!(cache.len(), 1);
    assert_eq!(cache[0].0, unrelated);
}

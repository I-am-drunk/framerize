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
        api.transport.authorization.accounts.lock().unwrap().push((id, crate::auth::AiToken::for_session(
            &session, "fixture-cache-token".to_owned(), i64::MAX)));
    }
    api.invalidate_account_authorization(&replaced);
    let cache = api.transport.authorization.accounts.lock().unwrap();
    assert_eq!(cache.len(), 1);
    assert_eq!(cache[0].0, unrelated);
}
#[test]
fn account_invalidation_keeps_session_tokens_and_unrelated_accounts() {
    use crate::authorization_fixture::{seed, cached};
    let client = crate::HttpFramerClient::new();
    let changed = framer_core::AccountId(uuid::Uuid::new_v4());
    let other = framer_core::AccountId(uuid::Uuid::new_v4());
    let session = framer_core::port::Session {
        active_user: framer_core::FramerUserId("fixture-shared-user".into()),
        cookie_header: "session=fixture-shared-cookie".into(),
    };
    seed(&client, changed, &session);
    seed(&client, other, &session);
    client.invalidate_account_authorization(&changed);
    assert_eq!(cached(&client, changed, &session), [false, true, true]);
    assert_eq!(cached(&client, other, &session), [true; 3]);
}

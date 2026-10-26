//! Account identifiers survive login/import, but their bearer must not.
use super::*;

fn replace_account(previous: &Fixture, replacement: &mut Fixture) {
    replacement.account = previous.account;
    replacement.browser.account = previous.account;
    replacement.outer = Mutex::new(previous.outer.lock().unwrap().clone());
}

fn clear_shared(fixture: &Fixture) {
    cache().lock().unwrap_or_else(std::sync::PoisonError::into_inner)
        .remove(&(fixture.session.cookie_header.clone(), AI.into()));
}

#[tokio::test]
async fn changed_user_with_same_cookie_cannot_reuse_cached_bearer() {
    for mode in [Mode::Proxy, Mode::Inference] {
        let mut fixture = Fixture::new(REJECTED,
            Err("replacement user requires authorization".into()), vec![]);
        fixture.session.active_user = FramerUserId("replacement-user".into());
        let error = fixture.run(mode).await.unwrap_err();
        assert_eq!(error, "replacement user requires authorization");
        assert!(fixture.sent().is_empty());
        assert_eq!(fixture.mint_calls(), 1);
    }
}

#[tokio::test]
async fn replacement_mint_failure_does_not_send_previous_users_bearer() {
    for mode in [Mode::Proxy, Mode::Inference] {
        let previous = Fixture::new(REJECTED, Err("unused".into()), vec![]);
        let mut replacement = Fixture::new(FRESH,
            Err("fixture mint refused".into()), vec![]);
        clear_shared(&replacement);
        replace_account(&previous, &mut replacement);
        let error = replacement.run(mode).await.unwrap_err();
        assert_eq!(error, "fixture mint refused");
        assert!(replacement.sent().is_empty());
        assert_eq!(replacement.mint_calls(), 1);
        assert_eq!(previous.cached(AI).as_deref(), Some(REJECTED));
    }
}

#[tokio::test]
async fn replacement_session_mints_when_its_shared_cache_is_empty() {
    for mode in [Mode::Proxy, Mode::Inference] {
        let previous = Fixture::new(REJECTED, Err("unused".into()), vec![]);
        let mut replacement = Fixture::new(FRESH, Ok(FRESH), vec![200]);
        clear_shared(&replacement);
        replace_account(&previous, &mut replacement);
        replacement.run(mode).await.unwrap();
        assert_eq!(replacement.sent(), [FRESH]);
        assert_eq!(replacement.mint_calls(), 1);
        assert_eq!(previous.cached(AI).as_deref(), Some(REJECTED));
    }
}

#[tokio::test]
async fn replaced_cookie_for_same_user_uses_replacement_session_bearer() {
    for mode in [Mode::Proxy, Mode::Inference] {
        let previous = Fixture::new(REJECTED, Err("unused".into()), vec![200]);
        let mut replacement = Fixture::new(FRESH, Err("unused".into()), vec![200]);
        replacement.session.active_user = previous.session.active_user.clone();
        store(&replacement.session, AI, token(&replacement.session, FRESH));
        replace_account(&previous, &mut replacement);
        replacement.run(mode).await.unwrap();
        assert_eq!(replacement.sent(), [FRESH]);
        assert_eq!(replacement.mint_calls(), 0);
        assert_eq!(previous.cached(AI).as_deref(), Some(REJECTED));
    }
}

#[tokio::test]
async fn replaced_user_on_same_account_uses_replacement_session_bearer() {
    for mode in [Mode::Proxy, Mode::Inference] {
        let previous = Fixture::new(REJECTED, Err("unused".into()), vec![200]);
        previous.run(mode).await.unwrap();
        let mut replacement = Fixture::new(FRESH, Err("unused".into()), vec![200]);
        replace_account(&previous, &mut replacement);
        replacement.run(mode).await.unwrap();
        assert_eq!(previous.sent(), [REJECTED]);
        assert_eq!(replacement.sent(), [FRESH]);
        assert_eq!(replacement.mint_calls(), 0);
        assert_eq!(previous.cached(AI).as_deref(), Some(REJECTED));
        assert_eq!(replacement.cached(AI).as_deref(), Some(FRESH));
    }
}

//! Inert cache setup for ownership tests. Never supplies production credentials.
use crate::{auth, HttpFramerClient};
use framer_core::{AccountId, port::Session};

/// Exercise both production cache-hit paths. Refuse misses before any I/O.
pub async fn reuse(client: &HttpFramerClient, session: &Session) {
    {
        let state = client.transport.authorization.sessions.lock().unwrap();
        for path in ["/auth/ai/access-token", "/auth/web/access-token"] {
            assert!(state.get(&(session.cookie_header.clone(), path.into()))
                .is_some_and(|token| token.usable_for(session)));
        }
    }
    assert_eq!(auth::ai_access_token(&client.transport, session).await.unwrap().bearer,
        "fixture-session");
    assert_eq!(auth::web_access_token(&client.transport, session).await.unwrap().bearer,
        "fixture-session");
}

pub fn seed(client: &HttpFramerClient, account: AccountId, session: &Session) {
    let state = &client.transport.authorization;
    state.accounts.lock().unwrap().push((account,
        auth::AiToken::for_session(session, "fixture-account".into(), i64::MAX)));
    let mut sessions = state.sessions.lock().unwrap();
    for path in ["/auth/ai/access-token", "/auth/web/access-token"] {
        sessions.insert((session.cookie_header.clone(), path.into()),
            auth::AiToken::for_session(session, "fixture-session".into(), i64::MAX));
    }
}

pub fn cached(client: &HttpFramerClient, account: AccountId, session: &Session) -> [bool; 3] {
    let state = &client.transport.authorization;
    let account = state.accounts.lock().unwrap().iter().any(|(id, _)| *id == account);
    let sessions = state.sessions.lock().unwrap();
    [account, sessions.contains_key(&(session.cookie_header.clone(), "/auth/ai/access-token".into())),
        sessions.contains_key(&(session.cookie_header.clone(), "/auth/web/access-token".into()))]
}

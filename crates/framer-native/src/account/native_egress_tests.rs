use super::*;
use crate::transport::http::egress_probe_fixture::{Peer, response, NONCE};

fn open(client: &NativeAccountClient, peer: &Peer) -> AccountScope {
    let account = AccountId::new();
    let user = FramerUserId(format!("fixture-{account}"));
    let session = Session { active_user: user.clone(), cookie_header: "session=fixture-only".into() };
    client.open_account_for_test(account, session, || NativeHttp::for_loopback(
        account, user, peer.address, NativeHttpLimits::default())).unwrap()
}

fn begin(scope: &AccountScope, peer: &Peer) -> PendingPreparation<NativeEgressObservation> {
    let budget = Duration::from_secs(2);
    let request = EgressProbeRequest::for_loopback(&peer.url, budget).unwrap();
    scope.begin_egress(request, budget).unwrap()
}

async fn close(client: &NativeAccountClient, scope: &AccountScope) -> CloseResult {
    client.close_account(scope).unwrap().await
}

fn good_reply() -> Vec<u8> {
    response(200, "application/json", br#"{"ip":"203.0.113.8"}"#)
}

#[tokio::test]
async fn observation_keeps_exact_scope_without_retaining_the_account() {
    let peer = Peer::start(Some(good_reply())).await;
    let client = NativeAccountClient::new().unwrap();
    let scope = open(&client, &peer);
    let result = begin(&scope, &peer).await;
    let expected_url = peer.url.clone();
    let (wire, joined) = peer.settle().await;
    let current = result.as_ref().is_ok_and(|value| value.owner_is_current());
    let matching = result.as_ref().is_ok_and(|value| value.matches_scope(&scope));
    let closed = close(&client, &scope).await;
    let stopped = client.shutdown_idle().await;
    assert!(joined && current && matching);
    assert_eq!(closed, Ok(()));
    assert!(stopped.is_ok());
    let observed = result.unwrap();
    assert!(!observed.owner_is_current());
    assert_eq!(observed.url(), expected_url);
    assert_eq!(observed.address().to_string(), "203.0.113.8");
    assert!(!wire.contains("fixture-only"));
    assert!(!wire.to_lowercase().contains("cookie:"));
}

#[tokio::test]
async fn cancelling_probe_joins_request_before_account_release() {
    let mut peer = Peer::start(None).await;
    let client = NativeAccountClient::new().unwrap();
    let scope = open(&client, &peer);
    let pending = begin(&scope, &peer);
    let settled = pending.completion();
    let entered = peer.entered().await;
    pending.cancellation().cancel();
    let result = pending.await;
    let completed = settled.await;
    let closed = close(&client, &scope).await;
    let stopped = client.shutdown_idle().await;
    let (_, peer_closed) = peer.settle().await;
    assert!(entered && peer_closed);
    assert_eq!(result.err(), Some(PreparationFailure::Cancelled));
    assert_eq!(completed, Ok(()));
    assert_eq!(closed, Ok(()));
    assert!(stopped.is_ok());
}

#[tokio::test]
async fn dropped_probe_waiter_leaves_cleanup_with_the_account() {
    let mut peer = Peer::start(None).await;
    let client = NativeAccountClient::new().unwrap();
    let scope = open(&client, &peer);
    let pending = begin(&scope, &peer);
    let settled = pending.completion();
    let entered = peer.entered().await;
    let held = lock(&client.inner.state).accounts[0].requests.len();
    drop(pending);
    let closed = close(&client, &scope).await;
    let completed = settled.await;
    let stopped = client.shutdown_idle().await;
    let (_, peer_closed) = peer.settle().await;
    assert!(entered && peer_closed);
    assert_eq!(held, 1);
    assert_eq!(completed, Ok(()));
    assert_eq!(closed, Ok(()));
    assert!(stopped.is_ok());
}

#[tokio::test]
async fn account_close_cancels_probe_and_refuses_stale_scope() {
    let mut peer = Peer::start(None).await;
    let client = NativeAccountClient::new().unwrap();
    let scope = open(&client, &peer);
    let pending = begin(&scope, &peer);
    let entered = peer.entered().await;
    let closing = client.close_account(&scope).unwrap();
    let url = format!("https://127.0.0.1:9/ip?framerize_probe={NONCE}");
    let during = scope.observe_egress(&url, Duration::from_secs(1));
    let outcome = pending.await;
    let closed = closing.await;
    let after = scope.observe_egress(&url, Duration::from_secs(1));
    let stopped = client.shutdown_idle().await;
    let (_, peer_closed) = peer.settle().await;
    assert!(entered && peer_closed);
    assert!(matches!(during, Err(ClientError::Draining | ClientError::StaleScope)));
    assert!(matches!(after, Err(ClientError::StaleScope)));
    assert_eq!(outcome.err(), Some(PreparationFailure::Cancelled));
    assert_eq!(closed, Ok(()));
    assert!(stopped.is_ok());
}

#[tokio::test]
async fn probes_share_preparation_capacity_and_validate_before_reserving() {
    let client = NativeAccountClient::new().unwrap();
    let scope = client.open_account(AccountId::new(), Session {
        active_user: FramerUserId("fixture-user".into()), cookie_header: "session=fixture".into(),
    }, NativeHttpLimits::default(), NativeEgress::Direct).unwrap();
    let release = Arc::new(tokio::sync::Semaphore::new(0));
    let mut pending = Vec::new();
    for _ in 0..CAPACITY {
        let ready = release.clone();
        pending.push(client.prepare_owned(&scope, Instant::now() + Duration::from_secs(30),
            move |_, _| async move { ready.acquire().await.unwrap().forget();
                PreparationResult::finished(Ok(())) }).unwrap());
    }
    let url = format!("https://127.0.0.1:9/ip?framerize_probe={NONCE}");
    let full = scope.observe_egress(&url, Duration::from_secs(1));
    let invalid = scope.observe_egress("http://127.0.0.1:9/", Duration::from_secs(1));
    let count = lock(&client.inner.state).accounts[0].requests.len();
    release.add_permits(CAPACITY);
    let mut completed = true;
    for work in pending { completed &= work.await.is_ok(); }
    let closed = close(&client, &scope).await;
    let stopped = client.shutdown_idle().await;
    assert!(completed && stopped.is_ok());
    assert_eq!(closed, Ok(()));
    assert_eq!(count, CAPACITY);
    assert!(matches!(full, Err(ClientError::Full)));
    assert!(matches!(invalid, Err(ClientError::InvalidContext)));
}

#[tokio::test]
async fn held_probe_does_not_block_or_match_another_account() {
    let mut first_peer = Peer::start(None).await;
    let second_peer = Peer::start(Some(good_reply())).await;
    let client = NativeAccountClient::new().unwrap();
    let first = open(&client, &first_peer);
    let second = open(&client, &second_peer);
    let pending = begin(&first, &first_peer);
    let entered = first_peer.entered().await;
    let result = begin(&second, &second_peer).await;
    let independent = pending.completion().now_or_never().is_none();
    let correct = result.as_ref().is_ok_and(|value| value.matches_scope(&second)
        && !value.matches_scope(&first) && value.owner_is_current());
    let closing = client.close_account(&first).unwrap();
    let cancelled = pending.await;
    let first_closed = closing.await;
    let second_closed = close(&client, &second).await;
    let stopped = client.shutdown_idle().await;
    let (_, first_joined) = first_peer.settle().await;
    let (_, second_joined) = second_peer.settle().await;
    assert!(entered && independent && correct && first_joined && second_joined);
    assert_eq!(cancelled.err(), Some(PreparationFailure::Cancelled));
    assert_eq!(first_closed, Ok(()));
    assert_eq!(second_closed, Ok(()));
    assert!(stopped.is_ok());
}

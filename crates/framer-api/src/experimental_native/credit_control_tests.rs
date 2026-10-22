use super::*;
use crate::experimental_native::native_account::{NativeAccountClient, PendingPreparation};
use crate::experimental_native::native_http::NativeEgress;
use tokio::{io::AsyncWriteExt, net::TcpListener, task::JoinHandle, time::Instant};
use std::time::Duration;

#[path = "control_authorization_fixture.rs"]
mod fixture;
use fixture::*;

#[tokio::test]
async fn routing_credit_rejects_oversized_response_as_unconfirmed() {
    let (client, account) = scope(NativeEgress::Direct);
    let target = listener().await;
    let api = bound(&account, target.local_addr().unwrap(), NativeEgress::Direct);
    let worker = pair(target, " ".repeat(65_537));
    assert_eq!(start(&client, &account, api, "team").await,
        Err(PreparationFailure::CleanupUnconfirmed));
    assert_eq!(worker.await.unwrap().len(), 2);
    assert!(client.close_account(&account).unwrap().await.is_err());
}

#[tokio::test]
async fn routing_credit_cancellation_waits_for_the_owned_http_read() {
    let (client, account) = scope(NativeEgress::Direct);
    let target = listener().await;
    let api = bound(&account, target.local_addr().unwrap(), NativeEgress::Direct);
    let (ready, started) = tokio::sync::oneshot::channel();
    let (release, released) = tokio::sync::oneshot::channel();
    let worker = held_ledger(target, ready, released);
    let pending = start(&client, &account, api, "team");
    started.await.unwrap();
    pending.cancellation().cancel();
    assert!(pending.completion().now_or_never().is_none());
    let closed = client.close_account(&account).unwrap();
    assert!(closed.clone().now_or_never().is_none());
    release.send(()).unwrap();
    assert_eq!(pending.await, Err(PreparationFailure::Cancelled));
    assert_eq!(closed.await, Ok(()));
    worker.await.unwrap();
}

fn held_ledger(listener: TcpListener, ready: tokio::sync::oneshot::Sender<()>,
    release: tokio::sync::oneshot::Receiver<()>) -> JoinHandle<()> {
    tokio::spawn(async move {
        tokio::time::timeout(BOUND, async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            read_request(&mut socket).await;
            socket.write_all(response(200, &token().to_string(), "").as_bytes()).await.unwrap();
            socket.shutdown().await.unwrap();
            let (mut socket, _) = listener.accept().await.unwrap();
            read_request(&mut socket).await;
            ready.send(()).unwrap();
            release.await.unwrap();
            socket.write_all(response(200, r#"{"availableCredits":20}"#, "").as_bytes()).await.unwrap();
            socket.shutdown().await.unwrap();
        }).await.unwrap()
    })
}

#[tokio::test]
async fn routing_credit_requires_a_finite_nonnegative_ledger_value() {
    for body in ["{}", r#"{"availableCredits":-1}"#, r#"{"availableCredits":"100"}"#] {
        let (client, account) = scope(NativeEgress::Direct);
        let target = listener().await;
        let api = bound(&account, target.local_addr().unwrap(), NativeEgress::Direct);
        let worker = pair(target, body.into());
        assert_eq!(start(&client, &account, api, "team").await, Err(PreparationFailure::Rejected));
        assert_eq!(worker.await.unwrap().len(), 2);
        close(&client, &account).await;
    }
}

#[tokio::test]
async fn routing_credit_rejects_invalid_team_without_network_io() {
    let (client, account) = scope(NativeEgress::Direct);
    let target = listener().await;
    for team in ["", "../other", "team?override=1", "team/other", "team\r\n"] {
        let api = bound(&account, target.local_addr().unwrap(), NativeEgress::Direct);
        assert_eq!(start(&client, &account, api, team).await, Err(PreparationFailure::Rejected));
    }
    assert!(target.accept().now_or_never().is_none());
    close(&client, &account).await;
}

#[tokio::test]
async fn routing_credit_rejects_different_generation_without_network_io() {
    let (client, account) = scope(NativeEgress::Direct);
    let (other, wrong) = scope(NativeEgress::Direct);
    let target = listener().await;
    let api = bound(&account, target.local_addr().unwrap(), NativeEgress::Direct);
    assert_eq!(start(&other, &wrong, api, "team").await, Err(PreparationFailure::Rejected));
    assert!(target.accept().now_or_never().is_none());
    close(&client, &account).await;
    close(&other, &wrong).await;
}

fn start(client: &NativeAccountClient, account: &AccountScope,
    api: BoundControlClient, team: &str) -> PendingPreparation<f64> {
    let expected = account.clone();
    let team = TeamId(team.into());
    client.prepare_owned(account, Instant::now() + Duration::from_secs(5),
        move |session, control| async move {
            api.credit_balance(&expected, &session, &team, &control).await
        }).unwrap()
}

fn pair(listener: TcpListener, ledger: String) -> JoinHandle<Vec<String>> {
    tokio::spawn(async move {
        tokio::time::timeout(BOUND, async move {
        let mut requests = Vec::new();
        for body in [token().to_string(), ledger] {
            let (mut socket, _) = listener.accept().await.unwrap();
            requests.push(read_request(&mut socket).await);
            let _ = socket.write_all(response(200, &body, "").as_bytes()).await;
            let _ = socket.shutdown().await;
        }
            requests
        }).await.unwrap()
    })
}

#[tokio::test]
async fn routing_credit_uses_exact_session_team_and_egress() {
    for proxied in [false, true] {
        let target = listener().await;
        let proxy = listener().await;
        let address = target.local_addr().unwrap();
        let url = format!("http://{}", proxy.local_addr().unwrap());
        let egress = if proxied { NativeEgress::Proxy(&url) } else { NativeEgress::Direct };
        let (client, account) = scope(if proxied { NativeEgress::Proxy(&url) } else { NativeEgress::Direct });
        let api = bound(&account, address, egress);
        let (server, unused) = if proxied { (proxy, target) } else { (target, proxy) };
        let worker = pair(server, r#"{"availableCredits":12.5}"#.into());
        assert_eq!(start(&client, &account, api, "exact-team").await.unwrap(), 12.5);
        let requests = worker.await.unwrap();
        assert!(requests[0].contains("/auth/web/access-token HTTP/1.1"));
        assert!(requests[0].contains("cookie: session=fixture-only-cookie\r\n"));
        assert!(requests[1].contains("/web/teams/exact-team/ai-credits HTTP/1.1"));
        assert!(requests[1].contains("authorization: Bearer fixture-bearer\r\n"));
        assert!(!requests[1].contains("cookie:"));
        assert!(unused.accept().now_or_never().is_none());
        close(&client, &account).await;
    }
}

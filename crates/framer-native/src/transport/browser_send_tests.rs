use super::*;
use crate::transport::browser::{BrowserOutcome, BrowserRequest, BrowserResponse, BrowserTransport};
use futures::{FutureExt, StreamExt};
use tokio::sync::{mpsc, oneshot};

type Call = (BrowserRequest, NativeCancellation, oneshot::Sender<BrowserOutcome>);
struct Remote(mpsc::UnboundedSender<Call>);
impl BrowserTransport for Remote {
    fn send(&self, request: BrowserRequest, cancellation: NativeCancellation)
        -> futures::future::BoxFuture<'static, BrowserOutcome> {
        let (sender, receiver) = oneshot::channel();
        assert!(self.0.send((request, cancellation, sender)).is_ok());
        async move { receiver.await.unwrap_or(BrowserOutcome {
            response: Err("remote lost"), cleanup_confirmed: false,
        }) }.boxed()
    }
}

fn fixture() -> (NativeHttp, Session, mpsc::UnboundedReceiver<Call>) {
    let (sender, receiver) = mpsc::unbounded_channel();
    let session = Session { active_user: FramerUserId("user-a".into()), cookie_header: "session=private-a".into() };
    let client = NativeHttp::new(AccountId(uuid::Uuid::new_v4()), session.active_user.clone(),
        NativeHttpLimits::default(), NativeEgress::Direct).unwrap().with_browser(Arc::new(Remote(sender)));
    (client, session, receiver)
}

fn token(account: AccountId) -> FreshCastleToken {
    FreshCastleToken::from_sdk_output(account, CastleToken("fresh-output-a".into()))
        .with_device(Some("granted-device-a".into()))
}
fn success() -> BrowserOutcome {
    BrowserOutcome { cleanup_confirmed: true, response: Ok(BrowserResponse {
        status: 200, usage: Some("fixture-usage".into()), content_type: Some("text/plain".into()),
        body: b"fixture body".to_vec(),
    }) }
}

async fn closed(observer: NativeClosed) -> Result<(), NativeCloseError> {
    tokio::time::timeout(Duration::from_secs(1), observer).await
        .expect("browser cleanup waiter did not wake after the real join")
}

#[tokio::test]
async fn supplied_token_device_and_session_reach_the_selected_sender_once() {
    let (client, session, mut calls) = fixture();
    let body = serde_json::json!({"projectId": "fixture-project"});
    let mut request = client.ai_post_stream_tracked(&client.account, "/ai/v3/responses/",
        AccountRequestAuth::new(client.account, &session, "fixture-bearer"), &body,
        token(client.account), NativeCancellation::new());
    assert!(futures::poll!(&mut request.response).is_pending());
    let (input, _, reply) = calls.recv().await.unwrap();
    assert_eq!(input.account, client.account);
    assert_eq!(input.user, session.active_user.0);
    assert_eq!(input.device, "granted-device-a");
    assert_eq!(input.cookie_header, "session=private-a; __cuid=granted-device-a");
    assert_eq!(input.bearer, "fixture-bearer");
    let sent: Value = serde_json::from_slice(&input.wire).unwrap();
    assert_eq!(sent, serde_json::json!({"projectId": "fixture-project", "castleToken": "fresh-output-a"}));
    assert!(reply.send(success()).is_ok());
    let mut response = request.response.await.unwrap();
    assert_eq!(response.status, 200);
    assert_eq!(response.usage_header.as_deref(), Some("fixture-usage"));
    assert_eq!(response.body.next().await.unwrap().unwrap(), &b"fixture body"[..]);
    assert!(response.body.next().await.is_none());
    assert_eq!(closed(request.closed).await, Ok(()));
    assert!(calls.try_recv().is_err());
}

#[tokio::test]
async fn dropping_the_response_waiter_keeps_cleanup_pending_until_the_browser_joins() {
    let (client, session, mut calls) = fixture();
    let body = serde_json::json!({});
    let mut request = client.ai_post_stream_tracked(&client.account, "/ai/v3/responses/",
        AccountRequestAuth::new(client.account, &session, "fixture-bearer"), &body,
        token(client.account), NativeCancellation::new());
    assert!(futures::poll!(&mut request.response).is_pending());
    let (_, cancel, reply) = calls.recv().await.unwrap();
    drop(request.response);
    assert!(cancel.is_cancelled());
    assert!(tokio::time::timeout(Duration::from_millis(25), &mut request.closed).await.is_err());
    assert!(reply.send(BrowserOutcome { response: Err("cancelled"), cleanup_confirmed: true }).is_ok());
    assert_eq!(closed(request.closed).await, Ok(()));
}

#[tokio::test]
async fn an_unconfirmed_browser_reply_cannot_become_successful_native_cleanup() {
    let (client, session, mut calls) = fixture();
    let body = serde_json::json!({});
    let mut request = client.ai_post_stream_tracked(&client.account, "/ai/v3/responses/",
        AccountRequestAuth::new(client.account, &session, "fixture-bearer"), &body,
        token(client.account), NativeCancellation::new());
    assert!(futures::poll!(&mut request.response).is_pending());
    let (_, _, reply) = calls.recv().await.unwrap();
    let mut outcome = success();
    outcome.cleanup_confirmed = false;
    assert!(reply.send(outcome).is_ok());
    assert!(request.response.await.is_err());
    assert_eq!(closed(request.closed).await, Err(NativeCloseError::AcknowledgementLost));
}

#[tokio::test]
async fn missing_device_crossed_account_and_expired_tokens_never_reach_the_browser() {
    let (client, session, mut calls) = fixture();
    let mut expired = token(client.account);
    expired.received_at = Instant::now() - Duration::from_secs(60);
    let cases = [FreshCastleToken::from_sdk_output(client.account, CastleToken("untagged".into())),
        token(AccountId(uuid::Uuid::new_v4())), expired];
    for supplied in cases {
        assert!(client.ai_post(&client.account, "/ai/v3/responses/",
            AccountRequestAuth::new(client.account, &session, "fixture-bearer"),
            &serde_json::json!({}), supplied, NativeCancellation::new()).await.is_err());
    }
    let conflict = Session { cookie_header: "session=private-a; __cuid=other-device".into(), ..session };
    assert!(client.ai_post(&client.account, "/ai/v3/responses/",
        AccountRequestAuth::new(client.account, &conflict, "fixture-bearer"),
        &serde_json::json!({}), token(client.account), NativeCancellation::new()).await.is_err());
    assert!(calls.try_recv().is_err());
}

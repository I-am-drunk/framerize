use super::*;
use crate::account::client::{RequestContext, SuppliedAuth};
use crate::transport::browser::{BrowserOutcome, BrowserRequest, BrowserResponse, BrowserTransport};
use crate::transport::http::{NativeCancellation, NativeHttpLimits};
use futures::StreamExt;

struct Receiver(Arc<Mutex<Vec<BrowserRequest>>>);
impl BrowserTransport for Receiver {
    fn send(&self, request: BrowserRequest, _: NativeCancellation)
        -> BoxFuture<'static, BrowserOutcome> {
        self.0.lock().unwrap().push(request);
        async { BrowserOutcome { cleanup_confirmed: true, response: Ok(BrowserResponse {
            status: 200, usage: None, content_type: Some("text/plain".into()), body: b"received".to_vec(),
        }) } }.boxed()
    }
}

#[tokio::test]
async fn offline_minted_device_and_output_reach_the_account_browser_unchanged() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let client = NativeAccountClient::with_browser_transport(Arc::new(Receiver(calls.clone()))).unwrap();
    let account = client.open_account(framer_core::ids::AccountId::new(), fixture::session(),
        NativeHttpLimits::default(), NativeEgress::Direct).unwrap();
    let listener = listener().await;
    let minted = minted(&account, "actual-fixture-output".into()).with_saved_device("minted-device".into());
    let minter = Arc::new(Minter::new(&account, PreparationResult::finished(Ok(minted))));
    let source = source(&account, listener.local_addr().unwrap(), minter.clone());
    let auth_server = auth(listener);
    let selected = account.clone();
    let prepared = client.prepare_owned(&account, tokio::time::Instant::now() + BOUND,
        move |session, control| async move {
            source.issue_for_target(&selected, &session, &target(), &control).await
        }).unwrap();
    let prepared = tokio::time::timeout(BOUND, prepared).await.unwrap().unwrap();
    assert!(fixture::result(auth_server).await.contains("__cuid=minted-device"));
    let auth = SuppliedAuth::new(&account, prepared.material.bearer, prepared.material.castle)
        .with_current_cookies(prepared.current_cookies);
    let context = RequestContext {
        dialect: framer_api::proxy::Dialect::OpenRouterResponses,
        project: ProjectId("fixture-project".into()), feature_category: "fixture-category".into(),
        feature_name: "fixture-feature".into(), step: "fixture-step".into(),
        chat_session: "fixture-conversation".into(), step_index: 0,
        turn_id: "fixture-turn".into(), user_prompt: "fixture".into(),
        payload: serde_json::json!({"model":"fixture-model", "input":"unchanged", "stream":false}),
    };
    let pending = client.start(&account, context, auth, tokio::time::Instant::now() + BOUND,
        "application/octet-stream".into()).unwrap();
    let mut response = tokio::time::timeout(BOUND, pending.response).await.unwrap().unwrap();
    let mut body = Vec::new();
    while let Some(chunk) = response.proxy.stream.next().await {
        body.extend_from_slice(&chunk.unwrap());
    }
    assert_eq!(body, b"received");
    assert_eq!(tokio::time::timeout(BOUND, pending.cleanup).await.unwrap(),
        Ok(crate::account::owner::EndReason::Response));
    {
        let calls = calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        let request = &calls[0];
        assert_eq!(request.account, account.account());
        assert_eq!(request.device, "minted-device");
        assert_eq!(request.cookie_header, "session=fixture-only-cookie; __cuid=minted-device");
        assert_eq!(request.bearer, "fixture-bearer");
        let wire: serde_json::Value = serde_json::from_slice(&request.wire).unwrap();
        assert_eq!(wire["castleToken"], "actual-fixture-output");
        assert_eq!(wire["request"]["input"], "unchanged");
    }
    assert_eq!(minter.calls.load(Ordering::SeqCst), 1);
    assert_eq!(minter.joined.load(Ordering::SeqCst), 1);
    close(&client, &account).await;
}

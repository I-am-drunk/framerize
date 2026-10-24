use super::*;
use crate::experimental_native::native_http::{AccountRequestAuth, NativeCancellation,
    NativeHttp, NativeHttpLimits};
use tokio::io::AsyncWriteExt;
use std::time::Duration;

#[tokio::test]
async fn bearer_wait_counts_toward_browser_token_expiry() {
    let (client, account) = scope(NativeEgress::Direct);
    let server = listener().await;
    let api = Arc::new(bound(&account, server.local_addr().unwrap(), NativeEgress::Direct));
    let worker = tokio::spawn(async move {
        let (mut socket, _) = server.accept().await.unwrap();
        read_request(&mut socket).await;
        tokio::time::sleep(Duration::from_millis(60)).await;
        socket.write_all(response(200, &token().to_string(), "").as_bytes()).await.unwrap();
        socket.shutdown().await.unwrap();
    });
    let source = BoundBrowserSessionSource::new(api, Browser::new(false));
    let expected = account.clone();
    let pending = client.prepare_owned(&account, tokio::time::Instant::now() + BOUND,
        move |session, control| async move {
            source.issue_for_target(&expected, &session, &target(), &control).await
        }).unwrap();
    let prepared = tokio::time::timeout(BOUND, pending).await.unwrap().unwrap();
    tokio::time::timeout(BOUND, worker).await.unwrap().unwrap();
    let sink = listener().await;
    let native = NativeHttp::for_loopback(account.account(), session().active_user,
        sink.local_addr().unwrap(), NativeHttpLimits {
            token_max_age: Duration::from_millis(20), ..NativeHttpLimits::default()
        }).unwrap();
    let current = prepared.current_cookies.unwrap().into_session(&session()).unwrap();
    let result = tokio::time::timeout(BOUND, native.ai_post(&account.account(), target().path,
        AccountRequestAuth::new(account.account(), &current, &prepared.material.bearer),
        &serde_json::json!({"stream": false}), prepared.material.castle,
        NativeCancellation::new())).await.unwrap();
    assert_eq!(result.unwrap_err(), "native-http: fresh Castle token required");
    assert!(sink.accept().now_or_never().is_none());
    close(&client, &account).await;
}

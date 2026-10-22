use super::*;
#[path = "control_authorization_fixture.rs"]
mod fixture;
use fixture::*;

#[tokio::test]
async fn bound_source_seeds_the_same_account_after_private_auth_completes() {
    let (client, account) = scope(NativeEgress::Direct);
    let server = listener().await;
    let api = Arc::new(bound(&account, server.local_addr().unwrap(), NativeEgress::Direct));
    let worker = serve_once(server, response(200, &token().to_string(), ""));
    let browser = Arc::new(RecordingBrowser::default());
    let source = BoundBrowserSessionSource::new(api, browser.clone());
    assert_eq!(tokio::time::timeout(BOUND, prepare(&client, &account, source)).await.unwrap().unwrap(),
        "fixture-bearer");
    result(worker).await;
    assert_eq!(*browser.calls.lock().unwrap(), vec![("seed", account.account()), ("token", account.account())]);
    close(&client, &account).await;
}

#[tokio::test]
async fn cancellation_joins_private_auth_before_account_close_and_skips_browser() {
    let (client, account) = scope(NativeEgress::Direct);
    let server = listener().await;
    let api = Arc::new(bound(&account, server.local_addr().unwrap(), NativeEgress::Direct));
    let (entered, entry) = tokio::sync::oneshot::channel();
    let (release, released) = tokio::sync::oneshot::channel();
    let worker = tokio::spawn(async move {
        tokio::time::timeout(BOUND, async move {
            let (mut socket, _) = server.accept().await.unwrap();
            let request = read_request(&mut socket).await;
            entered.send(()).unwrap();
            released.await.unwrap();
            use tokio::io::AsyncWriteExt;
            socket.write_all(response(200, &token().to_string(), "").as_bytes()).await.unwrap();
            socket.shutdown().await.unwrap();
            request
        }).await.unwrap()
    });
    let browser = Arc::new(RecordingBrowser::default());
    let pending = prepare(&client, &account, BoundBrowserSessionSource::new(api, browser.clone()));
    tokio::time::timeout(BOUND, entry).await.unwrap().unwrap();
    pending.cancellation().cancel();
    let mut completed = Box::pin(pending.completion());
    let mut closed = Box::pin(client.close_account(&account).unwrap());
    assert!(futures::poll!(completed.as_mut()).is_pending());
    assert!(futures::poll!(closed.as_mut()).is_pending());
    drop(pending);
    release.send(()).unwrap();
    result(worker).await;
    tokio::time::timeout(BOUND, completed).await.unwrap().unwrap();
    tokio::time::timeout(BOUND, closed).await.unwrap().unwrap();
    assert!(browser.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn oversized_declared_or_chunked_token_response_is_bounded() {
    let (client, account) = scope(NativeEgress::Direct);
    let bytes = "x".repeat(64 * 1024 + 1);
    let declared = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", bytes.len());
    let chunked = format!("HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n{:x}\r\n{}\r\n0\r\n\r\n",
        bytes.len(), bytes);
    for reply in [declared, chunked] {
        let server = listener().await;
        let api = bound(&account, server.local_addr().unwrap(), NativeEgress::Direct);
        let worker = serve_once(server, reply);
        assert_eq!(api.bearer(&account, &session()).await.unwrap_err(),
            "control authorization: response exceeds limit");
        result(worker).await;
    }
    close(&client, &account).await;
}

#[tokio::test]
async fn incomplete_token_body_is_not_accepted_or_included_in_error() {
    let (client, account) = scope(NativeEgress::Direct);
    let server = listener().await;
    let api = bound(&account, server.local_addr().unwrap(), NativeEgress::Direct);
    let worker = serve_once(server,
        "HTTP/1.1 200 OK\r\nContent-Length: 999\r\nConnection: close\r\n\r\nprivate-partial-fixture".into());
    assert_eq!(api.bearer(&account, &session()).await.unwrap_err(), "control authorization: body failed");
    result(worker).await;
    close(&client, &account).await;
}

#[tokio::test]
async fn failed_explicit_proxy_never_retries_directly() {
    let proxy = listener().await;
    let target = listener().await;
    let url = format!("http://{}", proxy.local_addr().unwrap());
    let (client, account) = scope(NativeEgress::Proxy(&url));
    let api = bound(&account, target.local_addr().unwrap(), NativeEgress::Proxy(&url));
    let worker = serve_once(proxy, response(502, "fixture proxy rejection", ""));
    assert_eq!(api.bearer(&account, &session()).await.unwrap_err(), "control authorization: HTTP 502");
    result(worker).await;
    assert!(target.accept().now_or_never().is_none());
    close(&client, &account).await;
}

#[tokio::test]
async fn replacement_generation_and_wrong_user_cannot_use_old_authorization_client() {
    let (client, account) = scope(NativeEgress::Direct);
    let server = listener().await;
    let api = bound(&account, server.local_addr().unwrap(), NativeEgress::Direct);
    let mut wrong_user = session();
    wrong_user.active_user.0.push_str("-different");
    assert!(api.bearer(&account, &wrong_user).await.is_err());
    close(&client, &account).await;
    let replacement = client.open_account(account.account(), session(),
        crate::experimental_native::native_http::NativeHttpLimits::default(), NativeEgress::Direct).unwrap();
    assert!(api.bearer(&replacement, &session()).await.is_err());
    assert!(server.accept().now_or_never().is_none());
    close(&client, &replacement).await;
}

#[tokio::test]
async fn auth_errors_preserve_status_without_disclosing_response_or_credentials() {
    let (client, account) = scope(NativeEgress::Direct);
    for status in [401, 403, 429, 500] {
        let server = listener().await;
        let api = bound(&account, server.local_addr().unwrap(), NativeEgress::Direct);
        let worker = serve_once(server, response(status,
            "private-fixture-response session=fixture-only-cookie fixture-bearer", ""));
        assert_eq!(api.bearer(&account, &session()).await.unwrap_err(),
            format!("control authorization: HTTP {status}"));
        result(worker).await;
    }
    close(&client, &account).await;
}

#[tokio::test]
async fn malformed_expired_empty_and_oversized_tokens_are_rejected() {
    let (client, account) = scope(NativeEgress::Direct);
    let mut expired = token();
    expired["expiresAt"] = serde_json::json!("2000-01-01T00:00:00Z");
    let mut invalid_expiry = token();
    invalid_expiry["expiresAt"] = serde_json::json!("not-a-timestamp");
    let mut empty = token();
    empty["accessToken"] = serde_json::json!("");
    let mut oversized = token();
    oversized["accessToken"] = serde_json::json!("x".repeat(16 * 1024 + 1));
    for (body, expected) in [
        ("not-json".to_owned(), "invalid response"),
        (expired.to_string(), "expired token"),
        (invalid_expiry.to_string(), "invalid expiry"),
        (empty.to_string(), "invalid token"),
        (oversized.to_string(), "invalid token"),
    ] {
        let server = listener().await;
        let api = bound(&account, server.local_addr().unwrap(), NativeEgress::Direct);
        let worker = serve_once(server, response(200, &body, ""));
        assert_eq!(api.bearer(&account, &session()).await.unwrap_err(),
            format!("control authorization: {expected}"));
        result(worker).await;
    }
    close(&client, &account).await;
}

#[tokio::test]
async fn explicit_direct_auth_sends_only_the_bound_cookie_to_token_path() {
    let (client, account) = scope(NativeEgress::Direct);
    let server = listener().await;
    let api = bound(&account, server.local_addr().unwrap(), NativeEgress::Direct);
    let worker = serve_once(server, response(200, &token().to_string(), ""));
    assert_eq!(api.bearer(&account, &session()).await.unwrap(), "fixture-bearer");
    let request = result(worker).await.to_ascii_lowercase();
    assert!(request.starts_with("get /auth/ai/access-token http/1.1\r\n"));
    assert!(request.contains("\r\ncookie: session=fixture-only-cookie\r\n"));
    assert!(request.contains("\r\nx-requested-by: framer\r\n"));
    assert!(!request.contains("authorization:"));
    close(&client, &account).await;
}

#[tokio::test]
async fn explicit_proxy_auth_uses_the_same_scope_proxy_without_direct_fallback() {
    let proxy = listener().await;
    let target = listener().await;
    let url = format!("http://{}", proxy.local_addr().unwrap());
    let (client, account) = scope(NativeEgress::Proxy(&url));
    let api = bound(&account, target.local_addr().unwrap(), NativeEgress::Proxy(&url));
    let worker = serve_once(proxy, response(200, &token().to_string(), ""));
    assert_eq!(api.bearer(&account, &session()).await.unwrap(), "fixture-bearer");
    let request = result(worker).await;
    assert!(request.starts_with(&format!("GET http://{}/auth/ai/access-token HTTP/1.1\r\n",
        target.local_addr().unwrap())));
    assert!(target.accept().now_or_never().is_none());
    close(&client, &account).await;
}

#[tokio::test]
async fn route_mismatch_and_closed_generation_are_rejected_before_client_creation() {
    let (client, account) = scope(NativeEgress::Direct);
    assert!(BoundControlClient::new(account.clone(), NativeEgress::Proxy("http://127.0.0.1:1")).is_err());
    close(&client, &account).await;
    assert!(BoundControlClient::new(account, NativeEgress::Direct).is_err());
}

#[tokio::test]
async fn authorization_redirect_does_not_forward_cookie_or_contact_another_origin() {
    let (client, account) = scope(NativeEgress::Direct);
    let server = listener().await;
    let other = listener().await;
    let api = bound(&account, server.local_addr().unwrap(), NativeEgress::Direct);
    let redirect = format!("Location: http://{}/capture\r\n", other.local_addr().unwrap());
    let worker = serve_once(server, response(302, "", &redirect));
    assert_eq!(api.bearer(&account, &session()).await.unwrap_err(), "control authorization: HTTP 302");
    result(worker).await;
    assert!(other.accept().now_or_never().is_none());
    close(&client, &account).await;
}

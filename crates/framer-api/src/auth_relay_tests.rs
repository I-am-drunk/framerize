use super::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

async fn reply(status: u16, body: &str, path: &str)
    -> (Result<Option<AiToken>, String>, String) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let wire = format!("HTTP/1.1 {status} Fixture\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut request = Vec::new();
        while !request.windows(4).any(|part| part == b"\r\n\r\n") {
            let mut chunk = [0; 1024];
            let n = socket.read(&mut chunk).await.unwrap();
            assert!(n > 0 && request.len() < 8192);
            request.extend_from_slice(&chunk[..n]);
        }
        socket.write_all(wire.as_bytes()).await.unwrap();
        String::from_utf8(request).unwrap()
    });
    let session = Session { active_user: framer_core::FramerUserId("fixture-user".into()),
        cookie_header: "fixture-cookie".into() };
    let result = relay_mint_from(&session, &format!("http://{address}{path}"), None).await;
    let request = tokio::time::timeout(std::time::Duration::from_secs(2), server)
        .await.unwrap().unwrap();
    (result, request)
}

#[tokio::test]
async fn non_success_keeps_http_cause_instead_of_a_json_error() {
    for status in [401, 403, 429, 503] {
        let (result, _) = reply(status, "<html>unavailable</html>", "/relay").await;
        assert_eq!(result.err().unwrap(), format!("JWT relay returned HTTP {status}"));
    }
}

#[tokio::test]
async fn cache_busting_preserves_existing_query_and_omitted_account_is_normal() {
    for path in ["/relay", "/relay?ref=default"] {
        let (result, request) = reply(200, r#"{"accounts":{}}"#, path).await;
        assert!(matches!(result, Ok(None)));
        let separator = if path.contains('?') { '&' } else { '?' };
        assert!(request.starts_with(&format!("GET {path}{separator}r=")));
        assert!(!request.to_ascii_lowercase().contains("cookie:"));
    }
}

#[tokio::test]
async fn malformed_relay_inventory_is_not_an_absent_account() {
    let (result, _) = reply(200, r#"{"error":"unavailable"}"#, "/relay").await;
    assert_eq!(result.err().unwrap(), "JWT relay response has no accounts object");
}

#[tokio::test]
async fn relay_retains_session_binding_and_rejects_expired_entries() {
    for (expiry, usable) in [("2099-01-01T00:00:00Z", true), ("2000-01-01T00:00:00Z", false)] {
        let body = serde_json::json!({"accounts": {"fixture-user": {
            "accessToken": "inert-fixture-value", "expiresAt": expiry }}}).to_string();
        let (result, _) = reply(200, &body, "/relay").await;
        if usable {
            let token = result.ok().flatten().unwrap();
            assert_eq!(token.bearer, "inert-fixture-value");
            assert_eq!(token.session.active_user.0, "fixture-user");
            assert_eq!(token.session.cookie_header, "fixture-cookie");
        } else {
            assert_eq!(result.err().unwrap(), "relay: token is expired or within the refresh window");
        }
    }
}

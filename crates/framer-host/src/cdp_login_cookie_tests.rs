//! Login extraction against an inert cookie-aware CDP peer.
use super::*;
use std::time::Duration;
use tokio::time::timeout;

fn cookie(domain: &str, value: &str) -> Value {
    json!({"name": "session", "value": value, "domain": domain, "path": "/"})
}

fn visible_cookies(cookies: &[Value], urls: &Value) -> Vec<Value> {
    cookies.iter().filter(|cookie| {
        let domain = cookie["domain"].as_str().unwrap();
        let path = cookie["path"].as_str().unwrap();
        urls.as_array().unwrap().iter().any(|url| {
            let address = url.as_str().unwrap().strip_prefix("https://").unwrap();
            let (host, tail) = address.split_once('/').unwrap_or((address, ""));
            (host == domain || (domain.starts_with('.')
                && (host == &domain[1..] || host.ends_with(domain))))
                && format!("/{tail}").starts_with(path)
        })
    }).cloned().collect()
}

async fn complete(cookies: Vec<Value>) -> (String, Value) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let peer = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_async(socket).await.unwrap();
        let mut calls = Vec::new();
        while let Some(frame) = socket.next().await {
            let frame = frame.unwrap();
            if frame.is_close() { break; }
            let command: Value = serde_json::from_str(frame.to_text().unwrap()).unwrap();
            let method = command["method"].as_str().unwrap();
            let result = if method == "Network.getCookies" {
                calls.push("cookies");
                json!({"cookies": visible_cookies(&cookies, &command["params"]["urls"])})
            } else {
                assert_eq!(method, "Runtime.evaluate");
                let expression = command["params"]["expression"].as_str().unwrap();
                let body = if expression.contains("/auth/v4/continue") {
                    calls.push("continue");
                    json!({})
                } else {
                    assert!(expression.contains("/web/users/me"));
                    calls.push("me");
                    let applicable = visible_cookies(&cookies, &json!([API_BASE]));
                    let session = applicable.iter().find(|cookie| cookie["name"] == "session")
                        .unwrap()["value"].as_str().unwrap();
                    json!({"id": format!("user-{session}")})
                };
                json!({"result": {"type": "object", "value": {
                    "status": 200, "body": body.to_string()
                }}})
            };
            socket.send(Message::Text(json!({"id": command["id"], "result": result})
                .to_string().into())).await.unwrap();
        }
        calls
    });
    let (ws, _) = connect_async(format!("ws://{address}")).await.unwrap();
    let account = AccountId::new();
    let bridge = CdpBridge::with_engine("unused", "/unused-login-fixture", EngineKind::Attach);
    bridge.instances.lock().await.insert(account, Arc::new(Mutex::new(
        CdpInstance { child: None, ws, next_id: 0, pending: Default::default() })));
    let result = timeout(Duration::from_secs(2),
        bridge.auth_complete(&account, "fixture-email-link")).await;
    let closed = timeout(Duration::from_secs(2), bridge.shutdown()).await;
    let joined = timeout(Duration::from_secs(2), peer).await;
    closed.unwrap().unwrap();
    assert_eq!(joined.unwrap().unwrap(), ["continue", "cookies", "me"]);
    assert!(bridge.instances.lock().await.is_empty());
    result.unwrap().unwrap()
}

#[tokio::test]
async fn website_sessions_do_not_replace_the_api_login_cookie() {
    for domain in [".framer.com", "api.framer.com"] {
        let mut login_only = cookie("framer.com", "login-only");
        login_only["path"] = json!("/login");
        let result = complete(vec![cookie("framer.com", "website-only"), login_only,
            cookie(domain, "api-session")]).await;
        assert_eq!(result, ("session=api-session".into(), json!({"id": "user-api-session"})));
    }
}

#[tokio::test]
async fn shared_framer_session_remains_available_to_api_login() {
    let result = complete(vec![cookie(".framer.com", "shared-session")]).await;
    assert_eq!(result, ("session=shared-session".into(), json!({"id": "user-shared-session"})));
}

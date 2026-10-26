use super::*;

#[test]
fn browser_reply_requires_supported_version_status_headers_and_bounded_utf8() {
    let valid = json!({"version":1, "cleanup_confirmed":true, "response":{
        "status":403, "usage":null, "content_type":"application/json", "body":"😀"}});
    let accepted = decode(valid.clone(), 4);
    assert!(accepted.cleanup_confirmed);
    assert_eq!(accepted.response.unwrap().status, 403);
    assert!(decode(valid.clone(), 3).response.is_err());
    for (field, value) in [("status", json!(0)), ("status", json!(600)),
        ("usage", json!([])), ("content_type", Value::String("x".repeat(16385))),
        ("body", Value::Null)] {
        let mut invalid = valid.clone();
        invalid["response"][field] = value;
        assert!(decode(invalid, 4).response.is_err());
    }
    let mut invalid = valid.clone();
    invalid["version"] = json!(2);
    assert!(decode(invalid, 4).response.is_err());
    let mut unconfirmed = valid;
    unconfirmed["cleanup_confirmed"] = json!(false);
    assert!(!decode(unconfirmed, 4).cleanup_confirmed);
}

fn request(case: &str) -> BrowserRequest {
    use std::time::{SystemTime, UNIX_EPOCH};
    BrowserRequest {
        account: framer_core::ids::AccountId::new(), user: "fixture-user".into(),
        cookie_header: "session=fixture-login; __cuid=fixture-device".into(),
        device: "fixture-device".into(), path: "/ai/v3/responses/".into(),
        bearer: "fixture-bearer".into(), proxy: None,
        wire: serde_json::to_vec(&json!({"castleToken":"fixture-native-output", "case":case})).unwrap(),
        expires_unix_ms: (SystemTime::now().duration_since(UNIX_EPOCH).unwrap()
            + Duration::from_secs(30)).as_millis().try_into().unwrap(),
        timeout_ms: 10000, response_limit: 1024 * 1024,
    }
}

#[tokio::test]
#[ignore = "requires the isolated real-browser fixture and its explicit environment"]
async fn actual_pipe_keeps_supplied_result_and_cancellation_owned_through_browser_close() {
    let script = std::path::PathBuf::from(std::env::var("FRAMERIZE_BROWSER_FIXTURE_SCRIPT")
        .expect("isolated fixture script required"));
    let root = std::env::temp_dir().join(format!("native-browser-pipe-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&root).unwrap();
    let host = BrowserHost::new(&script, &root).unwrap();
    let checks = std::panic::AssertUnwindSafe(async {
        tokio::time::timeout(Duration::from_secs(20), host.prepare()).await.unwrap().unwrap();
        let received = tokio::time::timeout(Duration::from_secs(15),
            host.send(request("bridge-exact"), NativeCancellation::new())).await.unwrap();
        assert!(received.cleanup_confirmed);
        let received = received.response.unwrap();
        assert_eq!(received.status, 200);
        assert_eq!(received.usage.as_deref(), Some("synthetic-usage"));
        assert_eq!(received.content_type.as_deref(), Some("text/event-stream"));
        assert_eq!(received.body, vec![b'z'; 524288]);
        let cancellation = NativeCancellation::new();
        let pending = tokio::spawn(host.send(request("bridge-held"), cancellation.clone()));
        tokio::time::timeout(Duration::from_secs(10), async {
            while !root.join("bridge-held").exists() {
                assert!(!pending.is_finished(), "held request ended before reaching the server");
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        }).await.unwrap();
        cancellation.cancel();
        let stopped = tokio::time::timeout(Duration::from_secs(15), pending).await.unwrap().unwrap();
        assert!(stopped.cleanup_confirmed);
        assert!(stopped.response.is_err());
    }).catch_unwind().await;
    let shutdown = tokio::time::timeout(Duration::from_secs(20), host.shutdown()).await;
    assert!(matches!(shutdown, Ok(Ok(()))), "browser process shutdown was unconfirmed");
    if let Err(panic) = checks { std::panic::resume_unwind(panic); }
    let receipt: Value = serde_json::from_slice(&std::fs::read(root.join("fixture-receipt.json")).unwrap()).unwrap();
    assert_eq!(receipt, json!({"posts":["bridge-exact", "bridge-held"],
        "results":[true, true], "errors":[], "contexts":0, "browser_closed":true,
        "jobs":0, "exchanges":0, "uncertain":0}));
    std::fs::remove_dir_all(root).unwrap();
}

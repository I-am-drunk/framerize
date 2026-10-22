//! HTTP forwarding evidence. Real rendering is covered by composed_model_tests.
use super::*;
use futures::StreamExt;
use serde_json::json;

#[path = "native_http_stream_fixture.rs"]
mod fixture;
use fixture::{headers, Control, Fixture, BOUND};

fn payload(dialect: ClientDialect, model: &str) -> Value {
    let mut payload = json!({"model":model,"stream":true,"max_tokens":32,
        "reasoning":{"effort":"max"},"fixture_extension":{"keep":true}});
    if dialect == ClientDialect::OpenAiResponses {
        payload["input"] = json!([{"role":"user","content":"fixture"}]);
    } else {
        payload["messages"] = json!([{"role":"user","content":"fixture"}]);
        if dialect == ClientDialect::OpenAiChat {
            payload["stream_options"] = json!({"include_usage":true});
        }
    }
    payload
}

async fn finish(response: Response, control: &mut Control) {
    let mut body = response.into_body().into_data_stream();
    assert!(tokio::time::timeout(BOUND, body.next()).await.unwrap().is_none());
    control.release().await;
}

#[tokio::test(flavor = "current_thread")]
async fn http_model_aliases_remain_local_to_each_account_invocation() {
    let fixture = Fixture::new(2, Duration::from_secs(1), None);
    let mut first = fixture.enqueue(0, headers(200, "application/json", None), "text/event-stream");
    let mut second = fixture.enqueue(1, headers(200, "application/json", None), "text/event-stream");
    first.open(); first.eof(); first.finish(Ok(()));
    second.open(); second.eof(); second.finish(Ok(()));
    let response_a = fixture.request("/openai/v1/responses",
        payload(ClientDialect::OpenAiResponses, "gpt-4.1-ultrafast"), 0, "session-a", false).await;
    let response_b = fixture.request("/openai/v1/responses",
        payload(ClientDialect::OpenAiResponses, "gpt-4.1"), 1, "session-b", false).await;
    assert_eq!(response_a.status(), StatusCode::OK);
    assert_eq!(response_b.status(), StatusCode::OK);
    let records = fixture.records();
    assert_eq!(records.len(), 2);
    assert_eq!(records[0].client_model.as_deref(), Some("gpt-4.1-ultrafast"));
    assert_eq!(records[1].client_model.as_deref(), Some("gpt-4.1"));
    assert_eq!(records[0].identity.account, fixture.accounts[0].id);
    assert_eq!(records[1].identity.account, fixture.accounts[1].id);
    assert_eq!(records[0].identity.chat_session, "session-a");
    assert_eq!(records[1].identity.chat_session, "session-b");
    finish(response_a, &mut first).await;
    assert!(fixture.slot_free(0, "gpt-4.1").await);
    assert!(!fixture.slot_free(1, "gpt-4.1").await);
    finish(response_b, &mut second).await;
    assert!(fixture.slot_free(1, "gpt-4.1").await);
    fixture.assert_legacy_unused();
}

#[tokio::test(flavor = "current_thread")]
async fn http_raw_does_not_apply_a_composed_model_alias() {
    for (path, dialect) in [
        ("/paid/anthropic/v1/messages", ClientDialect::AnthropicMessages),
        ("/paid/openai/v1/responses", ClientDialect::OpenAiResponses),
        ("/paid/openai/v1/chat/completions", ClientDialect::OpenAiChat),
    ] {
        let fixture = Fixture::new(1, Duration::from_secs(1), None);
        let mut control = fixture.enqueue(0,
            headers(200, "application/json", None), "application/json");
        const EXACT: &[u8] = b"{\"model\":\"upstream-exact\"}";
        control.open(); control.push(EXACT); control.eof(); control.finish(Ok(()));
        let response = fixture.request(path, payload(dialect, "gpt-4.1-ultrafast"),
            0, "raw-session", false).await;
        assert_eq!(response.status(), StatusCode::OK);
        let records = fixture.records();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].method, "raw_stream");
        assert!(records[0].client_model.is_none());
        assert_eq!(records[0].payload["model"], "gpt-4.1");
        assert_eq!(records[0].payload["service_tier"], "ultrafast");
        assert_eq!(records[0].payload["reasoning"]["effort"], "max");
        let mut body = response.into_body().into_data_stream();
        assert_eq!(tokio::time::timeout(BOUND, body.next()).await.unwrap()
            .unwrap().unwrap().as_ref(), EXACT);
        assert!(tokio::time::timeout(BOUND, body.next()).await.unwrap().is_none());
        control.release().await;
        fixture.assert_legacy_unused();
    }
}

#[tokio::test(flavor = "current_thread")]
async fn http_composed_passes_requested_alias_separately_in_every_dialect() {
    for (path, dialect) in [
        ("/anthropic/v1/messages", ClientDialect::AnthropicMessages),
        ("/openai/v1/responses", ClientDialect::OpenAiResponses),
        ("/openai/v1/chat/completions", ClientDialect::OpenAiChat),
    ] {
        for alias in ["gpt-4.1", "gpt-4.1-ultrafast"] {
            let fixture = Fixture::new(1, Duration::from_secs(1), None);
            let mut control = fixture.enqueue(0,
                headers(200, "application/json", None), "text/event-stream");
            control.open(); control.eof(); control.finish(Ok(()));
            let response = fixture.request(path, payload(dialect, alias), 0, "model-session", false).await;
            assert_eq!(response.status(), StatusCode::OK);
            let records = fixture.records();
            assert_eq!(records.len(), 1);
            let record = &records[0];
            assert_eq!(record.method, "composed_stream");
            assert_eq!(record.client_model.as_deref(), Some(alias));
            assert_eq!(record.payload["model"], "gpt-4.1");
            assert_eq!(record.payload["reasoning"]["effort"], "max");
            if alias.ends_with("-ultrafast") {
                assert_eq!(record.payload["service_tier"], "ultrafast");
            }
            assert_eq!(record.payload["fixture_extension"], json!({"keep":true}));
            assert_eq!(record.composed.unwrap().dialect, dialect);
            assert_eq!(record.identity.chat_session, "model-session");
            finish(response, &mut control).await;
            fixture.assert_legacy_unused();
        }
    }
}

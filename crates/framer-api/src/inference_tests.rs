use super::*;
use async_trait::async_trait;
use framer_core::ids::FramerUserId;
use framer_core::port::CastleToken;
use std::sync::atomic::{AtomicUsize, Ordering};

struct Fixture {
    account: AccountId,
    body: String,
    posts: AtomicUsize,
}

#[async_trait]
impl BrowserBridge for Fixture {
    async fn castle_token(&self, _: &AccountId) -> Result<CastleToken, String> {
        panic!("unexpected mint")
    }
    async fn auth_begin(&self, _: &AccountId, _: &str) -> Result<(), String> {
        panic!("unexpected login")
    }
    async fn auth_complete(&self, _: &AccountId, _: &str)
        -> Result<(String, serde_json::Value), String> {
        panic!("unexpected login")
    }
    async fn seed_session(&self, account: &AccountId, cookie: &str) -> Result<(), String> {
        assert_eq!(account, &self.account);
        assert_eq!(cookie, "fixture-only");
        Ok(())
    }
    async fn ai_post(&self, account: &AccountId, _: &str, _: &str, _: &serde_json::Value)
        -> Result<(u16, Option<String>, String), String> {
        assert_eq!(account, &self.account);
        self.posts.fetch_add(1, Ordering::Relaxed);
        Ok((200, Some("remaining=7".into()), self.body.clone()))
    }
}

async fn send_body(body: String) -> Result<Send, String> {
    let account = AccountId::new();
    let browser = Fixture { account, body, posts: AtomicUsize::new(0) };
    let session = Session {
        active_user: FramerUserId("fixture-user".into()),
        cookie_header: "fixture-only".into(),
    };
    let token = AiToken { bearer: "fixture-only".into(), expires_at_ms: i64::MAX };
    let request = InferenceRequest { prompt: "fixture prompt".into(), model: None, chat_session: None };
    let result = send(&browser, &session, &account, &token, &request,
        "fixture-chat", &ProjectId("fixture-project".into())).await;
    assert_eq!(browser.posts.load(Ordering::Relaxed), 1);
    result
}

fn event(value: serde_json::Value) -> String {
    format!("data: {value}\n\n")
}

fn delta(text: &str) -> String {
    event(json!({"type":"response.output_text.delta", "delta":text}))
}

fn completed() -> String {
    event(json!({"type":"response.completed",
        "response":{"usage":{"input_tokens":3,"output_tokens":5}}}))
}

#[tokio::test]
async fn completed_stream_preserves_text_usage_and_credits() {
    let body = format!("{}{}{}data: [DONE]\n\n", delta("hello "), delta("world"), completed());
    let Send::Done(outcome) = send_body(body).await.unwrap() else { panic!("expected completion") };
    assert_eq!(outcome.text, "hello world");
    assert_eq!(outcome.usage.input_tokens, 3);
    assert_eq!(outcome.usage.output_tokens, 5);
    assert_eq!(outcome.credits_remaining, Some(7.0));
    assert_eq!(outcome.chat_session, "fixture-chat");
}

#[tokio::test]
async fn empty_http_success_is_not_completed_inference() {
    for body in [String::new(), ": keepalive\n\n".into(), "data: [DONE]\n\n".into()] {
        assert!(send_body(body).await.is_err());
    }
}

#[tokio::test]
async fn partial_text_without_completion_is_refused() {
    for tail in ["", "data: [DONE]\n\n", "data: {\"type\":\"response.completed\",\"response\":"] {
        assert!(send_body(format!("{}{tail}", delta("partial"))).await.is_err());
    }
}

#[tokio::test]
async fn failed_or_incomplete_events_cannot_become_success() {
    for kind in ["response.failed", "response.incomplete", "error"] {
        for prefix in [String::new(), completed()] {
            let failure = event(json!({"type":kind,"error":{"message":"private fixture detail"}}));
            let error = send_body(format!("{prefix}{failure}{}", completed()))
                .await.err().expect("terminal failure must be refused");
            assert!(error.contains(kind));
            assert!(!error.contains("private fixture detail"));
        }
    }
}

#[tokio::test]
async fn multiline_crlf_completion_allows_an_empty_answer() {
    let body = concat!("event: response.completed\r\n",
        "data:{\"type\":\"response.completed\",\r\n",
        "data:\"response\":{\"usage\":{\"input_tokens\":2,\"output_tokens\":0}}}\r\n\r\n");
    let Send::Done(outcome) = send_body(body.into()).await.unwrap() else { panic!("expected completion") };
    assert_eq!(outcome.text, "");
    assert_eq!(outcome.usage.input_tokens, 2);
    assert_eq!(outcome.usage.output_tokens, 0);
}

#[tokio::test]
async fn complete_final_frame_without_blank_separator_still_works() {
    let body = format!("{}: keepalive\n\n{}", delta("answer"), completed().trim_end());
    let Send::Done(outcome) = send_body(body).await.unwrap() else { panic!("expected completion") };
    assert_eq!(outcome.text, "answer");
}

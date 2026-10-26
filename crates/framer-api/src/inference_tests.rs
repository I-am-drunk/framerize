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
    let token = AiToken::for_session(&session, "fixture-only".into(), i64::MAX);
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

fn completed_output(output: serde_json::Value) -> String {
    event(json!({"type":"response.completed", "response":{
        "status":"completed", "output":output,
        "usage":{"input_tokens":3,"output_tokens":5}}}))
}

fn message(text: &str) -> serde_json::Value {
    json!({"type":"message", "role":"assistant",
        "content":[{"type":"output_text", "text":text}]})
}

#[tokio::test]
async fn terminal_output_supplies_text_without_deltas() {
    let body = completed_output(json!([message("full answer")]));
    let Send::Done(outcome) = send_body(body).await.unwrap() else { panic!("expected completion") };
    assert_eq!(outcome.text, "full answer");
    assert_eq!((outcome.usage.input_tokens, outcome.usage.output_tokens), (3, 5));
    assert_eq!(outcome.credits_remaining, Some(7.0));
    assert_eq!(outcome.chat_session, "fixture-chat");
}

#[tokio::test]
async fn terminal_output_replaces_partial_deltas() {
    let body = format!("{}{}", delta("part"),
        completed_output(json!([message("complete answer")])));
    let Send::Done(outcome) = send_body(body).await.unwrap() else { panic!("expected completion") };
    assert_eq!(outcome.text, "complete answer");
}

#[tokio::test]
async fn terminal_output_does_not_duplicate_complete_deltas() {
    let body = format!("{}{}{}", delta("full "), delta("answer"),
        completed_output(json!([message("full answer")])));
    let Send::Done(outcome) = send_body(body).await.unwrap() else { panic!("expected completion") };
    assert_eq!(outcome.text, "full answer");
}

#[tokio::test]
async fn terminal_text_follows_message_and_part_order() {
    let output = json!([
        {"type":"reasoning", "summary":[{"type":"summary_text", "text":"hidden"}]},
        {"type":"message", "role":"assistant", "content":[
            {"type":"output_text", "text":" α\n"},
            {"type":"refusal", "refusal":"not answer text"},
            {"type":"output_text", "text":"β "}]},
        {"type":"function_call", "arguments":"not answer text"},
        message("γ")]);
    let Send::Done(outcome) = send_body(completed_output(output)).await.unwrap()
        else { panic!("expected completion") };
    assert_eq!(outcome.text, " α\nβ γ");
}

#[tokio::test]
async fn empty_terminal_text_replaces_prior_deltas() {
    for output in [json!([]), json!([message("")]), json!([{"type":"reasoning"}])] {
        let body = format!("{}{}", delta("stale text"), completed_output(output));
        let Send::Done(outcome) = send_body(body).await.unwrap()
            else { panic!("expected completion") };
        assert_eq!(outcome.text, "");
    }
}

#[tokio::test]
async fn terminal_output_is_not_extended_by_late_deltas() {
    let body = format!("{}{}", completed_output(json!([message("final answer")])),
        delta("stale text"));
    let Send::Done(outcome) = send_body(body).await.unwrap() else { panic!("expected completion") };
    assert_eq!(outcome.text, "final answer");
}

#[tokio::test]
async fn malformed_terminal_text_is_refused() {
    for output in [json!(null), json!("private fixture detail"),
        json!([{"type":"message"}]),
        json!([{"type":"message", "content":"private fixture detail"}]),
        json!([{"type":"message", "content":[{"type":"output_text"}]}]),
        json!([{"type":"message", "content":[{"type":"output_text", "text":17}]}])] {
        let body = format!("{}{}", delta("partial"), completed_output(output));
        let error = send_body(body).await.err().expect("malformed text must be refused");
        assert!(error.starts_with("invalid inference completion: "));
        assert!(!error.contains("private fixture detail"));
    }
}

#[tokio::test]
async fn completion_without_response_is_refused() {
    let missing = event(json!({"type":"response.completed"}));
    for prefix in [delta("partial"), completed_output(json!([message("answer")]))] {
        let body = format!("{prefix}{missing}{}", completed());
        let error = send_body(body).await.err().expect("missing response must be refused");
        assert_eq!(error, "invalid inference completion: invalid response object");
    }
}

#[tokio::test]
async fn completion_with_nonobject_response_is_refused() {
    for response in [json!(null), json!([]), json!(false), json!(17),
        json!("private fixture detail")] {
        let invalid = event(json!({"type":"response.completed", "response":response}));
        let error = send_body(format!("{}{invalid}", delta("partial")))
            .await.err().expect("nonobject response must be refused");
        assert_eq!(error, "invalid inference completion: invalid response object");
    }
}

#[tokio::test]
async fn completion_with_noncompleted_status_is_refused() {
    for status in [json!("incomplete"), json!("failed"), json!("in_progress"),
        json!("queued"), json!("cancelled"), json!(null), json!(17), json!(false),
        json!("private fixture detail")] {
        let invalid = event(json!({"type":"response.completed", "response":{
            "status":status, "output":[message("partial answer")]}}));
        let body = format!("{}{}{}", delta("partial"), invalid, completed());
        let error = send_body(body).await.err().expect("invalid status must be refused");
        assert_eq!(error, "invalid inference completion: invalid completed status");
    }
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
        for prefix in [String::new(), completed(), completed_output(json!([message("answer")]))] {
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

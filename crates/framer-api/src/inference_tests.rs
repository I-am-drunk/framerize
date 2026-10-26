use super::*;
use async_trait::async_trait;
use framer_core::ids::FramerUserId;
use framer_core::port::CastleToken;
use std::sync::atomic::{AtomicUsize, Ordering};

fn indexed_delta(output: u64, content: u64, text: &str) -> String {
    event(json!({"type":"response.output_text.delta", "output_index":output,
        "content_index":content, "delta":text}))
}

#[tokio::test]
async fn indexed_legacy_text_uses_output_and_content_order() {
    let mut body = String::new();
    for (output, content, text) in [(1, 0, "B"), (0, 1, "2"), (0, 0, "A"),
        (0, 1, "3"), (0, 0, "1"), (1, 0, "C")] {
        body.push_str(&indexed_delta(output, content, text));
    }
    body.push_str(&completed());
    let Send::Done(outcome) = send_body(body).await.unwrap()
        else { panic!("expected completion") };
    assert_eq!(outcome.text, "A123BC");
    assert_eq!((outcome.usage.input_tokens, outcome.usage.output_tokens), (3, 5));
}

#[tokio::test]
async fn sparse_indexed_legacy_text_preserves_empty_deltas() {
    let body = format!("{}{}{}{}{}", indexed_delta(u64::MAX, u64::MAX, "last"),
        delta(""), indexed_delta(7, 8, ""), indexed_delta(0, 0, "first"), completed());
    let Send::Done(outcome) = send_body(body).await.unwrap()
        else { panic!("expected completion") };
    assert_eq!(outcome.text, "firstlast");
    assert_eq!((outcome.usage.input_tokens, outcome.usage.output_tokens), (3, 5));
}

#[tokio::test]
async fn mixed_indexed_and_unindexed_legacy_text_is_rejected() {
    for prefix in [delta("first") + &indexed_delta(1, 0, "last"),
        indexed_delta(0, 0, "first") + &delta("last")] {
        assert_eq!(send_body(prefix + &completed()).await.err().unwrap(),
            "invalid inference completion: invalid output text delta");
    }
}

#[tokio::test]
async fn invalid_text_coordinates_cannot_produce_legacy_success() {
    for mut value in [json!({"output_index":0}), json!({"content_index":0}),
        json!({"output_index":null,"content_index":0}),
        json!({"output_index":0,"content_index":"0"}),
        json!({"output_index":-1,"content_index":0}),
        json!({"output_index":0,"content_index":0.5})] {
        value["type"] = json!("response.output_text.delta");
        value["delta"] = json!("unplaced");
        let body = event(value) + &completed();
        assert_eq!(send_body(body).await.err().unwrap(),
            "invalid inference completion: invalid output text delta");
    }
}

#[tokio::test]
async fn authoritative_output_overrides_unusable_text_coordinates() {
    let invalid = event(json!({"type":"response.output_text.delta",
        "output_index":null,"content_index":0,"delta":"unplaced"}));
    let mixed = indexed_delta(1, 0, "indexed") + &delta("unindexed");
    for prefix in [invalid, mixed] {
        for (output, expected) in [(json!([]), ""), (json!([message("final")]), "final")] {
            let body = prefix.clone() + &completed_output(output);
            let Send::Done(outcome) = send_body(body).await.unwrap()
                else { panic!("expected completion") };
            assert_eq!(outcome.text, expected);
            assert_eq!((outcome.usage.input_tokens, outcome.usage.output_tokens), (3, 5));
        }
    }
}

#[tokio::test]
async fn indexed_legacy_text_stops_at_its_last_completion() {
    for tail in [indexed_delta(0, 0, "discarded"), indexed_delta(1, 0, "discarded"),
        delta("discarded")] {
        let body = indexed_delta(0, 0, "α") + &completed() + &tail;
        let Send::Done(outcome) = send_body(body).await.unwrap()
            else { panic!("expected completion") };
        assert_eq!(outcome.text, "α");
    }
    let body = indexed_delta(1, 0, "B") + &completed()
        + &indexed_delta(0, 0, "A") + &completed() + &indexed_delta(0, 0, "discarded");
    let Send::Done(outcome) = send_body(body).await.unwrap()
        else { panic!("expected completion") };
    assert_eq!(outcome.text, "AB");
}

#[tokio::test]
async fn bare_data_named_text_delta_cannot_produce_legacy_success() {
    for ending in ["\n", "\r\n", "\r"] {
        for field in ["data", "data:", "data: "] {
            let bad = format!("event: response.output_text.delta{ending}{field}{ending}{ending}");
            for body in [format!("{}{bad}{}", delta("before"), completed()),
                format!("{}{}{bad}", delta("before"), completed())] {
                assert_eq!(send_body(body).await.err().expect("invalid legacy text"),
                    "invalid inference completion: invalid output text delta");
            }
        }
    }
}

#[tokio::test]
async fn authoritative_output_overrides_bare_data_delta() {
    let bad = "event: response.output_text.delta\ndata\n\n";
    for (output, expected) in [(json!([]), ""), (json!([message("final")]), "final")] {
        let terminal = completed_output(output);
        for body in [format!("{}{bad}{terminal}", delta("stale")),
            format!("{}{terminal}{bad}", delta("stale"))] {
            let Send::Done(outcome) = send_body(body).await.unwrap()
                else { panic!("expected completion") };
            assert_eq!(outcome.text, expected);
            assert_eq!((outcome.usage.input_tokens, outcome.usage.output_tokens), (3, 5));
        }
    }
}

#[tokio::test]
async fn bare_data_preserves_event_scope() {
    for prefix in ["data\n\n", "data\ndata\n\n",
        "event: response.output_text.delta\nevent: ping\ndata\n\n",
        "event: response.output_text.delta\nevent\ndata\n\n",
        "event: response.output_text.delta\n\n",
        "event: response.output_text.delta\nData\n\n",
        "event: response.output_text.delta\n:data\n\n"] {
        let body = format!("{prefix}{}{}", delta("accepted"), completed());
        let Send::Done(outcome) = send_body(body).await.unwrap()
            else { panic!("expected completion") };
        assert_eq!(outcome.text, "accepted");
        assert_eq!((outcome.usage.input_tokens, outcome.usage.output_tokens), (3, 5));
    }
}

#[tokio::test]
async fn cr_named_failure_or_invalid_completion_cannot_hide() {
    for (kind, expected) in [
        ("error", "inference stream ended with error"),
        ("response.failed", "inference stream ended with response.failed"),
        ("response.incomplete", "inference stream ended with response.incomplete"),
        ("response.completed", "invalid inference completion: invalid named terminal event"),
    ] {
        for ending in ["\r", "\r\r", "\r\n", "\r\n\r\n"] {
            let invalid = format!("event: {kind}{ending}");
            for body in [format!("{}{invalid}", completed()),
                format!("{invalid}\r\r{}", completed())] {
                assert_eq!(send_body(body).await.err().expect("invalid stream"), expected);
            }
        }
    }
}

#[tokio::test]
async fn mixed_line_endings_preserve_legacy_text_and_event_reset() {
    let body = format!("{}event: error\revent: response.completed\r\n\
        data: {{\"type\":\"response.completed\",\n\
        data: \"response\":{{\"usage\":{{\"input_tokens\":3,\"output_tokens\":5}}}}}}\r\n\r\
        event: response.failed\revent\r\nevent: ping\rdata: not-json\r\r",
        delta("α\rβ").replace('\n', "\r\n"));
    let Send::Done(outcome) = send_body(body).await.unwrap()
        else { panic!("expected completion") };
    assert_eq!(outcome.text, "α\rβ");
    assert_eq!((outcome.usage.input_tokens, outcome.usage.output_tokens), (3, 5));
}

#[tokio::test]
async fn cr_line_endings_preserve_legacy_and_authoritative_answers() {
    let answer = "α\rβ\nγ";
    for output in [None, Some(json!([message(answer)]))] {
        let mut response = json!({"usage":{"input_tokens":3,"output_tokens":5}});
        if let Some(output) = output { response["output"] = output; }
        let terminal = json!({"type":"response.completed", "response":response});
        let pretty = serde_json::to_string_pretty(&terminal).unwrap();
        for ending in ["\r", "\r\n", "\n"] {
            for eof in [false, true] {
                let data: String = pretty.lines()
                    .map(|line| format!("data:{line}{ending}")).collect();
                let body = format!("{}event: response.completed{ending}{data}{ending}",
                    delta(answer).replace('\n', ending));
                let body = if eof { body.trim_end_matches(['\r', '\n']) } else { &body };
                let Send::Done(outcome) = send_body(body.into()).await.unwrap()
                    else { panic!("expected completion") };
                assert_eq!(outcome.text, answer);
                assert_eq!((outcome.usage.input_tokens, outcome.usage.output_tokens), (3, 5));
                assert_eq!(outcome.credits_remaining, Some(7.0));
                assert_eq!(outcome.chat_session, "fixture-chat");
            }
        }
    }
}

#[tokio::test]
async fn malformed_named_text_deltas_cannot_produce_legacy_success() {
    for data in ["", "not-json", "[DONE]", "null", "[]", "{}",
        r#"{"delta":"lost text"}"#,
        r#"{"type":"response.output_text.done","text":"lost text"}"#] {
        let bad = format!("event: response.output_text.delta\ndata: {data}\n\n");
        for body in [format!("{}{bad}{}{}", delta("before"), delta("after"), completed()),
            format!("{}{}{bad}", delta("before"), completed())] {
            assert_eq!(send_body(body).await.err().expect("invalid legacy text"),
                "invalid inference completion: invalid output text delta");
        }
    }
}

#[tokio::test]
async fn authoritative_output_overrides_malformed_named_text_deltas() {
    for data in ["", "not-json", "[DONE]", "null", "{}",
        r#"{"type":"future_event","delta":"lost text"}"#] {
        let bad = format!("event: response.output_text.delta\ndata: {data}\n\n");
        for (output, expected) in [(json!([]), ""), (json!([message("final")]), "final")] {
            let terminal = completed_output(output);
            for body in [format!("{}{bad}{terminal}", delta("stale")),
                format!("{}{terminal}{bad}", delta("stale"))] {
                let Send::Done(outcome) = send_body(body).await.unwrap()
                    else { panic!("expected completion") };
                assert_eq!(outcome.text, expected);
                assert_eq!((outcome.usage.input_tokens, outcome.usage.output_tokens), (3, 5));
            }
        }
    }
}

#[tokio::test]
async fn named_text_delta_controls_preserve_event_scope_and_exact_text() {
    for prefix in ["", "event: error\nevent: response.output_text.delta\n",
        "event: response.output_text.delta\n\n",
        "event: response.output_text.delta\n\ndata: not-json\n\n",
        "event: response.output_text.delta\nevent: ping\ndata: not-json\n\n",
        "event: response.output_text.delta\nevent\ndata: not-json\n\n",
        "event:  response.output_text.delta\ndata: not-json\n\n"] {
        let body = format!("{prefix}event: response.output_text.delta\n{}{}{}",
            delta(" α"), delta("β "), completed());
        let Send::Done(outcome) = send_body(body).await.unwrap()
            else { panic!("expected completion") };
        assert_eq!(outcome.text, " αβ ");
        assert_eq!((outcome.usage.input_tokens, outcome.usage.output_tokens), (3, 5));
    }
}

#[tokio::test]
async fn named_text_delta_does_not_hide_typed_failure_events() {
    for kind in ["error", "response.failed", "response.incomplete"] {
        let bad = format!("event: response.output_text.delta\n{}",
            event(json!({"type":kind, "error":{"message":"private fixture"}})));
        let terminal = completed_output(json!([message("answer")]));
        for body in [format!("{bad}{terminal}"), format!("{terminal}{bad}")] {
            assert_eq!(send_body(body).await.err().unwrap(),
                format!("inference stream ended with {kind}"));
        }
    }
}

#[tokio::test]
async fn explicit_nonassistant_terminal_roles_cannot_become_answers() {
    for role in [json!("user"), json!("system"), json!("developer"), json!("tool"),
        json!(""), json!("Assistant"), json!(null), json!(false), json!(7), json!([]), json!({})] {
        let mut bad = message("not an assistant answer");
        bad["role"] = role;
        let invalid = completed_output(json!([message("withheld"), bad]));
        let valid = completed_output(json!([message("valid")]));
        for body in [invalid.clone(), format!("{invalid}{valid}"), format!("{valid}{invalid}")] {
            assert_eq!(send_body(body).await.err().expect("invalid output role"),
                "invalid inference completion: invalid message role");
        }
    }
}

#[tokio::test]
async fn absent_and_assistant_terminal_roles_preserve_direct_answers() {
    for role in [None, Some(json!("assistant"))] {
        let mut output = message("answer");
        output.as_object_mut().unwrap().remove("role");
        if let Some(role) = role { output["role"] = role; }
        output["extension"] = json!({"keep":true});
        let body = completed_output(json!([
            {"type":"future_item", "role":"user", "extension":7}, output
        ]));
        let Send::Done(outcome) = send_body(body).await.unwrap()
            else { panic!("expected completed answer") };
        assert_eq!(outcome.text, "answer");
        assert_eq!((outcome.usage.input_tokens, outcome.usage.output_tokens), (3, 5));
    }
}

#[tokio::test]
async fn intermediate_roles_do_not_change_terminal_message_validation() {
    let intermediate = event(json!({"type":"response.output_item.added",
        "item":{"type":"message", "role":"user", "status":"in_progress"}}));
    let body = format!("{intermediate}{}", completed_output(json!([message("final")])));
    let Send::Done(outcome) = send_body(body).await.unwrap()
        else { panic!("expected completed answer") };
    assert_eq!(outcome.text, "final");
}

#[tokio::test]
async fn malformed_usage_keeps_precedence_over_invalid_message_roles() {
    for role in [json!("user"), json!(null), json!(7)] {
        for (usage, reason) in [
            (json!("fixture usage"), "invalid usage object"),
            (json!([]), "invalid usage object"),
            (json!({"input_tokens":-1}), "invalid usage count"),
            (json!({"output_tokens":null}), "invalid usage count"),
        ] {
            let mut output = message("withheld");
            output["role"] = role.clone();
            let invalid = event(json!({"type":"response.completed", "response":{
                "status":"completed", "output":[output], "usage":usage}}));
            let valid = completed_output(json!([message("valid")]));
            for body in [invalid.clone(), format!("{invalid}{valid}"), format!("{valid}{invalid}")] {
                assert_eq!(send_body(body).await.err().expect("malformed usage"),
                    format!("invalid inference completion: {reason}"));
            }
        }
    }
}

#[tokio::test]
async fn invalid_content_keeps_precedence_over_usage_and_role_errors() {
    for (field, value, reason) in [
        ("status", json!("incomplete"), "invalid message status"),
        ("content", json!(null), "invalid message content"),
        ("content", json!([{"type":"output_text", "text":7}]), "invalid output text"),
        ("content", json!([{"type":"refusal", "refusal":7}]), "invalid refusal"),
    ] {
        let mut output = message("withheld");
        output["role"] = json!("user");
        output[field] = value;
        let body = event(json!({"type":"response.completed", "response":{
            "status":"completed", "output":[output], "usage":"invalid"}}));
        assert_eq!(send_body(body).await.err().expect("invalid content"),
            format!("invalid inference completion: {reason}"));
    }
}

#[tokio::test]
async fn terminal_response_error_cannot_become_a_direct_answer() {
    for error in [json!({"code":"server_error", "message":"fixture diagnostic"}),
        json!({}), json!("fixture diagnostic"), json!(false), json!(7), json!([])] {
        for status in [None, Some(json!("completed"))] {
            let mut response = json!({"output":[message("withheld")], "error":error});
            if let Some(status) = status { response["status"] = status; }
            let invalid = event(json!({"type":"response.completed", "response":response}));
            let valid = completed_output(json!([message("valid")]));
            for body in [invalid.clone(), format!("{invalid}{valid}"), format!("{valid}{invalid}")] {
                assert_eq!(send_body(body).await.err().expect("explicit response error"),
                    "invalid inference completion: non-null response error");
            }
        }
    }
}

#[tokio::test]
async fn absent_or_null_response_error_preserves_direct_output() {
    for error in [None, Some(json!(null))] {
        let mut response = json!({"status":"completed", "output":[
            {"type":"future_item", "error":{"opaque":true}}, message("answer")],
            "metadata":{"error":"opaque"}, "usage":{"input_tokens":2, "output_tokens":3}});
        if let Some(error) = error { response["error"] = error; }
        let ignored = event(json!({"type":"response.output_item.added",
            "item":{"type":"future_item", "error":{"opaque":true}}}));
        let body = format!("{ignored}{}", event(json!({"type":"response.completed", "response":response})));
        let Send::Done(outcome) = send_body(body).await.unwrap()
            else { panic!("expected completed answer") };
        assert_eq!(outcome.text, "answer");
        assert_eq!((outcome.usage.input_tokens, outcome.usage.output_tokens), (2, 3));
    }
}

#[tokio::test]
async fn response_error_preserves_existing_direct_validation_precedence() {
    for (field, value, reason) in [
        ("status", json!("incomplete"), "invalid completed status"),
        ("output", json!(null), "invalid output array"),
        ("output", json!([{"type":"message", "role":"user", "content":[]}]), "invalid message role"),
        ("usage", json!("malformed"), "invalid usage object"),
        ("usage", json!({"output_tokens":null}), "invalid usage count"),
    ] {
        let mut response = json!({"status":"completed", "output":[message("withheld")],
            "error":{"code":"server_error", "message":"fixture diagnostic"}});
        response[field] = value;
        let body = event(json!({"type":"response.completed", "response":response}));
        assert_eq!(send_body(body).await.err().expect("invalid response"),
            format!("invalid inference completion: {reason}"));
    }
}

#[tokio::test]
async fn response_error_cannot_fall_back_to_legacy_deltas() {
    for prefix in [String::new(), delta("withheld legacy text")] {
        for (usage, reason) in [
            (json!(null), "non-null response error"),
            (json!({"input_tokens":2, "output_tokens":3}), "non-null response error"),
            (json!([]), "invalid usage object"),
        ] {
            let terminal = event(json!({"type":"response.completed", "response":{
                "status":"completed", "usage":usage,
                "error":{"code":"server_error", "message":"fixture diagnostic"}}}));
            assert_eq!(send_body(format!("{prefix}{terminal}")).await.err().expect("response error"),
                format!("invalid inference completion: {reason}"));
        }
    }
}

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
async fn malformed_output_items_cannot_replace_text_with_success() {
    for bad in [json!(null), json!(true), json!(42), json!("private-fixture"),
        json!([]), json!({}), json!({"type":null}), json!({"type":false}),
        json!({"type":42}), json!({"type":[]}), json!({"type":{}}), json!({"type":""})] {
        for output in [json!([bad.clone()]), json!([message("unpublished"), bad])] {
            let body = format!("{}{}", delta("partial"), completed_output(output));
            let error = send_body(body).await.err().expect("invalid item must fail");
            assert_eq!(error, "invalid inference completion: invalid output item");
        }
    }
}

#[tokio::test]
async fn terminal_message_parts_require_typed_objects() {
    for bad in [json!(null), json!(false), json!(7), json!("private-fixture"),
        json!([]), json!({}), json!({"type":null}), json!({"type":false}),
        json!({"type":7}), json!({"type":[]}), json!({"type":{}}), json!({"type":""})] {
        let output = json!([{"type":"message", "content":[
            {"type":"output_text", "text":"unpublished"}, bad]}]);
        let body = format!("{}{}", delta("partial"), completed_output(output));
        let error = send_body(body).await.err().expect("invalid part must fail");
        assert_eq!(error, "invalid inference completion: invalid message part");
    }
}

#[tokio::test]
async fn terminal_refusal_requires_a_string_value() {
    for value in [None, Some(json!(null)), Some(json!(false)), Some(json!(7)),
        Some(json!([])), Some(json!({}))] {
        let mut part = json!({"type":"refusal"});
        if let Some(value) = value { part["refusal"] = value; }
        let output = json!([{"type":"message", "content":[part]}]);
        let error = send_body(completed_output(output)).await.err().unwrap();
        assert_eq!(error, "invalid inference completion: invalid refusal");
    }
}

#[tokio::test]
async fn opaque_parts_and_refusals_preserve_the_direct_text_contract() {
    let output = json!([{"type":"message", "content":[
        {"type":"output_text", "text":"first", "annotations":[{"fixture":true}]},
        {"type":"refusal", "refusal":"excluded from text", "extra":{}},
        {"type":"refusal", "refusal":""},
        {"type":"future_part", "opaque":{"untouched":true}},
        {"type":"output_text", "text":" last"}]}]);
    let Send::Done(outcome) = send_body(completed_output(output)).await.unwrap()
        else { panic!("expected completion") };
    assert_eq!(outcome.text, "first last");
    assert_eq!((outcome.usage.input_tokens, outcome.usage.output_tokens), (3, 5));
    assert_eq!(outcome.credits_remaining, Some(7.0));
}

#[tokio::test]
async fn intermediate_message_placeholders_do_not_validate_as_final_content() {
    let body = format!("{}{}{}",
        event(json!({"type":"response.output_item.added",
            "item":{"type":"message", "content":null}})),
        event(json!({"type":"response.content_part.added",
            "part":{"type":"output_text"}})),
        completed_output(json!([message("final text")])));
    let Send::Done(outcome) = send_body(body).await.unwrap()
        else { panic!("expected completion") };
    assert_eq!(outcome.text, "final text");
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
async fn malformed_output_item_is_not_hidden_by_another_completion() {
    let invalid = completed_output(json!([{}]));
    let valid = completed_output(json!([message("answer")]));
    for body in [format!("{invalid}{valid}"), format!("{valid}{invalid}")] {
        let error = send_body(body).await.err().expect("invalid item must fail");
        assert_eq!(error, "invalid inference completion: invalid output item");
    }
}

#[tokio::test]
async fn unknown_output_items_preserve_surrounding_terminal_text() {
    let output = json!([message("first"),
        {"type":"future_output", "id":null, "status":false,
            "extension":{"opaque":true}}, message(" second")]);
    let body = format!("{}{}", delta("stale"), completed_output(output));
    let Send::Done(outcome) = send_body(body).await.unwrap()
        else { panic!("expected completion") };
    assert_eq!(outcome.text, "first second");
    assert_eq!((outcome.usage.input_tokens, outcome.usage.output_tokens), (3, 5));
    assert_eq!(outcome.credits_remaining, Some(7.0));
    assert_eq!(outcome.chat_session, "fixture-chat");
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
async fn malformed_text_delta_cannot_produce_legacy_success() {
    for value in [None, Some(json!(null)), Some(json!(false)), Some(json!(7)),
        Some(json!([])), Some(json!({"private-fixture":true}))] {
        let mut payload = json!({"type":"response.output_text.delta"});
        if let Some(value) = value { payload["delta"] = value; }
        let bad = event(payload);
        for body in [format!("{}{bad}{}{}", delta("before"), delta("after"), completed()),
            format!("{}{}{bad}", delta("before"), completed())] {
            assert_eq!(send_body(body).await.err().unwrap(),
                "invalid inference completion: invalid output text delta");
        }
    }
}

#[tokio::test]
async fn explicit_terminal_output_overrides_malformed_text_deltas() {
    let bad = event(json!({"type":"response.output_text.delta", "delta":null}));
    for (output, expected) in [(json!([]), ""), (json!([message("final")]), "final")] {
        let terminal = completed_output(output);
        for body in [format!("{}{bad}{terminal}", delta("stale")),
            format!("{}{terminal}{bad}", delta("stale"))] {
            let Send::Done(outcome) = send_body(body).await.unwrap()
                else { panic!("expected completion") };
            assert_eq!(outcome.text, expected);
            assert_eq!((outcome.usage.input_tokens, outcome.usage.output_tokens), (3, 5));
            assert_eq!(outcome.credits_remaining, Some(7.0));
        }
    }
}

#[tokio::test]
async fn legacy_text_preserves_empty_deltas_and_unrelated_events() {
    let unrelated = event(json!({"type":"response.content_part.added",
        "part":{"type":"output_text"}}));
    let future = event(json!({"type":"future_event", "delta":null}));
    for (left, right) in [("", ""), (" α", "β ")] {
        let body = format!("{}{unrelated}{}{future}{}{}",
            delta(""), delta(left), delta(right), completed());
        let Send::Done(outcome) = send_body(body).await.unwrap()
            else { panic!("expected completion") };
        assert_eq!(outcome.text, format!("{left}{right}"));
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
async fn named_error_frames_cannot_produce_completed_inference() {
    let valid = format!("{}{}", delta("answer"), completed());
    for error in [
        "event:error\ndata: {\"message\":\"private-fixture\"}\n\n".to_string(),
        "event: error\ndata: not-json\n\n".into(),
        "event: error\ndata: [DONE]\n\n".into(),
        "data: {}\nevent:error\n\n".into(),
        "event: error\r\ndata: {\r\ndata:}\r\n\r\n".into(),
        "event: error\n\n".into(), "event: error".into(),
        format!("event: error\n{}", completed()),
        "event: ping\nevent:error\ndata: {}\n\n".into(),
    ] {
        for body in [format!("{}\n\n{valid}", error.trim_end()),
            format!("{valid}{error}")] {
            let error = send_body(body).await.err().expect("named error must fail");
            assert_eq!(error, "inference stream ended with error");
        }
    }
}

#[tokio::test]
async fn named_failed_or_incomplete_frames_cannot_complete_inference() {
    let valid = completed_output(json!([message("unpublished")]));
    for name in ["response.failed", "response.incomplete"] {
        for data in [String::new(), "data: not-json".into(),
            "data: [DONE]".into(), "data: {}".into(), valid.clone()] {
            let failure = format!("event: {name}\n{data}");
            for body in [format!("{failure}\n\n{valid}"), format!("{valid}{failure}")] {
                let error = send_body(body).await.err().unwrap();
                assert_eq!(error, format!("inference stream ended with {name}"));
            }
        }
    }
}

#[tokio::test]
async fn named_completion_requires_matching_typed_json() {
    let valid = completed_output(json!([message("unpublished")]));
    for data in ["", "data: not-json", "data: [DONE]", "data: null", "data: {}",
        "data: {\"type\":\"response.output_text.delta\",\"delta\":\"private-fixture\"}"] {
        let invalid = format!("event: response.completed\n{data}");
        for body in [format!("{invalid}\n\n{valid}"), format!("{valid}{invalid}")] {
            assert_eq!(send_body(body).await.err().unwrap(),
                "invalid inference completion: invalid named terminal event");
        }
    }
}

#[tokio::test]
async fn matching_named_completion_preserves_legacy_and_authoritative_text() {
    for (terminal, expected) in [(completed(), "deltas"),
        (completed_output(json!([message("final")])), "final"),
        (completed_output(json!([])), "")] {
        let body = format!("{}event: response.failed\nevent: response.completed\n{terminal}event: ping\ndata: not-json\n\n", delta("deltas"));
        let Send::Done(outcome) = send_body(body).await.unwrap()
            else { panic!("expected completion") };
        assert_eq!(outcome.text, expected);
        assert_eq!((outcome.usage.input_tokens, outcome.usage.output_tokens), (3, 5));
        assert_eq!(outcome.credits_remaining, Some(7.0));
    }
}

#[tokio::test]
async fn event_name_controls_preserve_inference_text_and_usage() {
    for prefix in ["event:error\nevent: ping\n", "event:error\nevent:\n\n",
        "event:error\nevent\n\n", "event:unknown\n", "event:  error\n",
        "event:ignored\n\n"] {
        let body = format!("{prefix}{}{}", delta("answer"), completed());
        let Send::Done(outcome) = send_body(body).await.unwrap()
            else { panic!("expected completion") };
        assert_eq!(outcome.text, "answer");
        assert_eq!((outcome.usage.input_tokens, outcome.usage.output_tokens), (3, 5));
        assert_eq!(outcome.credits_remaining, Some(7.0));
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

fn completion_with_usage(usage: Option<serde_json::Value>) -> String {
    let mut response = json!({"status":"completed", "output":[message("answer")]});
    if let Some(usage) = usage { response["usage"] = usage; }
    event(json!({"type":"response.completed", "response":response}))
}

#[tokio::test]
async fn terminal_usage_rejects_malformed_objects() {
    for usage in [json!("private-fixture"), json!(1), json!(false), json!([])] {
        let error = send_body(completion_with_usage(Some(usage))).await.err().unwrap();
        assert_eq!(error, "invalid inference completion: invalid usage object");
    }
}

#[tokio::test]
async fn terminal_usage_rejects_invalid_supplied_counts() {
    for field in ["input_tokens", "output_tokens"] {
        for bad in [serde_json::Value::Null, json!("7"), json!(-1), json!(1.5),
            json!(true), json!({}), json!([]), json!(1e100)] {
            let mut usage = json!({"input_tokens":3, "output_tokens":5});
            usage[field] = bad;
            let error = send_body(completion_with_usage(Some(usage))).await.err().unwrap();
            assert_eq!(error, "invalid inference completion: invalid usage count");
        }
    }
}

#[tokio::test]
async fn optional_and_representable_usage_preserves_completed_output() {
    for (usage, input, output) in [
        (None, 0, 0), (Some(serde_json::Value::Null), 0, 0), (Some(json!({})), 0, 0),
        (Some(json!({"input_tokens":7, "total_tokens":null})), 7, 0),
        (Some(json!({"output_tokens":9, "extension":{"keep":true}})), 0, 9),
        (Some(json!({"input_tokens":u64::MAX, "output_tokens":u64::MAX})), u64::MAX, u64::MAX),
    ] {
        let Send::Done(outcome) = send_body(completion_with_usage(usage)).await.unwrap()
            else { panic!("expected completed output") };
        assert_eq!(outcome.text, "answer");
        assert_eq!((outcome.usage.input_tokens, outcome.usage.output_tokens), (input, output));
        assert_eq!(outcome.credits_remaining, Some(7.0));
        assert_eq!(outcome.chat_session, "fixture-chat");
    }
}

#[tokio::test]
async fn invalid_usage_is_not_hidden_by_another_completion() {
    let bad = completion_with_usage(Some(json!({"output_tokens":"private-fixture"})));
    for body in [format!("{}{bad}", completed()), format!("{bad}{}", completed())] {
        let error = send_body(body).await.err().unwrap();
        assert_eq!(error, "invalid inference completion: invalid usage count");
    }
}

#[tokio::test]
async fn complete_final_frame_without_blank_separator_still_works() {
    let body = format!("{}: keepalive\n\n{}", delta("answer"), completed().trim_end());
    let Send::Done(outcome) = send_body(body).await.unwrap() else { panic!("expected completion") };
    assert_eq!(outcome.text, "answer");
}

#[tokio::test]
async fn unfinished_terminal_messages_cannot_complete_direct_inference() {
    for status in [json!("incomplete"), json!("in_progress"), json!("failed"),
        json!("cancelled"), json!(""), json!("private-fixture"), json!(null),
        json!(false), json!(7), json!([]), json!({})] {
        for explicit_parent in [false, true] {
            let mut item = message("unpublished partial text");
            item["status"] = status.clone();
            let mut response = json!({"output":[message("also unpublished"), item]});
            if explicit_parent { response["status"] = json!("completed"); }
            let terminal = event(json!({"type":"response.completed", "response":response}));
            for body in [terminal.clone(), format!("{terminal}{}", completed())] {
                let error = send_body(body).await.err().expect("unfinished message must fail");
                assert_eq!(error, "invalid inference completion: invalid message status");
            }
        }
    }
}

#[tokio::test]
async fn completed_and_legacy_message_statuses_preserve_direct_text() {
    for explicit_status in [false, true] {
        let mut first = message("α ");
        let mut empty = message("");
        if explicit_status {
            first["status"] = json!("completed");
            empty["status"] = json!("completed");
        }
        let opaque = json!({"type":"future_output", "status":"incomplete"});
        let body = completed_output(json!([first, opaque, empty, message("β")]));
        let Send::Done(outcome) = send_body(body).await.unwrap()
            else { panic!("expected completion") };
        assert_eq!(outcome.text, "α β");
        assert_eq!((outcome.usage.input_tokens, outcome.usage.output_tokens), (3, 5));
        assert_eq!(outcome.chat_session, "fixture-chat");
    }
}

#[tokio::test]
async fn unfinished_intermediate_messages_do_not_invalidate_terminal_output() {
    let mut intermediate = message("partial");
    intermediate["status"] = json!("in_progress");
    let mut final_item = message("final answer");
    final_item["status"] = json!("completed");
    let body = format!("{}{}{}", event(json!({"type":"response.output_item.added",
        "item":intermediate})), delta("partial"), completed_output(json!([final_item])));
    let Send::Done(outcome) = send_body(body).await.unwrap()
        else { panic!("expected completion") };
    assert_eq!(outcome.text, "final answer");
}

#[tokio::test]
async fn legacy_completion_is_not_extended_by_late_text() {
    for answer in ["", "α β"] {
        let body = format!("{}{}{}", delta(answer), completed(), delta(" stale text"));
        let Send::Done(outcome) = send_body(body).await.unwrap()
            else { panic!("expected completion") };
        assert_eq!(outcome.text, answer);
        assert_eq!((outcome.usage.input_tokens, outcome.usage.output_tokens), (3, 5));
    }
}

#[tokio::test]
async fn latest_legacy_completion_captures_its_text_and_usage() {
    let later = event(json!({"type":"response.completed", "response":{
        "usage":{"input_tokens":8, "output_tokens":13}}}));
    let body = format!("{}{}{}{}{}", delta("first "), completed(),
        delta("second"), later, delta(" stale text"));
    let Send::Done(outcome) = send_body(body).await.unwrap()
        else { panic!("expected completion") };
    assert_eq!(outcome.text, "first second");
    assert_eq!((outcome.usage.input_tokens, outcome.usage.output_tokens), (8, 13));
}

#[tokio::test]
async fn later_explicit_output_overrides_legacy_completion() {
    for (output, expected) in [(json!([]), ""), (json!([message("final")]), "final")] {
        let body = format!("{}{}{}{}{}", delta("first"), completed(),
            delta(" second"), completed_output(output), delta(" stale text"));
        let Send::Done(outcome) = send_body(body).await.unwrap()
            else { panic!("expected completion") };
        assert_eq!(outcome.text, expected);
    }
}

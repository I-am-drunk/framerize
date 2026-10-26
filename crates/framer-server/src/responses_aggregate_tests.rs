use super::parse;
use serde_json::{json, Value};

#[test]
fn terminal_response_errors_are_rejected_in_all_terminal_forms() {
    for (kind, status) in [("response.completed", "completed"),
        ("response.completed", "incomplete"), ("response.incomplete", "incomplete")] {
        for error in [json!({"message":"private-sentinel"}), json!({}), json!([]),
            json!(""), json!(false), json!(0)] {
            let mut value = response(status);
            value["error"] = error;
            for prefix in [String::new(), format!("event: {kind}\n")] {
                assert_eq!(parse(&(prefix + &frame(kind, value.clone()))).unwrap_err(),
                    "Responses stream has a terminal response error");
            }
        }
    }
}

#[tokio::test]
async fn terminal_response_error_obeys_caller_status_and_cleanup() {
    for upstream in [200, 206, 429] {
        for (kind, status) in [("response.completed", "completed"),
            ("response.completed", "incomplete"), ("response.incomplete", "incomplete")] {
            let mut value = response(status);
            value["error"] = json!({"message":"private-sentinel"});
            let wire = format!("event: {kind}\r\n{}", frame(kind, value));
            let (status, bytes, confirmed) = call(upstream, chunk(wire)).await;
            assert_eq!(status.as_u16(), if upstream < 300 { 502 } else { upstream });
            assert_eq!(serde_json::from_slice::<Value>(&bytes).unwrap(),
                json!({"error":{"code":"invalid_upstream_response",
                    "message":"Responses stream has a terminal response error"}}));
            assert!(confirmed);
        }
    }
}

#[tokio::test]
async fn null_response_error_preserves_caller_output_and_cleanup() {
    for (kind, status) in [("response.completed", "completed"),
        ("response.completed", "incomplete"), ("response.incomplete", "incomplete")] {
        let mut expected = response(status);
        expected["error"] = Value::Null;
        expected["output"][0]["error"] = json!({"opaque":"kept"});
        let wire = frame(kind, expected.clone());
        let (status, bytes, confirmed) = call(200, chunk(wire)).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(serde_json::from_slice::<Value>(&bytes).unwrap(), expected);
        assert!(confirmed);
    }
}

#[test]
fn terminal_response_errors_cannot_be_hidden_by_another_completion() {
    let valid = frame("response.completed", response("completed"));
    for (kind, status) in [("response.completed", "completed"),
        ("response.completed", "incomplete"), ("response.incomplete", "incomplete")] {
        let mut value = response(status);
        value["error"] = json!({"message":"private-sentinel"});
        let invalid = frame(kind, value);
        for wire in [valid.clone() + &invalid, invalid + &valid] {
            assert_eq!(parse(&wire).unwrap_err(),
                "Responses stream has a terminal response error");
        }
    }
}

#[test]
fn absent_or_null_response_errors_preserve_terminal_fields() {
    for (kind, status) in [("response.completed", "completed"),
        ("response.completed", "incomplete"), ("response.incomplete", "incomplete"),
        ("response.completed", "legacy")] {
        for include_error in [false, true] {
            let mut expected = response(status);
            if status == "legacy" { expected.as_object_mut().unwrap().remove("status"); }
            if include_error { expected["error"] = Value::Null; }
            expected["output"][0]["error"] = json!({"opaque":"kept"});
            assert_eq!(parse(&frame(kind, expected.clone())).unwrap(), expected);
        }
    }
}

#[test]
fn response_error_preserves_existing_aggregate_failure_precedence() {
    for (field, value, error) in [
        ("output", Value::Null, "Responses stream has an invalid output array"),
        ("status", json!("failed"), "Responses stream has a contradictory terminal status"),
        ("usage", json!([]), "Responses stream has an invalid usage object"),
    ] {
        let mut terminal = response("completed");
        terminal["error"] = json!({"message":"private-sentinel"});
        terminal[field] = value;
        assert_eq!(parse(&frame("response.completed", terminal)).unwrap_err(), error);
    }
    for (name, error) in [("error", "Responses stream ended with error"),
        ("response.failed", "Responses stream ended with response.failed")] {
        let mut terminal = response("completed");
        terminal["error"] = json!({"message":"private-sentinel"});
        let wire = format!("event: {name}\n{}", frame("response.completed", terminal));
        assert_eq!(parse(&wire).unwrap_err(), error);
    }
}

#[tokio::test]
async fn cr_streams_preserve_caller_status_output_and_cleanup() {
    for (kind, status) in [("response.completed", "completed"),
        ("response.completed", "incomplete"), ("response.incomplete", "incomplete")] {
        let expected = response(status);
        let wire = format!("event: {kind}\r{}", frame(kind, expected.clone()).replace('\n', "\r"));
        let (status, bytes, confirmed) = call(200, chunk(wire)).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(serde_json::from_slice::<Value>(&bytes).unwrap(), expected);
        assert!(confirmed);
    }
    for upstream in [200, 206, 429] {
        let wire = frame("response.completed", response("completed"))
            + "event: error\rdata: private-sentinel\r\r";
        let (status, bytes, confirmed) = call(upstream, chunk(wire)).await;
        assert_eq!(status.as_u16(), if upstream < 300 { 502 } else { upstream });
        assert_eq!(serde_json::from_slice::<Value>(&bytes).unwrap(),
            json!({"error":{"code":"invalid_upstream_response",
                "message":"Responses stream ended with error"}}));
        assert!(confirmed);
    }
}

#[test]
fn cr_named_failures_and_invalid_terminals_override_completion() {
    let valid = frame("response.completed", response("completed"));
    for (kind, expected) in [
        ("error", "Responses stream ended with error"),
        ("response.failed", "Responses stream ended with response.failed"),
        ("response.completed", "Responses stream has an invalid named terminal event"),
        ("response.incomplete", "Responses stream has an invalid named terminal event"),
    ] {
        for ending in ["\r", "\r\r", "\r\n", "\r\n\r\n"] {
            let invalid = format!("event: {kind}{ending}");
            for body in [format!("{valid}{invalid}"), format!("{invalid}\r\r{valid}")] {
                assert_eq!(parse(&body).unwrap_err(), expected);
            }
        }
    }
}

#[test]
fn mixed_endings_keep_crlf_atomic_and_preserve_event_reset() {
    let expected = response("incomplete");
    let event = json!({"type":"response.incomplete", "response":expected});
    let pretty = serde_json::to_string_pretty(&event).unwrap();
    let data: String = pretty.lines().enumerate().map(|(i, line)| {
        format!("data:{line}{}", ["\r", "\n", "\r\n"][i % 3])
    }).collect();
    let body = format!("event:error\revent:response.completed\r\n{}\r\n\
        event:response.failed\revent\r\n{data}\r\n",
        frame("response.completed", response("completed")));
    assert_eq!(parse(&body).unwrap(), expected);
}

#[test]
fn cr_line_endings_preserve_complete_and_partial_terminal_fields() {
    for (kind, status) in [("response.completed", "completed"),
        ("response.completed", "incomplete"), ("response.incomplete", "incomplete")] {
        let mut expected = response(status);
        expected["id"] = json!("α\rβ\nγ");
        let event = json!({"type":kind, "response":expected});
        let pretty = serde_json::to_string_pretty(&event).unwrap();
        for ending in ["\r", "\r\n", "\n"] {
            for eof in [false, true] {
                let data: String = pretty.lines()
                    .map(|line| format!("data:{line}{ending}")).collect();
                let body = format!("event:{kind}{ending}{data}{ending}");
                let body = if eof { body.trim_end_matches(['\r', '\n']) } else { &body };
                assert_eq!(parse(body).unwrap(), expected);
            }
        }
    }
}

fn response(status: &str) -> Value {
    json!({"id":"fixture-response", "status":status,
        "output":[{"type":"function_call", "name":"fixture", "arguments":"{}"}],
        "usage":{"input_tokens":3,"output_tokens":5,"details":{"cached":1}},
        "incomplete_details":if status == "incomplete" {
            json!({"reason":"max_output_tokens"})
        } else { Value::Null }})
}

fn frame(kind: &str, response: Value) -> String {
    format!("data: {}\n\n", json!({"type":kind,"response":response}))
}

#[test]
fn completed_response_preserves_all_fields() {
    let expected = response("completed");
    assert_eq!(parse(&frame("response.completed", expected.clone())).unwrap(), expected);
}

#[test]
fn incomplete_terminal_forms_preserve_output_and_reason() {
    for kind in ["response.incomplete", "response.completed"] {
        let expected = response("incomplete");
        assert_eq!(parse(&frame(kind, expected.clone())).unwrap(), expected);
    }
}

#[test]
fn empty_output_and_legacy_missing_status_remain_valid() {
    let expected = json!({"output":[], "usage":null, "extension":{"fixture":true}});
    assert_eq!(parse(&frame("response.completed", expected.clone())).unwrap(), expected);
}

#[test]
fn crlf_multiline_fields_and_eof_preserve_terminal_order() {
    let first = frame("response.completed", response("completed"));
    let expected = response("incomplete");
    let event = json!({"type":"response.incomplete","response":expected});
    let mut body = first + "event: response.incomplete\r\nid: fixture\r\n";
    for line in serde_json::to_string_pretty(&event).unwrap().lines() {
        body.push_str(&format!("data:{line}\r\n: keepalive\r\n"));
    }
    assert_eq!(parse(&body).unwrap(), expected);
}

#[test]
fn explicit_failures_cannot_be_masked_by_completion() {
    let complete = frame("response.completed", response("completed"));
    for kind in ["error", "response.failed"] {
        let failure = format!("data: {}\n\n", json!({"type":kind,"message":"private-sentinel"}));
        for body in [complete.clone() + &failure, failure + &complete] {
            assert_eq!(parse(&body).unwrap_err(), if kind == "error" {
                "Responses stream ended with error"
            } else { "Responses stream ended with response.failed" });
        }
    }
}

#[test]
fn missing_terminal_or_malformed_event_is_rejected() {
    for body in ["", ": keepalive\n\n", "data: [DONE]\n\n",
        "data: {\"type\":\"response.output_text.delta\",\"delta\":\"partial\"}\n\n",
        "data: {private-sentinel\n\n", "data: null\n\n", "data: {}\n\n"] {
        let error = parse(body).unwrap_err();
        assert!(!error.contains("private-sentinel"));
    }
}

#[test]
fn malformed_terminal_containers_are_rejected() {
    for value in [Value::Null, json!(false), json!(7), json!("sentinel"), json!([])] {
        assert_eq!(parse(&frame("response.completed", value)).unwrap_err(),
            "Responses stream has an invalid terminal response");
    }
    for output in [Value::Null, json!(7), json!({}), json!("sentinel")] {
        assert_eq!(parse(&frame("response.completed", json!({"output":output}))).unwrap_err(),
            "Responses stream has an invalid output array");
    }
    for usage in [json!(7), json!([]), json!("sentinel")] {
        assert_eq!(parse(&frame("response.completed", json!({"output":[],"usage":usage}))).unwrap_err(),
            "Responses stream has an invalid usage object");
    }
}

#[test]
fn contradictory_terminal_statuses_are_rejected() {
    for kind in ["response.completed", "response.incomplete"] {
        for status in [Value::Null, json!("failed"), json!("cancelled"), json!("in_progress"), json!(4)] {
            assert_eq!(parse(&frame(kind, json!({"output":[],"status":status}))).unwrap_err(),
                "Responses stream has a contradictory terminal status");
        }
    }
    for body in [response("completed"), json!({"output":[]})] {
        assert_eq!(parse(&frame("response.incomplete", body)).unwrap_err(),
            "Responses stream has a contradictory terminal status");
    }
}

#[path = "permit_lifetime_fixture.rs"]
mod fixture;
use axum::{body::{to_bytes, Bytes}, http::StatusCode, response::IntoResponse};
use framer_core::port::ProxyResponse;

async fn call(status: u16, chunks: Vec<Result<Bytes, String>>) -> (StatusCode, Vec<u8>, bool) {
    let mut f = fixture::Fixture::new();
    let forward = crate::compat_forward_observed(f.state.clone(), "openai-responses",
        Some(f.accounts[0].id.0.to_string()), Some("fixture-chat".into()),
        json!({"model":"gpt-6-astra","input":"fixture","stream":false}));
    let deliver = async {
        let call = f.next().await;
        assert_eq!(call.kind, "proxy");
        assert!(call.reply.send(Ok(fixture::Reply::Proxy(ProxyResponse {
            status, content_type:"text/event-stream".into(), queue_wait_ms:0,
            stream:Box::pin(futures::stream::iter(chunks)),
        }))).is_ok());
        call.dropped.await.unwrap();
    };
    let (outcome, ()) = tokio::join!(forward, deliver);
    let confirmed = outcome.cleanup_confirmed;
    if let Ok(response) = &outcome.result {
        assert_eq!(response.headers()["x-framerize-session"], "fixture-chat");
    }
    let response = outcome.result.into_response();
    let status = response.status();
    let body = to_bytes(response.into_body(), 8192).await.unwrap().to_vec();
    f.no_call();
    assert!(f.state.serving_accounts.residents.snapshot().unwrap().residents
        .iter().all(|slot| slot.users == 0));
    (status, body, confirmed)
}

fn chunk(body: impl Into<Bytes>) -> Vec<Result<Bytes, String>> { vec![Ok(body.into())] }

#[tokio::test]
async fn invalid_success_streams_return_bad_gateway_without_diagnostics() {
    for upstream in [200, 202, 206] {
        for body in ["data: [DONE]\n\n".to_string(),
            "data: {\"type\":\"response.failed\",\"error\":\"private-sentinel\"}\n\n".into(),
            frame("response.completed", Value::Null)] {
            let (status, bytes, confirmed) = call(upstream, chunk(body)).await;
            assert_eq!(status, StatusCode::BAD_GATEWAY);
            let body: Value = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(body["error"]["code"], "invalid_upstream_response");
            assert!(!String::from_utf8_lossy(&bytes).contains("private-sentinel"));
            assert!(confirmed);
        }
    }
}

#[tokio::test]
async fn invalid_non_success_streams_retain_upstream_status() {
    for upstream in [302, 400, 403, 429, 503] {
        let (status, bytes, confirmed) = call(upstream, chunk("data: [DONE]\n\n")).await;
        assert_eq!(status.as_u16(), upstream);
        assert_eq!(serde_json::from_slice::<Value>(&bytes).unwrap()["error"]["code"],
            "invalid_upstream_response");
        assert!(confirmed);
    }
}

#[tokio::test]
async fn invalid_utf8_is_refused_without_replacing_response_text() {
    let (status, bytes, confirmed) = call(200, chunk(vec![b'd', b'a', b't', b'a', b':', 0xff])).await;
    assert_eq!(status, StatusCode::BAD_GATEWAY);
    assert_eq!(serde_json::from_slice::<Value>(&bytes).unwrap()["error"]["message"],
        "Responses stream is not valid UTF-8");
    assert!(confirmed);
}

#[tokio::test]
async fn caller_preserves_completed_and_incomplete_terminal_output() {
    for (kind, status) in [("response.completed", "completed"),
        ("response.completed", "incomplete"), ("response.incomplete", "incomplete")] {
        let expected = response(status);
        let wire = frame(kind, expected.clone()).replace('\n', "\r\n");
        let split = wire.len() / 2;
        let chunks = vec![Ok(Bytes::copy_from_slice(&wire.as_bytes()[..split])),
            Ok(Bytes::copy_from_slice(&wire.as_bytes()[split..]))];
        let (status, bytes, confirmed) = call(200, chunks).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(serde_json::from_slice::<Value>(&bytes).unwrap(), expected);
        assert!(confirmed);
    }
}

#[tokio::test]
async fn upstream_cleanup_failure_remains_unconfirmed() {
    let chunks = vec![Ok(Bytes::from(frame("response.completed", response("completed")))),
        Err("account browser cleanup is unconfirmed; retry after cleanup".into())];
    let (status, _, confirmed) = call(200, chunks).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert!(!confirmed);
}

#[test]
fn malformed_event_after_terminal_cannot_be_hidden() {
    let body = frame("response.completed", response("completed"))
        + "data: {private-sentinel\n\n";
    assert_eq!(parse(&body).unwrap_err(), "Responses stream has malformed event JSON");
    assert_eq!(parse("data: {\"type\":\"response.completed\"}\n\n").unwrap_err(),
        "Responses stream has an invalid terminal response");
}

#[test]
fn named_failures_cannot_be_masked_by_terminal_data() {
    let complete = frame("response.completed", response("completed"));
    for kind in ["error", "response.failed"] {
        let expected = if kind == "error" { "Responses stream ended with error" }
            else { "Responses stream ended with response.failed" };
        for data in [String::new(), complete.clone(), "data: [DONE]\n\n".into(),
            "data: {private-sentinel\n\n".into()] {
            let failure = format!("event: {kind}\n{data}\n");
            for wire in [complete.clone() + &failure, failure + &complete] {
                assert_eq!(parse(&wire).unwrap_err(), expected);
            }
        }
        assert_eq!(parse(&(complete.clone() + &format!("event:{kind}"))).unwrap_err(), expected);
    }
}

#[test]
fn named_terminals_require_matching_typed_json() {
    let valid = frame("response.completed", response("completed"));
    for (kind, other) in [("response.completed", "response.incomplete"),
        ("response.incomplete", "response.completed")] {
        for data in [String::new(), "data:\n\n".into(), "data: [DONE]\n\n".into(),
            "data: {private-sentinel\n\n".into(), "data: null\n\n".into(),
            "data: {}\n\n".into(), frame(other, response("incomplete"))] {
            let bad = format!("event: {kind}\n{data}\n");
            for wire in [valid.clone() + &bad, bad + &valid] {
                assert_eq!(parse(&wire).unwrap_err(),
                    "Responses stream has an invalid named terminal event");
            }
        }
    }
}

#[test]
fn event_fields_keep_last_value_and_reset_between_frames() {
    let expected = response("completed");
    let complete = frame("response.completed", expected.clone());
    for prefix in ["event:error\nevent: response.completed\n",
        "event:response.failed\nevent: ping\n", "event:error\nevent\n",
        "event:error\nevent:\n", "event:ping\n\n"] {
        assert_eq!(parse(&(prefix.to_string() + &complete)).unwrap(), expected);
    }
    let partial = response("incomplete");
    let wire = format!("event:response.completed\n{complete}{}",
        frame("response.incomplete", partial.clone()));
    assert_eq!(parse(&wire).unwrap(), partial);
}

#[test]
fn named_terminal_forms_preserve_multiline_crlf_and_eof() {
    for (kind, status) in [("response.completed", "completed"),
        ("response.completed", "incomplete"), ("response.incomplete", "incomplete")] {
        let expected = response(status);
        let value = json!({"type":kind, "response":expected});
        let mut wire = frame("response.completed", response("completed"));
        wire.push_str(&format!("event: {kind}\r\n"));
        for line in serde_json::to_string_pretty(&value).unwrap().lines() {
            wire.push_str(&format!("data:{line}\r\n: keepalive\r\n"));
        }
        assert_eq!(parse(&wire).unwrap(), expected);
    }
}

#[tokio::test]
async fn named_failures_return_fixed_errors_and_preserve_cleanup() {
    let complete = frame("response.completed", response("completed"));
    for upstream in [200, 206, 403] {
        for (suffix, message) in [
            ("event:error\n\n".to_string(), "Responses stream ended with error"),
            (format!("event:response.failed\n{complete}"),
                "Responses stream ended with response.failed"),
        ] {
            let (status, bytes, confirmed) = call(upstream, chunk(complete.clone() + &suffix)).await;
            assert_eq!(status.as_u16(), if upstream < 300 { 502 } else { upstream });
            assert_eq!(serde_json::from_slice::<Value>(&bytes).unwrap(),
                json!({"error":{"code":"invalid_upstream_response", "message":message}}));
            assert!(confirmed);
        }
    }
}

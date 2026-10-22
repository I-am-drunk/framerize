//! HTTP boundary tests, using only bounded in-memory provider invocations.
use super::*;
use framer_core::native_dispatch::{ComposedStreamOptions, StreamingPolicy};
use futures::StreamExt;
use serde_json::json;
use std::{sync::atomic::Ordering, time::Duration};

#[path = "native_http_stream_fixture.rs"]
mod fixture;
use fixture::{headers, kind, responses, Fixture, BOUND};

const IDLE: Duration = Duration::from_millis(40);

async fn next<S: Stream + Unpin>(stream: &mut S) -> Option<S::Item> {
    tokio::time::timeout(BOUND, stream.next()).await.expect("bounded body observation")
}
async fn pending<S: Stream + Unpin>(stream: &mut S) {
    assert!(tokio::time::timeout(IDLE, stream.next()).await.is_err());
}

async fn completed(observers: &NativeStreamObservers) -> NativeStreamCompletion {
    tokio::time::timeout(BOUND, observers.completion.clone()).await
        .expect("bounded completion observation")
}

fn completion_result(value: NativeStreamCompletion) -> Result<(), FailureKind> {
    match value {
        NativeStreamCompletion::Raw(value) => value.result,
        NativeStreamCompletion::Composed(value) => value.result,
    }
}

#[tokio::test(flavor = "current_thread")]
async fn post_header_body_errors_remain_errors_and_cancel_once_before_cleanup() {
    for path in ["/paid/openai/v1/responses", "/openai/v1/responses"] {
        for failure in [FailureKind::Body, FailureKind::Transport, FailureKind::BodyTooLarge] {
            let fixture = Fixture::new(1, Duration::from_secs(1), None);
            let mut control = fixture.enqueue(0,
                headers(201, "text/event-stream", Some("real-usage")), "text/event-stream");
            control.open();
            control.push(b"data: before-error\n\n");
            control.fail_body(failure);
            let response = fixture.request(path, responses(), 0, "session", false).await;
            assert_eq!(response.status(), StatusCode::CREATED);
            let observers = response.extensions().get::<NativeStreamObservers>().unwrap().clone();
            let mut body = response.into_body().into_data_stream();
            assert_eq!(next(&mut body).await.unwrap().unwrap().as_ref(), b"data: before-error\n\n");
            assert_eq!(kind(&next(&mut body).await.unwrap().unwrap_err()), failure);
            let polls = control.probe.body_polls.load(Ordering::SeqCst);
            assert!(next(&mut body).await.is_none());
            assert_eq!(control.probe.body_polls.load(Ordering::SeqCst), polls);
            assert_eq!(control.probe.cancellations(), 1);
            assert!(observers.completion.clone().now_or_never().is_none());
            assert!(observers.cleanup.clone().now_or_never().is_none());
            assert!(!fixture.slot_free(0, "gpt-4.1").await);
            control.finish(Err(failure));
            assert_eq!(completion_result(completed(&observers).await), Err(failure));
            drop(body);
            assert_eq!(control.probe.cancellations(), 1);
            assert_eq!(control.probe.body_drops.load(Ordering::SeqCst), 1);
            control.release().await;
            assert!(fixture.slot_free(0, "gpt-4.1").await);
            fixture.assert_legacy_unused();
        }
    }
}

#[tokio::test(flavor = "current_thread")]
async fn post_header_deadline_is_a_body_error_and_keeps_capacity_until_cleanup() {
    for path in ["/paid/openai/v1/responses", "/openai/v1/responses"] {
        let fixture = Fixture::new(1, Duration::from_millis(120), None);
        let mut control = fixture.enqueue(0,
            headers(201, "text/event-stream", None), "text/event-stream");
        control.open();
        control.push(b"data: before-deadline\n\n");
        let response = fixture.request(path, responses(), 0, "session", false).await;
        assert_eq!(response.status(), StatusCode::CREATED);
        let observers = response.extensions().get::<NativeStreamObservers>().unwrap().clone();
        let deadline = fixture.records()[0].deadline;
        let mut body = response.into_body().into_data_stream();
        assert_eq!(next(&mut body).await.unwrap().unwrap().as_ref(), b"data: before-deadline\n\n");
        assert_eq!(kind(&next(&mut body).await.unwrap().unwrap_err()), FailureKind::Deadline);
        assert!(Instant::now() >= deadline);
        assert!(next(&mut body).await.is_none());
        assert_eq!(control.probe.cancellations(), 1);
        assert!(observers.completion.clone().now_or_never().is_none());
        assert!(observers.cleanup.clone().now_or_never().is_none());
        assert!(!fixture.slot_free(0, "gpt-4.1").await);
        control.finish(Err(FailureKind::Deadline));
        assert_eq!(completion_result(completed(&observers).await), Err(FailureKind::Deadline));
        drop(body);
        assert_eq!(control.probe.cancellations(), 1);
        control.release().await;
        assert!(fixture.slot_free(0, "gpt-4.1").await);
        fixture.assert_legacy_unused();
    }
}

#[tokio::test(flavor = "current_thread")]
async fn eof_waits_for_terminal_result_independently_of_cleanup() {
    for path in ["/paid/openai/v1/responses", "/openai/v1/responses"] {
        for result in [Ok(()), Err(FailureKind::TerminalFailure)] {
            let fixture = Fixture::new(1, Duration::from_secs(1), None);
            let mut control = fixture.enqueue(0,
                headers(200, "text/event-stream", None), "text/event-stream");
            control.open();
            control.push(b"data: final-source-frame\n\n");
            control.eof();
            let response = fixture.request(path, responses(), 0, "session", false).await;
            let observers = response.extensions().get::<NativeStreamObservers>().unwrap().clone();
            let mut body = response.into_body().into_data_stream();
            assert_eq!(next(&mut body).await.unwrap().unwrap().as_ref(), b"data: final-source-frame\n\n");
            pending(&mut body).await;
            let polls_at_eof = control.probe.body_polls.load(Ordering::SeqCst);
            pending(&mut body).await;
            assert_eq!(control.probe.body_polls.load(Ordering::SeqCst), polls_at_eof);
            assert_eq!(control.probe.cancellations(), 0);
            assert!(!fixture.slot_free(0, "gpt-4.1").await);
            control.finish(result);
            if let Err(expected) = result {
                assert_eq!(kind(&next(&mut body).await.unwrap().unwrap_err()), expected);
            }
            assert!(next(&mut body).await.is_none());
            assert_eq!(completion_result(completed(&observers).await), result);
            assert!(observers.cleanup.clone().now_or_never().is_none());
            assert!(!fixture.slot_free(0, "gpt-4.1").await);
            drop(body);
            assert_eq!(control.probe.cancellations(), usize::from(result.is_err()));
            assert_eq!(control.probe.body_drops.load(Ordering::SeqCst), 1);
            control.release().await;
            assert!(fixture.slot_free(0, "gpt-4.1").await);
            fixture.assert_legacy_unused();
        }
    }
}

#[tokio::test(flavor = "current_thread")]
async fn invalid_upstream_status_or_headers_cancel_without_committing_a_body() {
    let invalid = [
        (0, "text/event-stream", None),
        (200, "text/event-stream\r\nx-injected: yes", None),
        (200, "text/event-stream", Some("usage\r\nx-injected: yes")),
    ];
    for path in ["/paid/openai/v1/responses", "/openai/v1/responses"] {
        for (status, content_type, usage) in invalid {
            let fixture = Fixture::new(1, Duration::from_secs(1), None);
            let mut control = fixture.enqueue(0,
                headers(status, content_type, usage), content_type);
            control.open();
            control.push(b"data: must-not-be-read\n\n");
            let response = fixture.request(path, responses(), 0, "session", false).await;
            assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
            assert_eq!(response.extensions().get::<FailureKind>(),
                Some(&FailureKind::UnsupportedResponse));
            assert!(response.extensions().get::<NativeStreamObservers>().is_none());
            assert!(response.headers().get("x-framer-ai-usage").is_none());
            assert!(response.headers().get("x-injected").is_none());
            assert_eq!(control.probe.body_polls.load(Ordering::SeqCst), 0);
            assert_eq!(control.probe.body_drops.load(Ordering::SeqCst), 1);
            assert_eq!(control.probe.cancellations(), 1);
            assert!(!fixture.slot_free(0, "gpt-4.1").await);
            control.release().await;
            assert!(fixture.slot_free(0, "gpt-4.1").await);
            fixture.assert_legacy_unused();
        }
    }
}

#[tokio::test(flavor = "current_thread")]
async fn raw_heartbeat_never_splits_partial_fields_utf8_or_crlf() {
    let fixture = Fixture::new(1, Duration::from_secs(2), Some(Duration::from_millis(10)));
    let mut control = fixture.enqueue(0,
        headers(200, "text/event-stream; charset=utf-8", None), "text/event-stream");
    let fragments: [&'static [u8]; 6] = [
        b"event: delta\r", b"\ndata: {\"text\":\"\xf0", b"\x9f\x98",
        b"\x80\"}\r", b"\n\r", b"\n",
    ];
    control.open();
    let response = fixture.request("/paid/openai/v1/responses",
        responses(), 0, "session", false).await;
    let observers = response.extensions().get::<NativeStreamObservers>().unwrap().clone();
    let mut body = response.into_body().into_data_stream();
    let mut received = Vec::new();
    pending(&mut body).await;
    for fragment in fragments {
        control.push(fragment);
        let chunk = next(&mut body).await.unwrap().unwrap();
        assert_eq!(chunk.as_ref(), fragment);
        received.extend_from_slice(&chunk);
        pending(&mut body).await;
    }
    assert_eq!(received, b"event: delta\r\ndata: {\"text\":\"\xf0\x9f\x98\x80\"}\r\n\r\n");
    control.eof();
    pending(&mut body).await;
    control.finish(Ok(()));
    assert!(next(&mut body).await.is_none());
    match completed(&observers).await {
        NativeStreamCompletion::Raw(value) => {
            assert_eq!(value.result, Ok(()));
            assert_eq!(value.emitted_bytes, received.len());
        }
        _ => panic!("raw route changed its completion kind"),
    }
    assert!(observers.cleanup.clone().now_or_never().is_none());
    drop(body);
    assert_eq!(control.probe.cancellations(), 0);
    control.release().await;
    fixture.assert_legacy_unused();
}

#[tokio::test(flavor = "current_thread")]
async fn composed_heartbeat_starts_after_real_headers_and_preserves_complete_frames() {
    for content_type in ["text/event-stream", "Text/Event-Stream; charset=utf-8"] {
        let fixture = Fixture::new(1, Duration::from_secs(1), Some(Duration::from_millis(10)));
        let mut control = fixture.enqueue(0,
            headers(201, "application/json", Some("actual-usage")), content_type);
        let mut request = Box::pin(fixture.request("/openai/v1/responses",
            responses(), 0, "heartbeat-session", false));
        assert!(tokio::time::timeout(IDLE, request.as_mut()).await.is_err());
        assert_eq!(control.probe.body_polls.load(Ordering::SeqCst), 0);
        assert_eq!(control.probe.cancellations(), 0);
        control.open();
        let response = tokio::time::timeout(BOUND, request.as_mut()).await.unwrap();
        drop(request);
        assert_eq!(response.status(), StatusCode::CREATED);
        assert_eq!(response.headers()["content-type"], content_type);
        assert_eq!(response.headers()["x-framer-ai-usage"], "actual-usage");
        let observers = response.extensions().get::<NativeStreamObservers>().unwrap().clone();
        let mut body = response.into_body().into_data_stream();
        assert_eq!(next(&mut body).await.unwrap().unwrap().as_ref(), b": ping\n\n");
        const FRAME: &[u8] = b"event: delta\ndata: {\"text\":\"ok\"}\n\n";
        control.push(FRAME);
        assert_eq!(next(&mut body).await.unwrap().unwrap().as_ref(), FRAME);
        control.eof();
        control.finish(Ok(()));
        assert!(next(&mut body).await.is_none());
        match completed(&observers).await {
            NativeStreamCompletion::Composed(value) => {
                assert_eq!(value.result, Ok(()));
                assert_eq!(value.emitted_bytes, FRAME.len());
                assert_eq!(value.attempts.len(), 1);
                assert!(Arc::ptr_eq(&value.attempts[0].headers, &control.headers));
            }
            _ => panic!("composed route changed its completion kind"),
        }
        assert!(observers.cleanup.clone().now_or_never().is_none());
        drop(body);
        assert_eq!(control.probe.cancellations(), 0);
        control.release().await;
        fixture.assert_legacy_unused();
    }
}

#[tokio::test(flavor = "current_thread")]
async fn non_sse_content_never_receives_a_configured_heartbeat() {
    for path in ["/paid/openai/v1/responses", "/openai/v1/responses"] {
        let fixture = Fixture::new(1, Duration::from_secs(1), Some(Duration::from_millis(10)));
        let mut control = fixture.enqueue(0,
            headers(200, "application/json", None), "application/json");
        control.open();
        let response = fixture.request(path, responses(), 0, "session", false).await;
        assert_eq!(response.headers()["content-type"], "application/json");
        let mut body = response.into_body().into_data_stream();
        pending(&mut body).await;
        control.push(b"{\"exact\":true}");
        assert_eq!(next(&mut body).await.unwrap().unwrap().as_ref(), b"{\"exact\":true}");
        control.eof();
        pending(&mut body).await;
        control.finish(Ok(()));
        assert!(next(&mut body).await.is_none());
        drop(body);
        assert_eq!(control.probe.cancellations(), 0);
        control.release().await;
        fixture.assert_legacy_unused();
    }
}

#[tokio::test(flavor = "current_thread")]
async fn composed_heartbeats_never_extend_the_original_deadline() {
    let fixture = Fixture::new(1, Duration::from_millis(120), Some(Duration::from_millis(10)));
    let mut control = fixture.enqueue(0,
        headers(200, "application/json", None), "text/event-stream");
    control.open();
    control.eof();
    let response = fixture.request("/openai/v1/responses", responses(), 0, "session", false).await;
    let observers = response.extensions().get::<NativeStreamObservers>().unwrap().clone();
    let deadline = fixture.records()[0].deadline;
    let mut body = response.into_body().into_data_stream();
    let mut pings = 0;
    loop {
        match next(&mut body).await.expect("deadline must be an error, not clean EOF") {
            Ok(bytes) => {
                assert_eq!(bytes.as_ref(), b": ping\n\n");
                pings += 1;
                assert!(pings <= 100, "heartbeats must not reset the deadline");
            }
            Err(error) => { assert_eq!(kind(&error), FailureKind::Deadline); break; }
        }
    }
    assert!(pings > 0);
    assert!(Instant::now() >= deadline);
    assert!(next(&mut body).await.is_none());
    assert_eq!(control.probe.cancellations(), 1);
    assert!(observers.cleanup.clone().now_or_never().is_none());
    control.finish(Err(FailureKind::Deadline));
    assert_eq!(completion_result(completed(&observers).await), Err(FailureKind::Deadline));
    drop(body);
    control.release().await;
    fixture.assert_legacy_unused();
}

#[tokio::test(flavor = "current_thread")]
async fn cancelling_one_account_never_cancels_or_releases_another_account() {
    for path in ["/paid/openai/v1/responses", "/openai/v1/responses"] {
        let fixture = Fixture::new(2, Duration::from_secs(2), None);
        let mut first = fixture.enqueue(0,
            headers(200, "text/event-stream", Some("usage-first")), "text/event-stream");
        let mut second = fixture.enqueue(1,
            headers(200, "text/event-stream", Some("usage-second")), "text/event-stream");
        first.open();
        second.open();
        first.push(b"data: first-only\n\n");
        second.push(b"data: second-only\n\n");
        let response_a = fixture.request(path, responses(), 0, "session-first", false).await;
        let response_b = fixture.request(path, responses(), 1, "session-second", false).await;
        let records = fixture.records();
        assert_eq!(records.len(), 2);
        for (index, session) in ["session-first", "session-second"].into_iter().enumerate() {
            let actual = &records[index].identity;
            let expected = &fixture.accounts[index];
            assert_eq!(actual.account, expected.id);
            assert_eq!(actual.user, expected.framer_user_id);
            assert_eq!(actual.team, expected.current_team.clone().unwrap());
            assert_eq!(actual.project, expected.current_project.clone().unwrap());
            assert_eq!(actual.chat_session, session);
        }
        assert_ne!(records[0].identity.account, records[1].identity.account);
        assert_ne!(records[0].identity.user, records[1].identity.user);
        assert_ne!(records[0].identity.team, records[1].identity.team);
        assert_ne!(records[0].identity.project, records[1].identity.project);
        assert_eq!(response_a.headers()["x-framerize-session"], "session-first");
        assert_eq!(response_b.headers()["x-framerize-session"], "session-second");
        assert_eq!(response_a.headers()["x-framer-ai-usage"], "usage-first");
        assert_eq!(response_b.headers()["x-framer-ai-usage"], "usage-second");
        let observers_a = response_a.extensions().get::<NativeStreamObservers>().unwrap().clone();
        let observers_b = response_b.extensions().get::<NativeStreamObservers>().unwrap().clone();
        let mut body_a = response_a.into_body().into_data_stream();
        let mut body_b = response_b.into_body().into_data_stream();
        assert_eq!(next(&mut body_a).await.unwrap().unwrap().as_ref(), b"data: first-only\n\n");
        assert_eq!(next(&mut body_b).await.unwrap().unwrap().as_ref(), b"data: second-only\n\n");
        drop(body_a);
        assert_eq!(first.probe.cancellations(), 1);
        assert_eq!(second.probe.cancellations(), 0);
        assert!(!fixture.slot_free(0, "gpt-4.1").await);
        assert!(!fixture.slot_free(1, "gpt-4.1").await);
        assert!(observers_a.cleanup.clone().now_or_never().is_none());
        assert!(observers_b.cleanup.clone().now_or_never().is_none());
        first.finish(Err(FailureKind::Cancelled));
        assert_eq!(completion_result(completed(&observers_a).await), Err(FailureKind::Cancelled));
        first.release().await;
        assert!(fixture.slot_free(0, "gpt-4.1").await);
        assert!(!fixture.slot_free(1, "gpt-4.1").await);
        pending(&mut body_b).await;
        second.push(b"data: second-still-running\n\n");
        second.eof();
        second.finish(Ok(()));
        assert_eq!(next(&mut body_b).await.unwrap().unwrap().as_ref(), b"data: second-still-running\n\n");
        assert!(next(&mut body_b).await.is_none());
        assert_eq!(completion_result(completed(&observers_b).await), Ok(()));
        drop(body_b);
        assert_eq!(second.probe.cancellations(), 0);
        second.release().await;
        assert!(fixture.slot_free(1, "gpt-4.1").await);
        fixture.assert_legacy_unused();
    }
}

#[tokio::test(flavor = "current_thread")]
async fn raw_stream_preserves_actual_headers_status_usage_and_fragmented_bytes() {
    for status in [201, 429] {
        let fixture = Fixture::new(1, Duration::from_secs(1), None);
        let mut control = fixture.enqueue(0,
            headers(status, "application/octet-stream", Some("fixture-usage; exact=1")),
            "text/event-stream");
        let fragments: [&'static [u8]; 3] = [b"data: \xf0", b"\x9f\x98", b"\x80\n\n"];
        for fragment in fragments { control.push(fragment); }
        control.open();
        control.eof();
        control.finish(Ok(()));
        let response = fixture.request("/paid/openai/v1/responses", responses(), 0,
            "fixture-chat", false).await;
        assert_eq!(response.status().as_u16(), status);
        assert_eq!(response.headers()["content-type"], "application/octet-stream");
        assert_eq!(response.headers()["x-framer-ai-usage"], "fixture-usage; exact=1");
        assert_eq!(response.headers()["x-framerize-session"], "fixture-chat");
        assert!(Arc::ptr_eq(response.extensions().get::<Arc<RawStreamHeaders>>().unwrap(),
            &control.headers));
        let observers = response.extensions().get::<NativeStreamObservers>().unwrap().clone();
        let mut stream = response.into_body().into_data_stream();
        for fragment in fragments {
            assert_eq!(next(&mut stream).await.unwrap().unwrap().as_ref(), fragment);
        }
        assert!(next(&mut stream).await.is_none());
        match observers.completion.await {
            NativeStreamCompletion::Raw(value) => {
                assert_eq!(value.result, Ok(()));
                assert_eq!(value.emitted_bytes, fragments.iter().map(|f| f.len()).sum::<usize>());
                assert!(Arc::ptr_eq(value.headers.as_ref().unwrap(), &control.headers));
            }
            _ => panic!("wrong completion kind"),
        }
        assert!(observers.cleanup.now_or_never().is_none());
        assert!(!fixture.slot_free(0, "gpt-4.1").await);
        drop(stream);
        assert_eq!(control.probe.cancellations(), 0);
        control.release().await;
        assert!(fixture.slot_free(0, "gpt-4.1").await);
        fixture.assert_legacy_unused();
    }
}

const ROUTES: [(&str, ClientDialect, bool, bool); 9] = [
    ("/paid/anthropic/v1/messages", ClientDialect::AnthropicMessages, false, true),
    ("/paid/openai/v1/responses", ClientDialect::OpenAiResponses, false, true),
    ("/paid/openai/v1/chat/completions", ClientDialect::OpenAiChat, false, true),
    ("/anthropic/v1/messages", ClientDialect::AnthropicMessages, true, true),
    ("/openai/v1/responses", ClientDialect::OpenAiResponses, true, true),
    ("/openai/v1/chat/completions", ClientDialect::OpenAiChat, true, true),
    ("/anthropic/v1/messages", ClientDialect::AnthropicMessages, false, false),
    ("/openai/v1/responses", ClientDialect::OpenAiResponses, false, false),
    ("/openai/v1/chat/completions", ClientDialect::OpenAiChat, false, false),
];

fn payload(dialect: ClientDialect, raw: bool) -> Value {
    if dialect == ClientDialect::OpenAiResponses { return responses(); }
    let mut value = json!({"model":"gpt-4.1", "stream":true, "max_tokens":32,
        "messages":[{"role":"user","content":"fixture"}],
        "fixture_extension":{"opaque":true}});
    if dialect == ClientDialect::OpenAiChat {
        value["stream_options"] = if raw {
            json!({"include_usage":true,"opaque_raw_option":"kept"})
        } else { json!({"include_usage":true}) };
    }
    value
}

#[tokio::test(flavor = "current_thread")]
async fn all_nine_routes_choose_stream_kind_and_preserve_options_identity_deadline() {
    for (path, dialect, paid, raw) in ROUTES {
        let fixture = Fixture::new(1, Duration::from_secs(1), None);
        let mut control = fixture.enqueue(0, headers(200, "application/json", Some("usage")),
            "text/event-stream");
        let original = payload(dialect, raw);
        control.open();
        control.eof();
        control.finish(Ok(()));
        let before = Instant::now();
        let response = fixture.request(path, original.clone(), 0, "selected-session", paid).await;
        let after = Instant::now();
        assert_eq!(response.status(), StatusCode::OK, "{path}");
        let records = fixture.records();
        assert_eq!(records.len(), 1);
        let record = &records[0];
        let account = &fixture.accounts[0];
        assert_eq!(record.identity.account, account.id);
        assert_eq!(record.identity.user, account.framer_user_id);
        assert_eq!(record.identity.team, account.current_team.clone().unwrap());
        assert_eq!(record.identity.project, account.current_project.clone().unwrap());
        assert_eq!(record.identity.chat_session, "selected-session");
        assert_eq!(record.max_total_tokens, 32);
        assert!(record.deadline >= before + Duration::from_secs(1));
        assert!(record.deadline <= after + Duration::from_secs(1));

        if raw {
            assert_eq!(record.method, "raw_stream");
            assert_eq!(record.route, Route::Raw(dialect));
            assert_eq!(record.raw, Some(fixture.streaming.raw));
            assert!(record.composed.is_none());
            assert_eq!(record.payload, original);
            assert_eq!(response.headers()["content-type"], "application/json");
        } else {
            assert_eq!(record.method, "composed_stream");
            let expected_route = if dialect == ClientDialect::AnthropicMessages {
                Route::Messages
            } else { Route::Responses };
            assert_eq!(record.route, expected_route);
            assert!(record.raw.is_none());
            let options = record.composed.unwrap();
            assert_eq!(options.dialect, dialect);
            assert_eq!(options.include_usage, dialect == ClientDialect::OpenAiChat);
            assert_eq!(options.limits, fixture.streaming.composed);
            assert_eq!(record.payload["fixture_extension"], original["fixture_extension"]);
            if dialect == ClientDialect::OpenAiChat {
                assert!(record.payload.get("messages").is_none());
                assert_eq!(record.payload["input"][0]["content"][0]["text"], "fixture");
            } else {
                let field = if dialect == ClientDialect::AnthropicMessages { "messages" } else { "input" };
                assert_eq!(record.payload[field], original[field]);
            }
            assert_eq!(response.headers()["content-type"], "text/event-stream");
        }
        let mut body = response.into_body().into_data_stream();
        assert!(next(&mut body).await.is_none());
        control.release().await;
        fixture.assert_legacy_unused();
    }
}

#[tokio::test(flavor = "current_thread")]
async fn invalid_composed_options_fail_before_any_invocation() {
    for invalid in [json!(false), json!({"include_usage":"yes"}), json!({"unknown":true})] {
        let fixture = Fixture::new(1, Duration::from_secs(1), None);
        let mut body = payload(ClientDialect::OpenAiChat, false);
        body["stream_options"] = invalid;
        let response = fixture.request("/openai/v1/chat/completions", body, 0, "session", false).await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert_eq!(response.extensions().get::<FailureKind>(), Some(&FailureKind::InvalidRequest));
        assert!(fixture.records().is_empty());
        fixture.assert_legacy_unused();
    }
}

#[tokio::test(flavor = "current_thread")]
async fn pre_header_deadline_cancels_without_committing_success_or_waiting_cleanup() {
    for path in ["/paid/openai/v1/responses", "/openai/v1/responses"] {
        let fixture = Fixture::new(1, Duration::from_millis(80), None);
        let mut control = fixture.enqueue(0, headers(201, "text/event-stream", Some("real-usage")),
            "text/event-stream");
        let response = fixture.request(path, responses(), 0, "session", false).await;
        assert_eq!(response.status(), StatusCode::GATEWAY_TIMEOUT);
        assert_eq!(response.extensions().get::<FailureKind>(), Some(&FailureKind::Deadline));
        assert!(response.extensions().get::<NativeStreamObservers>().is_none());
        assert!(response.headers().get("x-framer-ai-usage").is_none());
        assert_eq!(control.probe.cancellations(), 1);
        assert_eq!(control.probe.body_polls.load(Ordering::SeqCst), 0);
        assert_eq!(control.probe.body_drops.load(Ordering::SeqCst), 1);
        assert!(!fixture.slot_free(0, "gpt-4.1").await);
        control.release().await;
        assert!(fixture.slot_free(0, "gpt-4.1").await);
        fixture.assert_legacy_unused();
    }
}

#[tokio::test(flavor = "current_thread")]
async fn dropping_pre_header_request_cancels_its_owned_run() {
    let fixture = Fixture::new(1, Duration::from_secs(1), None);
    let mut control = fixture.enqueue(0, headers(200, "text/event-stream", None), "text/event-stream");
    let mut request = Box::pin(fixture.request("/paid/openai/v1/responses",
        responses(), 0, "session", false));
    assert!(tokio::time::timeout(IDLE, request.as_mut()).await.is_err());
    assert_eq!(fixture.records().len(), 1);
    assert_eq!(control.probe.cancellations(), 0);
    drop(request);
    assert_eq!(control.probe.cancellations(), 1);
    assert_eq!(control.probe.body_drops.load(Ordering::SeqCst), 1);
    assert!(!fixture.slot_free(0, "gpt-4.1").await);
    control.release().await;
    fixture.assert_legacy_unused();
}

#[tokio::test(flavor = "current_thread")]
async fn pre_header_error_never_commits_a_streaming_success() {
    let fixture = Fixture::new(1, Duration::from_secs(1), None);
    let mut control = fixture.enqueue(0, headers(200, "text/event-stream", None), "text/event-stream");
    control.reject(FailureKind::Transport);
    let response = fixture.request("/openai/v1/responses", responses(), 0, "session", false).await;
    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    assert_eq!(response.extensions().get::<FailureKind>(), Some(&FailureKind::Transport));
    assert!(response.extensions().get::<NativeStreamObservers>().is_none());
    assert_eq!(control.probe.cancellations(), 1);
    assert_eq!(control.probe.body_polls.load(Ordering::SeqCst), 0);
    control.release().await;
    fixture.assert_legacy_unused();
}

#[tokio::test(flavor = "current_thread")]
async fn dropping_unpolled_body_cancels_without_consuming_or_releasing_cleanup_early() {
    for path in ["/paid/openai/v1/responses", "/openai/v1/responses"] {
        let fixture = Fixture::new(1, Duration::from_secs(1), None);
        let mut control = fixture.enqueue(0, headers(200, "text/event-stream", None), "text/event-stream");
        control.open();
        control.push(b"data: unconsumed\n\n");
        let response = fixture.request(path, responses(), 0, "session", false).await;
        let observers = response.extensions().get::<NativeStreamObservers>().unwrap().clone();
        assert_eq!(control.probe.body_polls.load(Ordering::SeqCst), 0);
        drop(response);
        assert_eq!(control.probe.cancellations(), 1);
        assert_eq!(control.probe.body_polls.load(Ordering::SeqCst), 0);
        assert_eq!(control.probe.body_drops.load(Ordering::SeqCst), 1);
        assert!(observers.cleanup.now_or_never().is_none());
        assert!(!fixture.slot_free(0, "gpt-4.1").await);
        control.finish(Err(FailureKind::Cancelled));
        let completed = tokio::time::timeout(BOUND, observers.completion).await.unwrap();
        let result = match completed {
            NativeStreamCompletion::Raw(value) => value.result,
            NativeStreamCompletion::Composed(value) => value.result,
        };
        assert_eq!(result, Err(FailureKind::Cancelled));
        control.release().await;
        fixture.assert_legacy_unused();
    }
}

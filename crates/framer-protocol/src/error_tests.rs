use super::{chain_error_cause, is_security_refusal};
use crate::convert::{error_frames, error_json, ClientDialect};

fn dialects() -> [ClientDialect; 3] {
    [ClientDialect::AnthropicMessages, ClientDialect::OpenAiResponses, ClientDialect::OpenAiChat]
}

#[test]
fn security_refusal_requires_exact_known_cause() {
    for error in ["risk-refused", "chunk 12 retry died (fallback off): risk-refused"] {
        assert!(is_security_refusal(error), "{error}");
        assert_eq!(chain_error_cause(error), "risk-refused");
    }
    for error in ["transport closed: risk-refused", "retryable 500 on /ai/v3/responses/: risk-refused",
        "upstream 403 on /ai/v3/responses/: risk-refused",
        "chunk 0 retry died (fallback off): upstream 403 on /p: risk-refused",
        "chunk 0 retry died (fallback off): retryable 500 on /p: risk-refused",
        "chunk x retry died (fallback off): risk-refused",
        "chunk +1 retry died (fallback off): risk-refused",
        "chunk 4294967296 retry died (fallback off): risk-refused",
        "chunk 1 failed: risk-refused", "risk-refused trailing"]
    {
        assert!(!is_security_refusal(error), "{error}");
    }
}

#[test]
fn temporary_internal_errors_are_readable_in_every_dialect() {
    for (error, expected) in [
        ("risk-refused", "The upstream service is temporarily unavailable. Please retry shortly."),
        ("account-cooling:584", "This account is temporarily cooling down. Retry in 584 seconds."),
        ("server-cooling:150", "Framerize is temporarily unavailable. Retry in 150 seconds."),
        ("castle-unavailable", "Framerize could not prepare the request. Please retry shortly."),
        ("retryable 0 on /ai/v3/responses/: castle-unavailable",
            "Framerize could not prepare the request. Please retry shortly."),
        ("device request budget unavailable", "No account is ready for this request yet. Please retry shortly."),
        ("velocity-paused: every device is cooling; retry in 19s",
            "No account is ready for this request yet. Please retry shortly."),
        ("host-lifecycle-unconfirmed: sidecar exited",
            "Framerize cannot safely start another request yet. Please retry later."),
    ] {
        for input in [error.to_owned(), format!("chunk 500 retry died (fallback off): {error}")] {
            for dialect in dialects() {
                let (status, body) = error_json(dialect, &input);
                assert_eq!(status, 503, "{input}");
                assert_eq!(body["error"]["type"], "overloaded_error");
                assert_eq!(body["error"]["message"], expected);
            }
        }
    }
}

#[test]
fn transport_and_http_errors_do_not_become_security_or_capacity_failures() {
    for error in ["transport closed: risk-refused", "retryable 500 on /p: risk-refused",
        "retryable 500 on /p: castle-unavailable", "transport closed: account-cooling:10"]
    {
        for input in [error.to_owned(), format!("chunk 0 retry died (fallback off): {error}")] {
            for dialect in dialects() {
                let (status, body) = error_json(dialect, &input);
                assert_eq!(status, 500, "{input}");
                assert_eq!(body["error"]["message"], input);
                assert!(!is_security_refusal(&input));
            }
        }
    }
}

#[test]
fn provider_refusals_keep_status_and_message_even_with_internal_words() {
    let message = "Provider policy rejected castle-unavailable and risk-refused";
    let body = serde_json::json!({"error": {"message": message, "type": "permission_error"}});
    let error = format!("upstream 403 on /ai/v3/responses/: {body}");
    for input in [error.clone(), format!("chunk 500 retry died (fallback off): {error}")] {
        for dialect in dialects() {
            let (status, body) = error_json(dialect, &input);
            assert_eq!(status, 403);
            assert_eq!(body["error"]["type"], "permission_error");
            assert_eq!(body["error"]["message"], message);
            assert!(!is_security_refusal(&input));
        }
    }
    let plain = "upstream 403 on /ai/v3/responses/: provider rejected risk-refused";
    for dialect in dialects() {
        let (status, body) = error_json(dialect, plain);
        assert_eq!(status, 403);
        assert_eq!(body["error"]["message"], plain);
    }
}

#[test]
fn wrapped_context_errors_keep_the_compaction_signal() {
    let body = serde_json::json!({"error": {"message": "prompt is too long",
        "type": "invalid_request_error"}});
    let error = format!("chunk 500 retry died (fallback off): retryable 400 on /p: {body}");
    for dialect in dialects() {
        let (status, body) = error_json(dialect, &error);
        assert_eq!(status, 400);
        assert_eq!(body["error"]["message"], "prompt is too long");
        assert_eq!(body["error"]["type"], "invalid_request_error");
        if !matches!(dialect, ClientDialect::AnthropicMessages) {
            assert_eq!(body["error"]["code"], "context_length_exceeded");
        }
    }
}

#[test]
fn terminal_frames_hide_internal_errors_without_reporting_success() {
    for error in ["risk-refused", "server-cooling:150",
        "chunk 0 retry died (fallback off): retryable 0 on /ai/v3/responses/: castle-unavailable",
        "host-lifecycle-unconfirmed: private diagnostic"]
    {
        for dialect in dialects() {
            let frames = error_frames(dialect, error).join("\n");
            for raw in ["risk-refused", "server-cooling", "castle-unavailable",
                "fallback off", "host-lifecycle-unconfirmed", "private diagnostic"]
            {
                assert!(!frames.contains(raw), "{frames}");
            }
            assert!(frames.contains("overloaded_error"));
            assert!(!frames.contains("response.completed"));
            if matches!(dialect, ClientDialect::OpenAiResponses) {
                assert!(frames.contains("response.failed"));
            }
        }
    }
}

#[test]
fn malformed_or_embedded_cooldown_text_is_not_internal_capacity() {
    for error in ["server-cooling:bad", "server-cooling:+1", "server-cooling:1 trailing",
        "server-cooling:18446744073709551616", "transport error: server-cooling:1",
        "chunk +1 retry died (fallback off): account-cooling:1"]
    {
        assert_eq!(error_json(ClientDialect::OpenAiResponses, error).0, 500, "{error}");
    }
    let (_, body) = error_json(ClientDialect::OpenAiResponses, "server-cooling:0");
    assert_eq!(body["error"]["message"], "Framerize is temporarily unavailable. Retry in 1 second.");
}

#[test]
fn cleanup_diagnostics_cannot_be_mistaken_for_provider_error_json() {
    let error = "host-lifecycle-unconfirmed: internal 500: {\"error\":{\"message\":\"private detail\"}}";
    let (status, body) = error_json(ClientDialect::OpenAiResponses, error);
    assert_eq!(status, 503);
    assert_eq!(body["error"]["message"],
        "Framerize cannot safely start another request yet. Please retry later.");
}

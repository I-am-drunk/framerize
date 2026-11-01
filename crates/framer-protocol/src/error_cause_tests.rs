use crate::convert::{error_json, error_frames, ClientDialect};

#[test]
fn host_failures_keep_command_deadline_and_io_cause_without_credentials() {
    for cause in [
        "sidecar account command timed out after 240s: ai_post",
        "sidecar write: Broken pipe (os error 32)",
        "host-lifecycle-unconfirmed: sidecar exited with signal 9",
    ] {
        let error = format!("{cause}; Authorization: Bearer private-fixture");
        let (status, body) = error_json(ClientDialect::OpenAiResponses, &error);
        assert_eq!(status, 503);
        let message = body["error"]["message"].as_str().unwrap();
        assert!(message.contains(cause.strip_prefix("host-lifecycle-unconfirmed: ")
            .unwrap_or(cause)), "{message}");
        assert!(!message.contains("private-fixture"));
    }
}

#[test]
fn remote_errors_keep_status_and_observed_cause_without_json() {
    for (status, reason) in [(502, "connection reset by peer"),
        (503, "Cloudflare Worker exceeded resource limits"),
        (400, "Model call failed: invalid request (400)")] {
        let error = format!("retryable {status} on /ai/v3/responses/: {reason}");
        let (actual, body) = error_json(ClientDialect::OpenAiResponses, &error);
        assert_eq!(actual, status);
        let message = body["error"]["message"].as_str().unwrap();
        assert!(message.contains(reason) || message.contains("exceeded its resource limits"), "{message}");
    }
    let (status, body) = error_json(ClientDialect::OpenAiResponses,
        "upstream 502 on /ai/v3/responses/: connection reset; Authorization: Bearer private-fixture");
    assert_eq!(status, 502);
    let message = body["error"]["message"].as_str().unwrap();
    assert!(message.contains("connection reset"));
    assert!(!message.contains("private-fixture"));
}

#[test]
fn workspace_saturation_is_distinct_from_provider_limits() {
    use crate::error::is_workspace_saturation;
    let prefix = "retryable 429 on /ai/v3/responses/: ";
    assert!(is_workspace_saturation(&format!("{prefix}too many concurrent AI requests for this workspace")));
    for body in ["upstream_provider_shared_pool", "Rate limit exceeded", "Too many requests"] {
        assert!(!is_workspace_saturation(&format!("{prefix}{body}")));
    }
}

#[test]
fn memory_pressure_survives_wrapping_in_json_and_stream_errors() {
    let cause = "Low available RAM: host available 32 MiB; request needs 64 MiB";
    for dialect in [ClientDialect::OpenAiResponses, ClientDialect::OpenAiChat,
        ClientDialect::AnthropicMessages] {
        let error = format!("chunk 1 retry died (fallback off): {cause}");
        let (status, body) = error_json(dialect, &error);
        assert_eq!(status, 503);
        assert_eq!(body["error"]["message"], cause);
        assert!(error_frames(dialect, &error).join("").contains(cause));
    }
}

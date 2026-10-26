use crate::convert::{error_json, ClientDialect, ResponseEncoder};
use crate::response_stream::collect;

#[test]
fn preclassified_failure_keeps_kernel_code_under_redacted_display() {
    let (_, mut shaped) = error_json(ClientDialect::OpenAiResponses,
        "maximum context length: private-fixture-sentinel");
    shaped["error"]["message"] = "upstream operation failed".into();
    let mut encoder = ResponseEncoder::new("fixture", "fixed-id");
    let frames = collect(|writer| encoder.fail_shaped(writer, &shaped["error"]));
    assert_eq!(frames.len(), 1);
    let data = frames[0].split_once("\ndata: ").unwrap().1.trim();
    let event: serde_json::Value = serde_json::from_str(data).unwrap();
    assert_eq!(event["response"]["error"]["message"], "upstream operation failed");
    assert_eq!(event["response"]["error"]["type"], "api_error");
    assert_eq!(event["response"]["error"]["code"], "context_length_exceeded");
    assert_eq!(event["sequence_number"], 0);
    assert!(!frames[0].contains("private-fixture-sentinel"));
    let (_, reparsed) = error_json(ClientDialect::OpenAiResponses, "upstream operation failed");
    assert_eq!(reparsed["error"]["code"], "api_error");
}

fn failure(message: &str, preclassified: bool) -> Vec<String> {
    let mut encoder = ResponseEncoder::new("fixture", "fixed-id");
    collect(|writer| {
        encoder.heartbeat(writer)?;
        if preclassified {
            let (_, shaped) = error_json(ClientDialect::OpenAiResponses, message);
            encoder.fail_shaped(writer, &shaped["error"])?;
        } else {
            encoder.fail(writer, message)?;
        }
        encoder.heartbeat(writer)
    })
}

#[test]
fn preclassified_failure_preserves_legacy_bytes_and_sequence() {
    for message in ["risk-refused", "ordinary failure", "maximum context length",
        r#"upstream 429: {"error":{"message":"busy","type":"api_error"}}"#] {
        let frames = failure(message, false);
        assert_eq!(frames, failure(message, true));
        assert!(frames[1].contains("\"sequence_number\":1"));
        assert!(frames[2].contains("\"sequence_number\":2"));
    }
}

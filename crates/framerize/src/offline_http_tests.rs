use super::*;

#[test]
fn completion_event_omits_private_data_and_preserves_final_cleanup() {
    use framer_api::native_dispatch::complete_json::{ContentType, Evidence};
    let mut outcome = offline_serving::Outcome {
        response: Some(serde_json::json!({"secret": "private-marker"})),
        failure: Some(FailureKind::Cleanup), cleanup_confirmed: false,
        evidence: Some(Evidence {
            accepted: false, path: "/private-marker", client_model: "private-marker".into(),
            wire_model: "private-marker".into(), status: Some(403), content_type: ContentType::Json,
            usage_header_present: true, input_tokens: Some(123), output_tokens: Some(456),
            output_items: 0, attempts: 1, authorization_calls: 1,
            transport_starts: 1, transport_finishes: 1, complete_json: false,
            cleanup_confirmed: true, failure: Some(FailureKind::Upstream), refusal: None,
        }),
    };
    let fields = completion_fields(&outcome);
    assert_eq!(fields["cleanup_confirmed"], false);
    assert_eq!(fields["failure"], "Cleanup");
    assert_eq!(fields["attempt"]["status"], 403);
    assert_eq!(fields["attempt"]["cleanup_confirmed"], true);
    let serialized = fields.to_string();
    for private in ["private-marker", "response", "path", "client_model", "wire_model",
        "input_tokens", "output_tokens"] {
        assert!(!serialized.contains(private), "unexpected private field: {private}");
    }
    outcome.evidence = None;
    assert!(completion_fields(&outcome)["attempt"].is_null());
    assert_eq!(completion_fields(&outcome)["cleanup_confirmed"], false);
}

fn input() -> serde_json::Value {
    serde_json::json!({
        "account_id": "00000000-0000-0000-0000-000000000001",
        "user_id":"fixture-user", "team_id":"fixture-team", "project_id":"fixture-project",
        "conversation_id":"fixture-conversation",
        "page_url":"https://framer.com/projects/fixture-project",
        "route":"/ai/v3/responses/", "feature_category":"fixture-category",
        "feature_name":"fixture-feature", "step":"fixture-step",
        "turn_id":"fixture-turn", "user_prompt":"fixture annotation",
        "timeout_ms":3000, "token_limit":32,
        "request":{"model":"gpt-4.1", "input":"fixture text", "stream":false}
    })
}

#[test]
fn explicit_http_request_requires_every_context_field() {
    for field in ["user_id", "team_id", "project_id", "conversation_id", "page_url",
        "feature_category", "feature_name", "step", "turn_id", "user_prompt"] {
        let mut value = input(); value.as_object_mut().unwrap().remove(field);
        assert!(serde_json::from_value::<Input>(value).is_err());
    }
}

#[test]
fn explicit_http_request_preserves_only_supported_bounded_routes() {
    assert!(request(serde_json::from_value(input()).unwrap()).is_ok());
    for change in 0..4 {
        let mut value = input();
        match change {
            0 => value["route"] = "/ai/v3/openai/responses/".into(),
            1 => value["timeout_ms"] = 60_001.into(),
            2 => value["request"]["stream"] = true.into(),
            _ => value["page_url"] = "https://framer.com/projects/other".into(),
        }
        assert!(request(serde_json::from_value(value).unwrap()).is_err());
    }
}

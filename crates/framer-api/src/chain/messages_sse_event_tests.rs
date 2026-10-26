use super::*;

#[test]
fn named_error_frames_fail_independently_of_payload_type() {
    let valid = encode(&text_events("end_turn"));
    for error in ["event:error\ndata: {\"type\":\"ping\"}\n\n",
        "event: error\n\n", "event: error\ndata: not-json\n\n"] {
        for input in [format!("{error}{valid}"), format!("{valid}{error}")] {
            assert!(parse(&input).unwrap_err().contains("upstream error"));
        }
    }
    assert!(parse(&(valid + "event: error")).unwrap_err().contains("upstream error"));
}

#[test]
fn event_fields_use_the_last_value_and_allow_empty_frames() {
    for prefix in ["event: error\nevent: ping\ndata: {\"type\":\"ping\"}\n\n",
        "event: error\nevent:\n\n", "event: error\nevent\n\n"] {
        let input = format!("{prefix}{}", encode(&text_events("end_turn")));
        assert_eq!(parse(&input).unwrap().content,
            vec![json!({"type":"text","text":"prefix suffix"})]);
    }
}

use super::from_payload;
use serde_json::{json, Value};

const FIELDS: [&str; 3] = ["max_tokens", "max_completion_tokens", "max_output_tokens"];
const TOO_LARGE: &str = "output token limit exceeds 4294967295";

#[test]
fn missing_limits_use_default() {
    assert_eq!(from_payload(&json!({})), Ok(4096));
}

#[test]
fn non_u64_limits_use_default() {
    for value in [
        Value::Null, json!(-1), json!(12.5), json!(32.0),
        json!(18446744073709551616.0), json!("32"), json!(true), json!([]), json!({}),
    ] {
        assert_eq!(from_payload(&json!({
            "max_tokens": value.clone(),
            "max_completion_tokens": value.clone(),
            "max_output_tokens": value,
        })), Ok(4096));
    }
}

#[test]
fn each_field_accepts_u32_bounds() {
    for field in FIELDS {
        for limit in [0, 1, 4096, u32::MAX] {
            let mut payload = json!({});
            payload[field] = json!(limit);
            assert_eq!(from_payload(&payload), Ok(limit), "{field}");
        }
    }
}

#[test]
fn max_tokens_has_first_precedence_including_zero() {
    for limit in [0, 17] {
        assert_eq!(from_payload(&json!({
            "max_tokens": limit,
            "max_completion_tokens": 31,
            "max_output_tokens": 47,
        })), Ok(limit));
    }
}

#[test]
fn invalid_max_tokens_falls_back_to_completion_before_output() {
    for value in [
        Value::Null, json!(-1), json!(1.5), json!(32.0),
        json!("32"), json!(false), json!([]), json!({}),
    ] {
        assert_eq!(from_payload(&json!({
            "max_tokens": value,
            "max_completion_tokens": 23,
            "max_output_tokens": 41,
        })), Ok(23));
    }
}

#[test]
fn invalid_preceding_limits_fall_back_to_output() {
    for value in [
        Value::Null, json!(-1), json!(1.5), json!(32.0),
        json!("32"), json!(false), json!([]), json!({}),
    ] {
        assert_eq!(from_payload(&json!({
            "max_tokens": value.clone(),
            "max_completion_tokens": value,
            "max_output_tokens": 29,
        })), Ok(29));
    }
}

#[test]
fn oversized_selected_limit_is_rejected_without_falling_back() {
    for (index, field) in FIELDS.into_iter().enumerate() {
        let mut payload = json!({"messages": "private-payload-sentinel"});
        payload[field] = json!(u64::from(u32::MAX) + 1);
        for fallback in &FIELDS[index + 1..] {
            payload[*fallback] = json!(17);
        }
        assert_eq!(from_payload(&payload), Err(TOO_LARGE), "{field}");
    }
}

#[test]
fn u64_max_selected_limit_is_rejected() {
    for field in FIELDS {
        let mut payload = json!({});
        payload[field] = json!(u64::MAX);
        assert_eq!(from_payload(&payload), Err(TOO_LARGE), "{field}");
    }
}

#[test]
fn ignored_oversized_limits_do_not_reject_valid_selection() {
    assert_eq!(from_payload(&json!({
        "max_tokens": 0,
        "max_completion_tokens": u64::MAX,
        "max_output_tokens": u64::MAX,
    })), Ok(0));
    assert_eq!(from_payload(&json!({
        "max_tokens": null,
        "max_completion_tokens": 31,
        "max_output_tokens": u64::MAX,
    })), Ok(31));
}

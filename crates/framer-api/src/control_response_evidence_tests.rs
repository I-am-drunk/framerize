use super::*;
use reqwest::header::HeaderValue;

#[test]
fn response_cookie_values_and_unknown_names_cannot_enter_evidence() {
    let mut headers = HeaderMap::new();
    for value in ["session=private-session; Secure", "__cuid=private-device",
        "private-name=private-value", "private-malformed"] {
        headers.append(SET_COOKIE, HeaderValue::from_static(value));
    }
    headers.insert("authorization", HeaderValue::from_static("private-bearer"));
    let value = summary(200, &headers);
    assert_eq!(value, json!({"status": 200, "set_cookie_count": 4,
        "session_count": 1, "device_count": 1, "other_count": 1,
        "malformed_count": 1, "truncated": false}));
    assert!(!value.to_string().contains("private"));
}

#[test]
fn absent_cookies_are_distinct_from_truncated_evidence() {
    assert_eq!(summary(403, &HeaderMap::new()), json!({"status": 403,
        "set_cookie_count": 0, "session_count": 0, "device_count": 0,
        "other_count": 0, "malformed_count": 0, "truncated": false}));
}

#[test]
fn cookie_evidence_is_bounded_and_names_are_case_sensitive() {
    let mut headers = HeaderMap::new();
    for _ in 0..64 {
        headers.append(SET_COOKIE, HeaderValue::from_static("Session=private"));
    }
    let exact = summary(200, &headers);
    assert_eq!(exact["other_count"], 64);
    assert_eq!(exact["session_count"], 0);
    assert_eq!(exact["truncated"], false);
    headers.append(SET_COOKIE, HeaderValue::from_static("__cuid=private"));
    let capped = summary(200, &headers);
    assert_eq!(capped["set_cookie_count"], 64);
    assert_eq!(capped["device_count"], 0);
    assert_eq!(capped["truncated"], true);
}

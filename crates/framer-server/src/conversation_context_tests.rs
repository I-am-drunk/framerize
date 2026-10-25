use super::*;
use axum::http::HeaderValue;

const FIRST: &str = "01a15bb1-94ab-77f2-abfa-d666d07e9131";
const SECOND: &str = "01a15bb1-953e-75d1-b3b8-d99428904f2f";
const FORK: &str = "01a15bb1-95b7-7f62-9894-fd4614334a2f";

fn native(thread: &'static str) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert("thread-id", HeaderValue::from_static(thread));
    headers.insert("session-id", HeaderValue::from_static(thread));
    headers
}

#[test]
fn invalid_explicit_context_cannot_fall_back() {
    for invalid in [HeaderValue::from_static(""), HeaderValue::from_bytes(&[255]).unwrap()] {
        let mut headers = native(FIRST);
        headers.insert("x-framerize-session", invalid);
        assert_eq!(session(&headers), Err(ContextError::Invalid));
    }
}

#[test]
fn duplicate_explicit_context_cannot_choose_one_value_or_fall_back() {
    for second in [FIRST, SECOND] {
        let mut headers = native(FORK);
        headers.append("x-framerize-session", HeaderValue::from_static(FIRST));
        headers.append("x-framerize-session", HeaderValue::from_static(second));
        assert_eq!(session(&headers), Err(ContextError::Ambiguous));
    }
}

#[test]
fn native_thread_is_opaque_and_does_not_require_a_second_header() {
    let mut headers = HeaderMap::new();
    headers.insert("thread-id", HeaderValue::from_static("thr_CaseSensitive-123"));
    assert_eq!(session(&headers), Ok(Some("thr_CaseSensitive-123")));
}

#[test]
fn conflicting_native_headers_fail_without_picking_a_conversation() {
    let mut headers = native(FIRST);
    headers.insert("session-id", HeaderValue::from_static(SECOND));
    assert_eq!(session(&headers), Err(ContextError::Ambiguous));
}

#[test]
fn duplicate_native_headers_are_rejected_even_when_the_values_match() {
    for name in ["thread-id", "session-id"] {
        for extra in [FIRST, SECOND] {
            let mut headers = native(FIRST);
            headers.append(name, HeaderValue::from_static(extra));
            assert_eq!(session(&headers), Err(ContextError::Ambiguous));
        }
    }
}

#[test]
fn malformed_native_identity_cannot_be_silently_normalized() {
    for name in ["thread-id", "session-id"] {
        for invalid in [
            HeaderValue::from_static(""),
            HeaderValue::from_static("two ids"),
            HeaderValue::from_static("one,two"),
            HeaderValue::from_static("tab\tvalue"),
            HeaderValue::from_bytes(&[255]).unwrap(),
        ] {
            let mut headers = native(FIRST);
            headers.insert(name, invalid);
            assert_eq!(session(&headers), Err(ContextError::Invalid));
        }
    }
}

#[test]
fn credentials_account_request_and_session_only_headers_are_not_identity() {
    let mut headers = HeaderMap::new();
    for name in ["authorization", "cookie", "x-framerize-account", "x-request-id",
        "x-client-request-id", "session_id", "session-id", "conversation_id"] {
        headers.insert(name, HeaderValue::from_static(FIRST));
    }
    assert_eq!(session(&headers), Ok(None));
    assert_eq!(session(&HeaderMap::new()), Ok(None));
}

#[test]
fn captured_lifecycle_ids_stay_stable_on_resume_and_distinct_on_fork() {
    // These are the local fixture's IDs, not user conversation IDs.
    // This parser is stateless; the streaming test exercises the full lifecycle.
    for expected in [FIRST, SECOND, FORK] {
        let headers = native(expected);
        let selected = session(&headers).unwrap().unwrap();
        assert_eq!(selected, expected);
        assert!(std::ptr::eq(selected.as_ptr(), headers["thread-id"].as_bytes().as_ptr()));
    }
    assert_ne!(FIRST, SECOND);
    assert_ne!(FIRST, FORK);
    assert_ne!(SECOND, FORK);
}

#[test]
fn explicit_context_overrides_unrelated_or_invalid_native_headers() {
    let mut headers = native(FIRST);
    headers.insert("x-framerize-session", HeaderValue::from_static("Legacy:Case-Sensitive/Chat"));
    headers.insert("session-id", HeaderValue::from_static(SECOND));
    headers.append("thread-id", HeaderValue::from_bytes(&[255]).unwrap());
    let selected = session(&headers).unwrap().unwrap();
    assert_eq!(selected, "Legacy:Case-Sensitive/Chat");
    assert!(std::ptr::eq(selected.as_ptr(), headers["x-framerize-session"].as_bytes().as_ptr()));
}

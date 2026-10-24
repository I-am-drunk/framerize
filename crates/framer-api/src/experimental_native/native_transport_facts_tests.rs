use super::*;
use reqwest::header::HeaderValue;

fn headers(values: &[&[u8]]) -> HeaderMap {
    let mut headers = HeaderMap::new();
    for value in values {
        headers.append(CONTENT_ENCODING, HeaderValue::from_bytes(value).unwrap());
    }
    headers
}

fn facts(headers: &HeaderMap) -> Value {
    response_head(StatusCode::FORBIDDEN, Version::HTTP_11, headers)
}

#[test]
fn response_status_is_observed_without_claiming_acceptance() {
    assert_eq!(facts(&HeaderMap::new()), json!({"status": 403,
        "http_version": "http_11", "content_encoding": "absent"}));
}

#[test]
fn reports_received_protocol_instead_of_assuming_client_configuration() {
    let versions = [(Version::HTTP_09, "http_09"), (Version::HTTP_10, "http_10"),
        (Version::HTTP_11, "http_11"), (Version::HTTP_2, "http_2"),
        (Version::HTTP_3, "http_3")];
    for (version, expected) in versions {
        let value = response_head(StatusCode::OK, version, &HeaderMap::new());
        assert_eq!(value, json!({"status": 200, "http_version": expected,
            "content_encoding": "absent"}));
    }
}

#[test]
fn identity_observation_handles_all_header_values_and_comma_lists() {
    for values in [vec![b"identity".as_slice()], vec![b" Identity , IDENTITY ".as_slice()],
        vec![b"identity".as_slice(), b"identity".as_slice()]] {
        assert_eq!(facts(&headers(&values))["content_encoding"], "identity");
    }
    for values in [vec![b"".as_slice()], vec![b"identity,".as_slice()],
        vec![b"gzip".as_slice()], vec![b"identity".as_slice(), b"br".as_slice()]] {
        assert_eq!(facts(&headers(&values))["content_encoding"], "other");
    }
}

#[test]
fn invalid_encoding_is_fixed_and_independent_of_header_order() {
    for values in [vec![b"\xff".as_slice()],
        vec![b"gzip".as_slice(), b"\xff".as_slice()],
        vec![b"\xff".as_slice(), b"gzip".as_slice()]] {
        assert_eq!(facts(&headers(&values)), json!({"status": 403,
            "http_version": "http_11", "content_encoding": "invalid"}));
    }
}

#[test]
fn encoding_value_count_is_bounded_even_after_nonidentity_or_invalid_data() {
    let mut values = vec![b"identity".as_slice(); MAX_ENCODING_VALUES];
    assert_eq!(facts(&headers(&values))["content_encoding"], "identity");
    values.push(b"identity");
    for first in [b"identity".as_slice(), b"gzip".as_slice(), b"\xff".as_slice()] {
        values[0] = first;
        assert_eq!(facts(&headers(&values))["content_encoding"], "unavailable");
    }
}

#[test]
fn encoding_byte_budget_covers_the_sum_of_all_values() {
    let exact = format!("{:width$}", "identity", width = MAX_ENCODING_BYTES);
    assert_eq!(facts(&headers(&[exact.as_bytes()]))["content_encoding"], "identity");
    let over = format!("{exact} ");
    assert_eq!(facts(&headers(&[over.as_bytes()]))["content_encoding"], "unavailable");
    let half = format!("{:width$}", "identity", width = MAX_ENCODING_BYTES / 2);
    assert_eq!(facts(&headers(&[half.as_bytes(), half.as_bytes()]))["content_encoding"], "identity");
    assert_eq!(facts(&headers(&[half.as_bytes(), half.as_bytes(), b"identity"]))
        ["content_encoding"], "unavailable");
}

#[test]
fn raw_headers_and_arbitrary_encoding_text_never_enter_facts() {
    let mut headers = headers(&[b"private-encoding-sentinel"]);
    for name in ["authorization", "cookie", "set-cookie", "location", "x-private"] {
        headers.append(name, HeaderValue::from_static("private-header-sentinel"));
    }
    assert_eq!(facts(&headers), json!({"status": 403,
        "http_version": "http_11", "content_encoding": "other"}));
    headers.remove(CONTENT_ENCODING);
    assert_eq!(facts(&headers), facts(&HeaderMap::new()));
}

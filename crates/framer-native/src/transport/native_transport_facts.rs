//! Bounded header observations only. Never authorize dispatch or acceptance.
use reqwest::header::HeaderMap;
use reqwest::header::CONTENT_ENCODING;
use reqwest::StatusCode;
use reqwest::Version;
use serde_json::json;
use serde_json::Value;

const MAX_ENCODING_VALUES: usize = 8;
const MAX_ENCODING_BYTES: usize = 512;

pub(super) fn response_head(status: StatusCode, version: Version, headers: &HeaderMap) -> Value {
    let protocol = match version {
        Version::HTTP_09 => "http_09", Version::HTTP_10 => "http_10",
        Version::HTTP_11 => "http_11", Version::HTTP_2 => "http_2",
        Version::HTTP_3 => "http_3", _ => "other",
    };
    json!({"status": status.as_u16(), "http_version": protocol,
        "content_encoding": content_encoding(headers)})
}

fn content_encoding(headers: &HeaderMap) -> &'static str {
    let mut bytes = 0;
    let mut present = false;
    let mut other = false;
    let mut invalid = false;
    for (index, value) in headers.get_all(CONTENT_ENCODING).iter().enumerate() {
        if index >= MAX_ENCODING_VALUES || value.as_bytes().len() > MAX_ENCODING_BYTES - bytes {
            return "unavailable";
        }
        bytes += value.as_bytes().len();
        present = true;
        match value.to_str() {
            Ok(value) => other |= value.split(',')
                .any(|coding| !coding.trim().eq_ignore_ascii_case("identity")),
            Err(_) => invalid = true,
        }
    }
    if invalid { "invalid" } else if other { "other" }
    else if present { "identity" } else { "absent" }
}

#[cfg(test)]
#[path = "native_transport_facts_tests.rs"]
mod tests;

//! Value-free evidence for response cookies that the native control path ignores.
use reqwest::header::{HeaderMap, SET_COOKIE};
use serde_json::{json, Value};

pub(super) fn summary(status: u16, headers: &HeaderMap) -> Value {
    let (mut total, mut session, mut device, mut other, mut malformed) = (0, 0, 0, 0, 0);
    let mut truncated = false;
    for header in headers.get_all(SET_COOKIE).iter().take(65) {
        if total == 64 {
            truncated = true;
            break;
        }
        total += 1;
        let bytes = header.as_bytes();
        match bytes.iter().position(|byte| *byte == b'=') {
            Some(at) => match &bytes[..at] {
                b"session" => session += 1,
                b"__cuid" => device += 1,
                _ => other += 1,
            },
            None => malformed += 1,
        }
    }
    json!({"status": status, "set_cookie_count": total,
        "session_count": session, "device_count": device, "other_count": other,
        "malformed_count": malformed, "truncated": truncated})
}

pub(super) fn emit(status: u16, headers: &HeaderMap) {
    framer_core::observability::Event::new("native.auth.response")
        .field("cookie_summary", summary(status, headers)).emit();
}

#[cfg(test)]
#[path = "control_response_evidence_tests.rs"]
mod tests;

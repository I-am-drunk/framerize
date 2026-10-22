//! Caller conversation identity. This never supplies account authorization.
//!
//! Codex 0.163.0's `thread-id` matched its native thread ID in all six local
//! lifecycle captures, including fork and resume after process restart.
use axum::http::HeaderMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ContextError {
    Invalid,
    Ambiguous,
}

/// Borrow the exact caller value without allocating or normalizing identity.
/// An explicit Framerize value is authoritative, including validation errors.
/// `session-id` only corroborates `thread-id`; it is never an identity fallback.
pub(crate) fn session(headers: &HeaderMap) -> Result<Option<&str>, ContextError> {
    if headers.contains_key("x-framerize-session") {
        return unique(headers, "x-framerize-session");
    }
    let Some(thread) = unique(headers, "thread-id")? else {
        return Ok(None);
    };
    native_value(thread)?;
    if let Some(session) = unique(headers, "session-id")? {
        native_value(session)?;
        if session != thread {
            return Err(ContextError::Ambiguous);
        }
    }
    Ok(Some(thread))
}

fn unique<'a>(headers: &'a HeaderMap, name: &str) -> Result<Option<&'a str>, ContextError> {
    let mut values = headers.get_all(name).iter();
    let Some(value) = values.next() else {
        return Ok(None);
    };
    if values.next().is_some() {
        return Err(ContextError::Ambiguous);
    }
    let value = value.to_str().map_err(|_| ContextError::Invalid)?;
    if value.is_empty() {
        return Err(ContextError::Invalid);
    }
    Ok(Some(value))
}

fn native_value(value: &str) -> Result<(), ContextError> {
    // These are opaque IDs, not UUIDs by contract. Reject joined header lists
    // and whitespace instead of interpreting them as another conversation.
    if value.bytes().any(|byte| byte.is_ascii_whitespace() || byte == b',') {
        return Err(ContextError::Invalid);
    }
    Ok(())
}

#[cfg(test)]
#[path = "conversation_context_tests.rs"]
mod tests;

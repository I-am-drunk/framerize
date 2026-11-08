//! Dialect front mapping: client path -> upstream shape + stream-tail table.
//!
//! One engine, three protocol fronts. Each front's abort must land before
//! its usage-bearing terminal event, so the table records exactly where
//! that event sits per dialect. Data only for now -- the engine consumes it
//! when the live transport learns per-dialect framing (a later step).

/// Client-facing dialect of the incoming request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dialect {
    /// OpenAI chat/completions wire format.
    Chat,
    /// OpenAI Responses wire format.
    Responses,
    /// Anthropic Messages wire format.
    Messages,
}

/// Map a client request path to its dialect. `None` = not a known front.
///
/// Segment-aware: matches "/responses", "/chat/completions", "/messages" as
/// full path segments, so "/v1/messagesx" or "/responses-other" do NOT
/// resolve. Precedence is Responses > Chat > Messages for paths containing
/// several markers. Case-sensitive: server paths arrive normalized lowercase.
/// Query strings and fragments are stripped before matching so
/// "/v1/messages?x=1" still resolves.
pub fn dialect_from_path(path: &str) -> Option<Dialect> {
    let clean = path.split(['?', '#']).next().unwrap_or(path);
    fn has_segment(path: &str, seg: &str) -> bool {
        path.split('/').any(|s| s == seg)
    }
    fn has_chat(path: &str) -> bool {
        let segs: Vec<&str> = path.split('/').collect();
        segs.windows(2).any(|w| w[0] == "chat" && w[1] == "completions")
    }
    if has_segment(clean, "responses") {
        Some(Dialect::Responses)
    } else if has_chat(clean) {
        Some(Dialect::Chat)
    } else if has_segment(clean, "messages") {
        Some(Dialect::Messages)
    } else {
        None
    }
}

/// Where the usage-bearing terminal sits in a dialect's stream tail.
#[derive(Debug)]
pub struct FormatTail {
    /// The stream's final event.
    pub terminal: &'static str,
    /// The event that carries usage (settles the ledger). The graceful
    /// abort must land BEFORE this event.
    pub usage_event: &'static str,
    /// Operator note on where to cut.
    pub note: &'static str,
}

/// Per-dialect tail shape (verified against the live wire shapes the old
/// chains parsed: chat usage chunk, response.completed scan, Anthropic
/// message_delta/message_stop framing).
pub fn format_tail(d: Dialect) -> FormatTail {
    match d {
        Dialect::Chat => FormatTail {
            terminal: "usage",
            usage_event: "usage",
            note: "usage rides the terminal/end chunk alongside finish_reason; abort before it",
        },
        Dialect::Responses => FormatTail {
            terminal: "response.completed",
            usage_event: "response.completed",
            note: "the terminal object carries the usage payload; abort before it",
        },
        Dialect::Messages => FormatTail {
            terminal: "message_stop",
            usage_event: "message_delta",
            note: "message_delta carries usage, then message_stop closes; abort before message_delta",
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_mapping() {
        assert_eq!(
            dialect_from_path("/openai/v1/chat/completions"),
            Some(Dialect::Chat)
        );
        assert_eq!(
            dialect_from_path("/openai/v1/responses"),
            Some(Dialect::Responses)
        );
        assert_eq!(
            dialect_from_path("/anthropic/v1/messages"),
            Some(Dialect::Messages)
        );
        assert_eq!(dialect_from_path("/openai/v1/models"), None);
        assert_eq!(dialect_from_path(""), None);
        // segment discipline: near-miss paths do NOT resolve
        assert_eq!(dialect_from_path("/v1/messagesx"), None);
        assert_eq!(dialect_from_path("/responses-other"), None);
        assert_eq!(dialect_from_path("/v1/chat/completionsx"), None);
        // query strings still resolve
        assert_eq!(
            dialect_from_path("/anthropic/v1/messages?beta=true"),
            Some(Dialect::Messages)
        );
        // precedence: a path containing both resolves Responses-first
        assert_eq!(
            dialect_from_path("/x/responses/messages"),
            Some(Dialect::Responses)
        );
    }

    #[test]
    fn tails_point_before_usage() {
        assert_eq!(format_tail(Dialect::Chat).usage_event, "usage");
        assert_eq!(
            format_tail(Dialect::Responses).terminal,
            "response.completed"
        );
        let m = format_tail(Dialect::Messages);
        assert_eq!(m.terminal, "message_stop");
        assert_eq!(m.usage_event, "message_delta");
    }
}

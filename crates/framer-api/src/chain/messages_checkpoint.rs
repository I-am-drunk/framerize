use serde_json::Value;
pub(super) use crate::protocol::messages::push_continuation as push;

pub(super) fn available(blocks: &[Value]) -> Result<bool, String> {
    let mut content = false;
    for block in blocks {
        let nonempty = |key: &str| block[key].as_str().is_some_and(|s| !s.trim().is_empty());
        match block["type"].as_str() {
            Some("text") => content |= nonempty("text"),
            Some("thinking") if nonempty("signature") && block["thinking"].is_string() => content = true,
            Some("redacted_thinking") if nonempty("data") => content = true,
            Some("thinking" | "redacted_thinking") => return Err(
                "Claude checkpoint has no complete signed thinking block; continuation is unavailable".into()),
            _ => return Ok(false),
        }
    }
    Ok(content)
}

// History layout version 1 binds these exact prompts. New wording requires a
// new layout version while the old decoder keeps its original prompt strings.
pub(in crate::chain) const CONTINUE: &str = "Continue the response to the user's request. Return the requested result without repeating completed output.";
pub(in crate::chain) const TOOL_CONTINUE: &str = "Your previous tool call did not finish within the output limit and was not executed. Continue fulfilling the same request. Keep each tool call small enough to finish, splitting file writes and edits when needed.";

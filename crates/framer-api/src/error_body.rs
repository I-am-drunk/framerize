//! Byte-limited response excerpts without splitting a UTF-8 character.

pub(crate) fn prefix(body: &str, max_bytes: usize) -> &str {
    let mut end = body.len().min(max_bytes);
    while !body.is_char_boundary(end) { end -= 1; }
    &body[..end]
}

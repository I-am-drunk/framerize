use serde_json::Value;
use std::io::{self, Write};

/// Keep serialization fragments out of the shared append file.
pub(super) fn event(writer: &mut impl Write, value: &Value) -> io::Result<()> {
    let mut line = serde_json::to_vec(value).map_err(io::Error::other)?;
    line.push(b'\n');
    // One append attempt, including the newline. Retrying a short write could
    // put another process's record between this event's prefix and suffix.
    // A failed/short append is lost evidence; callers must not infer success.
    let written = writer.write(&line)?;
    if written != line.len() {
        return Err(io::Error::new(io::ErrorKind::WriteZero, "incomplete event append"));
    }
    writer.flush()
}

#[cfg(test)]
#[path = "observability_append_tests.rs"]
mod tests;

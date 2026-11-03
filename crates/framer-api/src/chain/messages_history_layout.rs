use super::{failure, Connection, OptionalExtension, Value, MAX_LAYOUT_BYTES, MAX_TURNS};
use serde_json::json;
use std::collections::HashSet;

pub(super) fn read(db: &Connection, key: &str) -> Result<Option<Value>, String> {
    let raw: Option<String> = db.query_row(
        "SELECT substr(layout,1,262145) FROM checkpoints WHERE fingerprint=?", [key], |r| r.get(0))
        .optional().map_err(failure)?;
    raw.map(|text| {
        if text.len() > MAX_LAYOUT_BYTES { return Err(failure("stored layout exceeds 256 KiB")); }
        serde_json::from_str(&text).map_err(failure)
    }).transpose()
}

pub(super) fn resolve(db: &Connection, mut value: Value) -> Result<Value, String> {
    let invalid = || failure("stored checkpoint layout is invalid");
    let mut suffix = Vec::new();
    let mut seen = HashSet::new();
    let mut bytes = 0;
    while value["version"] == 2 {
        let parent = value["parent"].as_str().ok_or_else(invalid)?;
        if parent.len() != 64 || !parent.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(invalid());
        }
        if !seen.insert(parent.to_owned()) { return Err(failure("checkpoint layout contains a cycle")); }
        let turn = value.get("turn").filter(|v| v.is_object()).ok_or_else(invalid)?;
        bytes += serde_json::to_vec(turn).expect("JSON value").len() + 1;
        if suffix.len() + 1 >= MAX_TURNS || bytes > MAX_LAYOUT_BYTES {
            return Err(failure("stored checkpoint layout exceeds its size limit"));
        }
        suffix.push(turn.clone());
        value = read(db, parent)?.ok_or_else(|| failure("checkpoint parent layout is missing"))?;
    }
    if value["version"] != 1 { return Err(invalid()); }
    let turns = value["turns"].as_array().ok_or_else(invalid)?;
    if turns.is_empty() || turns.len() + suffix.len() > MAX_TURNS {
        return Err(invalid());
    }
    bytes += serde_json::to_vec(&value).expect("JSON value").len();
    if bytes > MAX_LAYOUT_BYTES { return Err(failure("stored layout exceeds 256 KiB")); }
    let mut turns = turns.clone();
    turns.extend(suffix.into_iter().rev());
    Ok(json!({"version":1,"turns":turns}))
}

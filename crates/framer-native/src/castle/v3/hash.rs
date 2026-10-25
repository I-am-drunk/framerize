//! UP at SDK3337eba7 bytes 95593..95721.
use super::Value;

/// Hash the supplied value's JSON text using the SDK's UTF-16 FNV-1a variant.
/// Returns eight lowercase hex digits, or `None` when JSON conversion fails
/// or produces no string. Custom getter/toJSON evaluation remains caller-owned.
pub fn json_hash(value: &Value) -> Option<String> {
    if matches!(value, Value::Undefined) {
        return None;
    }
    let text = super::json::stringify(value)?;
    let mut hash = 2_166_136_261u32;
    for unit in text {
        hash = (hash ^ u32::from(unit)).wrapping_mul(16_777_619);
    }
    Some(format!("{hash:08x}"))
}

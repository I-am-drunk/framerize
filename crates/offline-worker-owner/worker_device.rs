use super::{Failure, Target};

/// An explicit private value. The caller owns its account and rate authority.
/// Deliberately provides neither Debug nor serialization nor an identity default.
pub struct SavedDevice(String);

impl SavedDevice {
    pub fn new(value: String) -> Result<Self, Failure> {
        if value.is_empty() || value.len() > 256 || !value.bytes().all(|b|
            b.is_ascii_alphanumeric() || b"._~-".contains(&b)) {
            return Err(Failure::Input);
        }
        Ok(Self(value))
    }
}

pub(super) fn encode(target: Target, device: Option<&SavedDevice>, budget_ms: u64)
    -> Result<Vec<u8>, Failure> {
    let mut value = serde_json::json!({ "version": if device.is_some() { 2 } else { 1 },
        "budget_ms": budget_ms, "target": { "project_id": target.project_id,
            "url": target.page_url, "path": target.path } });
    if let Some(device) = device { value["device"] = serde_json::json!({ "id": device.0 }); }
    let bytes = serde_json::to_vec(&value).map_err(|_| Failure::Input)?;
    if bytes.len() > 8192 { return Err(Failure::Input); }
    Ok(bytes)
}

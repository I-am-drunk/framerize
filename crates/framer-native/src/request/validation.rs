use super::*;
pub(super) fn reject_namespace_repair(payload: &Value) -> Result<(), DispatchError> {
    let mut declared = std::collections::HashSet::new();
    for tool in payload["tools"].as_array().into_iter().flatten() {
        if tool["type"].as_str() == Some("namespace") {
            if tool["name"].is_string() {
                for inner in tool["tools"].as_array().into_iter().flatten() {
                    if let Some(name) = inner["name"].as_str() { declared.insert(name); }
                }
            }
        } else if let Some(name) = tool["name"].as_str() {
            if tool["namespace"].is_string() { declared.insert(name); }
            else if name.contains('.') { declared.insert(name.rsplit('.').next().unwrap_or(name)); }
        }
    }
    for item in payload["input"].as_array().into_iter().flatten() {
        if matches!(item["type"].as_str(), Some("function_call") | Some("custom_tool_call"))
            && item.get("namespace").is_none()
            && item["name"].as_str().is_some_and(|name| declared.contains(name)) {
            return Err(DispatchError::new(FailureKind::InvalidRequest));
        }
    }
    Ok(())
}

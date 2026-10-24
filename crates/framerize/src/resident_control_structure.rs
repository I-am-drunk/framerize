//! Fixed structural facts from the token already owned by the resident control.
#[path = "resident_token_structure.rs"]
mod decoder;

pub(super) fn record(token: &str, lease: uuid::Uuid) {
    let Some(fields) = facts(token, lease) else { return; };
    fields.into_iter().fold(
        framer_core::observability::Event::new("resident.mint.structure"),
        |event, (key, value)| event.field(&key, value),
    ).emit();
}

fn facts(token: &str, lease: uuid::Uuid) -> Option<serde_json::Map<String, serde_json::Value>> {
    if lease.is_nil() { return None; }
    let shape = decoder::describe(token).map(|shape| {
        let entries: Vec<_> = shape.entries.into_iter().map(|entry| {
            serde_json::json!({
                "group_id": entry.group_id,
                "field_id": entry.field_id,
                "type": entry.kind,
                "byte_length": entry.byte_length,
            })
        }).collect();
        serde_json::json!({"entries": entries, "consumed": shape.consumed})
    });
    Some(serde_json::Map::from_iter([
        ("lease".into(), serde_json::json!(lease.to_string())),
        ("phase".into(), serde_json::json!("native")),
        ("available".into(), serde_json::json!(shape.is_some())),
        ("structure".into(), shape.unwrap_or(serde_json::Value::Null)),
    ]))
}

#[cfg(test)]
#[path = "resident_control_structure_tests.rs"]
mod tests;

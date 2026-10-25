//! JSON.stringify semantics for native data, without executing JavaScript.
use super::Value;
use std::collections::HashMap;

enum Pending<'a> {
    Value(&'a Value),
    Key(&'a [u16]),
    Mark(u16),
}

pub(super) fn stringify(value: &Value) -> Option<Vec<u16>> {
    let mut output = Vec::new();
    let mut pending = vec![Pending::Value(value)];
    while let Some(item) = pending.pop() {
        let value = match item {
            Pending::Value(value) => value,
            Pending::Key(key) => {
                quote(key, &mut output);
                continue;
            }
            Pending::Mark(mark) => {
                output.push(mark);
                continue;
            }
        };
        match value {
            Value::Null | Value::Undefined => text("null", &mut output),
            Value::Bool(value) => text(if *value { "true" } else { "false" }, &mut output),
            Value::Number(value) => {
                if value.is_finite() {
                    text(ryu_js::Buffer::new().format(*value), &mut output);
                } else {
                    text("null", &mut output);
                }
            }
            Value::String(value) => quote(value, &mut output),
            Value::JsonFailure => return None,
            Value::JsonFallback(value) => output.extend_from_slice(value),
            Value::Array(values) => {
                output.push(b'[' as u16);
                pending.push(Pending::Mark(b']' as u16));
                for (index, value) in values.iter().enumerate().rev() {
                    pending.push(Pending::Value(value));
                    if index != 0 {
                        pending.push(Pending::Mark(b',' as u16));
                    }
                }
            }
            Value::Object(properties) => {
                output.push(b'{' as u16);
                pending.push(Pending::Mark(b'}' as u16));
                for (index, (key, value)) in ordered(properties).into_iter().enumerate().rev() {
                    pending.push(Pending::Value(value));
                    pending.push(Pending::Mark(b':' as u16));
                    pending.push(Pending::Key(key));
                    if index != 0 {
                        pending.push(Pending::Mark(b',' as u16));
                    }
                }
            }
        }
    }
    Some(output)
}

fn text(value: &str, output: &mut Vec<u16>) {
    output.extend(value.encode_utf16());
}

fn ordered(properties: &[(Vec<u16>, Value)]) -> Vec<(&[u16], &Value)> {
    let mut positions: HashMap<&[u16], usize> = HashMap::new();
    let mut entries: Vec<(&[u16], &Value)> = Vec::new();
    for (key, value) in properties {
        if let Some(&index) = positions.get(key.as_slice()) {
            entries[index] = (key.as_slice(), value);
        } else {
            positions.insert(key.as_slice(), entries.len());
            entries.push((key, value));
        }
    }
    entries.retain(|(_, value)| !matches!(value, Value::Undefined));
    entries.sort_by_key(|(key, _)| match array_index(key) {
        Some(index) => (0, index),
        None => (1, 0),
    });
    entries
}

fn array_index(key: &[u16]) -> Option<u32> {
    if key.is_empty() || (key.len() > 1 && key[0] == b'0' as u16) {
        return None;
    }
    let mut value = 0u32;
    for &unit in key {
        if !(48..=57).contains(&unit) {
            return None;
        }
        value = value.checked_mul(10)?.checked_add(u32::from(unit - 48))?;
    }
    (value != u32::MAX).then_some(value)
}

fn quote(value: &[u16], output: &mut Vec<u16>) {
    output.push(b'"' as u16);
    let mut index = 0;
    while let Some(&unit) = value.get(index) {
        match unit {
            8 => text("\\b", output),
            9 => text("\\t", output),
            10 => text("\\n", output),
            12 => text("\\f", output),
            13 => text("\\r", output),
            34 => text("\\\"", output),
            92 => text("\\\\", output),
            0xD800..=0xDBFF
                if value
                    .get(index + 1)
                    .is_some_and(|next| (0xDC00..=0xDFFF).contains(next)) =>
            {
                output.extend_from_slice(&value[index..index + 2]);
                index += 1;
            }
            0..=31 | 0xD800..=0xDFFF => escape(unit, output),
            _ => output.push(unit),
        }
        index += 1;
    }
    output.push(b'"' as u16);
}

fn escape(unit: u16, output: &mut Vec<u16>) {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    text("\\u", output);
    for shift in [12, 8, 4, 0] {
        output.push(HEX[((unit >> shift) & 15) as usize] as u16);
    }
}

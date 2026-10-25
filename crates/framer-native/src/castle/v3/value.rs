use super::{write_fixed_unsigned, write_signed, write_unsigned, write_utf16};
use std::num::TryFromIntError;

/// Typed values at the pinned SDK's serialization boundary.
#[derive(Debug)]
pub enum Value {
    /// Also represents undefined, or a failed/undefined JSON fallback.
    Null,
    Bool(bool),
    Number(f64),
    /// JavaScript UTF-16 code units, including unpaired surrogates.
    String(Vec<u16>),
    Array(Vec<Value>),
    /// The UTF-16 result of a successful JSON.stringify fallback.
    /// Object normalization and custom toJSON behavior are caller-owned.
    JsonFallback(Vec<u16>),
}

fn length(value: usize, output: &mut Vec<u8>) -> Result<(), TryFromIntError> {
    write_unsigned(u32::try_from(value)?, output);
    Ok(())
}

/// Serialize a collected value array without browser or JavaScript execution.
/// Type tags and dispatch follow SDK bytes 274985..276393, SHA256 3337eba7.
/// JSON fallback results are explicit inputs; this does not stringify objects.
pub fn serialize(values: &[Value]) -> Result<Vec<u8>, TryFromIntError> {
    let mut output = Vec::new();
    length(values.len(), &mut output)?;
    let mut pending: Vec<_> = values.iter().rev().collect();
    while let Some(value) = pending.pop() {
        write_value(value, &mut output, &mut pending)?;
    }
    Ok(output)
}

fn write_value<'a>(
    value: &'a Value,
    output: &mut Vec<u8>,
    pending: &mut Vec<&'a Value>,
) -> Result<(), TryFromIntError> {
    match value {
        Value::Null => output.push(0),
        Value::Bool(value) => output.push(if *value { 6 } else { 5 }),
        Value::Number(value) => number(*value, output),
        Value::String(value) => string(value, output)?,
        Value::Array(values) => {
            output.push(7);
            length(values.len(), output)?;
            pending.extend(values.iter().rev());
        }
        Value::JsonFallback(value) => {
            output.push(8);
            write_utf16(value, output)?;
        }
    }
    Ok(())
}

fn number(value: f64, output: &mut Vec<u8>) {
    if !value.is_finite() {
        output.push(0);
    } else if value.floor() == value && value >= i32::MIN as f64 && value <= i32::MAX as f64 {
        output.push(10);
        write_signed(value as i32, output);
    } else if value.floor() == value && value >= 0.0 && value <= u32::MAX as f64 {
        output.push(3);
        write_fixed_unsigned(value as u32, output);
    } else {
        output.push(4);
        output.extend_from_slice(&value.to_be_bytes());
    }
}

fn string(value: &[u16], output: &mut Vec<u8>) -> Result<(), TryFromIntError> {
    if let Some(bytes) = canonical_base64(value) {
        output.push(2);
        length(bytes.len(), output)?;
        output.extend(bytes);
    } else if value.iter().all(|unit| *unit <= 255) {
        output.push(9);
        length(value.len(), output)?;
        output.extend(value.iter().map(|unit| *unit as u8));
    } else {
        output.push(1);
        write_utf16(value, output)?;
    }
    Ok(())
}

fn sextet(unit: u16) -> Option<u8> {
    match unit {
        65..=90 => Some((unit - 65) as u8),
        97..=122 => Some((unit - 97 + 26) as u8),
        48..=57 => Some((unit - 48 + 52) as u8),
        43 => Some(62),
        47 => Some(63),
        _ => None,
    }
}

// Match the SDK's strict alphabet/padding check and atob/btoa round trip.
fn canonical_base64(units: &[u16]) -> Option<Vec<u8>> {
    if units.len() % 4 != 0 {
        return None;
    }
    let mut bytes = Vec::new();
    for (index, chunk) in units.chunks_exact(4).enumerate() {
        let a = sextet(chunk[0])?;
        let b = sextet(chunk[1])?;
        let last = (index + 1) * 4 == units.len();
        bytes.push(a << 2 | b >> 4);
        if chunk[2] == 61 {
            if !last || chunk[3] != 61 || b & 15 != 0 {
                return None;
            }
        } else {
            let c = sextet(chunk[2])?;
            bytes.push((b & 15) << 4 | c >> 2);
            if chunk[3] == 61 {
                if !last || c & 3 != 0 {
                    return None;
                }
            } else {
                bytes.push((c & 3) << 6 | sextet(chunk[3])?);
            }
        }
    }
    Some(bytes)
}

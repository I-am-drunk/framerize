//! Private per-operation data. Never serialized into diagnostics.
use crate::castle::v3::{self, Value};
use serde::Deserialize;

pub(super) const LIMIT: usize = 1024 * 1024;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Collection { version: u8, values: Captured, shared_worker_reply: Captured }

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase", deny_unknown_fields)]
enum Captured {
    Undefined {}, Null {}, Boolean { value: bool }, Number { value: String },
    String { units: Vec<u16> }, Array { values: Vec<Captured> },
}

pub(super) fn decode(bytes: &[u8]) -> Option<Vec<Value>> {
    if bytes.len() < 12 || bytes.len() > LIMIT + 12
        || &bytes[..8] != b"FCLT\x01\0\0\0" { return None; }
    let size = u32::from_be_bytes(bytes[8..12].try_into().ok()?) as usize;
    if size == 0 || size + 12 != bytes.len() { return None; }
    let input: Collection = serde_json::from_slice(&bytes[12..]).ok()?;
    if input.version != 2 { return None; }
    let Captured::Array { values } = input.values else { return None; };
    if values.len() != 765 { return None; }
    let (mut count, mut units) = (1usize, 0usize);
    let mut values: Vec<_> = values.into_iter()
        .map(|v| convert(v, 1, &mut count, &mut units)).collect::<Option<_>>()?;
    let reply = convert(input.shared_worker_reply, 0, &mut count, &mut units)?;
    v3::apply_collector_result(&mut values, v3::CollectorResult::SharedWorker(reply)).ok()?;
    Some(values)
}

fn convert(value: Captured, depth: usize, count: &mut usize, units: &mut usize)
    -> Option<Value> {
    *count += 1;
    if *count > 4096 || depth > 64 { return None; }
    Some(match value {
        Captured::Undefined {} => Value::Undefined,
        Captured::Null {} => Value::Null,
        Captured::Boolean { value } => Value::Bool(value),
        Captured::Number { value } => Value::Number(match value.as_str() {
            "NaN" => f64::NAN, "Infinity" => f64::INFINITY,
            "-Infinity" => f64::NEG_INFINITY, "-0" => -0.0,
            text => {
                let number: f64 = text.parse().ok()?;
                if !number.is_finite() || ryu_js::Buffer::new().format(number) != text {
                    return None;
                }
                number
            }
        }),
        Captured::String { units: value } => {
            *units += value.len();
            if *units > 65536 { return None; }
            Value::String(value)
        }
        Captured::Array { values } => {
            if values.len() > 2048 { return None; }
            Value::Array(values.into_iter().map(|v| convert(v, depth + 1, count, units))
                .collect::<Option<_>>()?)
        }
    })
}

pub(super) fn encode(mut bytes: Vec<u8>) -> Option<String> {
    let values = decode(&bytes);
    bytes.fill(0);
    let token = v3::encode_current(&values?).ok()?;
    (!token.is_empty() && token.len() <= 16384).then_some(token)
}

#[cfg(test)]
#[path = "worker_collection_tests.rs"]
mod tests;

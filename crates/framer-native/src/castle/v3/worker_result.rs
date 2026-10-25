//! Qon at SDK3337eba7 bytes 506333..506739, before serialization.
use super::Value;

/// Already-normalized JSON does not retain its original JavaScript truthiness.
#[derive(Debug, PartialEq, Eq)]
pub struct NormalizedWorkerValue;

/// Reduce a structured-clone worker result to its version field or marker.
/// `Undefined` means an actual undefined value here. Uncloneable functions and
/// symbols, and precomputed JSON conversions, are not worker-message inputs.
pub fn shared_worker_version(input: Value) -> Result<Value, NormalizedWorkerValue> {
    match input {
        Value::String(value) => {
            const MARKERS: [(&str, &str); 8] = [
                ("83au4jz", "7e93ex445j"), ("3mxkmwkle0", "zye88asu"),
                ("9q5p9p15pn", "w9qwrnt533"), ("99j32wg9x", "dixwrk"),
                ("gp9vlcc", "92ss6wlksf"), ("2v315vt89", "6c3rdk8n3"),
                ("vrbhklehy", "3jpaly3xq3"), ("ydlkmari0n", "phfj4lf8"),
            ];
            for (source, result) in MARKERS {
                if value.iter().copied().eq(source.encode_utf16()) {
                    return Ok(marker(result));
                }
            }
            Ok(Value::String(value))
        }
        Value::Array(values) => {
            if !matches!(values.first(), Some(Value::Number(tag)) if *tag == 2.0) {
                return Ok(marker("2nrh3qm"));
            }
            let value = values.into_iter().nth(10).unwrap_or(Value::Undefined);
            match value {
                Value::JsonFallback(_) | Value::JsonFailure => Err(NormalizedWorkerValue),
                Value::Null | Value::Undefined | Value::Bool(false) => {
                    Ok(marker("r0yv7z6cx"))
                }
                Value::Number(n) if n == 0.0 || n.is_nan() => Ok(marker("r0yv7z6cx")),
                Value::String(ref text) if text.is_empty() => Ok(marker("r0yv7z6cx")),
                value => Ok(value),
            }
        }
        _ => Ok(marker("a9opjk")),
    }
}

fn marker(value: &str) -> Value {
    Value::String(value.encode_utf16().collect())
}

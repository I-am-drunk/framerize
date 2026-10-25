//! The pinned SDK's krn/ud reduction of a supplied network observation.
use super::Value;

#[derive(Debug, PartialEq, Eq)]
pub enum NetworkValueError {
    /// The original getter throws when record zero is null or missing.
    MissingRecord,
    /// JSON normalization erased the type needed for property access/truthiness.
    NormalizedValue,
    /// Generic JS length/charCodeAt coercion is outside the string-hash boundary.
    NonStringHash,
}

/// Reduce an observation with the SDK's built-in `ud` getter present.
/// Matches SDK3337eba7 bytes 428835..429015 and 95795..95918 for plain data.
/// This does not gather candidates, validate addresses or alter supplied values.
/// Prototypes, accessors and a replaced JavaScript getter are outside this API.
pub fn network_address(input: Value) -> Result<Value, NetworkValueError> {
    network_field(input, 38)
}

/// Transform the completed N_ hash or its missing-field marker for Q[469].
/// SDK3337eba7 Brn (429659..429849) and Cr (207029..207266).
/// The stock producer supplies a string. Accessors, replaced helpers and
/// truthy non-string fields require JS behavior not represented by this API.
pub fn network_hash(input: Value) -> Result<Value, NetworkValueError> {
    match network_field(input, 47)? {
        Value::String(units) => Ok(marker(&super::envelope::encode_text(units))),
        _ => Err(NetworkValueError::NonStringHash),
    }
}

fn network_field(input: Value, index: usize) -> Result<Value, NetworkValueError> {
    match input {
        Value::Array(_) | Value::Object(_) => {}
        Value::JsonFallback(_) | Value::JsonFailure => {
            return Err(NetworkValueError::NormalizedValue);
        }
        _ => return Ok(marker("a9opjk")),
    }
    let record = property(input, 0)?;
    let value = property(record, index)?;
    match value {
        Value::JsonFallback(_) | Value::JsonFailure => Err(NetworkValueError::NormalizedValue),
        Value::Null | Value::Undefined | Value::Bool(false) => Ok(marker("2nrh3qm")),
        Value::Number(n) if n == 0.0 || n.is_nan() => Ok(marker("2nrh3qm")),
        Value::String(ref text) if text.is_empty() => Ok(marker("2nrh3qm")),
        value => Ok(value),
    }
}

fn property(value: Value, index: usize) -> Result<Value, NetworkValueError> {
    match value {
        Value::Null | Value::Undefined => Err(NetworkValueError::MissingRecord),
        Value::JsonFallback(_) | Value::JsonFailure => Err(NetworkValueError::NormalizedValue),
        Value::Array(values) => Ok(values.into_iter().nth(index).unwrap_or(Value::Undefined)),
        Value::Object(entries) => {
            let key: Vec<_> = index.to_string().encode_utf16().collect();
            Ok(entries.into_iter().rev().find(|(name, _)| name == &key)
                .map(|(_, value)| value).unwrap_or(Value::Undefined))
        }
        Value::String(units) => Ok(units.get(index)
            .map(|unit| Value::String(vec![*unit])).unwrap_or(Value::Undefined)),
        _ => Ok(Value::Undefined),
    }
}

fn marker(value: &str) -> Value {
    Value::String(value.encode_utf16().collect())
}

//! The pinned SDK's krn/ud reduction of a supplied network observation.
use super::Value;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NetworkValueError {
    /// The original getter throws when record zero is null or missing.
    MissingRecord,
    /// JSON normalization erased the type needed for property access/truthiness.
    NormalizedValue,
    /// Generic JS length/charCodeAt coercion is outside the string-hash boundary.
    NonStringHash,
    /// A transformed field needs JS coercion outside the plain-string boundary.
    NonStringField(usize),
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
    encoded_field(input, 47, NetworkValueError::NonStringHash)
}

/// Apply T0n/id and Cr to the supplied related address for Q[80].
/// SDK3337eba7: 554645..554825, 95918..95986 and 207029..207266.
pub fn network_related_address(input: Value) -> Result<Value, NetworkValueError> {
    encoded_field(input, 40, NetworkValueError::NonStringField(40))
}

/// Apply a$/i1 and Cr to the supplied protocol summary for Q[659].
/// SDK3337eba7: 367910..368101, 1267..1304 and 207029..207266.
/// The input is plain data; the original's repeated getter reads are not modeled.
pub fn network_protocols(input: Value) -> Result<Value, NetworkValueError> {
    encoded_field(input, 18, NetworkValueError::NonStringField(18))
}

fn encoded_field(input: Value, index: usize, error: NetworkValueError)
    -> Result<Value, NetworkValueError> {
    transform(network_field(input, index)?, error)
}

fn transform(value: Value, error: NetworkValueError) -> Result<Value, NetworkValueError> {
    match value {
        Value::String(units) => Ok(marker(&super::envelope::encode_text(units))),
        _ => Err(error),
    }
}

fn first_record(mut input: Value) -> Result<Option<Value>, NetworkValueError> {
    match &input {
        Value::Array(_) | Value::Object(_) => property(&mut input, 0).map(Some),
        Value::JsonFallback(_) | Value::JsonFailure => {
            Err(NetworkValueError::NormalizedValue)
        }
        _ => Ok(None),
    }
}

fn network_field(input: Value, index: usize) -> Result<Value, NetworkValueError> {
    match first_record(input)? {
        Some(mut record) => normalized(property(&mut record, index)?),
        None => Ok(marker("a9opjk")),
    }
}

pub(super) fn network_assignments(input: Value)
    -> impl Iterator<Item = Result<Value, NetworkValueError>> {
    let mut record = first_record(input);
    [40, 47, 38, 18].into_iter().map(move |index| {
        let value = match &mut record {
            Err(error) => return Err(*error),
            Ok(None) => marker("a9opjk"),
            Ok(Some(record)) => normalized(property(record, index)?)?,
        };
        match index {
            38 => Ok(value),
            47 => transform(value, NetworkValueError::NonStringHash),
            _ => transform(value, NetworkValueError::NonStringField(index)),
        }
    })
}

fn normalized(value: Value) -> Result<Value, NetworkValueError> {
    match value {
        Value::JsonFallback(_) | Value::JsonFailure => Err(NetworkValueError::NormalizedValue),
        Value::Null | Value::Undefined | Value::Bool(false) => Ok(marker("2nrh3qm")),
        Value::Number(n) if n == 0.0 || n.is_nan() => Ok(marker("2nrh3qm")),
        Value::String(ref text) if text.is_empty() => Ok(marker("2nrh3qm")),
        value => Ok(value),
    }
}

fn property(value: &mut Value, index: usize) -> Result<Value, NetworkValueError> {
    match value {
        Value::Null | Value::Undefined => Err(NetworkValueError::MissingRecord),
        Value::JsonFallback(_) | Value::JsonFailure => Err(NetworkValueError::NormalizedValue),
        Value::Array(values) => Ok(values.get_mut(index)
            .map(|value| std::mem::replace(value, Value::Undefined))
            .unwrap_or(Value::Undefined)),
        Value::Object(entries) => {
            let key: Vec<_> = index.to_string().encode_utf16().collect();
            Ok(entries.iter_mut().rev().find(|(name, _)| name.as_slice() == key.as_slice())
                .map(|(_, value)| std::mem::replace(value, Value::Undefined))
                .unwrap_or(Value::Undefined))
        }
        Value::String(units) => Ok(units.get(index)
            .map(|unit| Value::String(vec![*unit])).unwrap_or(Value::Undefined)),
        _ => Ok(Value::Undefined),
    }
}

fn marker(value: &str) -> Value {
    Value::String(value.encode_utf16().collect())
}

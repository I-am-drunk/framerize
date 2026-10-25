//! Pure record assembly from the pinned SDK's N_ finalizer.
use super::{json_hash, Value};
use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};

/// The seven scalar fields used by the SDK's candidate signature.
pub struct NetworkCandidate {
    pub kind: Value,
    pub protocol: Value,
    pub address_class: Value,
    pub related_address_class: Value,
    pub port_class: Value,
    pub tcp_type: Value,
    pub component: Value,
}

/// Collected inputs at finalization, after candidate/state event reduction.
pub struct NetworkObservation {
    pub address: Vec<u16>,
    pub related_address: Vec<u16>,
    pub candidates: Vec<NetworkCandidate>,
    pub connection_states: Vec<Vec<u16>>,
    pub gathering_states: Vec<Vec<u16>>,
    pub signaling_states: Vec<Vec<u16>>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct NonScalarCandidate;

struct Prepared {
    signature: Vec<u16>,
    kind: Vec<u16>,
    protocol: Vec<u16>,
    raw: Value,
}

/// Assemble the SDK record from already collected candidate/state histories.
/// `compare_signatures` supplies the observation runtime's locale collation,
/// matching the original `localeCompare`. Candidate selection precedes sorting.
pub fn assemble_network_record<F>(
    observation: NetworkObservation,
    mut compare_signatures: F,
) -> Result<Value, NonScalarCandidate>
where F: FnMut(&[u16], &[u16]) -> Ordering {
    let mut candidates: Vec<_> = observation.candidates.into_iter().take(16)
        .map(prepare).collect::<Result<_, _>>()?;
    candidates.sort_by(|a, b| compare_signatures(&a.signature, &b.signature));
    let mut protocols = BTreeSet::new();
    let mut groups: BTreeMap<Vec<u16>, usize> = BTreeMap::new();
    for candidate in &candidates {
        if !candidate.protocol.is_empty() && candidate.protocol != utf16("__proto__") {
            protocols.insert(candidate.protocol.clone());
        }
        let mut key = fallback(&candidate.kind);
        key.push(b'/' as u16);
        key.extend(fallback(&candidate.protocol));
        *groups.entry(key).or_default() += 1;
    }
    let protocol_summary = join(&protocols.into_iter().collect::<Vec<_>>(), b'+');
    let group_summary = join(&groups.into_iter().map(|(mut key, count)| {
        key.push(b':' as u16);
        key.extend(count.to_string().encode_utf16());
        key
    }).collect::<Vec<_>>(), b'|');
    let connection: Vec<_> = observation.connection_states.into_iter().take(8).collect();
    let gathering: Vec<_> = observation.gathering_states.into_iter().take(8).collect();
    let signaling: Vec<_> = observation.signaling_states.into_iter().take(8).collect();
    let signatures: Vec<_> = candidates.iter().map(|c| c.signature.clone()).collect();
    let metrics = Value::Array(vec![
        Value::String(protocol_summary.clone()), strings(&signatures),
        Value::String(group_summary.clone()), strings(&connection),
        strings(&gathering), strings(&signaling),
    ]);
    let hash = json_hash(&metrics).expect("string-only metrics have JSON");
    let mut record: Vec<_> = (0..56).map(|_| Value::Undefined).collect();
    record[38] = Value::String(observation.address);
    record[40] = Value::String(observation.related_address);
    record[18] = Value::String(protocol_summary);
    record[15] = Value::Array(candidates.into_iter().map(|c| c.raw).collect());
    record[24] = Value::String(group_summary);
    record[54] = strings(&connection);
    record[30] = strings(&gathering);
    record[55] = strings(&signaling);
    record[47] = Value::String(utf16(&hash));
    Ok(Value::Array(vec![Value::Array(record)]))
}

fn strings(values: &[Vec<u16>]) -> Value {
    Value::Array(values.iter().cloned().map(Value::String).collect())
}

fn utf16(value: &str) -> Vec<u16> {
    value.encode_utf16().collect()
}

fn prepare(candidate: NetworkCandidate) -> Result<Prepared, NonScalarCandidate> {
    let fields = [candidate.kind, candidate.protocol, candidate.address_class,
        candidate.related_address_class, candidate.port_class,
        candidate.tcp_type, candidate.component];
    let scalars: Vec<_> = fields.iter().map(scalar).collect::<Result<_, _>>()?;
    let signature = join(&scalars.iter().map(|(text, _)| text.clone()).collect::<Vec<_>>(), b':');
    let kind = if scalars[0].1 { scalars[0].0.clone() } else { Vec::new() };
    let protocol = if scalars[1].1 { scalars[1].0.clone() } else { Vec::new() };
    const PATHS: [&[usize]; 7] = [
        &[0, 5, 6], &[1, 7], &[2, 5, 3, 3], &[1, 4, 2, 4],
        &[2, 2, 1, 6], &[0, 10], &[0, 8, 8, 5],
    ];
    let mut raw = Value::Array(Vec::new());
    for (path, value) in PATHS.into_iter().zip(fields) {
        put(&mut raw, path, value);
    }
    Ok(Prepared { signature, kind, protocol, raw })
}

fn join(values: &[Vec<u16>], separator: u8) -> Vec<u16> {
    let mut output = Vec::new();
    for (index, value) in values.iter().enumerate() {
        if index > 0 { output.push(u16::from(separator)); }
        output.extend_from_slice(value);
    }
    output
}

fn fallback(value: &[u16]) -> Vec<u16> {
    if value.is_empty() { utf16("unknown") } else { value.to_vec() }
}

fn put(node: &mut Value, path: &[usize], value: Value) {
    let Value::Array(values) = node else { unreachable!() };
    let index = path[0];
    if values.len() <= index {
        values.resize_with(index + 1, || Value::Undefined);
    }
    if path.len() == 1 {
        values[index] = value;
    } else {
        if matches!(values[index], Value::Undefined) {
            values[index] = Value::Array(Vec::new());
        }
        put(&mut values[index], &path[1..], value);
    }
}

fn scalar(value: &Value) -> Result<(Vec<u16>, bool), NonScalarCandidate> {
    Ok(match value {
        Value::Null | Value::Undefined => (Vec::new(), false),
        Value::String(text) => (text.clone(), !text.is_empty()),
        Value::Bool(value) => (utf16(if *value { "true" } else { "false" }), *value),
        Value::Number(value) => (number(*value), *value != 0.0 && !value.is_nan()),
        _ => return Err(NonScalarCandidate),
    })
}

fn number(value: f64) -> Vec<u16> {
    if value.is_nan() {
        utf16("NaN")
    } else if value.is_infinite() {
        utf16(if value > 0.0 { "Infinity" } else { "-Infinity" })
    } else if value == 0.0 {
        utf16("0")
    } else {
        utf16(ryu_js::Buffer::new().format(value))
    }
}

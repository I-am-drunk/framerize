//! Normalization of supplied, typed RTC candidate observations.
use super::network_record::NetworkCandidate;
use super::Value;

/// Ordinary RTC property values. Missing and null properties share `None`.
/// Read getters before this boundary; no JavaScript coercion is performed here.
#[derive(Default)]
pub struct CandidateObservation {
    pub kind: Option<Vec<u16>>,
    pub protocol: Option<Vec<u16>>,
    pub address: Option<Vec<u16>>,
    pub related_address: Option<Vec<u16>>,
    pub port: Option<f64>,
    pub tcp_type: Option<Vec<u16>>,
    pub component: Option<Vec<u16>>,
    pub candidate: Option<Vec<u16>>,
}

fn text(value: &str) -> Vec<u16> {
    value.encode_utf16().collect()
}

/// SDK classification, deliberately without IP-address validation.
pub fn network_address_class(value: &[u16]) -> &'static str {
    if value.is_empty() {
        return "";
    }
    let parts: Vec<_> = value.split(|&unit| unit == b'.' as u16).collect();
    if parts.len() == 4 && parts.iter().all(|p| !p.is_empty() && digits(p)) {
        return "v4";
    }
    if value.contains(&(b':' as u16)) {
        return "v6";
    }
    if value
        .iter()
        .all(|&c| ascii_word(c) && c != 95 || c == 45 || c == 46)
    {
        "host"
    } else {
        "other"
    }
}

/// Apply IP after the caller's numeric RTC port conversion.
pub fn network_port_class(value: f64) -> &'static str {
    if !value.is_finite() || value <= 0.0 {
        ""
    } else if value < 1024.0 {
        "system"
    } else if value < 49152.0 {
        "registered"
    } else {
        "ephemeral"
    }
}

pub(super) fn whitespace(c: u16) -> bool {
    matches!(c, 9..=13 | 32 | 160 | 0x1680 | 0x2000..=0x200a |
        0x2028 | 0x2029 | 0x202f | 0x205f | 0x3000 | 0xfeff)
}
fn line_end(c: u16) -> bool {
    matches!(c, 10 | 13 | 0x2028 | 0x2029)
}
fn digits(value: &[u16]) -> bool {
    value.iter().all(|&c| matches!(c, 48..=57))
}
pub(super) fn ascii_word(c: u16) -> bool {
    matches!(c, 48..=57 | 65..=90 | 97..=122 | 95)
}
pub(super) fn ascii_equal(value: &[u16], expected: &[u8]) -> bool {
    value.len() == expected.len()
        && value
            .iter()
            .zip(expected)
            .all(|(&a, &b)| a <= 127 && (a as u8).eq_ignore_ascii_case(&b))
}

pub(super) fn lowercase(value: &[u16]) -> Vec<u16> {
    let mut result = Vec::new();
    let mut run = String::new();
    for item in char::decode_utf16(value.iter().copied()) {
        match item {
            Ok(c) => run.push(c),
            Err(error) => {
                result.extend(run.to_lowercase().encode_utf16());
                run.clear();
                result.push(error.unpaired_surrogate());
            }
        }
    }
    result.extend(run.to_lowercase().encode_utf16());
    result
}

fn present(value: &Option<Vec<u16>>) -> bool {
    value.as_ref().is_some_and(|value| !value.is_empty())
}
fn string_field(value: &Option<Vec<u16>>) -> &[u16] {
    value.as_deref().unwrap_or_default()
}
pub(super) fn candidate(fields: [Vec<u16>; 7]) -> NetworkCandidate {
    let [kind, protocol, address_class, related_address_class, port_class, tcp_type, component] =
        fields.map(Value::String);
    NetworkCandidate {
        kind,
        protocol,
        address_class,
        related_address_class,
        port_class,
        tcp_type,
        component,
    }
}

/// Normalize stable RTC properties, or parse the supplied candidate string.
pub fn normalize_network_candidate(value: &CandidateObservation) -> NetworkCandidate {
    candidate(normalize(value))
}

pub(super) fn normalize(value: &CandidateObservation) -> [Vec<u16>; 7] {
    if present(&value.kind) || present(&value.protocol) || present(&value.address) {
        let kind = if present(&value.kind) {
            lowercase(string_field(&value.kind))
        } else {
            text("unknown")
        };
        return [
            kind,
            lowercase(string_field(&value.protocol)),
            text(network_address_class(string_field(&value.address))),
            text(network_address_class(string_field(&value.related_address))),
            text(network_port_class(value.port.unwrap_or(f64::NAN))),
            lowercase(string_field(&value.tcp_type)),
            string_field(&value.component).to_vec(),
        ];
    }
    parse_candidate(string_field(&value.candidate)).unwrap_or_else(|| {
        [
            text("unknown"),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
        ]
    })
}

fn token<'a>(value: &'a [u16], at: &mut usize, separator: bool) -> Option<&'a [u16]> {
    let start = *at;
    if separator {
        while value.get(*at).copied().is_some_and(whitespace) {
            *at += 1;
        }
        if *at == start {
            return None;
        }
    }
    let start = *at;
    while value.get(*at).is_some_and(|&c| !whitespace(c)) {
        *at += 1;
    }
    (*at > start).then_some(&value[start..*at])
}

fn optional_attribute<'a>(
    value: &'a [u16],
    start: usize,
    key: &[u8],
) -> Option<(&'a [u16], usize)> {
    let stop = value[start..]
        .iter()
        .position(|&c| line_end(c))
        .map_or(value.len(), |index| start + index + 1);
    let mut found = None;
    for index in start..stop {
        let end = index + 1 + key.len();
        if whitespace(value[index])
            && end <= value.len()
            && ascii_equal(&value[index + 1..end], key)
        {
            let mut at = end;
            if let Some(field) = token(value, &mut at, true) {
                found = Some((field, at));
            }
        }
    }
    found
}

fn parse_candidate(value: &[u16]) -> Option<[Vec<u16>; 7]> {
    let mut at = 0;
    let foundation = token(value, &mut at, false)?;
    if foundation.len() <= 10 || !ascii_equal(&foundation[..10], b"candidate:") {
        return None;
    }
    let component = token(value, &mut at, true)?;
    if !digits(component) {
        return None;
    }
    let protocol = token(value, &mut at, true)?;
    token(value, &mut at, true)?;
    let address = token(value, &mut at, true)?;
    let port = token(value, &mut at, true)?;
    if !digits(port) || !ascii_equal(token(value, &mut at, true)?, b"typ") {
        return None;
    }
    let kind = token(value, &mut at, true)?;
    let related = optional_attribute(value, at, b"raddr");
    let tcp = optional_attribute(value, related.map_or(at, |(_, end)| end), b"tcptype");
    let port = String::from_utf16(port).ok()?.parse::<f64>().ok()?;
    Some([
        lowercase(kind),
        lowercase(protocol),
        text(network_address_class(address)),
        text(network_address_class(related.map_or(&[], |(v, _)| v))),
        text(network_port_class(port)),
        lowercase(tcp.map_or(&[], |(v, _)| v)),
        component.to_vec(),
    ])
}

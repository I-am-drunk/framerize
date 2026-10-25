use crate::castle::v3::{NetworkCandidate, NetworkObservation, Value};
use crate::collection::{CollationError, NetworkCollator};
use std::cmp::Ordering;

fn text(value: &str) -> Value {
    Value::String(value.encode_utf16().collect())
}

fn candidate(component: &str) -> NetworkCandidate {
    NetworkCandidate {
        kind: text("host"),
        protocol: text("udp"),
        address_class: text("v4"),
        related_address_class: text(""),
        port_class: text("ephemeral"),
        tcp_type: text(""),
        component: text(component),
    }
}

fn observation(candidates: Vec<NetworkCandidate>) -> NetworkObservation {
    NetworkObservation {
        address: Vec::new(),
        related_address: Vec::new(),
        candidates,
        connection_states: Vec::new(),
        gathering_states: Vec::new(),
        signaling_states: Vec::new(),
    }
}

#[test]
fn requires_an_explicit_supported_locale() {
    for locale in ["", "zz-ZZ"] {
        assert!(
            matches!(NetworkCollator::new(locale), Err(CollationError::UnsupportedLocale)),
            "locale {locale:?} must not silently select a default",
        );
    }
    assert!(NetworkCollator::new("en-US").is_ok());
}

#[test]
fn compare_rejects_lone_surrogates_on_either_side() {
    let collator = NetworkCollator::new("en-US").unwrap();
    let pair = [0xd83d, 0xde00];
    assert_eq!(collator.compare(&pair, &pair), Ok(Ordering::Equal));
    for malformed in [[0xd800], [0xdc00]] {
        assert_eq!(
            collator.compare(&malformed, &pair),
            Err(CollationError::IllFormedUtf16),
        );
        assert_eq!(
            collator.compare(&pair, &malformed),
            Err(CollationError::IllFormedUtf16),
        );
    }
}

#[test]
fn assemble_rejects_malformed_or_non_scalar_selected_candidates() {
    let collator = NetworkCollator::new("en-US").unwrap();
    let mut malformed = candidate("1");
    malformed.component = Value::String(vec![0xd800]);
    assert_eq!(
        collator.assemble(observation(vec![malformed])).unwrap_err(),
        CollationError::IllFormedUtf16,
    );
    let mut non_scalar = candidate("1");
    non_scalar.protocol = Value::Object(Vec::new());
    assert_eq!(
        collator.assemble(observation(vec![non_scalar])).unwrap_err(),
        CollationError::NonScalarCandidate,
    );
}

#[test]
fn assemble_ignores_tail_and_preserves_raw_history() {
    let collator = NetworkCollator::new("en-US").unwrap();
    let mut candidates: Vec<_> = (0..16).map(|index| candidate(&index.to_string())).collect();
    let mut malformed_tail = candidate("malformed");
    malformed_tail.kind = Value::String(vec![0xdc00]);
    candidates.push(malformed_tail);
    let mut non_scalar_tail = candidate("non-scalar");
    non_scalar_tail.protocol = Value::Array(Vec::new());
    candidates.push(non_scalar_tail);
    let expected_history = vec![vec![0xd800], vec![b'z' as u16], vec![b'a' as u16]];
    let mut input = observation(candidates);
    input.connection_states = expected_history.clone();
    let Value::Array(rows) = collator.assemble(input).unwrap() else {
        panic!("expected the outer network record array");
    };
    assert_eq!(rows.len(), 1);
    let Value::Array(record) = &rows[0] else {
        panic!("expected a network record");
    };
    let Value::Array(selected) = &record[15] else {
        panic!("expected the retained candidate array");
    };
    assert_eq!(selected.len(), 16);
    let Value::Array(history) = &record[54] else {
        panic!("expected the connection-state history");
    };
    assert_eq!(history.len(), expected_history.len());
    for (actual, expected) in history.iter().zip(&expected_history) {
        let Value::String(units) = actual else {
            panic!("expected raw UTF-16 history text");
        };
        assert_eq!(units, expected);
    }
}

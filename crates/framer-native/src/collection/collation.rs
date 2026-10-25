use crate::castle::v3::{assemble_network_record, NetworkObservation, Value};
use icu_collator::{options::CollatorOptions, Collator, CollatorBorrowed};
use icu_locale_core::Locale;
use std::cmp::Ordering;

#[derive(Debug, PartialEq, Eq)]
pub enum CollationError {
    UnsupportedLocale,
    IllFormedUtf16,
    NonScalarCandidate,
}

impl std::fmt::Display for CollationError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::UnsupportedLocale => "unsupported explicit collation locale",
            Self::IllFormedUtf16 => "native collation does not support unpaired surrogates",
            Self::NonScalarCandidate => "network candidate field is not scalar",
        })
    }
}

impl std::error::Error for CollationError {}

/// Explicit locale collation for the observation runtime's network signatures.
/// The locale must come from that runtime, not the service host's default.
/// These profiles were compared with Node 20.20.0 / ICU 77.1. Other engine
/// versions still require comparison; this type does not certify collection.
pub struct NetworkCollator(CollatorBorrowed<'static>);

impl NetworkCollator {
    pub fn new(locale: &str) -> Result<Self, CollationError> {
        let locale: Locale = locale.parse().map_err(|_| CollationError::UnsupportedLocale)?;
        match locale.to_string().as_str() {
            "en-US" | "en-GB" | "de" | "fr" | "fr-CA" | "sv" | "da" | "tr" |
            "pl" | "cs" | "ja" | "zh" | "th" | "ar" | "es-u-co-trad" |
            "de-u-co-phonebk" | "en-US-u-kn" | "en-US-u-kn-true" | "en-US-u-kf-upper" => {}
            _ => return Err(CollationError::UnsupportedLocale),
        }
        Collator::try_new(locale.into(), CollatorOptions::default())
            .map(Self).map_err(|_| CollationError::UnsupportedLocale)
    }

    /// ICU4X replaces lone surrogates whereas the selected JS engine preserves
    /// their ordering. Refuse those inputs instead of silently changing them.
    pub fn compare(&self, left: &[u16], right: &[u16]) -> Result<Ordering, CollationError> {
        valid_utf16(left)?;
        valid_utf16(right)?;
        Ok(self.0.compare_utf16(left, right))
    }

    /// Assemble using native stable sorting after validating selected fields.
    /// Histories and candidates beyond the SDK's first16 cap are not sorted.
    pub fn assemble(&self, observation: NetworkObservation) -> Result<Value, CollationError> {
        for c in observation.candidates.iter().take(16) {
            for value in [&c.kind, &c.protocol, &c.address_class, &c.related_address_class,
                          &c.port_class, &c.tcp_type, &c.component] {
                match value {
                    Value::String(text) => valid_utf16(text)?,
                    Value::Undefined | Value::Null | Value::Bool(_) | Value::Number(_) => {}
                    _ => return Err(CollationError::NonScalarCandidate),
                }
            }
        }
        assemble_network_record(observation, |a, b| self.0.compare_utf16(a, b))
            .map_err(|_| CollationError::NonScalarCandidate)
    }
}

fn valid_utf16(text: &[u16]) -> Result<(), CollationError> {
    if char::decode_utf16(text.iter().copied()).any(|c| c.is_err()) {
        Err(CollationError::IllFormedUtf16)
    } else { Ok(()) }
}

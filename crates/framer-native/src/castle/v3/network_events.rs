//! Ordered, data-only event reduction for the pinned SDK's N_ collector.
use super::network_candidate::{
    ascii_equal, ascii_word, candidate, lowercase, normalize, CandidateObservation,
};
use super::{NetworkObservation, Value};
use std::collections::BTreeSet;

#[derive(Clone, Copy)]
pub enum NetworkState {
    Connection,
    Gathering,
    Signaling,
}

/// One collection's supplied event sequence, without timers or network access.
#[derive(Default)]
pub struct NetworkEvents {
    candidates: Vec<[Vec<u16>; 7]>,
    signatures: BTreeSet<Vec<u16>>,
    connection: Vec<Vec<u16>>,
    gathering: Vec<Vec<u16>>,
    signaling: Vec<Vec<u16>>,
}

impl NetworkEvents {
    /// Consume the event sequence at the caller's finalization point.
    /// Both addresses are separate observations, never inferred from classes.
    pub fn into_observation(
        self,
        address: Vec<u16>,
        related_address: Vec<u16>,
    ) -> NetworkObservation {
        NetworkObservation {
            address,
            related_address,
            candidates: self.candidates.into_iter().map(candidate).collect(),
            connection_states: self.connection,
            gathering_states: self.gathering,
            signaling_states: self.signaling,
        }
    }

    /// Reduce SDP protocol mentions in their original order.
    pub fn observe_sdp(&mut self, value: &[u16]) {
        let patterns: [&[u8]; 3] = [b"UDP/TLS/RTP/SAVPF", b"DTLS/SCTP", b"UDP"];
        let mut at = 0;
        while at < value.len() && self.candidates.len() < 16 {
            let matched = if at > 0 && ascii_word(value[at - 1]) {
                None
            } else {
                patterns.iter().find(|p| {
                    at + p.len() <= value.len()
                        && ascii_equal(&value[at..at + p.len()], p)
                        && value.get(at + p.len()).is_none_or(|&c| !ascii_word(c))
                })
            };
            if let Some(pattern) = matched {
                let protocol = if pattern.starts_with(b"UDP") {
                    "udp"
                } else {
                    "dtls-sctp"
                };
                self.insert(
                    ["sdp", protocol, "", "", "", "", ""].map(|s| s.encode_utf16().collect()),
                );
                at += pattern.len();
            } else {
                at += 1;
            }
        }
    }

    fn insert(&mut self, fields: [Vec<u16>; 7]) {
        if self.candidates.len() >= 16 {
            return;
        }
        let mut signature = Vec::new();
        for (index, field) in fields.iter().enumerate() {
            if index > 0 {
                signature.push(b':' as u16);
            }
            signature.extend_from_slice(field);
        }
        if self.signatures.insert(signature) {
            self.candidates.push(fields);
        }
    }

    pub fn observe_candidate(&mut self, observation: &CandidateObservation) {
        self.insert(normalize(observation));
    }

    pub fn observe_state(&mut self, state: NetworkState, value: &Value) {
        let Value::String(value) = value else {
            return;
        };
        let value = lowercase(value);
        let history = match state {
            NetworkState::Connection => &mut self.connection,
            NetworkState::Gathering => &mut self.gathering,
            NetworkState::Signaling => &mut self.signaling,
        };
        if !value.is_empty() && history.len() < 8 && !history.contains(&value) {
            history.push(value);
        }
    }
}

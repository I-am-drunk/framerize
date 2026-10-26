use serde::{Deserialize, Serialize};
use crate::credit::Capacity;

/// Public fields fit independently; the encoded frame still has its own cap.
pub const TEXT_LIMIT: usize = 16 * 1024;
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct Text(String);
impl Text {
    pub fn new(value: String) -> Result<Self, crate::WireError> {
        if value.len() > TEXT_LIMIT { return Err(crate::WireError::Protocol); }
        Ok(Self(value))
    }
    pub fn as_str(&self) -> &str { &self.0 }
    pub fn into_string(self) -> String { self.0 }
}
impl<'de> Deserialize<'de> for Text {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Self::new(String::deserialize(d)?).map_err(serde::de::Error::custom)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Route { FreeResponses }

/// Opaque transport bytes. Interpretation belongs to the selected endpoint.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct Segment(Vec<u8>);
impl Segment {
    pub fn new(value: Vec<u8>) -> Result<Self, crate::WireError> {
        if value.len() > crate::SEGMENT_LIMIT { return Err(crate::WireError::Protocol); }
        Ok(Self(value))
    }
    pub fn as_slice(&self) -> &[u8] { &self.0 }
    pub fn into_vec(self) -> Vec<u8> { self.0 }
}
impl<'de> Deserialize<'de> for Segment {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct Visitor;
        impl<'de> serde::de::Visitor<'de> for Visitor {
            type Value = Segment;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, "at most {} opaque bytes", crate::SEGMENT_LIMIT)
            }
            fn visit_seq<A: serde::de::SeqAccess<'de>>(self, mut seq: A) -> Result<Segment, A::Error> {
                let mut bytes = Vec::new();
                while let Some(byte) = seq.next_element::<u8>()? {
                    if bytes.len() == crate::SEGMENT_LIMIT {
                        return Err(serde::de::Error::custom("segment byte limit"));
                    }
                    bytes.push(byte);
                }
                Ok(Segment(bytes))
            }
        }
        d.deserialize_seq(Visitor)
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Part {
    pub offset: u64,
    pub bytes: Segment,
    pub last: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Limits {
    pub chunk_tokens: u32, pub max_chunks: u32,
    pub max_total_tokens: Option<u32>, pub metered_fallback: bool,
    pub continue_prompt: Text,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Cause { pub op: u64, pub n: u32, pub fwd: u64 }
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureTag { Direct, SaturationExhausted, FallbackOff }
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FailureRecipe { pub cause: Cause, pub tag: FailureTag, pub chunk: u32 }

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorClass {
    Cancelled, RiskRefused, Unauthorized, CreditsExhausted,
    UpstreamForbidden, Retryable429, Retryable4, Cooldown, Other,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LocalFailure { Policy, InvalidResponse, IncompleteTool, Cancelled }
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutputFailure { Cancelled, Encoding, Closed, Unsupported }
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FinishReason { Completed, Incomplete }
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DataKind { Opening, Content, Heartbeat, Terminal }

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Summary {
    pub input_tokens: u64, pub output_tokens: u64,
    pub chunks: u32, pub metered_chunks: u32, pub finish: FinishReason,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Outcome {
    Success { summary: Summary },
    Forward { failure: FailureRecipe },
    Local { failure: LocalFailure },
    Output { failure: OutputFailure, completed: bool },
    Control,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublicFailure {
    #[serde(deserialize_with = "failure_status")]
    pub status: u16,
    pub message: Text,
    #[serde(rename = "type")]
    pub error_type: Text,
    pub code: Text,
}
fn failure_status<'de, D: serde::Deserializer<'de>>(d: D) -> Result<u16, D::Error> {
    let value = u16::deserialize(d)?;
    if !(400..600).contains(&value) {
        return Err(serde::de::Error::custom("invalid failure status"));
    }
    Ok(value)
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode { Sse }
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DoneOutcome { Completed, Cancelled, Panicked }

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Diagnostic {
    ChunkStart { chunk: u32, max_tokens: u32 },
    Backoff { chunk: u32, attempt: u32, wait_ms: u64 },
    RetryHalved { chunk: u32, elapsed_ms: u64, max_tokens: u32, cause: ErrorClass },
    MeteredFallback { chunk: u32, cause: ErrorClass },
    UpstreamQueued { chunk: u32, queue_ms: u64, wall_ms: u64 },
    ChunkDone { chunk: u32, tokens: u64, elapsed_ms: u64 },
    ToolBudgetWithheld { chunk: u32 },
    ToolRetry { chunk: u32, max_tokens: u32 },
    NamespaceBackfill { count: u64 },
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Dialect { OpenRouterResponses }

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Message {
    Hello { wire: u16, build: String, routes: Vec<Route> },
    Ready { generation: u64 },
    Start { op: u64, logical_id: Text, model: Text, mode: Mode, output: Capacity },
    Invoke { op: u64, n: u32, category: Text, limits: Limits, request: Part },
    Head { op: u64, status: u16, content_type: Text },
    Forward {
        op: u64, n: u32, fwd: u64, dialect: Dialect,
        step: Text, step_index: u32, payload: Part,
    },
    ForwardReply {
        op: u64, n: u32, fwd: u64, status: u16,
        content_type: Text, queue_wait_ms: u64, body: Part,
    },
    ForwardError { op: u64, n: u32, fwd: u64, class: ErrorClass, cooldown_seconds: Option<u64> },
    Saturated { op: u64, n: u32, fwd: u64 },
    SaturatedAck { op: u64, n: u32, fwd: u64 },
    Data { op: u64, event: DataKind, seq: u64, bytes: Segment, last: bool },
    Diag { op: u64, n: u32, event: Diagnostic },
    Result { op: u64, n: u32, outcome: Outcome, diag_dropped: u32 },
    Finish { op: u64, n: u32 },
    Fail { op: u64, n: u32, error: PublicFailure },
    Cancel { op: u64 },
    Done { op: u64, outcome: DoneOutcome, diag_dropped: u32 },
    Credit {
        op: u64,
        returned: Capacity,
        /// Exclusive frontier. Zero means that no event has been accepted.
        accepted_event_seq: u64,
    },
}

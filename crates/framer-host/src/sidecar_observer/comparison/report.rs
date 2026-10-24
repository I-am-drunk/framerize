use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContentType { Json, EventStream, Other, Missing }
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RefusalPolicy { Castle, Upstream }
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RefusalBasis { ProviderMarker, FramerSecurityEnvelope, FramerMarker, ElapsedFallback }
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Refusal { pub policy: RefusalPolicy, pub basis: RefusalBasis }

/// Only bounded facts cross into application diagnostics. The response body,
/// token, cookie jar and raw failure string are never retained here.
#[derive(Clone, Debug, serde::Deserialize)]
pub struct BaselineReport {
    pub accepted: bool,
    pub status: Option<u16>,
    pub content_type: ContentType,
    pub response_complete: bool,
    pub complete_json: bool,
    pub nonempty_text: bool,
    pub body_bytes: usize,
    pub transport_starts: Option<usize>,
    pub transport_finishes: Option<usize>,
    pub fetch_settled: bool,
    pub refusal: Option<Refusal>,
}

impl BaselineReport {
    pub(super) fn parse(value: Value) -> Result<Self, &'static str> {
        let failed = match value.get("failure") {
            Some(Value::Null) => false,
            Some(Value::String(s)) if !s.is_empty() && s.len() <= 96 => true,
            _ => return Err(FAILURE),
        };
        let report: Self = serde_json::from_value(value).map_err(|_| FAILURE)?;
        if report.status.is_some_and(|s| !(100..=599).contains(&s))
            || report.body_bytes > plan::BODY_LIMIT
            || report.transport_starts.is_some_and(|n| n > 1)
            || report.transport_finishes.is_some_and(|n| n > 1)
            || report.accepted && (failed || report.status != Some(200)
                || report.content_type != ContentType::Json || !report.response_complete
                || !report.complete_json || !report.nonempty_text || !report.fetch_settled
                || report.transport_starts != Some(1) || report.transport_finishes != Some(1)
                || report.body_bytes == 0 || report.refusal.is_some()) {
            return Err(FAILURE);
        }
        Ok(report)
    }
}

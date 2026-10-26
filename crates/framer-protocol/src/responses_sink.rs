//! Publication is awaited before policy mutates history or dispatches again.
use crate::convert::AssembledResponses;
use serde_json::Value;

pub struct ValidatedChunk {
    items: Vec<Value>,
    continues_text: bool,
}
impl ValidatedChunk {
    /// Validate an adapter's next chunk against its previously accepted output.
    pub fn validate(items: Vec<Value>, previous: &[Value], continues_text: bool)
        -> Result<Self, ResponseError> {
        crate::responses::validate_output_shape(&items)?;
        crate::responses::validate_tool_fields(&items, previous)?;
        if items.iter().any(|item| item["type"] == "message"
            && item.get("role").is_some_and(|role| role.as_str() != Some("assistant"))) {
            return Err("invalid message role".into());
        }
        Ok(Self::new(items, continues_text))
    }
    pub(crate) fn new(items: Vec<Value>, continues_text: bool) -> Self {
        Self { items, continues_text }
    }
    pub fn items(&self) -> &[Value] { &self.items }
    pub fn continues_text(&self) -> bool { self.continues_text }
    pub(crate) fn into_items(self) -> Vec<Value> { self.items }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum OutputError {
    #[error("client cancelled")] Cancelled,
    #[error("response output encoding failed")] Encoding,
    #[error("response output owner closed")] Closed,
    #[error("awaited Responses output unsupported")] Unsupported,
}

#[derive(Debug, thiserror::Error)]
pub enum ResponseError {
    #[error("{0}")] Upstream(String),
    #[error(transparent)] Output(#[from] OutputError),
}
impl From<String> for ResponseError {
    fn from(error: String) -> Self { Self::Upstream(error) }
}
impl From<&str> for ResponseError {
    fn from(error: &str) -> Self { Self::Upstream(error.to_owned()) }
}
pub type ResponseResult = Result<AssembledResponses, ResponseError>;

#[async_trait::async_trait]
pub trait ResponsesSink: Send + Sync {
    /// Success returns the same immutable owner after all events are accepted.
    async fn emit(&self, chunk: ValidatedChunk) -> Result<ValidatedChunk, OutputError>;
}

/// Validate and publish an assembled adapter result without cloning its output.
pub async fn emit_assembled(mut result: AssembledResponses,
    sink: &dyn ResponsesSink) -> ResponseResult {
    let chunk = ValidatedChunk::validate(std::mem::take(&mut result.output), &[], false)?;
    if !matches!(result.status.as_str(), "completed" | "incomplete") {
        return Err("invalid terminal status".into());
    }
    result.input_tokens.checked_add(result.output_tokens).ok_or("Responses total usage overflow")?;
    if !result.usage.is_null() && !result.usage.is_object() {
        return Err("invalid usage object".into());
    }
    if ["input_tokens", "output_tokens"].iter().any(|field|
        result.usage.get(*field).is_some_and(|value| value.as_u64().is_none())) {
        return Err("invalid usage count".into());
    }
    result.output = sink.emit(chunk).await?.into_items();
    Ok(result)
}

#[cfg(test)]
#[path = "responses_sink_tests.rs"]
mod tests;

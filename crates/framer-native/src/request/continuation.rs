//! Explicit caller bounds for continuing text, never retrying a refusal.
use super::{complete_json::Specification, invocation::AttemptContext, types::DispatchError};
use crate::authorization::BoundAttemptContext;

#[derive(Clone, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Continuation {
    pub max_output_tokens: u32,
    pub prompt: String,
    pub contexts: Vec<AttemptContext>,
}

impl Continuation {
    pub(crate) fn max_attempts(&self) -> usize { self.contexts.len() + 1 }

    pub(crate) fn matches_payload(&self, expected: &serde_json::Value,
        actual: &serde_json::Value, responses: bool, step: usize) -> bool {
        let (history, tokens) = if responses { ("input", "max_output_tokens") }
            else { ("messages", "max_tokens") };
        let (Some(expected), Some(actual)) = (expected.as_object(), actual.as_object())
            else { return false; };
        let Some(original) = expected[history].as_array() else { return false; };
        let Some(current) = actual.get(history).and_then(|v| v.as_array()) else { return false; };
        let prompt = if responses { serde_json::json!({"type":"message", "role":"user",
            "content":[{"type":"input_text", "text":self.prompt}]}) }
            else { serde_json::json!({"role":"user", "content":self.prompt}) };
        expected.len() == actual.len()
            && expected.iter().all(|(key, value)| key == history || key == tokens
                || actual.get(key) == Some(value))
            && current.starts_with(original) && current.len() >= original.len() + 2 * step
            && current.last() == Some(&prompt)
            && actual.get(tokens).and_then(|v| v.as_u64()).is_some_and(|tokens|
                tokens >= if responses { 16 } else { 1 }
                    && tokens <= u64::from(self.max_output_tokens))
    }

    pub(crate) fn validate(&self, spec: &Specification) -> Result<(), DispatchError> {
        let invalid = || DispatchError::new(super::types::FailureKind::InvalidRequest);
        if self.contexts.is_empty() || self.contexts.len() > 15
            || !(spec.token_limit..=32_768).contains(&self.max_output_tokens)
            || self.prompt.is_empty() || self.prompt.len() > 4096 {
            return Err(invalid());
        }
        for (step, context) in std::iter::once(&spec.context).chain(&self.contexts).enumerate() {
            BoundAttemptContext { project: spec.identity.project.clone(),
                chat_session: spec.identity.chat_session.clone(), step_index: step as u32,
                context: context.clone() }.validate().map_err(|_| invalid())?;
        }
        Ok(())
    }
}

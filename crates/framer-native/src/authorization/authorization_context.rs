use super::AttemptContext;
use super::ContextPolicy;
use super::PreparedAttempt;
use super::ProjectId;

/// Supplied by the real application, not inferred from an account or request ID.
/// The same binding must be supplied for this attempt in the request plan.
#[derive(Clone)]
pub struct BoundAttemptContext {
    pub project: ProjectId,
    pub chat_session: String,
    pub step_index: u32,
    pub context: AttemptContext,
}

pub(super) enum AuthorizationContext {
    Derived(ContextPolicy),
    Bound(BoundAttemptContext),
    Sequence(Vec<BoundAttemptContext>),
}

fn bounded(value: &str, bytes: usize) -> bool {
    !value.is_empty() && value.len() <= bytes && !value.chars().any(char::is_control)
}

impl BoundAttemptContext {
    pub(crate) fn validate(&self) -> Result<(), String> {
        let context = &self.context;
        if !bounded(&self.project.0, 256) || !bounded(&self.chat_session, 256)
            || [&context.feature_category, &context.feature_name, &context.step,
                &context.turn_id].iter().any(|text| !bounded(text, 256))
            || context.user_prompt.len() > 4096 {
            return Err("invalid supplied authorization context".to_owned());
        }
        Ok(())
    }
}

impl AuthorizationContext {
    pub(super) fn bound(value: BoundAttemptContext) -> Result<Self, String> {
        value.validate()?;
        Ok(Self::Bound(value))
    }

    pub(super) fn sequence(values: Vec<BoundAttemptContext>) -> Result<Self, String> {
        if values.is_empty() || values.len() > 16 {
            return Err("invalid authorization sequence length".into());
        }
        for (index, value) in values.iter().enumerate() {
            value.validate()?;
            if value.step_index as usize != index || value.project != values[0].project
                || value.chat_session != values[0].chat_session {
                return Err("invalid authorization sequence binding".into());
            }
        }
        Ok(Self::Sequence(values))
    }

    pub(super) fn context(&self, attempt: &PreparedAttempt<'_>) -> Result<AttemptContext, String> {
        let value = match self {
            Self::Derived(policy) => return policy.context(attempt),
            Self::Bound(value) => value,
            Self::Sequence(values) => values.get(attempt.step_index as usize)
                .ok_or_else(|| "authorization sequence exhausted".to_owned())?,
        };
        if attempt.project != &value.project || attempt.chat_session != value.chat_session
            || attempt.step_index != value.step_index {
            return Err("supplied authorization context mismatch".to_owned());
        }
        Ok(value.context.clone())
    }
}

#[cfg(test)]
#[path = "authorization_context_tests.rs"]
mod tests;

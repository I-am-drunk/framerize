use super::{AttemptContext, ContextPolicy, PreparedAttempt, ProjectId};

/// Supplied by the real application, not inferred from an account or request ID.
/// The same binding must be supplied to the one-shot request plan.
pub struct BoundAttemptContext {
    pub project: ProjectId,
    pub chat_session: String,
    pub step_index: u32,
    pub context: AttemptContext,
}

pub(super) enum AuthorizationContext {
    Derived(ContextPolicy),
    Bound(BoundAttemptContext),
}

fn bounded(value: &str, bytes: usize) -> bool {
    !value.is_empty() && value.len() <= bytes && !value.chars().any(char::is_control)
}

impl AuthorizationContext {
    pub(super) fn bound(value: BoundAttemptContext) -> Result<Self, String> {
        let context = &value.context;
        if !bounded(&value.project.0, 256) || !bounded(&value.chat_session, 256)
            || [&context.feature_category, &context.feature_name, &context.step,
                &context.turn_id].iter().any(|text| !bounded(text, 256))
            || context.user_prompt.len() > 4096 {
            return Err("invalid supplied authorization context".to_owned());
        }
        Ok(Self::Bound(value))
    }

    pub(super) fn context(&self, attempt: &PreparedAttempt<'_>) -> Result<AttemptContext, String> {
        match self {
            Self::Derived(policy) => policy.context(attempt),
            Self::Bound(value) => {
                if attempt.project != &value.project || attempt.chat_session != value.chat_session
                    || attempt.step_index != value.step_index {
                    return Err("supplied authorization context mismatch".to_owned());
                }
                let context = &value.context;
                Ok(AttemptContext {
                    feature_category: context.feature_category.clone(),
                    feature_name: context.feature_name.clone(),
                    step: context.step.clone(),
                    turn_id: context.turn_id.clone(),
                    user_prompt: context.user_prompt.clone(),
                })
            }
        }
    }
}

#[cfg(test)]
#[path = "authorization_context_tests.rs"]
mod tests;

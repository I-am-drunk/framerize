use thiserror::Error;

pub use framer_protocol::error::{AccountCooldown, chain_error_cause, is_security_refusal};

#[cfg(test)]
mod cooldown_tests {
    use super::AccountCooldown;

    #[test]
    fn account_cooldown_survives_chain_error_wrapping() {
        for cause in ["account-cooling:584", "chunk 12 retry died (fallback off): account-cooling:584"] {
            assert_eq!(AccountCooldown::from_error(cause), Some(AccountCooldown { retry_after_secs: 584 }));
        }
    }

    #[test]
    fn unrelated_or_malformed_errors_are_not_local_capacity() {
        for cause in ["risk-refused", "upstream 403: account-cooling:584", "retryable 500: account-cooling:584", "account-cooling:no", "account-cooling:2 trailing", "chunk x retry died (fallback off): account-cooling:2"] {
            assert_eq!(AccountCooldown::from_error(cause), None, "{cause}");
        }
    }

    #[test]
    fn local_cooldown_has_a_temporary_capacity_response_in_each_dialect() {
        use crate::convert::{error_json, ClientDialect};
        for dialect in [ClientDialect::AnthropicMessages, ClientDialect::OpenAiResponses, ClientDialect::OpenAiChat] {
            let (status, body) = error_json(dialect, "account-cooling:584");
            assert_eq!(status, 503);
            assert!(body["error"]["message"].as_str().unwrap().contains("584 seconds"));
        }
    }
}

#[derive(Debug, Error)]
pub enum CoreError {
    #[error("account not found")]
    AccountNotFound,
    #[error("no session for account (needs login)")]
    NoSession,
    #[error("account not eligible: {0}")]
    NotEligible(String),
    /// Local deadline refusal after account, session and scope checks.
    /// Carries the full remaining seconds; callers bound their retry interval.
    #[error(transparent)]
    AccountCooling(AccountCooldown),
    /// Local lifecycle refusal for an otherwise complete, matching scope.
    #[error("account is not ready")]
    AccountNotReady,
    #[error("workspace creation not permitted for this account")]
    CannotCreateWorkspace,
    /// Castle risk refusal (HTTP 403 on /ai/v3/*). Transient — cool down.
    #[error("risk-refused: {0}")]
    RiskRefused(String),
    /// Any failure surfaced by the fragile Framer layer.
    #[error("framer: {0}")]
    Framer(String),
    #[error("store: {0}")]
    Store(String),
}

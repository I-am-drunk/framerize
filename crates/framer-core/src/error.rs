use thiserror::Error;

#[derive(Debug, Error)]
pub enum CoreError {
    #[error("account not found")]
    AccountNotFound,
    #[error("no session for account (needs login)")]
    NoSession,
    #[error("account not eligible: {0}")]
    NotEligible(String),
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

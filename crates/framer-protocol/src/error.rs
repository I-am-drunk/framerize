use thiserror::Error;

/// Local account capacity, distinct from an upstream security refusal.
#[derive(Debug, Clone, Copy, Error, PartialEq, Eq)]
#[error("account-cooling:{retry_after_secs}")]
pub struct AccountCooldown {
    pub retry_after_secs: u64,
}

impl AccountCooldown {
    pub fn from_error(error: &str) -> Option<Self> {
        let cause = error.strip_prefix("chunk ")
            .and_then(|s| s.split_once(" retry died (fallback off): "))
            .filter(|(chunk, _)| chunk.parse::<u32>().is_ok())
            .map_or(error, |(_, cause)| cause);
        let retry_after_secs = cause.strip_prefix("account-cooling:")?.parse::<u64>().ok()?;
        Some(Self { retry_after_secs: retry_after_secs.max(1) })
    }
}

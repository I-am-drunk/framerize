//! Policy facts are distinct from legacy display and private runtime causes.
use crate::error::AccountCooldown;
use crate::responses_sink::{OutputError, ResponseError};

fn classify(text: &str, cooldown: Option<u64>) -> ErrorClass {
    match text {
        "client cancelled" => ErrorClass::Cancelled,
        "risk-refused" => ErrorClass::RiskRefused,
        "unauthorized" => ErrorClass::Unauthorized,
        "credits-exhausted" => ErrorClass::CreditsExhausted,
        _ if cooldown.is_some() => ErrorClass::Cooldown,
        _ if text.starts_with("upstream 403") => ErrorClass::UpstreamForbidden,
        _ if text.starts_with("retryable 429") => ErrorClass::Retryable429,
        _ if text.starts_with("retryable 4") => ErrorClass::Retryable4,
        _ => ErrorClass::Other,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorClass {
    Cancelled, RiskRefused, Unauthorized, CreditsExhausted,
    UpstreamForbidden, Retryable429, Retryable4, Cooldown, Other,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorOrigin { Local, Http }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorRecipe {
    Direct,
    SaturationExhausted { chunk: u32, retries: u32 },
    FallbackOff { chunk: u32 },
}
#[derive(Clone, Copy, Debug, thiserror::Error)]
#[error("policy control channel failed")]
pub struct ControlError;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LocalFailure { Policy, InvalidResponse, IncompleteTool, Cancelled }

#[derive(Debug, thiserror::Error)]
pub enum PolicyError {
    #[error(transparent)] Forward(#[from] ForwardError),
    #[error(transparent)] Control(#[from] ControlError),
    #[error(transparent)] Output(#[from] OutputError),
    #[error("{message}")] Local { kind: LocalFailure, message: String },
}
impl std::error::Error for ForwardError {}
impl From<String> for ForwardError {
    fn from(value: String) -> Self { Self::from_legacy(value) }
}
impl From<String> for PolicyError {
    fn from(message: String) -> Self {
        let kind = if message == "client cancelled" {
            LocalFailure::Cancelled
        } else { LocalFailure::Policy };
        Self::Local { kind, message }
    }
}
impl From<&str> for PolicyError {
    fn from(value: &str) -> Self { value.to_owned().into() }
}
impl From<ResponseError> for PolicyError {
    fn from(value: ResponseError) -> Self {
        match value {
            ResponseError::Upstream(message) => message.into(),
            ResponseError::Output(error) => Self::Output(error),
        }
    }
}
impl PolicyError {
    pub fn is_control(&self) -> bool { matches!(self, Self::Control(_)) }
    pub fn is_saturated(&self) -> bool {
        matches!(self, Self::Forward(error) if error.is_saturated())
    }
    pub fn cooldown(&self) -> Option<AccountCooldown> {
        match self { Self::Forward(error) => error.cooldown(), _ => None }
    }
    pub fn allows_transport_retry(&self, elapsed_secs: f32) -> bool {
        matches!(self, Self::Forward(error) if error.allows_transport_retry(elapsed_secs))
    }
    pub fn with_recipe(self, recipe: ErrorRecipe) -> Self {
        match self { Self::Forward(error) => error.with_recipe(recipe).into(), error => error }
    }
    pub fn class(&self) -> ErrorClass {
        match self { Self::Forward(error) => error.class, _ => ErrorClass::Other }
    }
    pub fn into_legacy(self) -> ResponseError {
        match self {
            Self::Output(error) => ResponseError::Output(error),
            error => ResponseError::Upstream(error.to_string()),
        }
    }
}

#[derive(Debug)]
pub struct ForwardError {
    pub class: ErrorClass,
    pub origin: ErrorOrigin,
    pub cooldown_seconds: Option<u64>,
    pub cause: Option<u64>,
    pub recipe: ErrorRecipe,
    display: String,
}
impl ForwardError {
    pub fn is_saturated(&self) -> bool {
        self.recipe == ErrorRecipe::Direct && self.class == ErrorClass::Retryable429
    }
    pub fn allows_transport_retry(&self, elapsed_secs: f32) -> bool {
        // Legacy wrappers no longer begin with a retry sentinel. In particular,
        // saturation exhaustion still gets the independent halve-and-retry.
        if self.recipe != ErrorRecipe::Direct { return true; }
        match self.class {
            ErrorClass::RiskRefused => elapsed_secs > 8.0,
            ErrorClass::Other | ErrorClass::Cancelled | ErrorClass::Cooldown => true,
            _ => false,
        }
    }
    /// Worker-side construction. Private runtime display never crosses here.
    pub fn from_facts(class: ErrorClass, cooldown_seconds: Option<u64>, cause: u64)
        -> Result<Self, ControlError> {
        if cause == 0 || cooldown_seconds == Some(0)
            || (class == ErrorClass::Cooldown) != cooldown_seconds.is_some() {
            return Err(ControlError);
        }
        Ok(Self { class, cooldown_seconds, cause: Some(cause),
            origin: ErrorOrigin::Local, recipe: ErrorRecipe::Direct,
            display: "upstream operation failed".into() })
    }
    pub fn http(mut self) -> Self { self.origin = ErrorOrigin::Http; self }
    pub fn with_recipe(mut self, recipe: ErrorRecipe) -> Self { self.recipe = recipe; self }
    pub fn original_message(&self) -> &str { &self.display }
    pub fn cooldown(&self) -> Option<AccountCooldown> {
        if self.recipe != ErrorRecipe::Direct { return None; }
        self.cooldown_seconds.map(|retry_after_secs| AccountCooldown { retry_after_secs })
    }
    pub fn from_legacy(display: String) -> Self {
        let cooldown_seconds = AccountCooldown::from_error(&display)
            .map(|value| value.retry_after_secs);
        let class = classify(&display, cooldown_seconds);
        Self { class, origin: ErrorOrigin::Local, cooldown_seconds,
            cause: None, recipe: ErrorRecipe::Direct, display }
    }
}
impl std::fmt::Display for ForwardError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.recipe {
            ErrorRecipe::Direct => f.write_str(&self.display),
            ErrorRecipe::SaturationExhausted { chunk, retries } => write!(f,
                "chunk {chunk}: workspace still saturated after {retries} retries ({})", self.display),
            ErrorRecipe::FallbackOff { chunk } => write!(f,
                "chunk {chunk} retry died (fallback off): {}", self.display),
        }
    }
}

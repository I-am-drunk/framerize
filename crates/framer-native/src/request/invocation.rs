//! Explicit native application entries. No browser, mint, retry or refund policy.
//! One immutable account generation and absolute deadline per invocation.
//! The generation-bound supplier registers preparation in the shared ten-work
//! limit. Other suppliers must provide their own observed cleanup contract.

use crate::transport::downstream::BridgeError;

use crate::transport::downstream::Cleanup;

use crate::transport::downstream::Pending;
use crate::account::client::AccountScope;
use crate::account::client::ClientError;
use crate::account::client::NativeAccountClient;
use crate::account::client::RequestContext;
use crate::account::client::SuppliedAuth;
use crate::account::owner::CompletionError;
use crate::account::owner::EndReason;
use crate::account::owner::StopReason;
use framer_api::proxy::Dialect;
use framer_core::ids::ProjectId;
use framer_core::port::ChainConfig;
use futures::future::BoxFuture;
use futures::FutureExt;
use futures::StreamExt;
use serde_json::Value;
use std::fmt;
use std::time::Duration;
use tokio::time::Instant;

pub struct NativeInvocation<'a> {
    pub client: &'a NativeAccountClient,
    pub scope: &'a AccountScope,
    pub authorization: Option<&'a dyn AttemptAuthorization>,
    pub deadline: Instant,
    pub missing_content_type: &'a str,
    /// Per-chunk JSON collection bound. Raw requests use native transport caps.
    pub max_body_bytes: usize,
}

/// Read-only view of the actual payload. Authorization cannot replace its
/// dialect, project, history or step index.
pub struct PreparedAttempt<'a> {
    pub scope: &'a AccountScope,
    pub project: &'a ProjectId,
    pub dialect: Dialect,
    pub payload: &'a Value,
    pub chat_session: &'a str,
    pub step_index: u32,
    pub deadline: Instant,
}

pub struct AttemptContext {
    pub feature_category: String,
    pub feature_name: String,
    pub step: String,
    pub turn_id: String,
    pub user_prompt: String,
}

pub struct AuthorizedAttempt {
    pub auth: SuppliedAuth,
    pub context: AttemptContext,
}

/// Records preparation before a waiter can be cancelled.
pub trait PreparationObserver: Send + Sync {
    fn preparing(&self, cleanup: crate::account::client::CloseObserver,
        cancellation: crate::account::client::PreparationControl);
}
impl<F> PreparationObserver for F
where F: Fn(crate::account::client::CloseObserver,
    crate::account::client::PreparationControl) + Send + Sync {
    fn preparing(&self, cleanup: crate::account::client::CloseObserver,
        cancellation: crate::account::client::PreparationControl) {
        self(cleanup, cancellation)
    }
}

/// Supplies credentials and context only, never an alternate transport.
pub trait AttemptAuthorization: Send + Sync {
    /// Suppliers that start external work must synchronously publish its
    /// original completion here before their first suspension. The default is
    /// only for immediate suppliers that start no independent work.
    fn authorize_observed<'a>(&'a self, attempt: PreparedAttempt<'a>,
        _observer: &'a dyn PreparationObserver)
        -> BoxFuture<'a, Result<AuthorizedAttempt, String>> {
        self.authorize(attempt)
    }

    /// Nonblocking ownership observation. Implementations must not detach work.
    fn observe_start(
        &self, _cleanup: Cleanup,
        _cancellation: crate::account::owner::Cancellation,
    ) {}

    /// Records a completed attempt without retaining its payload or credentials.
    fn observe_finish(&self, _metadata: &AttemptMetadata) {}

    fn authorize<'a>(
        &'a self,
        attempt: PreparedAttempt<'a>,
    ) -> BoxFuture<'a, Result<AuthorizedAttempt, String>>;
}

/// Original headers, distinct from provider token usage in the response body.
#[derive(Clone)]
pub struct AttemptMetadata {
    pub status: u16,
    pub usage_header: Option<String>,
    pub original_content_type: Option<String>,
    pub step_index: u32,
    /// Includes authorization, headers and body/cleanup completion.
    pub elapsed: Duration,
}

pub struct NativeChain<T> {
    pub response: T,
    pub attempts: Vec<AttemptMetadata>,
}

impl<T> fmt::Debug for NativeChain<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NativeChain").field("attempt_count", &self.attempts.len())
            .finish_non_exhaustive()
    }
}

impl fmt::Debug for AttemptMetadata {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AttemptMetadata")
            .field("status", &self.status)
            .field("step_index", &self.step_index)
            .field("elapsed", &self.elapsed)
            .finish_non_exhaustive()
    }
}

/// Details and bounded upstream bodies stay accessible without Debug logging.
/// Cleanup always means the original owner's completion, not merely Drop.
pub enum ApplicationError {
    MissingAuthorization,
    Deadline { cleanup: Option<Cleanup> },
    ScopeClosed { cleanup: Option<Cleanup> },
    Authorization(String),
    InvalidRequest(&'static str),
    Start(ClientError),
    Bridge { error: BridgeError, cleanup: Cleanup },
    Body { detail: String, metadata: AttemptMetadata, cleanup: Cleanup },
    BodyTooLarge { limit: usize, metadata: AttemptMetadata, cleanup: Cleanup },
    Cleanup { error: CompletionError, metadata: AttemptMetadata },
    Stopped { reason: StopReason, metadata: AttemptMetadata },
    Upstream { metadata: AttemptMetadata, body: Vec<u8> },
    TerminalFailure { metadata: AttemptMetadata, body: Vec<u8> },
    InvalidResponse { detail: &'static str, metadata: AttemptMetadata, body: Vec<u8> },
    Chain { error: Box<ApplicationError>, completed_attempts: Vec<AttemptMetadata> },
}

impl fmt::Debug for ApplicationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::MissingAuthorization => "missing authorization",
            Self::Deadline { .. } => "original deadline reached",
            Self::ScopeClosed { .. } => "account scope closed",
            Self::Authorization(_) => "authorization failed",
            Self::InvalidRequest(_) => "invalid native request",
            Self::Start(_) => "native admission failed",
            Self::Bridge { .. } => "native response failed",
            Self::Body { .. } => "native body failed",
            Self::BodyTooLarge { .. } => "native body exceeds explicit collection limit",
            Self::Cleanup { .. } => "native cleanup failed",
            Self::Stopped { .. } => "native owner stopped",
            Self::Upstream { .. } => "upstream rejected native request",
            Self::TerminalFailure { .. } => "upstream returned a terminal failure",
            Self::InvalidResponse { .. } => "invalid native response",
            Self::Chain { .. } => "native chain failed after completed attempts",
        })
    }
}

impl ApplicationError {
    pub(crate) fn with_attempts(self, completed_attempts: Vec<AttemptMetadata>) -> Self {
        if completed_attempts.is_empty() {
            self
        } else {
            Self::Chain { error: Box::new(self), completed_attempts }
        }
    }
}

pub(crate) fn validate_chain(cfg: &ChainConfig) -> Result<(), ApplicationError> {
    if cfg.chunk_tokens == 0 || cfg.max_chunks == 0 || cfg.max_total_tokens == Some(0) {
        return Err(ApplicationError::InvalidRequest("chain budget must be positive"));
    }
    Ok(())
}

impl NativeInvocation<'_> {
    /// One attempt, with no authorization refresh or alternate forwarding path.
    pub(crate) async fn start(
        &self,
        project: &ProjectId,
        dialect: Dialect,
        payload: Value,
        chat_session: &str,
        step_index: u32,
    ) -> Result<Pending, ApplicationError> {
        let supplier = self.authorization.ok_or(ApplicationError::MissingAuthorization)?;
        if Instant::now() >= self.deadline {
            return Err(ApplicationError::Deadline { cleanup: None });
        }
        if !payload.is_object() || project.0.is_empty() || chat_session.is_empty() {
            return Err(ApplicationError::InvalidRequest("invalid payload or request identity"));
        }
        let mut closed = self.scope.closed();
        if closed.clone().now_or_never().is_some() {
            return Err(ApplicationError::ScopeClosed { cleanup: None });
        }
        let attempt = PreparedAttempt {
            scope: self.scope, project, dialect, payload: &payload, chat_session,
            step_index, deadline: self.deadline,
        };
        let authorized = tokio::select! {
            biased;
            _ = tokio::time::sleep_until(self.deadline) => {
                return Err(ApplicationError::Deadline { cleanup: None });
            }
            _ = &mut closed => {
                return Err(ApplicationError::ScopeClosed { cleanup: None });
            }
            result = supplier.authorize(attempt) => {
                result.map_err(ApplicationError::Authorization)?
            }
        };
        // A supplier may complete at the same time as invalidation or expiry.
        if Instant::now() >= self.deadline {
            return Err(ApplicationError::Deadline { cleanup: None });
        }
        if closed.now_or_never().is_some() {
            return Err(ApplicationError::ScopeClosed { cleanup: None });
        }
        let context = RequestContext {
            dialect, project: project.clone(), payload,
            feature_category: authorized.context.feature_category,
            feature_name: authorized.context.feature_name,
            step: authorized.context.step,
            chat_session: chat_session.to_owned(),
            step_index, turn_id: authorized.context.turn_id,
            user_prompt: authorized.context.user_prompt,
        };
        // start rechecks the generation and closing state under its own lock.
        // The account supervisor retains cleanup even if this caller is lost.
        let pending = self.client.start(
            self.scope, context, authorized.auth, self.deadline,
            self.missing_content_type.to_owned(),
        ).map_err(ApplicationError::Start)?;
        supplier.observe_start(pending.cleanup.clone(), pending.cancellation.clone());
        Ok(pending)
    }

    /// Collect exactly one prepared JSON chunk. No retry, no synthetic response.
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn json(
        &self,
        project: &ProjectId,
        dialect: Dialect,
        payload: Value,
        chat_session: &str,
        step_index: u32,
        shape: fn(&Value) -> bool,
    ) -> Result<(Value, AttemptMetadata), ApplicationError> {
        if self.max_body_bytes == 0 {
            return Err(ApplicationError::InvalidRequest("collection limit must be positive"));
        }
        let started = Instant::now();
        let Pending { mut response, cleanup, cancellation } =
            self.start(project, dialect, payload, chat_session, step_index).await?;
        let mut closed = self.scope.closed();
        let response = tokio::select! {
            biased;
            _ = tokio::time::sleep_until(self.deadline) => {
                cancellation.cancel();
                return Err(ApplicationError::Deadline { cleanup: Some(cleanup) });
            }
            _ = &mut closed => {
                cancellation.cancel();
                return Err(ApplicationError::ScopeClosed { cleanup: Some(cleanup) });
            }
            result = &mut response => {
                result.map_err(|error| ApplicationError::Bridge {
                    error, cleanup: cleanup.clone(),
                })?
            }
        };
        let mut metadata = AttemptMetadata {
            status: response.proxy.status,
            usage_header: response.usage_header,
            original_content_type: response.original_content_type,
            step_index,
            elapsed: started.elapsed(),
        };
        let mut stream = response.proxy.stream;
        let mut bytes = Vec::new();
        loop {
            let item = tokio::select! {
                biased;
                _ = tokio::time::sleep_until(self.deadline) => {
                    cancellation.cancel();
                    return Err(ApplicationError::Deadline { cleanup: Some(cleanup) });
                }
                _ = &mut closed => {
                    cancellation.cancel();
                    return Err(ApplicationError::ScopeClosed { cleanup: Some(cleanup) });
                }
                item = stream.next() => item,
            };
            let Some(item) = item else { break; };
            let chunk = match item {
                Ok(chunk) => chunk,
                Err(detail) => {
                    cancellation.cancel();
                    metadata.elapsed = started.elapsed();
                    return Err(ApplicationError::Body { detail, metadata, cleanup });
                }
            };
            if chunk.len() > self.max_body_bytes.saturating_sub(bytes.len()) {
                cancellation.cancel();
                metadata.elapsed = started.elapsed();
                return Err(ApplicationError::BodyTooLarge {
                    limit: self.max_body_bytes, metadata, cleanup,
                });
            }
            bytes.extend_from_slice(&chunk);
        }
        // OwnedBody EOF already awaited this same cleanup handle.
        metadata.elapsed = started.elapsed();
        match cleanup.await {
            Ok(EndReason::Response) => {}
            Ok(EndReason::Stopped(reason)) => {
                return Err(ApplicationError::Stopped { reason, metadata });
            }
            Err(error) => return Err(ApplicationError::Cleanup { error, metadata }),
        }
        if let Some(supplier) = self.authorization { supplier.observe_finish(&metadata); }
        if !(200..300).contains(&metadata.status) {
            return Err(ApplicationError::Upstream { metadata, body: bytes });
        }
        let body: Value = match serde_json::from_slice(&bytes) {
            Ok(body) => body,
            Err(_) => return Err(ApplicationError::InvalidResponse {
                detail: "body is not JSON", metadata, body: bytes,
            }),
        };
        if matches!(body["status"].as_str(), Some("failed") | Some("cancelled"))
            || body.get("error").is_some_and(|value| !value.is_null()) {
            return Err(ApplicationError::TerminalFailure { metadata, body: bytes });
        }
        if !shape(&body) {
            return Err(ApplicationError::InvalidResponse {
                detail: "unexpected response structure", metadata, body: bytes,
            });
        }
        Ok((body, metadata))
    }
}

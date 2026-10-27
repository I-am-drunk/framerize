//! Kernel-owned invocation facts. Workers never supply authority or raw causes.
use framer_core::{convert::{self, ClientDialect}, port::{ErrorClass, ErrorOrigin,
    ErrorRecipe, ForwardError, OutputError, ResponseError}};
use framer_worker_wire::{message::{self as wire, Message}, WireError};
use std::sync::Arc;

fn to_wire_class(class: ErrorClass) -> wire::ErrorClass {
    use wire::ErrorClass as W;
    match class {
        ErrorClass::Cancelled => W::Cancelled, ErrorClass::RiskRefused => W::RiskRefused,
        ErrorClass::Unauthorized => W::Unauthorized, ErrorClass::CreditsExhausted => W::CreditsExhausted,
        ErrorClass::UpstreamForbidden => W::UpstreamForbidden, ErrorClass::Retryable429 => W::Retryable429,
        ErrorClass::Retryable4 => W::Retryable4, ErrorClass::Cooldown => W::Cooldown,
        ErrorClass::Other => W::Other,
    }
}

type Saturation = Option<Arc<dyn Fn() + Send + Sync>>;
fn public_failure(rendered: &str, origin: ErrorOrigin) -> Result<wire::PublicFailure, WireError> {
    public_failure_with_display(rendered, if origin == ErrorOrigin::Http {
        None
    } else { Some("upstream operation failed") })
}
fn public_failure_with_display(rendered: &str, fixed: Option<&str>)
    -> Result<wire::PublicFailure, WireError> {
    // Classification precedes redaction, once, with the exact legacy recipe.
    let (status, shaped) = convert::error_json(ClientDialect::OpenAiResponses, rendered);
    let field = |name: &str| shaped["error"][name].as_str().ok_or(WireError::Protocol);
    Ok(wire::PublicFailure {
        status, message: wire::Text::new(fixed.unwrap_or(field("message")?).into())?,
        error_type: wire::Text::new(field("type")?.into())?,
        code: wire::Text::new(field("code")?.into())?,
    })
}
fn resolve_local(decision: &mut Decision, outcome: wire::Outcome, cancelled: bool)
    -> Result<(), WireError> {
    match outcome {
        wire::Outcome::Local { failure } => {
            let message = match failure {
                wire::LocalFailure::Cancelled => "client cancelled",
                wire::LocalFailure::InvalidResponse => "invalid upstream response",
                wire::LocalFailure::IncompleteTool => "incomplete upstream tool call",
                wire::LocalFailure::Policy => "upstream operation failed",
            };
            decision.public_failure = Some(public_failure_with_display(message, Some(message))?);
            decision.result = Err(ResponseError::Upstream(message.into()));
        }
        wire::Outcome::Output { failure, completed } => {
            let error = match failure {
                wire::OutputFailure::Cancelled => OutputError::Cancelled,
                wire::OutputFailure::Closed => OutputError::Closed,
                wire::OutputFailure::Encoding => OutputError::Encoding,
                wire::OutputFailure::Unsupported => OutputError::Unsupported,
            };
            decision.completed_on_disconnect = completed && cancelled
                && matches!(error, OutputError::Cancelled | OutputError::Closed);
            decision.result = Err(error.into());
        }
        // This bypasses every string-based account and transport retry branch.
        wire::Outcome::Control => return Err(WireError::Protocol),
        _ => return Err(WireError::Protocol),
    }
    Ok(())
}
struct AcceptedForward { cause: wire::Cause, chunk: u32, saturation: Saturation }
struct FailedForward { accepted: AcceptedForward, error: ForwardError, consumed: bool }

/// One invocation, inside the existing FreeRequest account loop.
pub(crate) struct InvocationLedger {
    op: u64, n: u32, max_chunks: u32, next_forward: u64,
    saturation: Saturation, active: Option<AcceptedForward>,
    failure: Option<FailedForward>, ack: Option<wire::Cause>,
    saturations: u8, closed: bool, cleanup: bool, last_succeeded: bool,
}

impl InvocationLedger {
    pub(crate) fn resolve(self, outcome: wire::Outcome, cancelled: bool)
        -> Result<Decision, WireError> {
        if !self.closed || !self.settled() { return Err(WireError::Protocol); }
        let mut decision = Decision { result: Err(OutputError::Closed.into()),
            public_failure: None, completed_on_disconnect: false };
        match outcome {
            wire::Outcome::Success { summary } => {
                if summary.chunks > self.max_chunks || summary.metered_chunks > summary.chunks {
                    return Err(WireError::Protocol);
                }
                decision.result = Ok(summary);
            }
            wire::Outcome::Forward { failure } => {
                let record = self.failure.ok_or(WireError::Protocol)?;
                if record.accepted.cause != failure.cause || record.accepted.chunk != failure.chunk
                    || failure.chunk >= self.max_chunks { return Err(WireError::Protocol); }
                let recipe = match failure.tag {
                    wire::FailureTag::Direct => ErrorRecipe::Direct,
                    wire::FailureTag::FallbackOff => ErrorRecipe::FallbackOff { chunk: failure.chunk },
                    wire::FailureTag::SaturationExhausted if self.saturations == 9
                        && record.error.class == ErrorClass::Retryable429 && !record.consumed =>
                        ErrorRecipe::SaturationExhausted { chunk: failure.chunk, retries: 9 },
                    _ => return Err(WireError::Protocol),
                };
                let origin = record.error.origin;
                let rendered = record.error.with_recipe(recipe).to_string();
                decision.public_failure = Some(public_failure(&rendered, origin)?);
                decision.result = Err(ResponseError::Upstream(rendered));
            }
            other => resolve_local(&mut decision, other, cancelled && self.last_succeeded)?,
        }
        Ok(decision)
    }
}
pub(crate) struct Decision {
    pub result: Result<wire::Summary, ResponseError>,
    pub public_failure: Option<wire::PublicFailure>,
    pub completed_on_disconnect: bool,
}
impl InvocationLedger {
    pub(crate) fn new(op: u64, n: u32, max_chunks: u32, saturation: Saturation)
        -> Result<Self, WireError> {
        if op == 0 || n == 0 { return Err(WireError::Protocol); }
        Ok(Self { op, n, max_chunks, next_forward: 1, saturation, active: None,
            failure: None, ack: None, saturations: 0, closed: false, cleanup: true,
            last_succeeded: false })
    }
    pub(crate) fn cleanup_confirmed(&self) -> bool { self.cleanup }
    pub(crate) fn settled(&self) -> bool { self.active.is_none() && self.ack.is_none() }
    /// Synchronous receive-side effect, before the Ack can be enqueued.
    pub(crate) fn saturated(&mut self, cause: wire::Cause) -> Result<Message, WireError> {
        if self.closed || !self.settled() || self.saturations >= 9 {
            return Err(WireError::Protocol);
        }
        let record = self.failure.as_mut().ok_or(WireError::Protocol)?;
        if record.accepted.cause != cause || record.consumed
            || cause.fwd.checked_add(1) != Some(self.next_forward)
            || record.error.class != ErrorClass::Retryable429 {
            return Err(WireError::Protocol);
        }
        record.consumed = true;
        self.ack = Some(cause);
        self.saturations += 1;
        if let Some(mark) = &record.accepted.saturation { mark(); }
        Ok(Message::SaturatedAck { op: cause.op, n: cause.n, fwd: cause.fwd })
    }
    pub(crate) fn acknowledged(&mut self, cause: wire::Cause) -> Result<(), WireError> {
        if self.ack != Some(cause) { return Err(WireError::Protocol); }
        self.ack = None;
        Ok(())
    }
    /// Receipt closes dispatch immediately. Resolution waits for accepted work.
    pub(crate) fn close_result(&mut self, op: u64, n: u32) -> Result<(), WireError> {
        if self.closed || op != self.op || n != self.n { return Err(WireError::Protocol); }
        self.closed = true;
        Ok(())
    }
    pub(crate) fn accept_forward(&mut self, cause: wire::Cause, chunk: u32)
        -> Result<(), WireError> {
        if self.closed || !self.settled() || cause.op != self.op || cause.n != self.n
            || cause.fwd != self.next_forward || chunk >= self.max_chunks {
            return Err(WireError::Protocol);
        }
        self.next_forward = cause.fwd.checked_add(1).ok_or(WireError::Protocol)?;
        self.last_succeeded = false;
        self.active = Some(AcceptedForward { cause, chunk, saturation: self.saturation.clone() });
        Ok(())
    }
    /// Called only after the runtime Forward future and its cleanup have joined.
    pub(crate) fn forward_finished(&mut self, cause: wire::Cause, error: Option<ForwardError>)
        -> Result<Option<Message>, WireError> {
        let accepted = self.active.take().ok_or(WireError::Protocol)?;
        if accepted.cause != cause { return Err(WireError::Protocol); }
        match error {
            None => { self.last_succeeded = true; Ok(None) },
            Some(error) => {
                self.cleanup &= !crate::pool_control::cleanup_unconfirmed(error.original_message());
                let message = Message::ForwardError { op: cause.op, n: cause.n, fwd: cause.fwd,
                    class: to_wire_class(error.class), cooldown_seconds: error.cooldown_seconds };
                self.failure = Some(FailedForward { accepted, error, consumed: false });
                Ok(Some(message))
            }
        }
    }
}

use crate::request::invocation::ApplicationError;
use crate::request::invocation::AttemptMetadata;
use crate::account::client::ClientError;
use crate::account::owner::AdmissionError;
use crate::account::owner::StopReason;
use crate::request::types::self as port;
use crate::request::types::DispatchError;
use crate::request::types::FailureKind;

pub(super) fn metadata(m: AttemptMetadata) -> port::AttemptMetadata {
    port::AttemptMetadata {
        status: m.status, usage_header: m.usage_header,
        original_content_type: m.original_content_type,
        step_index: m.step_index, elapsed: m.elapsed,
    }
}

pub(super) fn error(e: ApplicationError) -> DispatchError {
    use ApplicationError as E;
    let kind = match e {
        E::MissingAuthorization => FailureKind::MissingAuthorization,
        E::Deadline { .. } => FailureKind::Deadline,
        E::ScopeClosed { .. } => FailureKind::ScopeClosed,
        E::Authorization(_) => FailureKind::Authorization,
        E::InvalidRequest(_) => FailureKind::InvalidRequest,
        E::Start(e) => client_error(e),
        E::Bridge { .. } => FailureKind::Transport,
        E::Body { metadata: m, .. } => return with_metadata(FailureKind::Body, m),
        E::BodyTooLarge { metadata: m, .. } => return with_metadata(FailureKind::BodyTooLarge, m),
        E::Cleanup { metadata: m, .. } => return with_metadata(FailureKind::Cleanup, m),
        E::Stopped { reason, metadata: m } => return with_metadata(
            if reason == StopReason::Deadline { FailureKind::Deadline }
            else { FailureKind::Cancelled }, m),
        E::Upstream { metadata: m, body } => return upstream(FailureKind::Upstream, m, body),
        E::TerminalFailure { metadata: m, body } => return upstream(FailureKind::TerminalFailure, m, body),
        E::InvalidResponse { metadata: m, body, .. } => return upstream(FailureKind::UnsupportedResponse, m, body),
        E::Chain { error: e, completed_attempts } => {
            let mut mapped = error(*e);
            let mut previous: Vec<_> = completed_attempts.into_iter().map(metadata).collect();
            previous.append(&mut mapped.attempts);
            mapped.attempts = previous;
            return mapped;
        }
    };
    DispatchError::new(kind)
}

fn with_metadata(kind: FailureKind, m: AttemptMetadata) -> DispatchError {
    DispatchError { kind, upstream: None, attempts: vec![metadata(m)] }
}

fn upstream(kind: FailureKind, m: AttemptMetadata, body: Vec<u8>) -> DispatchError {
    DispatchError { kind, upstream: Some(port::UpstreamFailure {
        metadata: metadata(m), body: body.into(),
    }), attempts: Vec::new() }
}

fn client_error(e: ClientError) -> FailureKind {
    match e {
        ClientError::InvalidContext => FailureKind::MissingContext,
        ClientError::Full | ClientError::Admission(AdmissionError::Full) => FailureKind::Overloaded,
        ClientError::StaleScope | ClientError::Draining | ClientError::SessionChanging
            | ClientError::NotReady => FailureKind::ScopeClosed,
        ClientError::WrongClient => FailureKind::InvalidScope,
        ClientError::CleanupFailed(_) => FailureKind::Cleanup,
        _ => FailureKind::Transport,
    }
}

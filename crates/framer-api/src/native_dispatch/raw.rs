use super::{errors, Parts, TrackedAuthorization};
use crate::experimental_native::application::ApplicationError;
use crate::proxy::Dialect;
use framer_core::convert::ClientDialect;
use framer_core::native_dispatch::{AttemptMetadata, DispatchError, FailureKind, RawResult};
use futures::StreamExt;
use serde_json::Value;
use tokio::time::Instant;

pub(super) async fn collect(parts: &Parts, auth: &TrackedAuthorization,
    dialect: ClientDialect, payload: Value) -> Result<RawResult, DispatchError> {
    let dialect = match dialect {
        ClientDialect::AnthropicMessages => Dialect::AnthropicMessages,
        ClientDialect::OpenAiChat => Dialect::OpenRouterChat,
        ClientDialect::OpenAiResponses => Dialect::OpenAiResponses,
    };
    let started = Instant::now();
    let invocation = parts.invocation(auth);
    let pending = parts.api.proxy_raw_native(&invocation, &parts.request.identity.project,
        dialect, payload, &parts.request.identity.chat_session).await.map_err(|error| auth.error(error))?;
    let response = pending.response.await.map_err(|error| errors::error(ApplicationError::Bridge {
        error, cleanup: pending.cleanup.clone(),
    }))?;
    let mut metadata = AttemptMetadata {
        status: response.proxy.status, usage_header: response.usage_header,
        original_content_type: response.original_content_type,
        step_index: 0, elapsed: started.elapsed(),
    };
    let mut stream = response.proxy.stream;
    let mut body = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = match chunk {
            Ok(chunk) => chunk,
            Err(_) => {
                metadata.elapsed = started.elapsed();
                return Err(DispatchError { kind: FailureKind::Body,
                    upstream: None, attempts: vec![metadata] });
            }
        };
        if chunk.len() > parts.policy.max_body_bytes.saturating_sub(body.len()) {
            metadata.elapsed = started.elapsed();
            return Err(DispatchError { kind: FailureKind::BodyTooLarge,
                upstream: None, attempts: vec![metadata] });
        }
        body.extend_from_slice(&chunk);
    }
    metadata.elapsed = started.elapsed();
    // Retain completed metadata if cleanup or public delivery hits the deadline.
    auth.registry.completed(metadata.clone());
    Ok(RawResult { body, metadata })
}

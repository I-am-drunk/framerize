use super::complete_json::CompletedJson;
use super::complete_json::FreeRoute;
use super::errors;
use super::lifetime;
use super::shared_lease::RunningAccountLease;
use super::tracked::TrackedAuthorization;
use super::types::*;
use crate::account::client::AccountScope;
use crate::account::client::NativeAccountClient;
use crate::request::invocation::AttemptAuthorization;
use crate::request::invocation::NativeInvocation;
use std::sync::Arc;
use serde_json::Value;
use tokio::sync::OwnedSemaphorePermit;
use tokio::time::Instant;

pub(super) struct Execution {
    pub client: NativeAccountClient,
    pub scope: AccountScope,
    pub identity: Identity,
    pub authorization: Arc<dyn AttemptAuthorization>,
    pub payload: Value,
    pub route: FreeRoute,
    pub config: framer_core::port::ChainConfig,
    pub deadline: Instant,
    pub permit: OwnedSemaphorePermit,
    pub admission: PoolAdmission,
    pub events: Option<super::ChunkSender>,
}

pub(super) fn execute(input: Execution) -> InvocationRun<Completed<CompletedJson>> {
    let Execution { client, scope, identity, authorization, payload, route,
        mut config, deadline, permit, admission, events } = input;
    let chained = config.max_chunks > 1;
    let max_attempts = config.max_chunks as usize;
    lifetime::launch_with_lease(client.clone(), scope.clone(), permit,
        RunningAccountLease::start(admission), deadline, max_attempts, move |registry| async move {
        let auth = TrackedAuthorization { inner: authorization, registry: registry.clone() };
        let invocation = NativeInvocation { client: &client, scope: &scope,
            authorization: Some(&auth), deadline, missing_content_type: "application/json",
            max_body_bytes: 64 * 1024 };
        let failed = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let cancelled = failed.clone();
        let downstream = events.clone();
        config.cancelled = Some(Arc::new(move || {
            cancelled.load(std::sync::atomic::Ordering::Acquire)
                || downstream.as_ref().is_some_and(|sender| sender.is_closed())
        }));
        let emit = |event| {
            if registry.last_response_accepted()
                && events.as_ref().is_some_and(|sender| sender.try_send(event).is_err()) {
                failed.store(true, std::sync::atomic::Ordering::Release);
            }
        };
        let callback = chained.then_some(&emit as &(dyn Fn(_) + Send + Sync));
        let (value, attempts) = match route {
            FreeRoute::Responses => {
                let result = super::responses::complete_long_native(&invocation,
                    &identity.project, payload, &config, &identity.chat_session, callback)
                    .await.map_err(|error| auth.error(error))?;
                (CompletedJson::Responses(result.response), result.attempts)
            },
            FreeRoute::Messages => {
                let result = super::messages::complete_long_native(&invocation,
                    &identity.project, payload, &config, &identity.chat_session, callback)
                    .await.map_err(|error| auth.error(error))?;
                (CompletedJson::Messages(result.response), result.attempts)
            },
        };
        // The one-attempt contract publishes only complete output. In particular,
        // do not expose a cut-off tool call before the final acceptance check.
        let json_success = attempts.len() == 1 && attempts[0].status == 200
            && super::complete_json::content_type(attempts[0].original_content_type.as_deref())
                == super::complete_json::ContentType::Json;
        if config.is_cancelled() { return Err(DispatchError::new(FailureKind::Cancelled)); }
        if let Some(sender) = events.as_ref().filter(|_| !chained && value.complete() && json_success) {
            let event = match &value {
                CompletedJson::Responses(value) => framer_core::convert::ChainEvent::ResponsesItems {
                    items: value.output.clone(), continues_text: false,
                },
                CompletedJson::Messages(value) => framer_core::convert::ChainEvent::Chunk(value.content.clone()),
            };
            sender.try_send(event).map_err(|_| DispatchError::new(FailureKind::Cancelled))?;
        }
        Ok(Completed { value, attempts: attempts.into_iter().map(errors::metadata).collect() })
    })
}

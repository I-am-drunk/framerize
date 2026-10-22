use super::{NativeCall, Parts};
use framer_core::{convert::ClientDialect, native_dispatch::{self as port,
    ComposedStreamOptions, ComposedStreamRun, DispatchError, FailureKind,
    InvocationRun, Route, stream_state::StateBudget}};
use futures::FutureExt;
use serde_json::Value;
use std::sync::Arc;
use tokio::sync::oneshot;

#[path = "composed_delivery.rs"]
mod delivery;
#[path = "composed_chain.rs"]
mod chain;
#[path = "composed_render.rs"]
mod render;
use delivery::{Control, HeadReceiver, lock};

#[derive(Default)]
pub(super) struct Hooks {
    #[cfg(test)]
    start: Option<super::raw_stream::TestStart>,
}

pub(super) fn launch(call: Box<NativeCall>, payload: Value,
    options: ComposedStreamOptions) -> ComposedStreamRun {
    launch_inner(call, payload, options, None, Hooks::default())
}

pub(super) fn launch_for_client(call: Box<NativeCall>, payload: Value,
    options: ComposedStreamOptions, client_model: String) -> ComposedStreamRun {
    launch_inner(call, payload, options, Some(client_model), Hooks::default())
}

#[cfg(test)]
pub(super) fn launch_for_test(call: Box<NativeCall>, payload: Value,
    options: ComposedStreamOptions, start: super::raw_stream::TestStart) -> ComposedStreamRun {
    launch_inner(call, payload, options, None, Hooks { start: Some(start) })
}

fn launch_inner(call: Box<NativeCall>, payload: Value,
    options: ComposedStreamOptions, client_model: Option<String>, hooks: Hooks) -> ComposedStreamRun {
    let validation = options.limits.validate().and_then(|_| {
        if client_model.as_ref().is_some_and(String::is_empty) {
            return Err(DispatchError::new(FailureKind::InvalidRequest));
        }
        let terminal_frames = if options.include_usage { 3 } else { 2 };
        if options.dialect == ClientDialect::OpenAiChat && options.limits.retained_frames
            < terminal_frames {
            return Err(DispatchError::new(FailureKind::InvalidRequest));
        }
        if !payload.is_object() || payload.get("stream").is_some_and(|v| !v.is_boolean())
            || !matches!((call.request.route, options.dialect),
                (Route::Messages, ClientDialect::AnthropicMessages)
                | (Route::Responses, ClientDialect::OpenAiResponses | ClientDialect::OpenAiChat)) {
            return Err(DispatchError::new(FailureKind::InvalidRequest));
        }
        if call.request.route == Route::Responses {
            if ["store", "background", "previous_response_id"].iter()
                .any(|key| payload.get(*key).is_some())
                || payload["model"].as_str().is_some_and(|m| m.ends_with(":batch")) {
                return Err(DispatchError::new(FailureKind::InvalidRequest));
            }
        }
        Ok(())
    });
    let mut budget = StateBudget::new(options.limits.state_bytes);
    let validation = validation.and_then(|_| {
        // The caller transfers this allocation. Admit its full capacity before
        // spawning or authorizing; the renderer charges its own copy separately.
        budget.charge(client_model.as_ref().map_or(0, String::capacity))
            .map_err(DispatchError::new)?;
        // Admit transferred input and all local identity/policy copies before
        // namespace validation allocates its temporary lookup table.
        budget.retain(&payload).map_err(DispatchError::new)?;
        budget.retain(&payload).map_err(DispatchError::new)?;
        let identity = &call.request.identity;
        let strings = [identity.user.0.capacity(), identity.team.0.capacity(),
            identity.project.0.capacity(), identity.chat_session.capacity(),
            call.policy.missing_content_type.capacity(),
            call.policy.continuation.as_ref().map_or(0, String::capacity)]
            .into_iter().try_fold(0usize, usize::checked_add)
            .ok_or_else(|| DispatchError::new(FailureKind::BodyTooLarge))?;
        let copies = strings.checked_mul(4 * (call.policy.max_chunks as usize + 1))
            .and_then(|n| n.checked_add(options.limits.header_bytes * 8))
            .and_then(|n| n.checked_add(call.policy.max_chunks as usize * 8192
                + options.limits.retained_frames * 128 + 8192))
            .ok_or_else(|| DispatchError::new(FailureKind::BodyTooLarge))?;
        budget.charge(copies).map_err(DispatchError::new)?;
        if call.request.route == Route::Responses {
            super::reject_namespace_repair(&payload)?;
        }
        Ok(())
    });
    if let Err(error) = validation { return rejected(error.kind); }
    let NativeCall { api, native, scope, authorization, request, policy, permit, account_lease } = *call;
    let account_lease = account_lease.into_running();
    let control = Control::new(options.limits, request.deadline, policy.max_chunks as usize);
    let parts = Parts { api, native: native.clone(), scope: scope.clone(), authorization, request, policy };
    let (head_tx, head_rx) = oneshot::channel();
    let work = control.clone();
    let mut worker = tokio::spawn(async move {
        let mut head_tx = Some(head_tx);
        let result = tokio::select! {
            biased;
            _ = tokio::time::sleep_until(work.deadline) => Err(work.error(FailureKind::Deadline)),
            kind = work.stopped() => Err(work.error(kind)),
            value = chain::produce(parts, payload, options, client_model, budget,
                work.clone(), &mut head_tx, hooks) => value,
        };
        let outcome = result.as_ref().map(|_| ()).map_err(|e| e.kind);
        if let Err(error) = &result { work.failure_frame(options.dialect, error.kind); }
        if let Some(sender) = head_tx {
            let _ = sender.send(Err(result.err().unwrap_or_else(|| work.error(FailureKind::Internal))));
        }
        work.complete(outcome);
    });
    let (finished_tx, finished_rx) = oneshot::channel();
    let (cleanup_tx, cleanup_rx) = oneshot::channel();
    let guarded = control.clone();
    let guardian = tokio::spawn(async move {
        // Both phases observe this handle. Shared cannot be polled after it
        // returns Ready; a finished body may also suppress the stop signal.
        let mut closed = scope.closed().fuse();
        let joined = tokio::select! {
            biased;
            _ = tokio::time::sleep_until(guarded.deadline) => {
                guarded.abort(FailureKind::Deadline); None
            }
            kind = guarded.stopped() => { guarded.abort(kind); None }
            result = &mut closed => {
                guarded.abort(if result.is_ok() { FailureKind::ScopeClosed } else { FailureKind::Cleanup });
                None
            }
            result = &mut worker => Some(result),
        };
        let worker_lost = match joined { Some(r) => r.is_err(), None => worker.await.is_err() };
        if worker_lost { guarded.abort(FailureKind::Internal); }
        tokio::select! {
            biased;
            _ = tokio::time::sleep_until(guarded.deadline) => guarded.abort(FailureKind::Deadline),
            kind = guarded.stopped() => guarded.abort(kind),
            result = &mut closed => guarded.abort(if result.is_ok() {
                FailureKind::ScopeClosed
            } else { FailureKind::Cleanup }),
            _ = guarded.body_finished() => {},
        }
        let _ = finished_tx.send(guarded.summary());
        let attempts = lock(&guarded.state).cleanups.clone();
        let mut failed = worker_lost;
        for (cleanup, _) in attempts {
            if cleanup.await.is_err() { failed = true; }
        }
        failed |= account_lease.scope_failed(&scope);
        if failed {
            drop(account_lease);
            let _ = cleanup_tx.send(Err(FailureKind::Cleanup));
            futures::future::pending::<()>().await;
            drop((permit, native, scope));
            return;
        }
        let bytes = guarded.bytes.clone()
            .acquire_many_owned(options.limits.retained_bytes as u32).await;
        let frames = guarded.frames.clone()
            .acquire_many_owned(options.limits.retained_frames as u32).await;
        let terminal = guarded.terminal.clone()
            .acquire_many_owned(options.limits.terminal_bytes as u32).await;
        // A first refusal has no SSE body, but its returned Bytes can still
        // own the fixed JSON buffer. Do not release invocation capacity while
        // that allocation survives in a result, clone, slice or HTTP body.
        let json = guarded.json.clone()
            .acquire_many_owned(options.limits.json_bytes as u32).await;
        if bytes.is_err() || frames.is_err() || terminal.is_err() || json.is_err() {
            drop(account_lease);
            let _ = cleanup_tx.send(Err(FailureKind::Cleanup));
            futures::future::pending::<()>().await;
            drop((bytes, frames, terminal, json, permit, native, scope));
            return;
        }
        if let Err(kind) = account_lease.complete(&scope) {
            let _ = cleanup_tx.send(Err(kind));
            futures::future::pending::<()>().await;
            drop((bytes, frames, terminal, json, permit, native, scope));
            return;
        }
        drop((bytes, frames, terminal, json, permit, native, scope));
        let _ = cleanup_tx.send(Ok(()));
    });
    let cleanup = async move {
        match cleanup_rx.await {
            Ok(Ok(())) => guardian.await.map_err(|_| FailureKind::Cleanup),
            Ok(Err(kind)) => { drop(guardian); Err(kind) }
            Err(_) => { drop(guardian); Err(FailureKind::Cleanup) }
        }
    }.boxed().shared();
    let observed = control.clone();
    let finished = async move {
        match finished_rx.await {
            Ok(summary) => summary,
            Err(_) => { observed.abort(FailureKind::Internal); observed.summary() }
        }
    }.boxed().shared();
    let progress = control.clone();
    ComposedStreamRun {
        response: InvocationRun {
            result: Box::pin(HeadReceiver { receiver: head_rx, control: control.clone(), done: false }),
            cleanup, cancellation: control,
            completed_attempts: Some(Arc::new(move || progress.progress())),
        },
        finished,
    }
}

struct NoWork;
impl port::Cancellation for NoWork { fn cancel(&self) {} }
fn rejected(kind: FailureKind) -> ComposedStreamRun {
    ComposedStreamRun {
        response: InvocationRun {
            result: async move { Err(DispatchError::new(kind)) }.boxed(),
            cleanup: async { Ok(()) }.boxed().shared(),
            cancellation: Arc::new(NoWork), completed_attempts: None,
        },
        finished: async move { port::ComposedStreamCompletion {
            result: Err(kind), attempts: Arc::from([]), emitted_bytes: 0,
            elapsed: std::time::Duration::ZERO,
        } }.boxed().shared(),
    }
}

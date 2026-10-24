//! Inert private coordinator. No endpoint, startup hook or live owner exists.
//! Runtime teardown still requires the application's independent owner.
use framer_api::{experimental_native::{
    authorization::CredentialTarget,
    native_account::{AccountScope, NativeAccountClient, PreparationControl,
        PreparationFailure, PreparationResult},
    native_http::{NativeEgress, NativeHttpLimits},
}, native_dispatch::complete_json::{FreeRoute, Plan, Specification}};
use framer_core::native_dispatch::{self as port, FailureKind, Identity};
use framer_server::{AppState, native_control::{self, AdmittedControl, HealthOutcome}};
use futures::{future::BoxFuture, FutureExt};
use std::{panic::AssertUnwindSafe, sync::{Arc, atomic::{AtomicBool, Ordering}}};
use tokio::{sync::{oneshot, watch}, time::Instant};

#[path = "resident_control_owner.rs"]
mod owner;
#[path = "resident_control_bootstrap.rs"]
mod bootstrap;
pub(crate) use bootstrap::StartupOwner;
#[path = "resident_control_wire.rs"]
mod wire;
#[path = "resident_control_attempt.rs"]
mod attempt;
pub(crate) use owner::{Capture, CaptureError, CapturedPage, Owner};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Failure {
    MissingOwner, Admission, Context, Acquire, Bearer, Baseline, Mint, Native,
    Cancelled, Deadline, Publication, Cleanup, OwnerLost,
}

/// Only bounded facts leave this module. A complete HTTP response is not
/// evidence of accepted inference, SDK provenance or a memory improvement.
#[derive(Default)]
pub(crate) struct Report {
    pub failure: Option<Failure>,
    pub baseline_accepted: bool,
    pub baseline_status: Option<u16>,
    pub baseline_body_complete: bool,
    pub baseline_json_object: bool,
    pub native_invoked: bool,
    pub status: Option<u16>,
    pub body_complete: bool,
    pub json_object: bool,
    /// Completed nonempty model text in this response; independent of cleanup.
    pub native_output_complete: bool,
    pub refusal: Option<framer_api::classify::RefusalObservation>,
    pub cleanup_confirmed: bool,
    pub host_fenced: bool,
}

impl Report {
    fn failed(failure: Failure) -> Self {
        Self { failure: Some(failure), ..Self::default() }
    }
}
struct Stop(watch::Sender<bool>);
impl Stop {
    fn cancel(&self) { self.0.send_replace(true); }
    async fn cancelled(&self) {
        let mut receiver = self.0.subscribe();
        while !*receiver.borrow_and_update() {
            if receiver.changed().await.is_err() { break; }
        }
    }
    fn reason(&self, deadline: Instant) -> Option<Failure> {
        if Instant::now() >= deadline { Some(Failure::Deadline) }
        else if *self.0.borrow() { Some(Failure::Cancelled) } else { None }
    }
}
struct CancelOnDrop(Arc<Stop>);
impl Drop for CancelOnDrop { fn drop(&mut self) { self.0.cancel(); } }

#[must_use = "dropping a waiter requests cancellation but never releases ownership"]
pub(crate) struct Run {
    pub result: BoxFuture<'static, Report>,
    pub cleanup: port::Cleanup,
    stop: Arc<Stop>,
}
impl Run { pub(crate) fn cancel(&self) { self.stop.cancel(); } }

/// No default owner or URL route exists. The startup owner must explicitly
/// supply authoritative capture before this can be invoked. The baseline uses
/// the same ordered private RPC lease as acquire and the separate native mint.
pub(crate) fn launch(state: AppState, spec: Specification, baseline_turn: String,
    owner: Arc<dyn Owner>) -> Result<Run, Failure> {
    Plan::validate(&spec).map_err(|_| Failure::Context)?;
    framer_host::sidecar::resident::ResidentRpcV2::validate_baseline_body(&spec.payload)
        .map_err(|_| Failure::Context)?;
    if baseline_turn.is_empty() || baseline_turn.len() > 256
        || baseline_turn.chars().any(char::is_control)
        || baseline_turn == spec.context.turn_id { return Err(Failure::Context); }
    let ownership = state.operation_ownership.clone().ok_or(Failure::MissingOwner)?;
    let stop = Arc::new(Stop(watch::channel(false).0));
    let cancel = stop.clone();
    let (sender, receiver) = oneshot::channel();
    let task = framer_server::operation_ownership::spawn(Some(&ownership),
        framer_core::operation_gate::OperationKind::Inference, async move {
            let report = AssertUnwindSafe(owned(state, spec, baseline_turn, owner, cancel))
                .catch_unwind().await.unwrap_or_else(|_| Report::failed(Failure::OwnerLost));
            let settled = report.cleanup_confirmed;
            let _ = sender.send(report);
            if settled { Ok(()) } else { Err(FailureKind::Cleanup) }
        }).map_err(|_| Failure::Admission)?;
    let cleanup = async move { task.await.unwrap_or(Err(FailureKind::Cleanup)) }.boxed().shared();
    let guard = CancelOnDrop(stop.clone());
    let result = async move { let _guard = guard;
        receiver.await.unwrap_or_else(|_| Report::failed(Failure::OwnerLost)) }.boxed();
    Ok(Run { result, cleanup, stop })
}

async fn owned(state: AppState, spec: Specification, baseline_turn: String,
    owner: Arc<dyn Owner>, stop: Arc<Stop>) -> Report {
    let admitted = match native_control::admit_existing(&state, spec.identity.clone(),
        spec.payload["model"].as_str().unwrap_or_default(), spec.deadline).await {
        Ok(admitted) => admitted,
        Err(_) => return Report { cleanup_confirmed: true,
            ..Report::failed(Failure::Admission) },
    };
    if let Some(failure) = stop.reason(spec.deadline) {
        return Report { cleanup_confirmed: true, ..Report::failed(failure) };
    }
    let hold = owner::Hold::new(admitted);
    let path = match spec.route {
        FreeRoute::Responses => "/ai/v3/responses/", FreeRoute::Messages => "/ai/v3/messages/",
    };
    let captured = AssertUnwindSafe(async {
        // Cancellation never drops an in-flight bootstrap or exclusive scope job.
        retain_until_settled(owner.capture(&hold.admitted().identity,
            &hold.admitted().proxy, path, spec.deadline), &stop, spec.deadline, || {}).await
    }).catch_unwind().await;
    let capture = match captured {
        Ok(Ok(capture)) => Arc::new(capture),
        Ok(Err(error)) => {
            let settled = error.cleanup_confirmed && hold.complete();
            return Report { cleanup_confirmed: settled, ..Report::failed(error.failure) };
        }
        Err(_) => return Report::failed(Failure::OwnerLost),
    };
    let outcome = AssertUnwindSafe(async {
        if let Some(failure) = stop.reason(spec.deadline) {
            return Report { cleanup_confirmed: true, ..Report::failed(failure) };
        }
        session(&state, spec, baseline_turn, capture.clone(), &hold, stop).await
    }).catch_unwind().await;
    let mut report = outcome.unwrap_or_else(|_| Report::failed(Failure::OwnerLost));
    // A final host receipt follows the session's native close. Early errors
    // and panics still need this close attempt, while retaining uncertain Hold.
    let closed = if report.host_fenced { true } else {
        match capture.native.close_account(&capture.scope) {
            Ok(closed) => closed.await.is_ok(), Err(_) => false,
        }
    };
    report.cleanup_confirmed &= closed;
    if report.cleanup_confirmed {
        report.cleanup_confirmed = hold.complete();
    }
    if !report.cleanup_confirmed { report.failure = Some(Failure::Cleanup); }
    report
}

/// Cancellation is a signal only. The retained future always joins before
/// its owner can release any reservation or consume a settlement receipt.
async fn retain_until_settled<F: std::future::Future>(work: F, stop: &Stop,
    deadline: Instant, cancel: impl FnOnce()) -> F::Output {
    tokio::pin!(work);
    tokio::select! {
        biased;
        _ = stop.cancelled() => { cancel(); work.await }
        _ = tokio::time::sleep_until(deadline) => { cancel(); work.await }
        result = &mut work => result,
    }
}

#[cfg(test)]
#[path = "resident_control_tests.rs"]
mod tests;

async fn session(state: &AppState, spec: Specification, baseline_turn: String,
    capture: Arc<Capture>, hold: &owner::Hold, stop: Arc<Stop>) -> Report {
    let admitted = hold.admitted();
    if capture.identity != admitted.identity || capture.scope.account() != admitted.identity.account
        || capture.scope.active_user() != &admitted.identity.user
        || !owner::routes_match(&capture.scope, &capture.configured_proxy,
            &admitted.proxy, &capture.page.effective_proxy) {
        return Report::failed(Failure::Context);
    }
    let deadline = spec.deadline;
    if admitted.completion.revalidate(state, &admitted.identity, &admitted.proxy).await.is_err() {
        return Report { cleanup_confirmed: true, ..Report::failed(Failure::Admission) };
    }
    let operation = match attempt::Operation::bind(capture.clone()).await {
        Ok(value) => Arc::new(value), Err(failure) => return Report {
            cleanup_confirmed: true, ..Report::failed(failure) },
    };
    let worker = operation.clone();
    let saved = Arc::new(std::sync::Mutex::new(None));
    let saved_worker = saved.clone();
    let pending = capture.native.prepare_owned(&capture.scope, deadline,
        move |session, control| async move {
            let result = attempt::run(worker, spec, baseline_turn, session, control).await;
            let value = result.value.map(|report| {
                *saved_worker.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(report);
            });
            PreparationResult { value, cleanup_confirmed: result.cleanup_confirmed }
        });
    let mut report = match pending {
        Ok(pending) => {
            let control = pending.cancellation();
            let completion = pending.completion();
            let _ = retain_until_settled(pending, &stop, deadline, || control.cancel()).await;
            let settled = completion.await.is_ok();
            let mut report = saved.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
                .take().unwrap_or_else(|| {
                    if settled && !operation.acquire_started.load(Ordering::Acquire) {
                        Report { cleanup_confirmed: true,
                            ..Report::failed(stop.reason(deadline).unwrap_or(Failure::Bearer)) }
                    } else { Report::failed(Failure::Cleanup) }
                });
            report.cleanup_confirmed &= settled;
            report
        }
        Err(_) => Report { cleanup_confirmed: true, ..Report::failed(Failure::Admission) },
    };
    let closed = match capture.native.close_account(&capture.scope) {
        Ok(closed) => closed.await.is_ok(), Err(_) => false,
    };
    report.cleanup_confirmed &= closed;
    let health = if report.refusal.is_some_and(|r| r.policy == framer_api::classify::Refusal::Castle) {
        HealthOutcome::RiskRefused
    } else { HealthOutcome::Unclassified };
    if admitted.completion.publish(state, &admitted.identity, &admitted.proxy, health)
        .await.is_err() {
        report.failure = Some(Failure::Publication);
        report.cleanup_confirmed = false;
    }
    if let Some(reason) = stop.reason(deadline) { report.failure = Some(reason); }
    if operation.acquire_started.load(Ordering::Acquire) {
        report.host_fenced = operation.settle(report.cleanup_confirmed).await;
        report.cleanup_confirmed &= report.host_fenced;
    }
    report
}

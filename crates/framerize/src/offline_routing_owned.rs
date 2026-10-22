use super::*;
use framer_api::experimental_native::{native_http::{NativeEgress, NativeHttpLimits},
    session_store::OpenAccountError};
use framer_server::native_control::{self, ControlOwner, HealthOutcome};
use std::panic::AssertUnwindSafe;
use tokio::sync::oneshot;

pub(super) struct Completion {
    pub error: Option<DispatchError>,
    pub confirmed: bool,
    pub health: HealthOutcome,
}
impl Completion {
    fn rejected(error: DispatchError) -> Self {
        Self { error: Some(error), confirmed: true, health: HealthOutcome::Unclassified }
    }
    fn lost() -> Self {
        Self { confirmed: false, ..Self::rejected(DispatchError::new(FailureKind::Cleanup)) }
    }
}

type Admission = (Preparation, Value, tokio::sync::OwnedSemaphorePermit, port::PoolAdmission);

pub(super) trait Delivered {
    fn health(&self) -> HealthOutcome { HealthOutcome::Unclassified }
}
impl<T> Delivered for port::Completed<T> {}
impl Delivered for port::RawStreamHead {}
impl Delivered for port::ComposedStreamHead {}
impl Delivered for port::RawResult {
    fn health(&self) -> HealthOutcome { classify(&self.metadata, &self.body) }
}

pub(super) async fn run<T, F>(prepared: Prepared, payload: Value, valid: bool,
    control: Arc<running::Control>, operation: F, sender: oneshot::Sender<Result<T, DispatchError>>)
    -> Result<(), FailureKind>
where T: Delivered + Send + 'static,
    F: FnOnce(Box<dyn OwnedInvocation>, Value) -> port::InvocationRun<T> + Send + 'static,
{
    let Prepared { shared, request, permit, pool } = prepared;
    let identity = request.identity.clone();
    let binding = if valid && payload.is_object() {
        control.stopped(request.deadline).and_then(|_| context::resolve(&shared, &request, &payload))
    } else { Err(DispatchError::new(FailureKind::InvalidRequest)) };
    let (binding, source) = match binding {
        Ok(value) => value,
        Err(error) => { control.fail(error.kind); let _ = sender.send(Err(error)); return Ok(()); }
    };
    let proxy = binding.proxy.clone();
    let mut owner = match native_control::retain_prepared(&shared.state, &identity, &pool) {
        Ok(owner) => owner,
        Err(error) => { control.fail(error.kind); let _ = sender.send(Err(error)); return Ok(()); }
    };
    let mut admission = Some((request, payload, permit, pool));
    let mut sender = Some(sender);
    owner.start();
    let mut completion = AssertUnwindSafe(session(&shared, binding, source,
        &mut admission, &owner, control.clone(), operation, &mut sender))
        .catch_unwind().await.unwrap_or_else(|_| Completion::lost());
    let publication = owner.publish(&shared.state, &identity, &proxy, completion.health).await;
    let mut cleanup = if completion.confirmed { Ok(()) } else { Err(FailureKind::Cleanup) };
    if completion.confirmed {
        if owner.complete().is_err() { cleanup = Err(FailureKind::Cleanup); }
    } else {
        drop(owner);
        if let Some((_, _, permit, _)) = admission.take() { permit.forget(); }
    }
    if let Err(error) = publication {
        if cleanup.is_ok() { cleanup = Err(error.kind); }
        completion.error.get_or_insert(error);
    }
    if matches!(cleanup, Err(FailureKind::Cleanup)) {
        completion.error.get_or_insert_with(|| DispatchError::new(FailureKind::Cleanup))
            .kind = FailureKind::Cleanup;
    }
    if let Some(error) = completion.error {
        control.fail(error.kind);
        if let Some(sender) = sender.take() { let _ = sender.send(Err(error)); }
    }
    cleanup
}

async fn session<T, F>(shared: &Shared, binding: RequestContext, source: Arc<dyn SourceBuilder>,
    admission: &mut Option<Admission>, owner: &ControlOwner, control: Arc<running::Control>,
    operation: F, sender: &mut Option<oneshot::Sender<Result<T, DispatchError>>>) -> Completion
where T: Delivered + Send + 'static,
    F: FnOnce(Box<dyn OwnedInvocation>, Value) -> port::InvocationRun<T> + Send + 'static,
{
    if !shared.sessions.matches(&shared.state) {
        return Completion::rejected(DispatchError::new(FailureKind::InvalidScope));
    }
    if let Err(error) = owner.revalidate(&shared.state, &binding.identity, &binding.proxy).await {
        return Completion::rejected(error);
    }
    if let Err(error) = control.stopped(admission.as_ref().unwrap().0.deadline) {
        return Completion::rejected(error);
    }
    let egress = binding.proxy.as_deref().map(NativeEgress::Proxy).unwrap_or(NativeEgress::Direct);
    let opened = shared.sessions.bridge().open_account_observed(binding.identity.account,
        binding.identity.user.clone(), NativeHttpLimits::default(), egress).await;
    let scope = match opened {
        Ok(scope) => scope,
        Err(OpenAccountError::Rejected(_)) =>
            return Completion::rejected(DispatchError::new(FailureKind::Authorization)),
        Err(OpenAccountError::CleanupUnconfirmed) => return Completion::lost(),
    };
    let mut completed = AssertUnwindSafe(execute(shared, binding, source, admission,
        owner, control, &scope, operation, sender)).catch_unwind().await
        .unwrap_or_else(|_| Completion::lost());
    let closed = match shared.sessions.native().close_account(&scope) {
        Ok(observer) => observer.await.is_ok(),
        Err(_) => false,
    };
    if !closed { completed.confirmed = false; }
    completed
}

async fn execute<T, F>(shared: &Shared, mut binding: RequestContext, source: Arc<dyn SourceBuilder>,
    admission: &mut Option<Admission>, owner: &ControlOwner, control: Arc<running::Control>,
    scope: &framer_api::experimental_native::native_account::AccountScope,
    operation: F, sender: &mut Option<oneshot::Sender<Result<T, DispatchError>>>) -> Completion
where T: Delivered + Send + 'static,
    F: FnOnce(Box<dyn OwnedInvocation>, Value) -> port::InvocationRun<T> + Send + 'static,
{
    let deadline = admission.as_ref().unwrap().0.deadline;
    if let Err(error) = control.stopped(deadline) { return Completion::rejected(error); }
    if let Err(error) = owner.revalidate(&shared.state, &binding.identity, &binding.proxy).await {
        return Completion::rejected(error);
    }
    if matches!(admission.as_ref().unwrap().0.route, port::Route::Raw(_)) {
        binding.paid_credit = match credit::read(shared, scope, &binding, deadline, &control).await {
            Ok(value) => Some(value),
            Err(error) if error.kind == FailureKind::Cleanup => return Completion::lost(),
            Err(error) => return Completion::rejected(error),
        };
        if let Err(error) = control.stopped(deadline) { return Completion::rejected(error); }
        if let Err(error) = owner.revalidate(&shared.state, &binding.identity, &binding.proxy).await {
            return Completion::rejected(error);
        }
    }
    let source = match source.build(scope.clone(), binding.proxy) {
        Ok(source) => source,
        Err(_) => return Completion::rejected(DispatchError::new(FailureKind::Authorization)),
    };
    let authorization = context::Authorization { native: shared.sessions.native().clone(),
        scope: scope.clone(), source, identity: binding.identity.clone(), contexts: binding.attempts };
    let provider = NativeDispatchProvider::new(Arc::new(HttpFramerClient::new()),
        shared.sessions.native().clone(), vec![Binding {
            scope: scope.clone(), team: binding.identity.team, project: binding.identity.project,
            authorization: Some(Arc::new(authorization)), paid_credit: binding.paid_credit,
        }], shared.policy.clone());
    let provider = provider.and_then(|provider| match shared.streaming {
        Some(policy) => provider.with_streaming_policy(policy),
        None => Ok(provider),
    });
    let provider = match provider {
        Ok(provider) => provider,
        Err(error) => return Completion::rejected(error),
    };
    if let Err(error) = control.stopped(deadline) { return Completion::rejected(error); }
    let (request, payload, permit, pool) = admission.take().unwrap();
    let call = match provider.prepare_in_pool(request, permit, pool).await {
        Ok(call) => call,
        Err(error) => return Completion::rejected(error),
    };
    forward(operation(call, payload), deadline, control, sender).await
}

pub(super) async fn forward<T: Delivered>(run: port::InvocationRun<T>, deadline: tokio::time::Instant,
    control: Arc<running::Control>, sender: &mut Option<oneshot::Sender<Result<T, DispatchError>>>)
    -> Completion {
    control.attach(&run);
    let value = run.result.await;
    let health = match &value {
        Ok(delivered) => delivered.health(),
        Err(error) => error.upstream.as_ref().map(|e| classify(&e.metadata, &e.body))
            .unwrap_or(HealthOutcome::Unclassified),
    };
    let error = match value {
        Ok(value) => {
            if let Some(sender) = sender.take() { let _ = sender.send(Ok(value)); }
            None
        }
        Err(error) => Some(error),
    };
    let confirmed = tokio::select! {
        biased;
        _ = tokio::time::sleep_until(deadline) => {
            run.cancellation.cancel();
            run.cleanup.await.is_ok()
        }
        value = run.cleanup.clone() => value.is_ok(),
    };
    Completion { error, confirmed, health }
}

fn classify(metadata: &port::AttemptMetadata, body: &[u8]) -> HealthOutcome {
    use framer_api::classify::{observe_403, Refusal, RefusalBasis};
    if metadata.status != 403 || body.len() > 16 * 1024 {
        return HealthOutcome::Unclassified;
    }
    let Ok(text) = std::str::from_utf8(body) else { return HealthOutcome::Unclassified; };
    let result = observe_403(text, metadata.elapsed.as_millis().min(u64::MAX as u128) as u64);
    if result.policy == Refusal::Castle && result.basis != RefusalBasis::ElapsedFallback {
        HealthOutcome::RiskRefused
    } else { HealthOutcome::Unclassified }
}

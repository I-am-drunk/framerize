//! Test-only adaptation of the existing synthetic providers to pool ownership.
use framer_core::native_dispatch::*;
use framer_core::convert::{AssembledResponse, AssembledResponses};
use futures::FutureExt;
use serde_json::Value;

pub(crate) fn hold(inner: Box<dyn OwnedInvocation>, admission: PoolAdmission)
    -> Box<dyn OwnedInvocation> {
    Box::new(PoolCall { inner, admission })
}
struct PoolCall { inner: Box<dyn OwnedInvocation>, admission: PoolAdmission }

impl PoolCall {
    fn start(mut self: Box<Self>) -> (Box<dyn OwnedInvocation>, PoolAdmission) {
        self.admission.start();
        let Self { inner, admission } = *self;
        (inner, admission)
    }
}

impl OwnedInvocation for PoolCall {
    fn raw(self: Box<Self>, payload: Value) -> InvocationRun<RawResult> {
        let (inner, admission) = self.start();
        retain(inner.raw(payload), admission)
    }
    fn messages(self: Box<Self>, payload: Value) -> InvocationRun<Completed<AssembledResponse>> {
        let (inner, admission) = self.start();
        retain(inner.messages(payload), admission)
    }
    fn responses(self: Box<Self>, payload: Value) -> InvocationRun<Completed<AssembledResponses>> {
        let (inner, admission) = self.start();
        retain(inner.responses(payload), admission)
    }
    fn raw_stream(self: Box<Self>, payload: Value, limits: RawStreamLimits) -> RawStreamRun {
        let (inner, admission) = self.start();
        let run = inner.raw_stream(payload, limits);
        RawStreamRun { response: retain(run.response, admission), finished: run.finished }
    }
    fn composed_stream(self: Box<Self>, payload: Value, options: ComposedStreamOptions)
        -> ComposedStreamRun {
        let (inner, admission) = self.start();
        let run = inner.composed_stream(payload, options);
        ComposedStreamRun { response: retain(run.response, admission), finished: run.finished }
    }
    fn composed_stream_for_client(self: Box<Self>, payload: Value,
        options: ComposedStreamOptions, model: String) -> ComposedStreamRun {
        let (inner, admission) = self.start();
        let run = inner.composed_stream_for_client(payload, options, model);
        ComposedStreamRun { response: retain(run.response, admission), finished: run.finished }
    }
}

fn retain<T>(mut run: InvocationRun<T>, admission: PoolAdmission) -> InvocationRun<T> {
    let cleanup = run.cleanup.clone();
    let task = tokio::spawn(async move {
        cleanup.await?;
        admission.complete()
    });
    run.cleanup = async move { task.await.unwrap_or(Err(FailureKind::Cleanup)) }
        .boxed().shared();
    run
}

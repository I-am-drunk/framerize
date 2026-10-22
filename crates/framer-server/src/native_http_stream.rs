//! HTTP handoff for explicitly enabled native streams. No additional body queue.
use super::*;
use framer_core::native_dispatch::{Cancellation, Cleanup, ComposedStreamCompletion,
    ComposedStreamRun, InvocationProvider, OwnedInvocation, RawStreamBody,
    RawStreamCompletion, RawStreamHeaders, RawStreamRun};
use futures::{future::{BoxFuture, Shared}, FutureExt, Stream};
use std::{future::Future, pin::Pin, task::{Context, Poll}, time::Duration};
use tokio::sync::OwnedSemaphorePermit;

struct StreamingProvider {
    inner: Arc<dyn InvocationProvider>, limits: StreamingPolicy,
}

/// Caller-supplied opt-in, with no environment or startup default changes.
pub fn with_native_streaming(inner: Arc<dyn InvocationProvider>, limits: StreamingPolicy)
    -> Result<Arc<dyn InvocationProvider>, DispatchError> {
    limits.validate()?;
    Ok(Arc::new(StreamingProvider { inner, limits }))
}

impl InvocationProvider for StreamingProvider {
    fn policy(&self) -> Policy { self.inner.policy() }
    fn streaming_policy(&self) -> Option<StreamingPolicy> { Some(self.limits) }
    fn contains_account(&self, account: framer_core::ids::AccountId) -> bool {
        self.inner.contains_account(account)
    }
    fn supports_pool_admission(&self) -> bool { self.inner.supports_pool_admission() }
    fn prepare_in_pool(&self, request: Preparation, permit: OwnedSemaphorePermit,
        admission: framer_core::native_dispatch::PoolAdmission)
        -> BoxFuture<'_, Result<Box<dyn OwnedInvocation>, DispatchError>> {
        self.inner.prepare_in_pool(request, permit, admission)
    }
    fn prepare(&self, request: Preparation, permit: OwnedSemaphorePermit)
        -> BoxFuture<'_, Result<Box<dyn OwnedInvocation>, DispatchError>> {
        self.inner.prepare(request, permit)
    }
}

#[derive(Clone)]
pub enum NativeStreamCompletion {
    Raw(RawStreamCompletion), Composed(ComposedStreamCompletion),
}

impl NativeStreamCompletion {
    fn result(&self) -> Result<(), FailureKind> {
        match self { Self::Raw(value) => value.result, Self::Composed(value) => value.result }
    }
}

/// Application completion and real cleanup remain separate observations.
/// Byte counts in completion describe yielded bytes, not remote delivery.
#[derive(Clone)]
pub struct NativeStreamObservers {
    pub completion: Shared<BoxFuture<'static, NativeStreamCompletion>>,
    pub cleanup: Cleanup,
}

struct CancelOnDrop { cancellation: Arc<dyn Cancellation>, armed: bool }
impl CancelOnDrop {
    fn cancel(&mut self) {
        if self.armed { self.armed = false; self.cancellation.cancel(); }
    }
}
impl Drop for CancelOnDrop { fn drop(&mut self) { self.cancel(); } }

async fn head<T>(run: InvocationRun<T>,
    completion: Shared<BoxFuture<'static, NativeStreamCompletion>>, deadline: Instant)
    -> Result<(T, CancelOnDrop, NativeStreamObservers), DispatchError> {
    let InvocationRun { result, cleanup, cancellation, completed_attempts } = run;
    let guard = CancelOnDrop { cancellation, armed: true };
    let value = match tokio::time::timeout_at(deadline, result).await {
        Ok(value) => value?,
        Err(_) => {
            let mut error = DispatchError::new(FailureKind::Deadline);
            if let Some(snapshot) = completed_attempts { error.attempts = snapshot(); }
            return Err(error);
        }
    };
    Ok((value, guard, NativeStreamObservers { completion, cleanup }))
}

#[derive(Debug)]
struct HttpStreamError(FailureKind);
impl std::fmt::Display for HttpStreamError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "native stream {:?}", self.0)
    }
}
impl std::error::Error for HttpStreamError {}

struct HttpBody {
    source: RawStreamBody,
    guard: CancelOnDrop,
    observers: NativeStreamObservers,
    deadline: Pin<Box<tokio::time::Sleep>>,
    heartbeat: Option<(Duration, Pin<Box<tokio::time::Sleep>>)>,
    exhausted: bool,
    done: bool,
}

impl HttpBody {
    fn idle(&mut self, cx: &mut Context<'_>) -> Poll<Option<Result<axum::body::Bytes, HttpStreamError>>> {
        if let Some((period, timer)) = &mut self.heartbeat {
            if timer.as_mut().poll(cx).is_ready() {
                timer.as_mut().reset(Instant::now() + *period);
                return Poll::Ready(Some(Ok(axum::body::Bytes::from_static(b": ping\n\n"))));
            }
        }
        Poll::Pending
    }
    fn activity(&mut self) {
        if let Some((period, timer)) = &mut self.heartbeat {
            timer.as_mut().reset(Instant::now() + *period);
        }
    }
}

impl Stream for HttpBody {
    type Item = Result<axum::body::Bytes, HttpStreamError>;
    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        if this.done { return Poll::Ready(None); }
        if this.deadline.as_mut().poll(cx).is_ready() {
            this.done = true;
            this.guard.cancel();
            return Poll::Ready(Some(Err(HttpStreamError(FailureKind::Deadline))));
        }
        if !this.exhausted {
            match this.source.as_mut().poll_next(cx) {
                Poll::Pending => return this.idle(cx),
                Poll::Ready(Some(Ok(bytes))) => {
                    this.activity();
                    return Poll::Ready(Some(Ok(bytes)));
                }
                Poll::Ready(Some(Err(kind))) => {
                    this.done = true;
                    this.guard.cancel();
                    return Poll::Ready(Some(Err(HttpStreamError(kind))));
                }
                Poll::Ready(None) => this.exhausted = true,
            }
        }
        match Pin::new(&mut this.observers.completion).poll(cx) {
            Poll::Pending => this.idle(cx),
            Poll::Ready(result) => {
                this.done = true;
                match result.result() {
                    Ok(()) => { this.guard.armed = false; Poll::Ready(None) }
                    Err(kind) => {
                        this.guard.cancel();
                        Poll::Ready(Some(Err(HttpStreamError(kind))))
                    }
                }
            }
        }
    }
}

pub(super) async fn raw(run: RawStreamRun, deadline: Instant, session: &str, _heartbeat: Option<Duration>)
    -> Result<Response, DispatchError> {
    let completion = run.finished.map(NativeStreamCompletion::Raw).boxed().shared();
    let (head, guard, observers) = head(run.response, completion, deadline).await?;
    let content_type = head.headers.content_type.clone();
    // Raw chunks can end inside an SSE field, UTF-8 sequence, or CRLF pair.
    // A local comment inserted while that chunk is pending would change the
    // upstream bytes. Only the composed path owns complete SSE frames.
    response(head.body, head.headers, &content_type, guard, observers, deadline, session, None)
}

pub(super) async fn composed(run: ComposedStreamRun, deadline: Instant, session: &str, heartbeat: Option<Duration>)
    -> Result<Response, DispatchError> {
    let completion = run.finished.map(NativeStreamCompletion::Composed).boxed().shared();
    let (head, guard, observers) = head(run.response, completion, deadline).await?;
    response(head.body, head.first_attempt, head.content_type, guard, observers, deadline, session, heartbeat)
}

#[allow(clippy::too_many_arguments)]
fn response(source: RawStreamBody, headers: Arc<RawStreamHeaders>, content_type: &str,
    guard: CancelOnDrop, observers: NativeStreamObservers, deadline: Instant, session: &str,
    heartbeat: Option<Duration>)
    -> Result<Response, DispatchError> {
    let invalid = || DispatchError::new(FailureKind::UnsupportedResponse);
    let status = StatusCode::from_u16(headers.status).map_err(|_| invalid())?;
    let is_sse = content_type.split(';').next().unwrap_or("").trim()
        .eq_ignore_ascii_case("text/event-stream");
    let content_type = HeaderValue::from_str(content_type).map_err(|_| invalid())?;
    let session = HeaderValue::from_str(session)
        .map_err(|_| DispatchError::new(FailureKind::MissingContext))?;
    let usage = headers.usage_header.as_deref().map(HeaderValue::from_str)
        .transpose().map_err(|_| invalid())?;
    let body = HttpBody { source, guard, observers: observers.clone(),
        deadline: Box::pin(tokio::time::sleep_until(deadline)),
        heartbeat: heartbeat.filter(|_| is_sse).map(|period|
            (period, Box::pin(tokio::time::sleep(period)))), exhausted: false, done: false };
    let mut response = Response::new(axum::body::Body::from_stream(body));
    *response.status_mut() = status;
    response.headers_mut().insert("content-type", content_type);
    response.headers_mut().insert("x-framerize-session", session);
    response.headers_mut().insert("cache-control", HeaderValue::from_static("no-cache"));
    response.headers_mut().insert("x-accel-buffering", HeaderValue::from_static("no"));
    if let Some(usage) = usage { response.headers_mut().insert("x-framer-ai-usage", usage); }
    response.extensions_mut().insert(headers);
    response.extensions_mut().insert(observers);
    Ok(response)
}

#[cfg(test)]
#[path = "native_http_stream_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "native_http_stream_model_tests.rs"]
mod model_tests;

#[cfg(test)]
#[path = "native_context_stream_tests.rs"]
mod context_tests;

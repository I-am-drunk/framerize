//! Source-only native request/body closure acknowledgement.
//! Account clients may retain idle connections after body completion.
use super::{lock, NativeByteStream, NativeResponse, StreamingState};
use futures::channel::oneshot;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};

type ResponseFuture<'a> =
    Pin<Box<dyn Future<Output = Result<NativeResponse, String>> + Send + 'a>>;

pub struct NativeTrackedRequest<'a> {
    pub response: NativeResponseFuture<'a>,
    pub closed: NativeClosed,
}

pub struct NativeResponseFuture<'a> {
    inner: Option<ResponseFuture<'a>>,
    completion: Option<CompletionSignal>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NativeCloseError { AcknowledgementLost }

impl std::fmt::Display for NativeCloseError {
    fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        out.write_str("native-http: cleanup acknowledgement lost")
    }
}
impl std::error::Error for NativeCloseError {}

pub struct NativeClosed {
    receiver: oneshot::Receiver<()>,
    result: Option<Result<(), NativeCloseError>>,
}

impl Future for NativeClosed {
    type Output = Result<(), NativeCloseError>;
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        if let Some(result) = this.result { return Poll::Ready(result); }
        let result = match Pin::new(&mut this.receiver).poll(cx) {
            Poll::Pending => return Poll::Pending,
            Poll::Ready(Ok(())) => Ok(()),
            Poll::Ready(Err(_)) => Err(NativeCloseError::AcknowledgementLost),
        };
        this.result = Some(result);
        Poll::Ready(result)
    }
}

// Drop deliberately does not send success. A lost sender is an error.
pub(super) struct CompletionSignal(oneshot::Sender<()>);

impl CompletionSignal {
    fn complete(self) { let _ = self.0.send(()); }
}

fn pair() -> (CompletionSignal, NativeClosed) {
    let (sender, receiver) = oneshot::channel();
    (CompletionSignal(sender), NativeClosed { receiver, result: None })
}

pub(super) fn track<'a, F>(future: F) -> NativeTrackedRequest<'a>
where F: Future<Output = Result<NativeResponse, String>> + Send + 'a {
    let (signal, closed) = pair();
    NativeTrackedRequest {
        response: NativeResponseFuture {
            inner: Some(Box::pin(future)), completion: Some(signal),
        },
        closed,
    }
}

impl Future for NativeResponseFuture<'_> {
    type Output = Result<NativeResponse, String>;
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        let result = match this.inner.as_mut().expect("response future already completed")
            .as_mut().poll(cx)
        {
            Poll::Pending => return Poll::Pending,
            Poll::Ready(result) => result,
        };
        // Take the signal FIRST. If a destructor panics, it is dropped without
        // a success acknowledgement instead of being sent by our later Drop.
        let signal = this.completion.take().expect("missing completion owner");
        drop(this.inner.take());
        match result {
            Ok(response) => match attach(&response.body, signal) {
                Ok(()) => Poll::Ready(Ok(response)),
                Err(signal) => {
                    drop(response);
                    drop(signal);
                    Poll::Ready(Err("native-http: closure observer already attached".into()))
                }
            },
            Err(error) => {
                signal.complete();
                Poll::Ready(Err(error))
            }
        }
    }
}

impl Drop for NativeResponseFuture<'_> {
    fn drop(&mut self) {
        let signal = self.completion.take();
        // Drop the actual future, including its captured resources, before
        // waking any completion observer. Cancel invocation is not cleanup.
        drop(self.inner.take());
        notify(signal);
    }
}

pub(super) fn notify(signal: Option<CompletionSignal>) {
    if let Some(signal) = signal { signal.complete(); }
}

fn attach(body: &NativeByteStream, signal: CompletionSignal)
    -> Result<(), CompletionSignal>
{
    let ready = {
        let mut state = lock(&body.state);
        if state.closure.is_some() { return Err(signal); }
        state.closure = Some(signal);
        state.take_closed_signal()
    };
    // A cancellation may have finished the body before this transfer. It
    // must neither strand the new signal nor wake while holding the mutex.
    notify(ready);
    Ok(())
}

impl StreamingState {
    pub(super) fn take_closed_signal(&mut self) -> Option<CompletionSignal> {
        if self.finished && self.body.is_none() && self.pending.is_empty()
            && self.permit.is_none()
        {
            self.closure.take()
        } else { None }
    }
}

pub(super) fn finish_and_notify(
    state: &Arc<Mutex<StreamingState>>, error: Option<&'static str>,
) {
    let signal = {
        let mut state = lock(state);
        state.finish(error);
        state.take_closed_signal()
    };
    notify(signal);
}

#[cfg(test)]
pub fn track_for_test<'a, F>(future: F) -> NativeTrackedRequest<'a>
where F: Future<Output = Result<NativeResponse, String>> + Send + 'a {
    track(future)
}

#[cfg(test)]
pub async fn cancelled_for_test(cancellation: super::NativeCancellation) {
    cancellation.cancelled().await;
}

#[cfg(test)]
pub fn lost_signal_for_test() -> NativeClosed {
    let (signal, closed) = pair();
    drop(signal);
    closed
}

#[cfg(test)]
#[derive(Clone)]
pub struct TestStateProbe(std::sync::Weak<Mutex<StreamingState>>);

#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TestStateSnapshot {
    pub present: bool,
    pub unlocked: bool,
    pub resources_released: bool,
}

#[cfg(test)]
pub fn observe_for_test(body: &NativeByteStream) -> TestStateProbe {
    TestStateProbe(Arc::downgrade(&body.state))
}

#[cfg(test)]
impl TestStateProbe {
    pub fn snapshot(&self) -> TestStateSnapshot {
        let Some(state) = self.0.upgrade() else {
            return TestStateSnapshot {
                present: false, unlocked: true, resources_released: true,
            };
        };
        let result = match state.try_lock() {
            Ok(state) => TestStateSnapshot {
                present: true, unlocked: true,
                resources_released: state.finished && state.body.is_none()
                    && state.pending.is_empty() && state.permit.is_none(),
            },
            Err(_) => TestStateSnapshot {
                present: true, unlocked: false, resources_released: false,
            },
        };
        result
    }
}
